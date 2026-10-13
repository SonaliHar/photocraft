use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    BackedLayer, Document, Layer, LayerContent,
    accounted_document::{AccountedDocument, AccountedEdit},
};
use photocraft_engine::Session;
use photocraft_geom::{Rect, Size};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Store {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        for (i, byte) in output.iter_mut().enumerate() {
            let at = offset.checked_add(i as u64).ok_or(BackingError::Bounds)?;
            *byte = *self.descriptor.default_bytes().get((at % 32) as usize).ok_or(BackingError::Length)?;
        }
        Ok(())
    }
}
fn owner() -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    owner_with(|_| {})
}

#[test]
fn registered_accounted_dispatch_keeps_authorization_and_unqualified_refusals_atomic() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn gate(_: &str, params: &serde_json::Value) -> photocraft_engine::Result<()> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        if params.get("name").and_then(serde_json::Value::as_str) == Some("denied") {
            return Err(photocraft_engine::EngineError::Other("own authorization denial".into()));
        }
        Ok(())
    }
    let (document, q, dropped) = owner();
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    session.authorize = Some(gate);
    let original = session.active().unwrap().doc.clone();
    let usage = q.usage().unwrap();
    let revision = session.active().unwrap().revision;
    assert!(session.execute("layer.setProps", serde_json::json!({"name":"denied"})).is_err());
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(session.journal.is_empty());
    assert_eq!(q.usage().unwrap(), usage);
    assert!(session.execute("layer.renameLayer", serde_json::json!({"name":"unqualified"})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert!(session.journal.is_empty());
    assert_eq!(q.usage().unwrap(), usage);
    assert!(session.is_enabled_with("layer.setProps", &serde_json::json!({"name":"accepted"})));
    assert!(matches!(
        session.start("layer.setProps", serde_json::json!({"name":"accepted"})).unwrap(),
        photocraft_engine::jobs::Started::Done(serde_json::Value::Null)
    ));
    assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(session.active().unwrap().doc.layers[0].name, "accepted");
    assert_eq!(session.journal.len(), 1);
    assert!(session.execute("edit.undo", serde_json::Value::Null).unwrap().as_bool().unwrap());
    assert!(session.active().unwrap().doc.shares_state(&original));
    drop(original);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn registered_accounted_dispatch_still_validates_other_resident_tabs_before_publication() {
    let (document, q, dropped) = owner();
    let mut session = Session::new();
    let mut resident = Document::new("own resident tab", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    resident.layers.push(Layer::raster("ordinary", resident.pixel_format()));
    session.add_document(resident, None).unwrap();
    let mut ordinary = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    ordinary.try_write_pixel_f64(0, 0, &[1.0; 4]).unwrap();
    let tile = ordinary.tiles().next().unwrap().1.clone();
    let mut forged = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    forged.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, i32::MIN), tile)]);
    session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].content = LayerContent::Raster(forged);
    session.add_accounted_document(document, None, &control()).unwrap();
    let original = session.active().unwrap().doc.clone();
    let usage = q.usage().unwrap();
    let revision = session.active().unwrap().revision;
    assert!(session.execute("layer.setProps", serde_json::json!({"name":"must not publish"})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&original));
    assert_eq!(session.active().unwrap().revision, revision);
    assert!(session.journal.is_empty());
    assert_eq!(q.usage().unwrap(), usage);
    drop(original);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
fn owner_with(configure: impl FnOnce(&mut Document)) -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    let retention = q.reserve(QuotaKind::Resident, 65536).unwrap();
    let descriptor =
        NativePlaneDescriptor::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(-2, 1, 2, 3), &(-0.0f64).to_ne_bytes().repeat(4))
            .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let raster = BackedRaster::new(Arc::new(Store { descriptor, dropped: dropped.clone() }), q.clone(), 2, &control()).unwrap();
    let mut document = Document::new("accounted session", Size::new(4, 4), ColorMode::Rgb, SampleType::F64);
    document.layers.push(Layer::new("current", LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &control()).unwrap())));
    configure(&mut document);
    (AccountedDocument::from_owned(document, Arc::new(retention), q.clone(), &control()).unwrap(), q, dropped)
}

