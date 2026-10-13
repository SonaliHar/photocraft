//! Original 24 MP native F64 gradient measurement, no user or corpus assets.
use photocraft_algo::{
    paint::GradientShape,
    wide_paint::{GradientOptions, NativeStop, PaintLimits, try_paint_gradient},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let mut target = Surface::try_with_default_bytes(format, &[0.125_f64.to_ne_bytes(), 1.0_f64.to_ne_bytes()].concat())?;
    let stops = [NativeStop { position: 0.0, color: [0.25 + 2.0_f64.powi(-40), 1.0, 0.0, 0.0] }, NativeStop { position: 1.0, color: [1.125, 1.0, 0.0, 0.0] }];
    let options = GradientOptions { from: (0.0, 0.0), to: (6000.0, 0.0), shape: GradientShape::Linear, reverse: false, opacity: 0.5, lock_transparency: false };
    let start = Instant::now();
    try_paint_gradient(&mut target, Rect::new(0, 0, 6000, 4000), options, &stops, None, PaintLimits::default(), &Interrupt::NONE)?;
    println!("24 MP native Gray+Alpha F64 checked gradient: {:.3} ms, {} tiles", start.elapsed().as_secs_f64() * 1000.0, target.tile_count());
    Ok(())
}
