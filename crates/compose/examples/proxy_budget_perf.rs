//! Scoped release measurement: original24MP ordinary proxy preview, no GUI/assets.
use photocraft_compose::{preview_budget::PreviewPreparationBudget, proxy_preview::thumbnail_with_proxy_preparation_budget, thumbnail};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_raster::{Interrupt, RasterInputError};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("run with --release".into());
    }
    let mut doc = Document::with_background(
        "Own charged proxy Ω".repeat(200),
        Size::new(6000, 4000),
        ColorMode::Rgb,
        SampleType::U16,
        Color::rgba(0.21, 0.63, 0.17, 0.71),
    );
    doc.layers[0].name = "Own layer λ".repeat(1000);
    doc.layers[0].surface_mut().ok_or("missing own raster")?.fill_rect(photocraft_geom::Rect::new(-7, 3, 1612, 1124), &[0.73, 0.18, 0.51, 0.0]);
    doc.metadata.xmp = Some("Own inert metadata Ω".repeat(100000));
    let before = doc.clone();
    // Optional bounded diagnostics describe reservation overlap only, not RSS.
    let diagnostic = std::env::args().any(|a| a == "--diagnose");
    let events = Mutex::new(VecDeque::with_capacity(16));
    let observer = |event| {
        if diagnostic {
            let mut events = events.lock().unwrap_or_else(|e| e.into_inner());
            if events.len() == 16 {
                events.pop_front();
            }
            events.push_back(event);
        }
    };
    let cap_mib = std::env::args().find_map(|a| a.strip_prefix("--budget-mib=").and_then(|v| v.parse::<usize>().ok())).unwrap_or(64);
    let budget = PreviewPreparationBudget::new(cap_mib.checked_mul(1 << 20).ok_or("budget overflow")?, &observer);
    let started = Instant::now();
    let first = thumbnail_with_proxy_preparation_budget(&doc, 256, &Interrupt::NONE, &budget).map_err(|e| format!("256px: {e}; usage={:?}", budget.usage()))?;
    let second = match thumbnail_with_proxy_preparation_budget(&doc, 1024, &Interrupt::NONE, &budget) {
        Ok(image) => image,
        Err(RasterInputError::LimitExceeded) => {
            let elapsed = started.elapsed().as_secs_f64() * 1000.;
            let peak = budget.usage().peak_bytes;
            if first.image() != &thumbnail(&doc, 256)? || doc != before {
                return Err("denied proxy/source parity failure".into());
            }
            drop(first);
            if budget.usage().live_bytes != 0 {
                return Err("denied proxy lease imbalance".into());
            }
            println!(
                "24MP proxy U16 cap_mib={cap_mib} 1024px_denied=LimitExceeded elapsed_ms={elapsed:.3} conservative_reservation_peak={peak} first_byte_exact=true source_exact=true final_live=0 tail={:?}",
                events.lock().unwrap_or_else(|e| e.into_inner())
            );
            return Ok(());
        }
        Err(e) => return Err(format!("1024px: {e}; usage={:?}", budget.usage()).into()),
    };
    let elapsed = started.elapsed().as_secs_f64() * 1000.;
    if first.image() != &thumbnail(&doc, 256)? || second.image() != &thumbnail(&doc, 1024)? || doc != before {
        return Err("proxy/source parity failure".into());
    }
    let peak = budget.usage().peak_bytes;
    let outputs = budget.usage().live_bytes;
    drop(first);
    drop(second);
    if budget.usage().live_bytes != 0 {
        return Err("proxy lease imbalance".into());
    }
    let started = Instant::now();
    let cancel = || started.elapsed() >= Duration::from_millis(20);
    if !matches!(thumbnail_with_proxy_preparation_budget(&doc, 1024, &Interrupt::cancel_only(&cancel), &budget), Err(RasterInputError::Cancelled))
        || budget.usage().live_bytes != 0
        || doc != before
    {
        return Err("proxy cancellation/source failure".into());
    }
    let stop = started.elapsed().as_secs_f64() * 1000.;
    println!(
        "24MP proxy U16 cap_mib={cap_mib} pair_ms={elapsed:.3} conservative_reservation_peak={peak} concurrent_output_bytes={outputs} final_live=0 byte_exact=true cancel_deadline_ms=20 stop_total_ms={stop:.3} deadline_to_stop_ms={:.3}",
        (stop - 20.).max(0.)
    );
    Ok(())
}
