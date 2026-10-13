//! Local release 24 MP authoritative Gaussian measurements; no GUI or corpus assets.
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_young, try_gaussian_young_recursive};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let recursive = std::env::args().any(|v| v == "--recursive");
    let extent = Rect::new(0, 0, 6000, 4000);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut raw = Vec::new();
    raw.try_reserve_exact(24_000_000 * 8)?;
    for i in 0..24_000_000_u32 {
        raw.extend_from_slice(&(f64::from(i % 1024) / 1023.0).to_ne_bytes());
    }
    let source = Surface::try_from_interleaved(format, extent, &raw)?;
    drop(raw);
    for sigma in [2.25, 3.75] {
        let start = Instant::now();
        let evaluate = if recursive { try_gaussian_young_recursive } else { try_gaussian_young };
        let result = evaluate(
            &source,
            extent,
            extent,
            GaussianParams { std_dev_x: sigma, std_dev_y: 1.5, abyss: GaussianAbyss::Clamp, clip_extent: true },
            GaussianLimits::default(),
            &Interrupt::NONE,
        )?;
        println!(
            "24 MP Gray F64 Young Gaussian ({sigma},1.5), recursive={recursive}: {:.3} ms, {} output tiles",
            start.elapsed().as_secs_f64() * 1000.0,
            result.tile_count()
        );
    }
    Ok(())
}
