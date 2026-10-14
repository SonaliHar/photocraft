//! End-to-end CR3 native/preview/metadata inspection using PhotoCraft's import path.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for filename in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(filename);
        let bytes = std::fs::read(&path)?;
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("camera.CR3");
        let started = std::time::Instant::now();
        let result = photocraft_io::import(name, &bytes)?;
        let elapsed = started.elapsed();
        println!(
            "{}: {}x{} {:?}, EXIF {}, XMP {}, import {elapsed:?}",
            path.display(),
            result.document.size.width,
            result.document.size.height,
            result.document.depth,
            result.document.metadata.exif.as_ref().map_or(0, |v| v.len()),
            result.document.metadata.xmp.as_ref().map_or(0, |v| v.len())
        );
        for warning in result.warnings {
            println!("  {warning}");
        }
    }
    Ok(())
}
