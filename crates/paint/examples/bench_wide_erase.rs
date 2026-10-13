//! Release alpha-erase measurement on an original uniform 24 MP Gray+Alpha F64 image.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::wide_erase::{EraseColorPolicy, EraseLimits, try_erase_coverage};
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw = [(1.0 + 2.0_f64.powi(-40)).to_ne_bytes(), 1.0_f64.to_ne_bytes()].concat();
    let mut target = Surface::try_with_default_bytes(format, &raw)?;
    let mut coverage = Vec::new();
    coverage.try_reserve_exact(24_000_000)?;
    coverage.resize(24_000_000, 0.5);
    let start = Instant::now();
    try_erase_coverage(
        &mut target,
        Rect::new(0, 0, 6000, 4000),
        &coverage,
        0.5,
        None,
        false,
        EraseColorPolicy::Preserve,
        EraseLimits::default(),
        &Interrupt::NONE,
    )?;
    println!("24 MP uniform Gray+Alpha F64 checked erase: {:.3} ms, {} tiles", start.elapsed().as_secs_f64() * 1000.0, target.tile_count());
    Ok(())
}
