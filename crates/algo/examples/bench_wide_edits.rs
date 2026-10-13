//! Release measurement of authoritative 24 MP U32 edits; no GUI or corpus assets.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bounds = Rect::new(0, 0, 6000, 4000);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let mut raw = Vec::new();
    raw.try_reserve_exact(24_000_000 * 4)?;
    for index in 0..24_000_000_u32 {
        raw.extend_from_slice(&index.wrapping_mul(271).to_ne_bytes());
    }
    let source = Surface::try_from_interleaved(format, bounds, &raw)?;
    drop(raw);
    let start = Instant::now();
    let inverted = photocraft_algo::wide::try_invert(&source, bounds, None, &Interrupt::NONE)?;
    println!("24 MP Gray U32 checked invert: {:.3} ms, {} result tiles", start.elapsed().as_secs_f64() * 1000.0, inverted.tile_count());
    drop(inverted);
    let start = Instant::now();
    let translated = photocraft_algo::wide::try_translate(&source, bounds, 17, -9, &Interrupt::NONE)?;
    println!("24 MP Gray U32 checked raw translate: {:.3} ms, {} result tiles", start.elapsed().as_secs_f64() * 1000.0, translated.tile_count());
    Ok(())
}
