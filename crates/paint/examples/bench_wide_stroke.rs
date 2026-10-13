//! Original 24 MP wide eraser recomposition; no corpus or user image.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::{BrushSettings, StrokePoint, StrokeRenderer};
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw = [(0.5 + 2.0_f64.powi(-40)).to_ne_bytes(), 1.0_f64.to_ne_bytes()].concat();
    let pre = Surface::try_with_default_bytes(format, &raw)?;
    let mut target = pre.clone();
    let brush = BrushSettings { size: 6000.0, aliased: true, hardness: 1.0, flow: 1.0, opacity: 0.5, erase: true, ..Default::default() };
    let start = Instant::now();
    let mut renderer = StrokeRenderer::new(&brush, Some(format), 1.0);
    renderer.push(&[StrokePoint::new(3000.0, 2000.0, 1.0)]);
    renderer.finish();
    let geometry = start.elapsed();
    let start = Instant::now();
    renderer.try_composite_wide(&pre, &mut target, &pre, None, false, Some(Rect::new(0, 0, 6000, 4000)), &Interrupt::NONE)?;
    println!(
        "24 MP Gray+Alpha F64 stroke: geometry {:.3} ms, checked recompose {:.3} ms, {} result tiles",
        geometry.as_secs_f64() * 1000.0,
        start.elapsed().as_secs_f64() * 1000.0,
        target.tile_count()
    );
    Ok(())
}
