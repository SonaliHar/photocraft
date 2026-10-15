//! UNRUN per-document exact-key metadata gesture lifecycle drafts.
use super::*;
use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size,
    item_identity::ItemRef,
    xcf_state::{ItemLocks, ItemState, XcfState},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, out: &mut [u8], c: &ReadControl<'_>) -> std::result::Result<(), BackingError> {
        c.check(out.len() as u64)?;
        out.fill(0);
        Ok(())
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session() -> Session {
    let q = BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 16 << 20, index: 8 << 20 });
    let guard = Arc::new(q.reserve(QuotaKind::Resident, 65536).unwrap());
    let mut doc = Document::new("original.xcf", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let mut xcf = XcfState::default();
    for name in ["lower", "upper"] {
        let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA32F, Rect::new(0, 0, 2, 1), &[0; 16]).unwrap();
        let raster = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
        let layer = Layer::new(name, LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap()));
        xcf.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: name.into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(descriptor.bounds()),
            mask: None,
            composition: None,
        });
        doc.layers.push(layer);
    }
    doc.xcf_state = Some(xcf);
    let owner = AccountedDocument::from_owned(doc, guard, q, &control()).unwrap();
    let mut s = Session::new();
    s.add_accounted_document(owner, None, &control()).unwrap();
    s
}
fn opacity(layer: photocraft_doc::LayerId, value: f32) -> AccountedEdit<'static> {
    AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: Some(value) }
}
fn run(s: &mut Session, layer: photocraft_doc::LayerId, value: f32, key: Option<&str>) {
    s.edit_accounted_metadata_with_coalesce("Layer Properties", opacity(layer, value), key, &control()).unwrap();
}
fn marker(s: &Session) -> PaidMetadataGesture {
    s.active().unwrap()._accounted_metadata_gesture.clone().unwrap()
}
fn same(a: &PaidMetadataGesture, b: &PaidMetadataGesture) -> bool {
    Arc::ptr_eq(&a.owner, &b.owner)
}
#[test]
fn exact_same_key_different_requested_layers_keep_one_anchor() {
    let mut s = session();
    let a = s.active().unwrap().doc.layers[0].id;
    let b = s.active().unwrap().doc.layers[1].id;
    run(&mut s, a, 0.2, Some("layer-opacity-slider:17"));
    run(&mut s, b, 0.4, Some("layer-opacity-slider:17"));
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.2);
    assert_eq!(s.active().unwrap().doc.layers[1].opacity, 0.4);
    assert!(s.undo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 1.0);
    assert_eq!(s.active().unwrap().doc.layers[1].opacity, 1.0);
    assert!(!s.active().unwrap().history.can_undo());
    assert!(s.redo_with_control(&control()).unwrap());
    assert_eq!(s.active().unwrap().doc.layers[1].opacity, 0.4);
}
#[test]
fn pure_focus_transition_does_not_invent_a_key_scope() {
    let mut s = session();
    let a = s.active().unwrap().doc.layers[0].id;
    let b = s.active().unwrap().doc.layers[1].id;
    run(&mut s, a, 0.2, Some("same"));
    let key = marker(&s);
    s.select_accounted_with_control(a, &control()).unwrap();
    assert!(same(&key, &marker(&s)));
    run(&mut s, b, 0.4, Some("same"));
    assert!(s.undo_with_control(&control()).unwrap());
    assert!(!s.active().unwrap().history.can_undo());
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 1.0);
}
#[test]
fn noop_unkeyed_or_different_key_preserves_paid_marker_and_redo() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("one"));
    let key = marker(&s);
    run(&mut s, id, 0.2, None);
    assert!(same(&key, &marker(&s)));
    run(&mut s, id, 0.2, Some("different"));
    assert!(same(&key, &marker(&s)));
    s.undo_with_control(&control()).unwrap();
    assert!(s.active().unwrap()._accounted_metadata_gesture.is_none());
    run(&mut s, id, 1.0, Some("noop"));
    assert!(s.active().unwrap()._accounted_metadata_gesture.is_none());
    assert!(s.active().unwrap().history.can_redo());
}
#[test]
fn different_key_and_changed_unkeyed_edits_start_separate_steps() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("one"));
    run(&mut s, id, 0.4, Some("two"));
    run(&mut s, id, 0.6, None);
    assert!(s.active().unwrap()._accounted_metadata_gesture.is_none());
    for expected in [0.4, 0.2, 1.0] {
        assert!(s.undo_with_control(&control()).unwrap());
        assert_eq!(s.active().unwrap().doc.layers[0].opacity, expected);
    }
}
#[test]
fn empty_and_nul_keys_remain_exact_inert_strings() {
    for key in ["", "a\0b"] {
        let mut s = session();
        let id = s.active().unwrap().doc.layers[0].id;
        run(&mut s, id, 0.2, Some(key));
        run(&mut s, id, 0.4, Some(key));
        assert_eq!(marker(&s).owner.key, key);
        s.undo_with_control(&control()).unwrap();
        assert!(!s.active().unwrap().history.can_undo());
    }
}
#[test]
fn cancel_and_oversize_key_leave_original_marker_and_snapshot() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("one"));
    let key = marker(&s);
    let before = s.active().unwrap().doc.clone();
    let stop = AtomicBool::new(true);
    let cancelled = || stop.load(Ordering::Relaxed);
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(s.edit_accounted_metadata_with_coalesce("Layer Properties", opacity(id, 0.4), Some("one"), &c).is_err());
    assert!(before.shares_state(&s.active().unwrap().doc));
    assert!(same(&key, &marker(&s)));
    assert!(s.edit_accounted_metadata_with_coalesce("Layer Properties", opacity(id, 0.4), Some(&"x".repeat(129)), &control()).is_err());
    assert!(same(&key, &marker(&s)));
}
#[test]
fn paid_save_identity_replacement_does_not_end_upstream_key() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("one"));
    let key = marker(&s);
    crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "copy.pcraft", &control())
        .unwrap()
        .write_and_publish(&mut s, &control(), |_, _| Ok(()))
        .unwrap();
    assert!(same(&key, &marker(&s)));
    run(&mut s, id, 0.4, Some("one"));
    s.undo_with_control(&control()).unwrap();
    assert!(!s.active().unwrap().history.can_undo());
    assert_eq!(s.active().unwrap().doc.name, "copy.pcraft");
}
#[test]
fn compound_metadata_is_not_implicitly_split_by_changed_field_or_layer() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("compound"));
    s.edit_accounted_metadata_with_coalesce(
        "Layer Properties",
        AccountedEdit::LayerMetadata { layer: id, name: Some("renamed"), visible: Some(false), opacity: None },
        Some("compound"),
        &control(),
    )
    .unwrap();
    s.undo_with_control(&control()).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].name, "lower");
    assert!(s.active().unwrap().doc.layers[0].visible);
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 1.0);
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn minimum_history_and_missing_anchor_append_instead_of_refusing_same_key() {
    let mut s = session();
    s.active_mut().unwrap().history.max_states = 1;
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("repeat"));
    run(&mut s, id, 0.4, Some("repeat"));
    assert!(s.active().unwrap().history.can_undo());
    // Exercise the lower legitimate no-anchor state independently of Session
    // purge's key-reset policy: continuation must append rather than requiretip.
    s.active_mut().unwrap().history.clear(&control()).unwrap();
    assert!(!s.active().unwrap().history.can_undo());
    let key = marker(&s);
    run(&mut s, id, 0.6, Some("repeat"));
    assert!(same(&key, &marker(&s)));
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.6);
    s.undo_with_control(&control()).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].opacity, 0.4);
}

