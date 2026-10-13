//! Run with --release. Original synthetic 24 MP input; no GUI or corpus dependency.
use photocraft_compose::{thumbnail, thumbnail_with_interrupt};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_raster::{Interrupt, RasterInputError};
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        return Err("run with --release".into());
    }
    let mut doc = Document::with_background("own 24 MP preview", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U16, Color::rgba(0.2, 0.6, 0.8, 1.0));
    let shadow_case = std::env::args().any(|arg| arg == "--shadow");
    if shadow_case {
        use photocraft_doc::{Contour, Effect, FxCommon, Layer, LayerContent, Rect, Shadow};
        let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA16);
        surface.fill_rect(Rect::new(100, 100, 612, 484), &[0.73, 0.21, 0.39, 0.63]);
        let mut layer = Layer::new("own bounded shadow", LayerContent::Raster(surface));
        layer.effects.items.push(Effect::DropShadow(Shadow {
            common: FxCommon::new(photocraft_color::BlendMode::Multiply, 0.61),
            color: Color::BLACK,
            angle: 37.0,
            use_global_light: false,
            distance: 3.4,
            spread: 0.0,
            size: 17.25,
            contour: Contour::Linear,
            anti_alias: true,
            noise: 0.0,
            knocks_out: true,
        }));
        doc.layers.push(layer);
    }
    let feather_case = std::env::args().any(|arg| arg == "--feather");
    let vector_case = std::env::args().any(|arg| arg == "--vector");
    if vector_case {
        use photocraft_doc::VectorMask;
        let mut path = if feather_case {
            photocraft_vector::shapes::ellipse(300.25, 150.75, 1700.5, 1100.25)
        } else {
            photocraft_vector::shapes::ellipse(300.25, 150.75, 4700.5, 3100.25)
        };
        path.inverted = true;
        let mut mask = VectorMask::new(path);
        mask.density = 0.73;
        doc.layers[0].vector_mask = Some(mask);
    }
    if feather_case {
        let mut mask = photocraft_doc::LayerMask::hide_all();
        std::sync::Arc::make_mut(&mut mask.surface).fill_rect(photocraft_geom::Rect::new(400, 500, 1700, 1400), &[0.73]);
        mask.feather = 16.25;
        mask.density = 0.87;
        doc.layers[0].mask = Some(mask);
        if let Some(vector) = doc.layers[0].vector_mask.as_mut() {
            vector.feather = 7.25;
        }
    }
    let before = doc.clone();
    if std::env::args().any(|arg| arg == "--budget") {
        use photocraft_compose::{preview_budget::PreviewPreparationBudget, styled_preview::thumbnail_with_styled_preparation_budget};
        let observer = |_| {};
        let cap_mib = std::env::args().find_map(|a| a.strip_prefix("--budget-mib=").and_then(|v| v.parse::<usize>().ok())).unwrap_or(64);
        let cap = cap_mib.checked_mul(1 << 20).ok_or("budget overflow")?;
        let budget = PreviewPreparationBudget::new(cap, &observer);
        let started = Instant::now();
        let small = thumbnail_with_styled_preparation_budget(&doc, 256, &Interrupt::NONE, &budget)
            .map_err(|e| format!("256px preview: {e}; usage={:?}", budget.usage()))?;
        let large = thumbnail_with_styled_preparation_budget(&doc, 1024, &Interrupt::NONE, &budget)
            .map_err(|e| format!("1024px preview: {e}; usage={:?}", budget.usage()))?;
        let pair_ms = started.elapsed().as_secs_f64() * 1000.;
        if small.image() != &thumbnail(&doc, 256)? || large.image() != &thumbnail(&doc, 1024)? || doc != before {
            return Err("styled budget preview/source parity failure".into());
        }
        let peak = budget.usage().peak_bytes;
        let retained = budget.usage().live_bytes;
        drop(small);
        drop(large);
        if budget.usage().live_bytes != 0 {
            return Err("styled preparation lease leak".into());
        }
        let started = Instant::now();
        let cancel = || started.elapsed() >= Duration::from_millis(20);
        if !matches!(thumbnail_with_styled_preparation_budget(&doc, 1024, &Interrupt::cancel_only(&cancel), &budget), Err(RasterInputError::Cancelled))
            || budget.usage().live_bytes != 0
            || doc != before
        {
            return Err("styled cancellation/source/lease failure".into());
        }
        let total_ms = started.elapsed().as_secs_f64() * 1000.;
        println!(
            "24MP bounded styled U16 cap_mib={cap_mib} shadow={shadow_case} vector={vector_case} feather={feather_case} pair_ms={pair_ms:.3} conservative_reservation_peak={peak} retained_output_bytes={retained} final_live=0 byte_exact=true cancel_deadline_ms=20 stop_total_ms={total_ms:.3} deadline_to_stop_ms={:.3}",
            (total_ms - 20.).max(0.)
        );
        return Ok(());
    }
    let started = Instant::now();
    let legacy = thumbnail(&doc, 256)?;
    let legacy_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let checked = thumbnail_with_interrupt(&doc, 256, &Interrupt::NONE)?;
    let checked_ms = started.elapsed().as_secs_f64() * 1000.0;
    if checked != legacy || doc != before {
        return Err("preview bytes or source changed".into());
    }
    let started = Instant::now();
    let cancel = || started.elapsed() >= Duration::from_millis(20);
    let result = thumbnail_with_interrupt(&doc, 4096, &Interrupt::cancel_only(&cancel));
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    if !matches!(result, Err(RasterInputError::Cancelled)) || doc != before {
        return Err("cancel did not discard output atomically".into());
    }
    println!(
        "24MP U16 shadow={shadow_case} vector={vector_case} feather={feather_case} legacy_ms={legacy_ms:.3} checked_ms={checked_ms:.3} byte_exact=true cancel_deadline_ms=20 stop_total_ms={total_ms:.3} deadline_to_stop_ms={:.3}",
        (total_ms - 20.0).max(0.0)
    );
    Ok(())
}
