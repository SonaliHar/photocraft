//! Local release measurements on an original 24 MP Gray F64 image, no GUI.
use photocraft_algo::{
    resample::Resample,
    wide_geometry::{GeometryLimits, OrthogonalTransform, try_transform_orthogonal},
    wide_resample::{ResizeOptions, try_resize},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let bounds = Rect::new(0, 0, 6000, 4000);
    let mut raw = Vec::new();
    raw.try_reserve_exact(24_000_000 * 8)?;
    for i in 0..24_000_000_u32 {
        raw.extend_from_slice(&(f64::from(i % 1024) / 1023.0 + 2.0_f64.powi(-40)).to_ne_bytes());
    }
    let source = Surface::try_from_interleaved(format, bounds, &raw)?;
    drop(raw);
    let start = Instant::now();
    let (flipped, area) = try_transform_orthogonal(&source, bounds, (0, 0), OrthogonalTransform::FlipHorizontal, GeometryLimits::default(), &Interrupt::NONE)?;
    println!("24 MP Gray F64 raw horizontal flip: {:.3} ms, {}x{} output", start.elapsed().as_secs_f64() * 1000.0, area.width(), area.height());
    drop(flipped);
    let start = Instant::now();
    let (resized, area) = try_resize(
        &source,
        bounds,
        ResizeOptions { scale_x: 0.5, scale_y: 0.5, filter: Resample::Bilinear, clamp_edges: true },
        GeometryLimits::default(),
        &Interrupt::NONE,
    )?;
    println!("24 MP Gray F64 -> 6 MP checked bilinear resize: {:.3} ms, {} tiles", start.elapsed().as_secs_f64() * 1000.0, resized.tile_count());
    println!("Output bounds: {}x{}", area.width(), area.height());
    Ok(())
}
