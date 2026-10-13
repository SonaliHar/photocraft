//! Scoped release measurement of the checked 24MP normalized GrayF64 path.
use photocraft_algo::{
    transform::{Homography, try_warp_bilinear_hidden_surface_with_limits},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("run with --release".into());
    }
    let source = Surface::try_with_default_bytes(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
        &[0.25f64, 2f64.powi(-20)].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )?;
    let bounds = Rect::new(0, 0, 6000, 4000);
    let h = Homography([0.9, 0.05, 0.25, -0.025, 0.9, 0.5, 0.00001, 0.00001, 1.]);
    let started = std::time::Instant::now();
    let output = try_warp_bilinear_hidden_surface_with_limits(&source, bounds, &h, GeometryLimits::default(), &Interrupt::NONE)?;
    println!(
        "24MP GrayF64 projective associated Linear {:.3}ms, {} tiles, {} resident bytes",
        started.elapsed().as_secs_f64() * 1000.,
        output.tile_count(),
        output.resident_payload_bytes()?
    );
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 32;
    let started = std::time::Instant::now();
    let result = try_warp_bilinear_hidden_surface_with_limits(&source, bounds, &h, GeometryLimits::default(), &Interrupt::cancel_only(&cancel));
    if result != Err(photocraft_raster::RasterInputError::Cancelled) {
        return Err("cancellation failed".into());
    }
    println!("cancel after32 polls {:.3}ms; source tiles {}", started.elapsed().as_secs_f64() * 1000., source.tile_count());
    Ok(())
}
