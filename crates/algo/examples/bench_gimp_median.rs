//! Local 24MP checked f64 Median performance, resource-cap and cancellation.
use photocraft_algo::gimp::neighborhood::{GaussianLimits, MedianNeighborhood, MedianParams, try_median_blur};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let extent = Rect::new(0, 0, 6000, 4000);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut raw = Vec::new();
    raw.try_reserve_exact(24_000_000 * 8)?;
    for i in 0..24_000_000u32 {
        raw.extend_from_slice(&(f64::from(i % 1024) / 1023.).to_ne_bytes());
    }
    let source = Surface::try_from_interleaved(format, extent, &raw)?;
    drop(raw);
    {
        let radius = 3;
        let start = Instant::now();
        let result = try_median_blur(
            &source,
            extent,
            extent,
            MedianParams { radius, neighborhood: MedianNeighborhood::Circle, percentile: 50., alpha_percentile: 50., high_precision: true },
            Default::default(),
            &Interrupt::NONE,
        )?;
        println!(
            "24MPGrayF64 Median{radius}: {:.3}ms, {} bytes, {} tiles",
            start.elapsed().as_secs_f64() * 1000.,
            result.resident_payload_bytes()?,
            result.tile_count()
        );
    }
    let start = Instant::now();
    let cancel = || start.elapsed() >= Duration::from_millis(50);
    let error = try_median_blur(
        &source,
        extent,
        extent,
        MedianParams { radius: 3, neighborhood: MedianNeighborhood::Circle, percentile: 50., alpha_percentile: 50., high_precision: true },
        Default::default(),
        &Interrupt::new(&cancel, &|_| {}),
    );
    println!("deadline50ms {:?}: {:.3}ms", error.as_ref().err(), start.elapsed().as_secs_f64() * 1000.);
    let start = Instant::now();
    let error = try_median_blur(
        &source,
        extent,
        extent,
        MedianParams { radius: 3, neighborhood: MedianNeighborhood::Circle, percentile: 50., alpha_percentile: 50., high_precision: true },
        GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: u64::MAX },
        &Interrupt::NONE,
    );
    println!("64MiB cap {:?}: {:.3}ms", error.as_ref().err(), start.elapsed().as_secs_f64() * 1000.);
    Ok(())
}
