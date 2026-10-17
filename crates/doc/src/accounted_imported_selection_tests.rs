//! UNRUN imported anonymous selection lifecycle and rollback drafts.
use super::*;
use crate::accounted_selection::CoverageDomain;
use crate::{BackedLayer, ColorMode, Layer, PixelFormat, SampleType, Size};
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 16 << 20, index: 8 << 20 })
}
fn model(q: &BackingQuota, name: &str, filtered: bool, unguarded: bool) -> AccountedDocument {
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 2, 1), &[0; 16]).unwrap();
    let backed = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new("retained", LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut doc = Document::new(name, Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    doc.layers.push(layer);
    doc.source_color_context = Some(crate::SourceColorContext { source_precision: 675, original_icc: Some(Arc::new(vec![7; 128])) });
    doc.icc_profile = Some(Arc::new(vec![9; 128]));
    let mut state = XcfState::default();
    state.items.push(ItemState {
        item: crate::item_identity::ItemRef::Layer(id),
        name: "retained".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 0,
        raster_bounds: Some(descriptor.bounds()),
        mask: None,
        composition: None,
    });
    state.selection.selected.push(crate::item_identity::ItemRef::Layer(id));
    state.selection.active = Some(crate::item_identity::ItemRef::Layer(id));
    state.provenance.original_locator = Some("inert/not/a/read/grant".into());
    state.provenance.protected_source_fingerprints.push("a".repeat(64));
    state.opaque.push(OpaqueRecord {
        owner: None,
        namespace: "retained".into(),
        identity: "raw".into(),
        source_version: 22,
        order: 3,
        flags: 8,
        bytes: Arc::new(vec![1, 2, 3]),
        blake3: "b".repeat(64),
        rendering_required: false,
    });
    if filtered {
        state.filter_stacks.push(crate::xcf_filters::FilterStack { owner: crate::item_identity::ItemRef::Layer(id), nodes: Vec::new(), revision: 0 });
    }
    doc.xcf_state = Some(state);
    if unguarded {
        doc.selection = Some(crate::Surface::new(PixelFormat::GRAY8));
    }
    let guard = Arc::new(q.reserve(QuotaKind::Resident, 16384).unwrap());
    AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap()
}
fn selection(q: &BackingQuota) -> AccountedSelection {
    AccountedSelection::try_from_native(PixelFormat::GRAY8, Rect::new(0, 0, 2, 1), &[0], &[255, 128], CoverageDomain::FiniteUnit, q, &control()).unwrap()
}
#[test]
fn anonymous_install_clear_history_preserves_whole_source_and_shared_payloads() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let selected = selection(&q);
    let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
    history.try_replace_selection(Some(&selected), &control()).unwrap();
    let current = history.current().unwrap();
    assert_eq!(current.document().xcf_state, original.document().xcf_state);
    assert_eq!(current.document().source_color_context, original.document().source_color_context);
    let a = current.document().xcf_state.as_ref().unwrap();
    let b = original.document().xcf_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&a.opaque[0].bytes, &b.opaque[0].bytes));
    assert!(Arc::ptr_eq(current.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
    assert_eq!(current.document().selection_channel_id, None);
    history.try_replace_selection(None, &control()).unwrap();
    assert!(history.current().unwrap().document().selection.is_none());
    assert_eq!(history.current().unwrap().document().xcf_state, original.document().xcf_state);
    history.undo(&control()).unwrap();
    let before = history.current().unwrap().clone();
    history.try_replace_selection(Some(&selected), &control()).unwrap();
    assert!(before.shares_model(history.current().unwrap()));
    assert!(history.can_redo());
    history.redo(&control()).unwrap();
    assert!(history.current().unwrap().document().selection.is_none());
}
#[test]
fn unguarded_selection_and_filtered_state_do_not_gain_selection_authority() {
    for (filtered, unguarded) in [(true, false), (false, true)] {
        let q = quota();
        let doc = model(&q, "refused", filtered, unguarded);
        let s = selection(&q);
        let before = q.usage().unwrap();
        assert!(doc.try_replace_selection(Some(&s), &control()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert!(doc.document().selection_channel_id.is_none());
    }
}
#[test]
fn cancellation_inside_owned_text_copy_rolls_back_all_temporary_leases() {
    let q = quota();
    let doc = model(&q, &"é".repeat(32768), false, false);
    let s = selection(&q);
    let before = q.usage().unwrap();
    let work = |units| if units == 32768 { Err(BackingError::Cancelled) } else { Ok(()) };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(matches!(doc.try_replace_selection(Some(&s), &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert!(doc.document().selection.is_none());
}
#[test]
fn exhausted_index_refuses_before_cloning_model_or_mutating_source() {
    let q = quota();
    let doc = model(&q, "source", false, false);
    let s = selection(&q);
    let remaining = (8 << 20) - q.usage().unwrap().index;
    let held = q.reserve(QuotaKind::Index, remaining).unwrap();
    let before = q.usage().unwrap();
    assert!(matches!(doc.try_replace_selection(Some(&s), &control()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), before);
    assert!(doc.document().selection.is_none());
    drop(held);
}
#[test]
fn anonymous_imported_selection_metadata_copy_preserves_real_guard_and_source() {
    let q = quota();
    let doc = model(&q, "source", false, false);
    let s = selection(&q);
    let selected = doc.try_replace_selection(Some(&s), &control()).unwrap();
    let layer = selected.document().layers[0].id;
    let changed = selected.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &control()).unwrap();
    assert!(!changed.document().layers[0].visible);
    assert!(s.matches(changed.document().selection.as_ref().unwrap(), &q, &control()).unwrap());
    assert_eq!(changed.document().source_color_context, selected.document().source_color_context);
    assert_eq!(changed.document().icc_profile, selected.document().icc_profile);
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    let used = q.usage().unwrap();
    assert!(matches!(selected.try_edit(AccountedEdit::LayerVisibility { layer, visible: false }, &denied), Err(BackingError::Cancelled)));
    assert!(selected.try_edit(AccountedEdit::LayerVisibility { layer: LayerId(u64::MAX), visible: false }, &control()).is_err());
    assert_eq!(q.usage().unwrap(), used);
    assert!(selected.document().layers[0].visible);
}
