//! Synthetic RAW fixture and sensor-development timing; no personal camera files.
use photocraft_doc::RawDevelopment;
use photocraft_raw::testgen::{DngSpec, mosaic, scene};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = args.get(1).ok_or("usage: raw_smart_fixture OUTPUT.dng [WIDTH HEIGHT]")?;
    let w: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(32);
    let h: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(24);
    if w < 16 || h < 16 || w.checked_mul(h).is_none_or(|n| n > 36_000_000) {
        return Err("fixture must be 16x16 to 36 megapixels".into());
    }
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let bytes = spec.build();
    std::fs::write(path, &bytes)?;
    let start = Instant::now();
    let ordinary = photocraft_io::raw::import_raw("fixture.dng", &bytes)?;
    let baseline = start.elapsed().as_millis();
    drop(ordinary);
    let start = Instant::now();
    let smart = photocraft_io::raw_smart::import("fixture.dng", &bytes)?;
    let cold = start.elapsed().as_millis();
    drop(smart);
    let start = Instant::now();
    let changed = photocraft_io::raw_smart::develop("fixture.dng", &bytes, &RawDevelopment { exposure: -1.0, ..Default::default() })?;
    let warm = start.elapsed().as_millis();
    drop(changed);
    println!("{}", serde_json::json!({"width":w,"height":h,"ordinaryDevelopMs":baseline,"rawSmartColdMs":cold,"cachedSensorDevelopMs":warm}));
    Ok(())
}