fn selected_xcf() -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    selected_xcf_with(|_| {})
}
fn selected_xcf_with(configure: impl FnOnce(&mut Document)) -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    use photocraft_doc::{
        item_identity::ItemRef,
        xcf_editing::EditingSelection,
        xcf_provenance::ProjectProvenance,
        xcf_state::{ItemState, XcfState},
    };
    owner_with(|document| {
        document.layers.push(Layer::new("other", document.layers[0].content.clone()));
        let first = document.layers[0].id;
        let second = document.layers[1].id;
        document.xcf_state = Some(XcfState {
            provenance: ProjectProvenance {
                format: "xcf".into(),
                format_version: 22,
                container: "plain".into(),
                source_label: "input.xcf".into(),
                ..Default::default()
            },
            items: document
                .layers
                .iter()
                .map(|layer| ItemState {
                    item: ItemRef::Layer(layer.id),
                    name: layer.name.clone(),
                    custom_name: false,
                    visible: layer.visible,
                    locks: Default::default(),
                    parent: None,
                    source_tattoo: None,
                    source_flags: 0,
                    raster_bounds: Some(Rect::new(-2, 1, 2, 3)),
                    mask: None,
                    composition: None,
                })
                .collect(),
            selection: EditingSelection {
                selected: vec![ItemRef::Layer(second), ItemRef::Layer(first)],
                active: Some(ItemRef::Layer(first)),
                ..Default::default()
            },
            ..Default::default()
        });
        configure(document);
        document.validate_xcf_state().unwrap();
    })
}

fn grouped_xcf_with(configure: impl FnOnce(&mut Document)) -> (AccountedDocument, BackingQuota, Arc<AtomicUsize>) {
    use photocraft_doc::item_identity::ItemRef;
    selected_xcf_with(|doc| {
        let group = Layer::group("parent", std::mem::take(&mut doc.layers));
        let group_id = group.id;
        let state = doc.xcf_state.as_mut().unwrap();
        for item in &mut state.items {
            item.parent = Some(ItemRef::Layer(group_id));
        }
        let mut parent = state.items[0].clone();
        parent.item = ItemRef::Layer(group_id);
        parent.parent = None;
        parent.name = "parent".into();
        parent.raster_bounds = None;
        state.items.push(parent);
        doc.layers.push(group);
        configure(doc);
        doc.validate_xcf_state().unwrap();
    })
}

