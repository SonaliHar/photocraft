//! Read-only validation of a public camera RAW and its host-unpacked sensor DNG.
use photocraft_doc::{LayerContent, RawDevelopment, SmartSource};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let original_path = args.get(1).ok_or("usage: raw_adapter_check CAMERA.NEF SENSOR.dng OUTPUT.png")?;
    let sensor_path = args.get(2).ok_or("missing sensor path")?;
    let output = args.get(3).ok_or("missing output path")?;
    let original = std::fs::read(original_path)?;
    let sensor = std::fs::read(sensor_path)?;
    let decoded = photocraft_raw::decode(&sensor, &Default::default())?;
    let calibrated = !decoded.color.calibrations.is_empty();
    if !calibrated || decoded.cfa.is_none() {
        return Err("sensor adapter lost calibration or CFA".into());
    }
    let crop = decoded.crop;
    drop(decoded);
    let start = Instant::now();
    let imported = photocraft_io::raw_smart::import_with_sensor("camera.NEF", &original, Some(("sensor.dng", &sensor)))?;
    let open_ms = start.elapsed().as_millis();
    let encoded = photocraft_format::save_to_bytes(&imported.document, &Default::default())?;
    let project_bytes = encoded.len();
    drop(imported);
    let restored = photocraft_format::load_from_bytes(&encoded)?;
    let layer = restored.layers.first().ok_or("missing layer")?;
    let LayerContent::Smart(sm) = &layer.content else {
        return Err("not RAW smart".into());
    };
    let SmartSource::Embedded { bytes, .. } = &sm.source else {
        return Err("missing original".into());
    };
    if bytes.as_slice() != original {
        return Err("original changed".into());
    }
    let Some(SmartSource::Embedded { bytes, .. }) = &sm.raw_decode_source else {
        return Err("missing sensor".into());
    };
    if bytes.as_slice() != sensor {
        return Err("sensor changed".into());
    }
    let image = photocraft_io::export(&restored, output, &Default::default())?;
    std::fs::write(output, image.bytes)?;
    drop(restored);
    drop(encoded);
    let start = Instant::now();
    let changed = photocraft_io::raw_smart::develop("sensor.dng", &sensor, &RawDevelopment { exposure: -1.0, ..Default::default() })?;
    let develop_ms = start.elapsed().as_millis();
    println!(
        "{}",
        serde_json::json!({"openMs":open_ms,"redevelopMs":develop_ms,"width":changed.document.size.width,
        "height":changed.document.size.height,"crop":[crop.x,crop.y,crop.width,crop.height],"projectBytes":project_bytes,
        "originalRetained":true,"sensorRetained":true,"calibrated":calibrated,"warnings":changed.warnings})
    );
    Ok(())
}
