//! TEMP ordinary Shape alpha/source ledger regressions; uncompiled and unrun.
use super::*;
use photocraft_color::SampleType;
use photocraft_doc::{Fill, Path, ShapeLayer, Size, Subpath};
use std::sync::atomic::AtomicUsize;
#[test]
fn one_parent_work_boundary_is_shared_without_failed_debit_refund() {
    let prep = SourcePreparation::new("image.calculations", &Interrupt::NONE).unwrap();
    prep.debit(WORK - 1).unwrap();
    assert_eq!(prep.remaining.load(Ordering::Relaxed), 1);
    assert!(prep.debit(2).unwrap_err().to_string().contains("aggregate source preparation Work"));
    assert_eq!(prep.remaining.load(Ordering::Relaxed), 1);
    prep.debit(1).unwrap();
    assert!(prep.debit(1).is_err());
    assert_eq!(prep.remaining.load(Ordering::Relaxed), 0);
    // Independent tiny-canvas physical scope: far more than one logical word.
    let tiny = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    tiny.physical_output(1, 8).unwrap();
    let assigned = WORK - tiny.remaining.load(Ordering::Relaxed);
    assert_eq!(assigned, 65536 * (64 + 4 * 8) + 1024 + 2 * 256);
    let lower = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    lower.debit(WORK - assigned + 1).unwrap();
    assert!(lower.physical_output(1, 8).is_err());
    assert_eq!(lower.remaining.load(Ordering::Relaxed), assigned - 1);
}
fn ordinary_shape() -> ShapeLayer {
    ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(2.5, 3.0), (90.5, 5.5), (76.0, 66.0), (3.0, 54.0)])]),
        fill: Some(Fill::Solid(Color::rgba(0.2, 0.3, 0.7, 0.375))),
        ..Default::default()
    }
}
#[test]
fn shape_alpha_bands_keep_exact_old_alpha_absolute_frame_and_original_source() {
    let shape = ordinary_shape();
    let before = shape.clone();
    let area = Rect::new(0, 0, 100, 80);
    let prep = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    prep.admit_peak(100 * 80 * 48).unwrap();
    let mut actual = vec![0.0; 8000];
    prep.alpha(&shape, area, &mut actual).unwrap();
    let expected: Vec<_> =
        photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, area).render_rgba(area).into_iter().map(|p| p[3]).collect();
    assert_eq!(actual, expected);
    assert_eq!(shape, before);
    assert!(prep.remaining.load(Ordering::Relaxed) < WORK);
    // Resident envelope is the real overlap left after parent admission.
    let denied = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    denied.admit_peak(photocraft_raster::MAX_CHECKED_RASTER_BYTES).unwrap();
    let mut untouched = vec![0.625; 8000];
    assert!(denied.alpha(&shape, area, &mut untouched).is_err());
    assert!(untouched.iter().all(|p| p.to_bits() == 0.625f32.to_bits()));
}
#[test]
fn original_cancelled_census_and_shape_leave_unpublished_source_unchanged() {
    let shape = ordinary_shape();
    let before = shape.clone();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 12;
    let interrupt = Interrupt::cancel_only(&cancel);
    let prep = SourcePreparation::new("select.loadSelection", &interrupt).unwrap();
    let mut unpublished = vec![0.0; 8000];
    assert!(matches!(prep.alpha(&shape, Rect::new(0, 0, 100, 80), &mut unpublished), Err(EngineError::Cancelled)));
    assert_eq!(shape, before);
    assert!(calls.load(Ordering::Relaxed) > 12);
    // Calibrate the actual bounded Source path, then deny its final positive
    // Work with the same actual immutable input. A privately partial plane is
    // never published and the original Shape model remains exact.
    let mut original = Document::new("shape source", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
    let layer = Layer::new("actual shape", LayerContent::Shape(shape.clone()));
    let id = layer.id;
    original.layers.push(layer);
    let source_before = original.clone();
    let request = SnapshotSource { layer: Some(id), channel: ChanRef::Transparency, gray: true, invert: false };
    let full = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    let positive = source_from_snapshot_prepared(&original, Some(id), request, "select.loadSelection", &Interrupt::NONE, &full).unwrap();
    assert!(positive[0].iter().any(|v| *v > 0.0));
    let used = WORK - full.remaining.load(Ordering::Relaxed);
    assert!(used > 1);
    let lower = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
    lower.debit(WORK - used + 1).unwrap();
    assert!(source_from_snapshot_prepared(&original, Some(id), request, "select.loadSelection", &Interrupt::NONE, &lower).is_err());
    assert_eq!(original, source_before);

    let doc = Document::new("retained", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(SourcePreparation::new("image.calculations", &interrupt), Err(EngineError::Cancelled)));
    assert_eq!(doc.name, "retained");
}
#[test]
fn ordinary_24mp_calculation_admission_and_shape_load_keep_one_real_undo() {
    // Metadata/admission proof only: no 24MP execution/throughput claim. Sparse
    // ordinary raster leaves keep real runtime format and wholecanvas bounds.
    let mut doc = Document::new("24MP", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("ordinary sparse", doc.pixel_format()));
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    let prep = SourcePreparation::new("image.calculations", &Interrupt::NONE).unwrap();
    let admitted = calculation_admit(&session, &[&json!({})], 24_000_000, "image.calculations", &Interrupt::NONE, &prep);
    if cfg!(target_arch = "wasm32") {
        assert!(admitted.is_err(), "unchanged 512MiB wasm Resident ceiling");
    } else {
        admitted.unwrap();
        assert!(prep.remaining.load(Ordering::Relaxed) > 0);
    }
    let mut session = Session::new();
    session.execute("file.new", json!({"width":100,"height":80,"depth":32,"background":"transparent"})).unwrap();
    let id = session.execute("shape.create", json!({"kind":"rect","rect":[20,10,40,30],"fill":"#cc3333"})).unwrap()["layer"].as_u64().unwrap();
    session
        .edit("drop cache", |doc, _| {
            let layer = doc.layer_mut(LayerId(id)).unwrap();
            if let LayerContent::Shape(shape) = &mut layer.content {
                shape.cache = None;
            }
            layer.visible = false;
            layer.opacity = 0.25;
            Ok(())
        })
        .unwrap();
    let original = session.active().unwrap().doc.clone();
    let past = session.active().unwrap().history.past_len();
    session.execute("select.loadSelection", json!({"layer":id,"channel":"transparency"})).unwrap();
    let loaded = session.active().unwrap().doc.clone();
    assert_eq!(session.active().unwrap().history.past_len(), past + 1);
    assert_eq!(loaded.selection.as_ref().unwrap().content_bounds(), Rect::new(20, 10, 60, 40));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, original);
    // Existing native selection equality publishes no replacement on an empty
    // selection source. The real shape-load redo must remain available.
    let empty_owner = session.active().unwrap().doc.clone();
    let empty_revision = session.active().unwrap().revision;
    let empty_past = session.active().unwrap().history.past_len();
    session.execute("select.loadSelection", json!({"channel":"selection"})).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&empty_owner));
    assert_eq!(session.active().unwrap().revision, empty_revision);
    assert_eq!(session.active().unwrap().history.past_len(), empty_past);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, loaded);
    // Three actual source preparations share the single calculation ledger:
    // two cacheless Shape transparency inputs and the same Shape mask.
    let before_calculation = session.active().unwrap().doc.clone();
    let before_channels = before_calculation.channels.len();
    let before_past = session.active().unwrap().history.past_len();
    let result = calculations_with_interrupt(
        &mut session,
        &json!({
            "source1":{"layer":id,"channel":"transparency"},
            "source2":{"layer":id,"channel":"transparency"},
            "mask":{"layer":id,"channel":"transparency"},
            "blending":"multiply", "opacity":100, "result":"newChannel", "name":"Shape product"
        }),
        &Interrupt::NONE,
    )
    .unwrap();
    assert_eq!(result["channel"].as_u64(), Some(before_channels as u64));
    let calculated = session.active().unwrap().doc.clone();
    assert_eq!(calculated.channels.len(), before_channels + 1);
    assert_eq!(session.active().unwrap().history.past_len(), before_past + 1);
    let surface = &calculated.channels.last().unwrap().surface;
    let raw = surface.try_read_interleaved(calculated.bounds()).unwrap();
    assert_eq!(surface.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let (words, remainder) = raw.as_chunks::<4>();
    assert!(remainder.is_empty());
    for (index, word) in words.iter().enumerate() {
        let (x, y) = (index % 100, index / 100);
        let expected = if (20..60).contains(&x) && (10..40).contains(&y) { 1.0f32 } else { 0.0f32 };
        assert_eq!(*word, expected.to_ne_bytes());
    }
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, before_calculation);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc, calculated);
}

#[test]
fn shape_alpha_row_strips_keep_whole_area_prefix_bits_with_shifted_and_wide_frames() {
    let shape = ordinary_shape();
    let before = shape.clone();
    for area in [Rect::new(-17, -9, 48, 81), Rect::new(-23, -11, 234, 89), Rect::new(7, 3, 107, 83), Rect::new(-20, -10, 4980, 80)] {
        let count = area.width() as usize * area.height() as usize;
        let prep = SourcePreparation::new("select.loadSelection", &Interrupt::NONE).unwrap();
        prep.admit_peak(count as u64 * 48).unwrap();
        let mut actual = vec![0.0; count];
        prep.alpha(&shape, area, &mut actual).unwrap();
        let expected: Vec<_> =
            photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, area).render_rgba(area).into_iter().map(|p| p[3]).collect();
        assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "area={area:?}");
        assert_eq!(shape, before);
        assert!(actual.iter().any(|v| *v > 0.0), "real covered source, area={area:?}");
    }
}