#[test]
fn nested_xcf_session_retains_selection_exact_leaf_edit_and_group_history() {
    use photocraft_doc::{accounted_document::accounted_layer_access, item_identity::ItemRef};
    let (document, q, dropped) = grouped_xcf_with(|_| {});
    let state = document.document().xcf_state.as_ref().unwrap();
    let ItemRef::Layer(leaf) = state.selection.active.unwrap() else { panic!() };
    let selected = state
        .selection
        .selected
        .iter()
        .map(|item| match item {
            ItemRef::Layer(id) => *id,
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    let group = document.document().layers[0].id;
    let original = document.clone();
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    assert_eq!(session.active().unwrap().selection_ids(), selected);
    let bytes = f64::from_bits(0x3fd0000000000001).to_ne_bytes().repeat(4);
    session.edit_accounted("Leaf native", AccountedEdit::NativePixels { layer: leaf, rect: Rect::new(-1, 1, 0, 2), bytes: &bytes }, &control()).unwrap();
    let leaf_access = accounted_layer_access(&session.active().unwrap().doc, leaf, &control()).unwrap().unwrap();
    let LayerContent::BackedRaster(raster) = &leaf_access.layer.content else { panic!() };
    let mut actual = [0; 32];
    raster.checked_view().read_pixel_native_into(-1, 1, &mut actual, &control()).unwrap();
    assert_eq!(actual.as_slice(), bytes);
    assert!(session.undo_with_control(&control()).unwrap());
    assert!(session.active().unwrap().doc.accounted().unwrap().shares_model(&original));
    assert!(session.redo_with_control(&control()).unwrap());
    session.edit_accounted("Group name", AccountedEdit::LayerName { layer: group, name: "renamed group" }, &control()).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "renamed group");
    assert_eq!(session.active().unwrap().selection_ids(), selected);
    assert!(session.active().unwrap().source.is_none());
    drop(original);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn nested_xcf_session_inherits_physical_and_canonical_locks_without_history_change() {
    use photocraft_doc::item_identity::ItemRef;
    let (document, q, _) = grouped_xcf_with(|doc| {
        doc.layers[0].locks.transparency = true;
        let parent = doc.xcf_state.as_mut().unwrap().items.last_mut().unwrap();
        parent.locks.visibility = true;
    });
    let ItemRef::Layer(leaf) = document.document().xcf_state.as_ref().unwrap().selection.active.unwrap() else { panic!() };
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = q.usage().unwrap();
    assert!(session.edit_accounted("Hide child", AccountedEdit::LayerVisibility { layer: leaf, visible: false }, &control()).is_err());
    let bytes = 0.5f64.to_ne_bytes().repeat(4);
    assert!(session.edit_accounted("Pixels", AccountedEdit::NativePixels { layer: leaf, rect: Rect::new(-1, 1, 0, 2), bytes: &bytes }, &control()).is_err());
    session.edit_accounted("No-op", AccountedEdit::LayerVisibility { layer: leaf, visible: true }, &control()).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(q.usage().unwrap(), usage);
    drop(before);
    drop(session);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn nested_xcf_edit_cancellation_preserves_redo_and_refunds_each_stage() {
    use photocraft_doc::item_identity::ItemRef;
    let (document, q, _) = grouped_xcf_with(|_| {});
    let ItemRef::Layer(leaf) = document.document().xcf_state.as_ref().unwrap().selection.active.unwrap() else { panic!() };
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    session.edit_accounted("First name", AccountedEdit::LayerName { layer: leaf, name: "first" }, &control()).unwrap();
    assert!(session.undo_with_control(&control()).unwrap());
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = q.usage().unwrap();
    let mut cancelled = 0;
    for fence in 1..=512 {
        let calls = AtomicUsize::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::SeqCst) + 1 == fence { Err(BackingError::Cancelled) } else { Ok(()) };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        match session.edit_accounted("Cancelled name", AccountedEdit::LayerName { layer: leaf, name: "replacement" }, &ctl) {
            Err(photocraft_engine::EngineError::Cancelled) => {
                cancelled += 1;
                assert!(session.active().unwrap().doc.shares_state(&before));
                assert_eq!(session.active().unwrap().revision, revision);
                assert_eq!(session.active().unwrap().history.redo_label(), Some("First name"));
                assert_eq!(q.usage().unwrap(), usage, "cancellation fence {fence}");
            }
            Ok(()) => break,
            other => panic!("unexpected nested edit result {other:?}"),
        }
    }
    assert!(cancelled >= 20);
    drop(before);
    drop(session);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn accounted_mask_flags_keep_original_surface_canonical_state_and_session_history() {
    use photocraft_doc::{
        LayerMask,
        item_identity::{ItemRef, MaskId},
        xcf_state::MaskState,
    };
    let exact = f64::from_bits(0x3fd0000000000001).to_ne_bytes();
    let (document, q, dropped) = selected_xcf_with(|doc| {
        let id = doc.layers[0].id;
        doc.layers[0].locks.all = true;
        let surface = photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &exact).unwrap();
        doc.layers[0].mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: false, density: 0.75, feather: 1.5 });
        let state = doc.xcf_state.as_mut().unwrap();
        let mut item = state.items[0].clone();
        item.item = ItemRef::Mask(MaskId(1));
        item.name = "mask".into();
        item.raster_bounds = None;
        item.locks.content = true;
        item.locks.position = true;
        item.mask = Some(MaskState {
            owner: ItemRef::Layer(id),
            bounds: Rect::new(-2, 1, 2, 3),
            offset: [0, 0],
            outside_f64_bits: u64::from_ne_bytes(exact),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        });
        state.items.push(item);
    });
    let id = document.document().layers[0].id;
    let original = document.clone();
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    session.edit_accounted("Disable mask", AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()).unwrap();
    session.edit_accounted("Link mask", AccountedEdit::MaskLinked { layer: id, linked: true }, &control()).unwrap();
    let current = session.active().unwrap().doc.clone();
    let mask = current.layers[0].mask.as_ref().unwrap();
    assert!(!mask.enabled && mask.linked);
    assert!(Arc::ptr_eq(&mask.surface, &original.document().layers[0].mask.as_ref().unwrap().surface));
    assert_eq!(mask.surface.default_bytes(), exact);
    let mut expected = original.document().xcf_state.clone().unwrap();
    let state = expected.items.last_mut().unwrap().mask.as_mut().unwrap();
    state.enabled = false;
    state.linked = true;
    assert_eq!(current.xcf_state.as_ref(), Some(&expected));
    assert!(session.undo_with_control(&control()).unwrap());
    assert!(session.undo_with_control(&control()).unwrap());
    assert!(session.active().unwrap().doc.accounted().unwrap().shares_model(&original));
    assert!(session.redo_with_control(&control()).unwrap());
    assert!(session.redo_with_control(&control()).unwrap());
    assert!(session.active().unwrap().doc.shares_state(&current));
    drop(current);
    drop(original);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn accounted_typed_edits_preserve_imported_visibility_and_pixel_locks() {
    let (document, q, _) = selected_xcf_with(|doc| {
        doc.layers[0].locks.transparency = true;
        let item = &mut doc.xcf_state.as_mut().unwrap().items[0];
        item.locks.visibility = true;
        item.locks.content = true;
        item.locks.alpha = true;
    });
    let id = document.document().layers[0].id;
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = q.usage().unwrap();
    assert!(session.edit_accounted("Hide", AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).is_err());
    let changed = 0.75f64.to_ne_bytes().repeat(4);
    assert!(
        session.edit_accounted("Locked pixels", AccountedEdit::NativePixels { layer: id, rect: Rect::new(-1, 1, 0, 2), bytes: &changed }, &control()).is_err()
    );
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(q.usage().unwrap(), usage);
    session.edit_accounted("Visibility no-op", AccountedEdit::LayerVisibility { layer: id, visible: true }, &control()).unwrap();
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), usage);
    // Existing property semantics allow renaming/opacity under content locks.
    session.edit_accounted("Opacity", AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap();
    session.edit_accounted("Name", AccountedEdit::LayerName { layer: id, name: "renamed" }, &control()).unwrap();
    assert_eq!(session.active().unwrap().doc.layers[0].name, "renamed");
    drop(before);
    drop(session);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn canonical_xcf_admission_restores_owned_ordered_selection_and_history_without_source_authority() {
    let (document, q, dropped) = selected_xcf();
    let first = document.document().layers[0].id;
    let second = document.document().layers[1].id;
    let original = document.clone();
    let mut session = Session::new();
    session.add_accounted_document(document, Some("opened.pcraft".into()), &control()).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.active_layer, Some(first));
    assert_eq!(state.layer_anchor, Some(first));
    assert_eq!(state.selection_ids(), &[second, first]);
    assert_eq!(state.doc.xcf_state, original.document().xcf_state);
    assert!(state.source.is_none());
    session.edit_accounted("Hide", AccountedEdit::LayerVisibility { layer: first, visible: false }, &control()).unwrap();
    assert!(!session.active().unwrap().doc.xcf_state.as_ref().unwrap().items[0].visible);
    assert!(session.undo_with_control(&control()).unwrap());
    assert!(session.active().unwrap().doc.accounted().unwrap().shares_model(&original));
    assert_eq!(session.active().unwrap().selection_ids(), &[second, first]);
    assert!(session.redo_with_control(&control()).unwrap());
    assert_eq!(session.active().unwrap().selection_ids(), &[second, first]);
    drop(original);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn canonical_selection_admission_cancellation_preserves_dirty_tab_and_refunds_all_stages() {
    let (document, q, dropped) = selected_xcf();
    let usage = q.usage().unwrap();
    let mut session = Session::new();
    session.execute("file.new", serde_json::json!({"width":4,"height":4})).unwrap();
    session
        .edit("Dirty", |doc, _| {
            doc.name = "unsaved".into();
            Ok(())
        })
        .unwrap();
    let previous = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let mut cancelled_fences = 0;
    for fence in 1..=64 {
        let calls = AtomicUsize::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::SeqCst) + 1 == fence { Err(BackingError::Cancelled) } else { Ok(()) };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        match session.add_accounted_document(document.clone(), None, &ctl) {
            Err(_) => {
                cancelled_fences += 1;
                assert_eq!(session.documents().len(), 1);
                assert!(session.active().unwrap().doc.shares_state(&previous));
                assert_eq!(session.active().unwrap().revision, revision);
                assert_eq!(session.active().unwrap().history.undo_label(), Some("Dirty"));
                assert_eq!(q.usage().unwrap(), usage);
            }
            Ok(index) => {
                drop(session.close(index));
                break;
            }
        }
    }
    assert!(cancelled_fences >= 9);
    // A successful admission replaced the still-live resident tab vector with
    // paid slots. Closing the accounted tab keeps that physical allocation.
    let after_close = q.usage().unwrap();
    assert!(after_close.index > usage.index);
    assert_eq!(after_close.resident - usage.resident, after_close.index - usage.index);
    assert!(session.active().unwrap().doc.shares_state(&previous));
    let fill = q.reserve(QuotaKind::Index, (1 << 20) - after_close.index).unwrap();
    assert!(session.add_accounted_document(document.clone(), None, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&previous));
    drop(fill);
    assert_eq!(q.usage().unwrap(), after_close);
    drop(session);
    assert_eq!(q.usage().unwrap(), usage);
    drop(document);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn admission_retains_owned_native_payload_until_session_close() {
    let (document, q, dropped) = owner();
    let original_id = document.document().id;
    let before = q.usage().unwrap();
    let mut session = Session::new();
    let admitted = session.add_accounted_document(document, None, &control()).unwrap();
    assert_eq!(admitted, 0);
    assert_eq!(session.active().unwrap().doc.id, original_id);
    assert!(session.active().unwrap().doc.has_backed_rasters().unwrap());
    assert!(q.usage().unwrap().index > before.index);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(session.close(admitted));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

fn pixel(session: &Session) -> [u8; 32] {
    let document = &session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &document.layers[0].content else { panic!("backed layer required") };
    let mut bytes = [0; 32];
    layer.checked_view().read_pixel_native_into(-1, 1, &mut bytes, &control()).unwrap();
    bytes
}

#[test]
fn typed_native_edit_undo_redo_and_snapshots_keep_exact_bits_and_owned_selection() {
    let (document, q, dropped) = owner();
    let id = document.document().layers[0].id;
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    let initial = session.active().unwrap().doc.clone();
    let weak = initial.downgrade();
    let original = (-0.0f64).to_ne_bytes().repeat(4);
    assert_eq!(pixel(&session).as_slice(), original);
    assert_eq!(session.active().unwrap().selection_ids(), &[id]);
    let changed = f64::from_bits(0x3fd0000000000001).to_ne_bytes().repeat(4);
    session.edit_accounted("Native exact", AccountedEdit::NativePixels { layer: id, rect: Rect::new(-1, 1, 0, 2), bytes: &changed }, &control()).unwrap();
    assert_eq!(pixel(&session).as_slice(), changed);
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Native exact"));
    assert!(session.undo_with_control(&control()).unwrap());
    assert!(session.active().unwrap().doc.shares_state(&initial));
    assert_eq!(pixel(&session).as_slice(), original);
    assert_eq!(session.active().unwrap().selection_ids(), &[id]);
    assert!(session.redo_with_control(&control()).unwrap());
    assert_eq!(pixel(&session).as_slice(), changed);
    assert_eq!(session.active().unwrap().selection_ids(), &[id]);
    drop(session);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    assert!(weak.upgrade().is_some());
    drop(initial);
    assert!(weak.upgrade().is_none());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    let weak_charge = q.usage().unwrap();
    // Index is a subquota of resident. The weak Arc still owns its inline
    // control/model allocation, while the source store and all pixel payloads
    // have already been destroyed.
    assert_eq!(weak_charge.resident, weak_charge.index);
    assert!(weak_charge.index > 0 && weak_charge.index < 65536);
    assert_eq!(weak_charge.scratch, 0);
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn denied_and_cancelled_transactions_preserve_current_redo_selection_and_quota() {
    let (document, q, _) = owner();
    let id = document.document().layers[0].id;
    let mut session = Session::new();
    session.add_accounted_document(document, None, &control()).unwrap();
    session.edit_accounted("Hide", AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap();
    assert!(session.undo_with_control(&control()).unwrap());
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let usage = q.usage().unwrap();
    let fill = q.reserve(QuotaKind::Index, (1 << 20) - usage.index).unwrap();
    let full = q.usage().unwrap();
    assert!(session.edit_accounted("Denied", AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).is_err());
    assert!(session.redo_with_control(&control()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().history.redo_label(), Some("Hide"));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().selection_ids(), &[id]);
    drop(fill);
    let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    assert!(session.edit_accounted("Cancelled", AccountedEdit::LayerVisibility { layer: id, visible: false }, &cancelled).is_err());
    assert_eq!(q.usage().unwrap(), usage);
    let mut callback_ran = false;
    assert!(
        session
            .edit("Unmetered", |_, _| {
                callback_ran = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!callback_ran);
    assert!(session.active().unwrap().try_clone_resident().is_err());
    assert!(session.active().unwrap().selected_layers().is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().history.redo_label(), Some("Hide"));
    drop(before);
    drop(session);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn failed_admission_preserves_existing_dirty_resident_document() {
    let (document, q, dropped) = owner();
    let mut session = Session::new();
    session.execute("file.new", serde_json::json!({"width":4,"height":4})).unwrap();
    session
        .edit("Dirty", |doc, _| {
            doc.name = "unsaved".into();
            Ok(())
        })
        .unwrap();
    let old = session.active().unwrap().try_clone_resident().unwrap();
    let usage = q.usage().unwrap();
    let fill = q.reserve(QuotaKind::Index, (1 << 20) - usage.index).unwrap();
    assert!(session.add_accounted_document(document, None, &control()).is_err());
    assert_eq!(session.documents().len(), 1);
    assert!(session.active().unwrap().doc.shares_state(&old.doc));
    assert_eq!(session.active().unwrap().revision, old.revision);
    assert_eq!(session.active().unwrap().history.undo_label(), Some("Dirty"));
    assert!(session.active().unwrap().is_dirty());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    drop(fill);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn every_admission_cancellation_fence_preserves_tabs_and_refunds_prepared_state() {
    let (document, q, _) = owner();
    let mut observed = 0;
    for phase in 1..=64 {
        let calls = AtomicUsize::new(0);
        let work = |_| if calls.fetch_add(1, Ordering::SeqCst) + 1 == phase { Err(BackingError::Cancelled) } else { Ok(()) };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let mut session = Session::new();
        session.execute("file.new", serde_json::json!({"width":4,"height":4})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let usage = q.usage().unwrap();
        match session.add_accounted_document(document.clone(), None, &ctl) {
            Err(photocraft_engine::EngineError::Cancelled) => {
                observed += 1;
                assert_eq!(session.documents().len(), 1);
                assert!(session.active().unwrap().doc.shares_state(&before));
                assert_eq!(q.usage().unwrap(), usage, "fence {phase}");
            }
            Ok(_) => {
                drop(session);
                break;
            }
            other => panic!("unexpected admission result at {phase}: {other:?}"),
        }
    }
    assert!(observed >= 7, "publication must include constructor, history and slot fences");
    drop(document);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn resident_admission_rejects_detached_backing_before_publication() {
    let (document, q, _) = owner();
    let mut session = Session::new();
    assert!(session.add_document(document.document().clone(), None).is_err());
    assert!(session.documents().is_empty());
    assert!(session.active().is_none());
    drop(document);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
