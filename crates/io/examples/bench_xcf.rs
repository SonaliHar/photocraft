//! Release-mode real-file benchmark. Paths are development inputs, never runtime converters.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path)?;
        let (result, stats) = photocraft_io::xcf_import::import_with_stats(&path, &bytes, &photocraft_raster::Interrupt::NONE)?;
        let doc = &result.document;
        let before = std::time::Instant::now();
        let mut checksum = 0f64;
        photocraft_compose::render_bands(doc, doc.bounds(), 256, |band| -> Result<(), std::convert::Infallible> {
            checksum += band.px.iter().map(|p| f64::from(p.first().copied().unwrap_or(0.0))).sum::<f64>();
            Ok(())
        })?;
        println!(
            "{} {}x{} {:?} decode_ms={:.3} map_ms={:.3} composite_ms={:.3} peak_charged_mib={:.3} checksum={:.3}",
            path,
            doc.size.width,
            doc.size.height,
            doc.depth,
            stats.decode.as_secs_f64() * 1000.0,
            stats.mapping.as_secs_f64() * 1000.0,
            before.elapsed().as_secs_f64() * 1000.0,
            stats.peak_charged_bytes as f64 / (1 << 20) as f64,
            checksum
        );
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
