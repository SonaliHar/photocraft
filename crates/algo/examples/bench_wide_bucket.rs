//! Original24MP default-plane exact native bucket timing; no external image assets.
use photocraft_algo::wide_paint::{BucketOptions, try_paint_bucket_exact};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let default = [0.5_f64, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let mut surface = Surface::try_with_default_bytes(format, &default)?;
    let start = std::time::Instant::now();
    let filled = try_paint_bucket_exact(
        &mut surface,
        Rect::new(0, 0, 6000, 4000),
        BucketOptions { seed: (0, 0), contiguous: true, opacity: 1.0, lock_transparency: false },
        &[0.0, 1.0],
        None,
        Default::default(),
        &Interrupt::NONE,
    )?;
    println!("24MP GrayAlphaF64 uniform-default exact bucket: {:.3}ms; {filled}; {} tiles", start.elapsed().as_secs_f64() * 1000.0, surface.tile_count());
    Ok(())
}