#[test]
fn actual_quota_failure_keeps_current_marker_and_model() {
    let mut s = session();
    let id = s.active().unwrap().doc.layers[0].id;
    run(&mut s, id, 0.2, Some("one"));
    let key = marker(&s);
    let before = s.active().unwrap().doc.clone();
    let q = before.quota().unwrap().clone();
    let used = q.usage().unwrap();
    let filler = q.reserve(QuotaKind::Index, (8 << 20) - used.index).unwrap();
    assert!(s.edit_accounted_metadata_with_coalesce("Layer Properties", opacity(id, 0.4), Some("two"), &control()).is_err());
    assert!(same(&key, &marker(&s)));
    assert!(before.shares_state(&s.active().unwrap().doc));
    drop(filler);
    assert_eq!(q.usage().unwrap(), used);
}
#[test]
fn interrupted_private_continuation_returns_all_temporary_charges() {
    for threshold in 0..96 {
        let mut s = session();
        let id = s.active().unwrap().doc.layers[0].id;
        run(&mut s, id, 0.2, Some("one"));
        let key = marker(&s);
        let before = s.active().unwrap().doc.clone();
        let q = before.quota().unwrap().clone();
        let used = q.usage().unwrap();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
        let interrupt = Interrupt::cancel_only(&stop);
        let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        if s.edit_accounted_metadata_with_coalesce("Layer Properties", opacity(id, 0.4), Some("one"), &c).is_err() {
            assert!(same(&key, &marker(&s)));
            assert!(before.shares_state(&s.active().unwrap().doc));
            assert_eq!(q.usage().unwrap(), used);
        }
    }
}
