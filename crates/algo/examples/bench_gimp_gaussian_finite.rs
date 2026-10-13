//! Checked finite Gaussian 24 MP resource and deadline measurement.
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianFilter, GaussianLimits, GaussianParams, try_gaussian_finite_clamp};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let area = Rect::new(0, 0, 6000, 4000);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(24_000_000 * 8)?;
    for i in 0..24_000_000u32 {
        bytes.extend_from_slice(&(f64::from(i % 1024) / 1023.).to_ne_bytes());
    }
    let source = Surface::try_from_interleaved(format, area, &bytes)?;
    drop(bytes);
    let params = GaussianParams { std_dev_x: 2.25, std_dev_y: 3.75, abyss: GaussianAbyss::Clamp, clip_extent: true };
    println!("source retained {} bytes", source.resident_payload_bytes()?);
    let start = Instant::now();
    let output = try_gaussian_finite_clamp(&source, area, area, params, GaussianFilter::Auto, GaussianLimits::default(), &Interrupt::NONE)?;
    println!(
        "24MP GrayF64 finite Auto 2.25x3.75: {:.3}ms; {} retained bytes; {} tiles",
        start.elapsed().as_secs_f64() * 1000.,
        output.resident_payload_bytes()?,
        output.tile_count()
    );
    drop(output);
    let start = Instant::now();
    let cancel = || start.elapsed() >= Duration::from_millis(50);
    let interrupt = Interrupt::new(&cancel, &|_| {});
    let cancelled = try_gaussian_finite_clamp(&source, area, area, params, GaussianFilter::Auto, GaussianLimits::default(), &interrupt);
    println!("deadline50ms result {:?} elapsed {:.3}ms", cancelled.as_ref().err(), start.elapsed().as_secs_f64() * 1000.);
    let start = Instant::now();
    let rejected = try_gaussian_finite_clamp(
        &source,
        area,
        area,
        params,
        GaussianFilter::Auto,
        GaussianLimits { resident_bytes: 64 * 1024 * 1024, work_units: u64::MAX },
        &Interrupt::NONE,
    );
    println!("64MiB cap result {:?} elapsed {:.3}ms", rejected.as_ref().err(), start.elapsed().as_secs_f64() * 1000.);
    Ok(())
}
