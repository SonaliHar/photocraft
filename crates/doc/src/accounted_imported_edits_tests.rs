//! UNRUN closed imported metadata lifecycle and rollback drafts.
use super::*;
use crate::xcf_provenance::OpaqueRecord;
use crate::xcf_state::*;
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

#[test]
fn three_metadata_edits_synchronize_current_fields_and_preserve_original_owners() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let id = original.document().layers[0].id;
    let mut history = AccountedHistory::new(original.clone(), 8, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerName { layer: id, name: "changed" }, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.25 }, &control()).unwrap();
    let current = history.current().unwrap();
    let state = current.document().xcf_state.as_ref().unwrap();
    assert_eq!(current.document().layers[0].name, "changed");
    assert_eq!(state.items[0].name, "changed");
    assert!(state.items[0].custom_name);
    assert!(!current.document().layers[0].visible);
    assert!(!state.items[0].visible);
    assert_eq!(current.document().layers[0].opacity, 0.25);
    let prior = original.document().xcf_state.as_ref().unwrap();
    assert_eq!(state.items[0].composition, prior.items[0].composition);
    assert_eq!(state.provenance, prior.provenance);
    assert!(Arc::ptr_eq(&state.opaque[0].bytes, &prior.opaque[0].bytes));
    assert!(Arc::ptr_eq(current.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
    assert_eq!(current.document().source_color_context, original.document().source_color_context);
    history.undo(&control()).unwrap();
    let before = history.current().unwrap().downgrade();
    history.try_commit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap();
    assert!(before.ptr_eq(&history.current().unwrap().downgrade()));
    history.redo(&control()).unwrap();
    assert_eq!(history.current().unwrap().document().layers[0].opacity, 0.25);
    assert_eq!(original.document().layers[0].name, "retained");
}

#[test]
fn cancelled_quota_and_unsupported_edits_preserve_original() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let id = original.document().layers[0].id;
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(original.try_edit(AccountedEdit::LayerName { layer: id, name: "new" }, &cancelled).is_err());
    let used = q.usage().unwrap().resident;
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - used).unwrap();
    assert!(original.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).is_err());
    drop(blocker);
    assert_eq!(original.document().layers[0].name, "retained");
    assert!(original.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 16] }, &control()).is_err());
    let filtered = model(&q, "filtered", true, false);
    let id = filtered.document().layers[0].id;
    assert!(filtered.try_edit(AccountedEdit::LayerName { layer: id, name: "new" }, &control()).is_err());
    let raw_selection = model(&q, "unguarded", false, true);
    let id = raw_selection.document().layers[0].id;
    assert!(raw_selection.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).is_err());
}

#[test]
fn compound_properties_are_one_undo_and_cancel_rolls_back() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let id = original.document().layers[0].id;
    let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
    let edit = AccountedEdit::LayerMetadata { layer: id, name: Some("compound"), visible: Some(false), opacity: Some(0.3) };
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(history.try_commit(edit, &cancelled).is_err());
    assert!(history.current().unwrap().downgrade().ptr_eq(&original.downgrade()));
    history.try_commit(edit, &control()).unwrap();
    assert_eq!(history.current().unwrap().document().layers[0].opacity, 0.3);
    history.undo(&control()).unwrap();
    assert!(history.current().unwrap().downgrade().ptr_eq(&original.downgrade()));
    history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None }, &control()).unwrap();
    history.redo(&control()).unwrap();
    assert_eq!(history.current().unwrap().document().layers[0].name, "compound");
}

#[test]
fn equal_name_in_compound_preserves_original_custom_flag() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let id = original.document().layers[0].id;
    assert!(!original.document().xcf_state.as_ref().unwrap().items[0].custom_name);
    let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerMetadata { layer: id, name: Some("retained"), visible: None, opacity: Some(0.5) }, &control()).unwrap();
    let state = history.current().unwrap().document().xcf_state.as_ref().unwrap();
    assert_eq!(state.items[0].name, "retained");
    assert!(!state.items[0].custom_name);
    history.undo(&control()).unwrap();
    assert!(history.current().unwrap().downgrade().ptr_eq(&original.downgrade()));
    history.redo(&control()).unwrap();
    assert!(!history.current().unwrap().document().xcf_state.as_ref().unwrap().items[0].custom_name);
}

#[test]
fn late_cancel_during_long_name_validation_and_repeated_comparison() {
    let q = quota();
    let original = model(&q, "source", false, false);
    let id = original.document().layers[0].id;
    let name = "n".repeat(1 << 20);
    let bytes = std::sync::atomic::AtomicU64::new(0);
    let late = ReadControl {
        interrupt: &Interrupt::NONE,
        work: &|n| {
            let previous =
                bytes.fetch_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |used| Some(used.saturating_add(n))).unwrap();
            if previous.saturating_add(n) > 65536 { Err(BackingError::Cancelled) } else { Ok(()) }
        },
    };
    assert!(matches!(
        original.try_edit(AccountedEdit::LayerMetadata { layer: id, name: Some(&name), visible: None, opacity: Some(0.5) }, &late),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(original.document().layers[0].name, "retained");
    let renamed = original.try_edit(AccountedEdit::LayerName { layer: id, name: &name }, &control()).unwrap();
    let bytes = std::sync::atomic::AtomicU64::new(0);
    let late = ReadControl {
        interrupt: &Interrupt::NONE,
        work: &|n| {
            let previous =
                bytes.fetch_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |used| Some(used.saturating_add(n))).unwrap();
            if previous.saturating_add(n) > 65536 { Err(BackingError::Cancelled) } else { Ok(()) }
        },
    };
    assert!(matches!(names_equal(&renamed.document().layers[0].name, &name, &late), Err(BackingError::Cancelled)));
}
