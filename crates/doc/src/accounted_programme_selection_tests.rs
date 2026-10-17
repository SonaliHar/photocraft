//! Ignored UNRUN/uncompiled actual programme-selection ownership contracts.
//! The inherited same-q native tree is paid synthetic CURRENT, never Source,
//! CMS or operation evidence. Input/expected raw buffers are external harness.
use super::*;
// Selection construction owns a real temporary Scratch reservation. Keep this
// child-local so the older hierarchy fixture policy remains unchanged.
fn quota() -> BackingQuota {
    BackingQuota::new(photocraft_raster::backing::quota::QuotaLimits { resident: 128 << 20, index: 64 << 20, scratch: 16 << 20 })
}

use crate::accounted_document::ProgrammeNativeStorageKind;
use crate::accounted_selection::{AccountedSelection, CoverageDomain, SelectionViewCharges};
fn owner(q: &BackingQuota, sample: SampleType) -> AccountedDocument {
    let (d, h) = fixture(q, sample);
    AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap()
}
fn coverage(q: &BackingQuota, sample: SampleType) -> AccountedSelection {
    let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let (default, raw) = match sample {
        SampleType::U8 => (vec![85], vec![0, 128, 255, 85, 0, 255]),
        SampleType::F32 => (0.25f32.to_ne_bytes().to_vec(), [-0f32, 0.375, 1., 0.5, 0., 0.75].into_iter().flat_map(f32::to_ne_bytes).collect()),
        SampleType::F64 => {
            (0.25f64.to_ne_bytes().to_vec(), [-0f64, 0.375 + 2f64.powi(-44), 1., 0.5, 0., 0.75].into_iter().flat_map(f64::to_ne_bytes).collect())
        }
        _ => panic!("chosen native coverage fixture"),
    };
    AccountedSelection::try_from_native(format, Rect::new(1, 1, 4, 3), &default, &raw, CoverageDomain::FiniteUnit, q, &ctl()).unwrap()
}
fn bound(owner: &AccountedDocument, guard: &AccountedSelection) {
    let t = owner.checked_programme_selection(&ctl()).unwrap();
    assert!(t.matches_owner(owner));
    assert!(t.matches_document(owner.document()));
    assert_eq!(t.storage_kind(), ProgrammeNativeStorageKind::NormalHierarchy);
    let surface = t.selection().unwrap();
    assert!(guard.matches_native_surface(surface, &ctl()).unwrap());
    assert_eq!(surface.default_bytes(), guard.surface().default_bytes());
    for ((x, a), (y, b)) in surface.tiles().zip(guard.surface().tiles()) {
        assert_eq!(x, y);
        assert!(Arc::ptr_eq(a, b));
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: real same-q native coverage guard, exact sparse default/lowbits/Arc views and Source-vs-Current pointer binding"]
fn installed_selection_is_a_real_guarded_current_capability() {
    for sample in [SampleType::F32, SampleType::F64] {
        for coverage_sample in [SampleType::U8, sample] {
            let q = quota();
            let original = owner(&q, sample);
            let g = coverage(&q, coverage_sample);
            let before = original.document().xcf_state.clone(); // external fixture expectation, already preheld model
            let selected = original.try_replace_programme_selection(Some(&g), &ctl()).unwrap();
            assert!(!selected.shares_model(&original));
            assert!(selected.is_readonly_normal_hierarchy_storage());
            assert!(original.document().selection.is_none());
            bound(&selected, &g);
            assert_eq!(selected.document().xcf_state, before);
            assert_eq!(selected.document().source_color_context, original.document().source_color_context);
            assert_eq!(selected.document().icc_profile, original.document().icc_profile);
            assert!(selected.validate_selection_replacement(&ctl()).is_err()); // old Legacy-only API
            assert!(AccountedDocument::readonly_normal_hierarchy_current_storage_requirements(selected.document(), &q, &ctl()).is_err()); // raw Doc API stays None
            let token = selected.checked_programme_selection(&ctl()).unwrap();
            assert!(!token.matches_owner(&original));
            assert!(!token.matches_document(original.document()));
            let clone = selected.clone();
            assert!(token.matches_owner(&clone));
            drop(token);
            assert!(selected.try_replace_programme_selection(Some(&g), &ctl()).unwrap().shares_model(&selected));
            let cleared = selected.try_replace_programme_selection(None, &ctl()).unwrap();
            assert!(cleared.document().selection.is_none());
            assert_eq!(cleared.document().xcf_state, before);
            assert!(cleared.checked_programme_selection(&ctl()).unwrap().selection().is_none());
            drop(original);
            drop(selected);
            cleared.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: existing typed programme/native/title/metadata/move/remove edits retain real selection guard/view and empty Current continuation"]
fn every_reached_real_copier_preserves_selected_ownership() {
    let q = quota();
    let original = owner(&q, SampleType::F64);
    let g = coverage(&q, SampleType::F64);
    let selected = original.try_replace_programme_selection(Some(&g), &ctl()).unwrap();
    let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
    let changed = selected
        .try_with_readonly_normal_hierarchy_filter(
            PointFilterPatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 7, arguments: &args, enabled: Some(true) },
            &ctl(),
        )
        .unwrap();
    bound(&changed, &g);
    let moved = changed
        .try_move_readonly_normal_hierarchy_filter(
            PointFilterMovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 8, index: 0 },
            &ctl(),
        )
        .unwrap();
    bound(&moved, &g);
    let meta = moved
        .try_edit(AccountedEdit::LayerMetadata { layer: LayerId(11), name: Some("selected parent"), visible: Some(false), opacity: Some(-0.) }, &ctl())
        .unwrap();
    bound(&meta, &g);
    let titled = meta.try_with_document_name("selected current", &ctl()).unwrap();
    bound(&titled, &g);
    let pixel: Vec<_> = [0.125f64, -0., 0.375 + 2f64.powi(-44), -0.].into_iter().flat_map(f64::to_ne_bytes).collect();
    let native = titled
        .try_edit_programme_preserving_native(AccountedEdit::NativePixels { layer: LayerId(2), rect: Rect::new(0, 0, 1, 1), bytes: &pixel }, &ctl())
        .unwrap();
    bound(&native, &g);
    let one = native
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 9 }, &ctl())
        .unwrap();
    bound(&one, &g);
    let empty = one
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 10 }, &ctl())
        .unwrap();
    bound(&empty, &g);
    assert!(empty.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
    assert!(empty.document().xcf_state.as_ref().unwrap().filter_masks.is_empty());
    assert_eq!(empty.document().source_color_context, original.document().source_color_context);
    assert_eq!(empty.document().icc_profile, original.document().icc_profile);
    assert!(empty.document().xcf_state.as_ref().unwrap().opaque.iter().all(|r| r.owner.is_none()));
    drop(g);
    drop(original);
    drop(selected);
    drop(changed);
    drop(moved);
    drop(meta);
    drop(titled);
    drop(native);
    drop(one);
    empty.checked_programme_selection(&ctl()).unwrap();
    let plain = empty.try_replace_programme_selection(None, &ctl()).unwrap();
    plain.validate_readonly_normal_hierarchy_current_storage(&ctl()).unwrap();
}
struct CodecRetention {
    _original: Arc<Hold>,
    _guard: AccountedSelection,
    _view: SelectionViewCharges,
}
fn codec_input(q: &BackingQuota) -> (Document, Arc<CodecRetention>) {
    let (mut doc, h) = fixture(q, SampleType::F64);
    let g = coverage(q, SampleType::F64);
    let view = g.clone_view(&ctl()).unwrap();
    doc.selection = Some(view.surface);
    (doc, Arc::new(CodecRetention { _original: h, _guard: g, _view: view.charges }))
}
#[test]
#[ignore = "UNRUN/uncompiled: direct codec selection adoption creates same actual Current class/guard; None/wrongkind/Source shape refuse without Legacy laundering"]
fn codec_adopts_real_selection_bytes_and_same_class_directly() {
    let q = quota();
    let (doc, retention) = codec_input(&q);
    assert!(AccountedDocument::readonly_normal_hierarchy_current_storage_requirements(&doc, &q, &ctl()).is_err());
    let expected = doc.selection.as_ref().unwrap().clone(); // external expected view; retention remains live below
    let selected =
        AccountedDocument::from_owned_programme_rehydrating_selection(ProgrammeNativeStorageKind::NormalHierarchy, doc, retention.clone(), q.clone(), &ctl())
            .unwrap();
    assert!(selected.is_readonly_normal_hierarchy_storage());
    assert!(!selected.is_legacy_storage());
    let token = selected.checked_programme_selection(&ctl()).unwrap();
    let actual = token.selection().unwrap();
    assert_eq!(actual.default_bytes(), expected.default_bytes());
    for ((x, a), (y, b)) in actual.tiles().zip(expected.tiles()) {
        assert_eq!(x, y);
        assert!(Arc::ptr_eq(a, b));
    }
    drop(token);
    drop(expected);
    drop(retention);
    let renamed = selected.try_with_document_name("native selected", &ctl()).unwrap();
    drop(selected);
    renamed.checked_programme_selection(&ctl()).unwrap();
    let (doc, h) = fixture(&q, SampleType::F64);
    assert!(AccountedDocument::from_owned_programme_rehydrating_selection(ProgrammeNativeStorageKind::NormalHierarchy, doc, h, q.clone(), &ctl()).is_err());
    let (doc, h) = codec_input(&q);
    let baseline = q.usage().unwrap();
    assert!(AccountedDocument::from_owned_programme_rehydrating_selection(ProgrammeNativeStorageKind::Point, doc, h.clone(), q.clone(), &ctl()).is_err());
    // The returned Document was consumed on refusal; no fabricated owner escaped.
    assert!(q.usage().unwrap().resident <= baseline.resident);
}
#[test]
#[ignore = "UNRUN/uncompiled: foreign/wrongdepth guards, independent view fees, exact no-op and calibrated late Work/cancel preserve original selection and all receipts"]
fn guard_and_error_lifetimes_are_not_receipt_assertions() {
    let q = quota();
    let base = owner(&q, SampleType::F64);
    let g = coverage(&q, SampleType::F64);
    let selected = base.try_replace_programme_selection(Some(&g), &ctl()).unwrap();
    let replacement = coverage(&q, SampleType::U8);
    let foreign_q = quota();
    let foreign = coverage(&foreign_q, SampleType::U8);
    let baseline = q.usage().unwrap();
    assert!(selected.try_replace_programme_selection(Some(&foreign), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let no_space = q.reserve(QuotaKind::Resident, (128u64 << 20) - baseline.resident).unwrap();
    assert!(selected.try_replace_programme_selection(Some(&replacement), &ctl()).is_err());
    drop(no_space);
    assert_eq!(q.usage().unwrap(), baseline);
    bound(&selected, &g);
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = selected.try_replace_programme_selection(Some(&replacement), &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), baseline);
    let remaining = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(selected.try_replace_programme_selection(Some(&replacement), &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    bound(&selected, &g);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(selected.try_replace_programme_selection(None, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(selected.try_replace_programme_selection(Some(&g), &ctl()).unwrap().shares_model(&selected));
    let wrong = coverage(&q, SampleType::F32);
    assert!(selected.try_replace_programme_selection(Some(&wrong), &ctl()).is_err());
    bound(&selected, &g);
}
