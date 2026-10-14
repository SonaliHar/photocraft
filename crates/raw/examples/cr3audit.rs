//! Read-only CR3 coverage audit: descriptors, crop, sample hash and support status.
use photocraft_raw::{Limits, RawError, cr3, decode, embedded_preview};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("id\tsensor_w\tsensor_h\tversion\tlevels\tbits\tpreview\tpreview_w\tpreview_h\tcrop_w\tcrop_h\tstatus\tfnv\treason");
    for arg in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(arg);
        let bytes = std::fs::read(&path)?;
        let id = path.file_stem().and_then(|v| v.to_str()).ok_or("file id")?;
        let track = cr3::sensor_track(&bytes, &Limits::default())?;
        let (preview, pw, ph) = if let Some(p) = embedded_preview(&bytes) {
            ("jpeg", p.width, p.height)
        } else if let Some(p) = cr3::hevc_previews(&bytes).iter().max_by_key(|p| u64::from(p.width) * u64::from(p.height)) {
            ("hevc", p.width, p.height)
        } else {
            ("none", 0, 0)
        };
        print!("{id}\t{}\t{}\t{:x}\t{}\t{}\t{preview}\t{pw}\t{ph}\t", track.width, track.height, track.version, track.wavelet_levels, track.bits);
        match decode(&bytes, &Limits::default()) {
            Ok(sensor) => {
                let hash = sensor.data.iter().flat_map(|v| v.to_le_bytes()).fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
                println!("{}\t{}\tnative\t{hash:016x}\t", sensor.crop.width, sensor.crop.height);
            }
            Err(e) => {
                let status = if matches!(e, RawError::Unsupported(_)) { "preview" } else { "error" };
                println!("0\t0\t{status}\t-\t{}", e.to_string().replace(['\t', '\n', '\r'], " "));
            }
        }
    }
    Ok(())
}
