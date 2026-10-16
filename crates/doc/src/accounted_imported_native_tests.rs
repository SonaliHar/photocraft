//! UNRUN closed imported NativePixels transactions. Synthetic codec retention
//! supplies actual charges; these fixtures grant no renderer/source authority.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

fn native_model(q: &BackingQuota, sample: SampleType) -> AccountedDocument {
    let base = model(q, "native", false, false);
    let fee = q.reserve(QuotaKind::Resident, 32768).unwrap();
    let mut doc = base.document().clone();
    doc.depth = sample;
    doc.source_color_context.as_mut().unwrap().source_precision = if sample == SampleType::F64 { 750 } else { 350 };
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(0, 0, 2, 1), &vec![0; format.bytes_per_pixel()]).unwrap();
    let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    doc.layers[0].content = LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap());
    AccountedDocument::from_owned(doc, Arc::new((base, fee)), q.clone(), &control()).unwrap()
}
#[test]
fn native_patch_preserves_f64_u32_and_current_source_metadata_without_synchronizing_it() {
    for sample in [SampleType::F64, SampleType::U32] {
        let q = quota();
        let original = native_model(&q, sample);
        let id = original.document().layers[0].id;
        let rect = Rect::new(0, 0, 1, 1);
        let mut bytes = vec![0; original.document().pixel_format().bytes_per_pixel()];
        if sample == SampleType::F64 {
            bytes[..8].copy_from_slice(&(0.5 + 2f64.powi(-45)).to_ne_bytes());
        } else {
            bytes[..4].copy_from_slice(&0x8000_0001u32.to_ne_bytes());
        }
        let edit = AccountedEdit::NativePixels { layer: id, rect, bytes: &bytes };
        original.validate_imported_native_edit(edit, &control()).unwrap();
        assert!(original.validate_imported_metadata_edit(edit, &control()).is_err());
        let changed = original.try_edit(edit, &control()).unwrap();
        assert!(!changed.shares_model(&original));
        assert_eq!(changed.document().xcf_state, original.document().xcf_state);
        assert_eq!(changed.document().source_color_context, original.document().source_color_context);
        assert!(Arc::ptr_eq(changed.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
        let LayerContent::BackedRaster(backed) = &changed.document().layers[0].content else { panic!() };
        let mut actual = vec![0; bytes.len()];
        backed.checked_view().read_logical_pixel_native_into(0, 0, &mut actual, &control()).unwrap();
        assert_eq!(actual, bytes);
        backed.checked_view().read_logical_pixel_native_into(1, 0, &mut actual, &control()).unwrap();
        assert!(actual.iter().all(|v| *v == 0), "outside patch remains exact");
        assert!(backed.default_bytes().iter().all(|v| *v == 0));
        assert!(changed.try_edit(edit, &control()).unwrap().shares_model(&changed));
        let LayerContent::BackedRaster(backed) = &original.document().layers[0].content else { panic!() };
        backed.checked_view().read_logical_pixel_native_into(0, 0, &mut actual, &control()).unwrap();
        assert!(actual.iter().all(|v| *v == 0), "original source not rewritten");
    }
}

#[test]
fn exact_imported_noop_preserves_redo_but_negative_zero_is_a_real_change() {
    let q = quota();
    let original = model(&q, "history", false, false);
    let id = original.document().layers[0].id;
    let rect = Rect::new(0, 0, 1, 1);
    let zeros = [0u8; 16];
    let mut negative = zeros;
    negative[..4].copy_from_slice(&(-0.0f32).to_ne_bytes());
    let mut history = AccountedHistory::new(original.clone(), 3, &control()).unwrap();
    history.try_commit(AccountedEdit::NativePixels { layer: id, rect, bytes: &negative }, &control()).unwrap();
    assert!(!history.current().unwrap().shares_model(&original));
    history.undo(&control()).unwrap();
    history.try_commit(AccountedEdit::NativePixels { layer: id, rect, bytes: &zeros }, &control()).unwrap();
    assert!(history.current().unwrap().shares_model(&original));
    assert!(history.can_redo());
    history.redo(&control()).unwrap();
    let previous = history.current().unwrap().clone();
    assert!(previous.try_edit(AccountedEdit::NativePixels { layer: id, rect, bytes: &negative }, &control()).unwrap().shares_model(&previous));
}

#[test]
fn malformed_regions_target_length_and_unsupported_imported_families_refuse_before_write() {
    let q = quota();
    let owner = model(&q, "gates", false, false);
    let id = owner.document().layers[0].id;
    for (layer, rect, bytes) in [
        (id, Rect::EMPTY, &[][..]),
        (id, Rect::new(-1, 0, 0, 1), &[0u8; 16][..]),
        (id, Rect::new(0, 0, 1, 1), &[0u8; 15][..]),
        (LayerId(u64::MAX), Rect::new(0, 0, 1, 1), &[0u8; 16][..]),
    ] {
        assert!(owner.try_edit(AccountedEdit::NativePixels { layer, rect, bytes }, &control()).is_err());
    }
    assert!(owner.validate_imported_native_edit(AccountedEdit::LayerName { layer: id, name: "not native" }, &control()).is_err());
    for (filtered, selection) in [(true, false), (false, true)] {
        let owner = model(&q, "unsupported", filtered, selection);
        let id = owner.document().layers[0].id;
        assert!(owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 16] }, &control()).is_err());
    }
}

#[test]
fn cancellation_work_and_quota_denial_leave_original_model_and_metadata_intact() {
    let q = quota();
    let owner = model(&q, "rollback", false, false);
    let id = owner.document().layers[0].id;
    let edit = AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 2, 1), bytes: &[0; 32] };
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(owner.try_edit(edit, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).err(), Some(BackingError::Cancelled));
    // Direct fixed-scratch comparison checks the actual callback at every
    // pixel; this isolates a late failure from preceding model-census work.
    let used = AtomicU64::new(0);
    let work = |_: u64| {
        if used.fetch_add(1, Ordering::Relaxed) >= 3 { Err(BackingError::Work) } else { Ok(()) }
    };
    assert!(
        super::super::native_is_noop(&owner.document().layers[0], Rect::new(0, 0, 2, 1), &[0; 32], &ReadControl { interrupt: &Interrupt::NONE, work: &work })
            .is_err()
    );
    // Warm source cache first; the blocked operation then cannot claim a new
    // changed model, native tile or index allocation without real headroom.
    assert!(owner.try_edit(edit, &control()).unwrap().shares_model(&owner));
    let baseline = q.usage().unwrap();
    let hold = q.reserve(QuotaKind::Resident, (64 << 20) - baseline.resident).unwrap();
    let mut changed = [0u8; 16];
    changed[..4].copy_from_slice(&1f32.to_ne_bytes());
    assert!(owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &changed }, &control()).is_err());
    drop(hold);
    assert_eq!(q.usage().unwrap(), baseline);
}

#[test]
fn source_free_legacy_native_equality_does_not_gain_new_noop_behavior() {
    let q = quota();
    let imported = model(&q, "legacy", false, false);
    let fee = q.reserve(QuotaKind::Resident, 32768).unwrap();
    let mut doc = imported.document().clone();
    doc.xcf_state = None;
    doc.source_color_context = None;
    let owner = AccountedDocument::from_owned(doc, Arc::new((imported, fee)), q, &control()).unwrap();
    let id = owner.document().layers[0].id;
    let changed = owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 16] }, &control()).unwrap();
    assert!(!changed.shares_model(&owner));
}

#[path="accounted_imported_native_wide_tests.rs"]
mod wide_contracts;
