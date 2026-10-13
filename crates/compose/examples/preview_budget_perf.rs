//! Release-only original 8MP plain-raster preview/accounting measurement.
use photocraft_compose::{
    preview_budget::{PreviewPreparationBudget, thumbnail_with_preparation_budget},
    thumbnail,
};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_raster::{Interrupt, RasterInputError};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("run with --release".into());
    }
    let mut doc =
        Document::with_background("Own 8MP plain preview", Size::new(4000, 2000), ColorMode::Rgb, SampleType::U16, Color::rgba(0.21, 0.73, 0.41, 0.61));
    let surface = doc.layers[0].surface_mut().ok_or("missing own raster")?;
    surface.fill_rect(photocraft_geom::Rect::new(100, 250, 3000, 1600), &[0.63, 0.11, 0.81, 0.47]);
    surface.fill_rect(photocraft_geom::Rect::new(600, 800, 1300, 1600), &[0.82, 0.42, 0.13, 0.0]);
    let before = doc.clone();
    let reserves = AtomicUsize::new(0);
    let releases = AtomicUsize::new(0);
    let observe = |event| match event {
        photocraft_compose::preview_budget::PreparationCharge::Reserve(_) => {
            reserves.fetch_add(1, Ordering::Relaxed);
        }
        photocraft_compose::preview_budget::PreparationCharge::Release(_) => {
            releases.fetch_add(1, Ordering::Relaxed);
        }
    };
    let budget = PreviewPreparationBudget::new(64 << 20, &observe);
    let started = Instant::now();
    let small = thumbnail_with_preparation_budget(&doc, 256, &Interrupt::NONE, &budget)?;
    let large = thumbnail_with_preparation_budget(&doc, 1024, &Interrupt::NONE, &budget)?;
    let elapsed = started.elapsed().as_secs_f64() * 1000.;
    if small.image() != &thumbnail(&doc, 256)? || large.image() != &thumbnail(&doc, 1024)? || doc != before {
        return Err("preview/source parity failure".into());
    }
    let peak = budget.usage().peak_bytes;
    let retained = budget.usage().live_bytes;
    drop(small);
    drop(large);
    if budget.usage().live_bytes != 0 || reserves.load(Ordering::Relaxed) != releases.load(Ordering::Relaxed) {
        return Err("lease imbalance".into());
    }
    let started = Instant::now();
    let cancel = || started.elapsed() >= Duration::from_millis(20);
    if !matches!(thumbnail_with_preparation_budget(&doc, 1024, &Interrupt::cancel_only(&cancel), &budget), Err(RasterInputError::Cancelled))
        || doc != before
        || budget.usage().live_bytes != 0
    {
        return Err("cancellation/lease/source failure".into());
    }
    let stop = started.elapsed().as_secs_f64() * 1000.;
    println!(
        "8MP plain U16 pair_ms={elapsed:.3} known_payload_peak={peak} retained_output_bytes={retained} final_live=0 byte_exact=true cancel_deadline_ms=20 stop_total_ms={stop:.3} deadline_to_stop_ms={:.3}",
        (stop - 20.).max(0.)
    );
    Ok(())
}
