use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size,
    accounted_document::{AccountedDocument, AccountedEdit},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
    fail: AtomicBool,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            return Err(BackingError::Io);
        }
        let n = self.descriptor.format().bytes_per_pixel() as u64;
        for (i, value) in output.iter_mut().enumerate() {
            *value = self.descriptor.default_bytes()[((offset + i as u64) % n) as usize];
        }
        Ok(())
    }
}
struct Retention {
    source: Weak<Store>,
    dropped_after_source: Arc<AtomicBool>,
    _lease: QuotaLease,
}
impl Drop for Retention {
    fn drop(&mut self) {
        self.dropped_after_source.store(self.source.upgrade().is_none(), Ordering::Relaxed);
    }
}
fn fixture() -> (Document, Arc<Retention>, Arc<Store>, BackingQuota, Arc<AtomicBool>) {
    let q = BackingQuota::new(QuotaLimits { resident: 32 << 20, scratch: 0, index: 1 << 20 });
    // Prepaid fixture model envelope; the transfer moves this existing ownership.
    let lease = q.reserve(QuotaKind::Resident, 16 << 10).unwrap();
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let bytes: Vec<_> = [0.25_f64, -0.0, 0.5, 1.0].into_iter().flat_map(f64::to_le_bytes).collect();
    let descriptor = NativePlaneDescriptor::new(format, Rect::new(-1, -1, 3, 2), &bytes).unwrap();
    let store = Arc::new(Store { descriptor, reads: AtomicU64::new(0), fail: AtomicBool::new(false) });
    let backing = BackedRaster::new(store.clone(), q.clone(), 1, &control()).unwrap();
    let mut doc = Document::new("retained native model", Size::new(3, 2), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(Layer::new("current layer", LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &control()).unwrap())));
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = Arc::new(Retention { source: Arc::downgrade(&store), dropped_after_source: dropped.clone(), _lease: lease });
    (doc, guard, store, q, dropped)
}

use photocraft_doc::{
    SourceColorContext,
    import_compositing::*,
    item_identity::*,
    xcf_editing::*,
    xcf_filters::*,
    xcf_items::{EditableSource, SourceCache, SourceItem},
    xcf_provenance::{OpaqueRecord, ProjectProvenance},
    xcf_resources::{ResourceKind, ResourceReference},
    xcf_state::{ItemLocks, ItemState, XcfState},
};
fn metadata(doc: &mut Document, source: bool) {
    let id = doc.layers[0].id;
    let policy = GimpComposition {
        compatibility_epoch: XCF12_FINITE_NORMAL_SOURCE_EPOCH.into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 1,
        stored_composite_space: 1,
        stored_operator: 1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    };
    doc.layers[0].import_compositing = Some(ImportCompositing {
        blend_encoding: photocraft_doc::SampleEncoding::Linear,
        composite_encoding: photocraft_doc::SampleEncoding::Linear,
        pass_through: false,
    });
    doc.source_color_context = Some(SourceColorContext { original_icc: Some(Arc::new(vec![1, 2, 3, 4])), source_precision: 700 });
    let raw = vec![0, 1, 2, 255, 0];
    let mut state = XcfState {
        provenance: ProjectProvenance {
            format: "xcf".into(),
            format_version: 22,
            container: "raw".into(),
            producer: Some("GIMP".into()),
            release_id: Some("3.0".into()),
            source_label: "original.xcf".into(),
            source_hash: Some(blake3::hash(b"original bytes").to_hex().to_string()),
            original_locator: Some("/inert/source/path".into()),
            protected_source_fingerprints: vec![blake3::hash(b"protected").to_hex().to_string()],
        },
        items: vec![ItemState {
            item: ItemRef::Layer(id),
            name: doc.layers[0].name.clone(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(123),
            source_flags: 0,
            raster_bounds: Some(Rect::new(-1, -1, 3, 2)),
            mask: None,
            composition: Some(policy),
        }],
        opaque: vec![OpaqueRecord {
            owner: Some(ItemRef::Layer(id)),
            namespace: "test".into(),
            identity: "inert-property".into(),
            source_version: 22,
            order: 4,
            flags: 7,
            blake3: blake3::hash(&raw).to_hex().to_string(),
            bytes: raw.into(),
            rendering_required: false,
        }],
        selection: EditingSelection { selected: vec![ItemRef::Layer(id)], active: Some(ItemRef::Layer(id)), ..Default::default() },
        item_sets: vec![ItemSet {
            id: ItemSetId(1),
            name: "set".into(),
            family: ItemFamily::Layer,
            rule: ItemSetRule::Glob("current*".into()),
            members: vec![],
        }],
        ..Default::default()
    };
    if source {
        state.resources.push(ResourceReference {
            id: ResourceId(1),
            kind: ResourceKind::Font,
            name: "unresolved font".into(),
            collection: Some("retained".into()),
            internal: false,
            locator: Some("inert:font".into()),
            content_hash: None,
            owned_bytes: None,
            resolution: None,
        });
        let bytes = [(-0.0f64).to_bits(), 0x3fd0000000000001, 0x7ff8000000000042, 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let operation = OperationIdentity { name: "gegl:nop".into(), stored_version: "1".into(), schema_epoch: "retained-schema".into() };
        let graph = FilterGraph {
            input: None,
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation,
                version_origin: GraphVersionOrigin::Stored,
                arguments: vec![FilterArgument {
                    name: "curve".into(),
                    value: FilterValue::Curve {
                        free: false,
                        points_f64_bits: vec![[0.0f64.to_bits(), (-0.0f64).to_bits()]],
                        point_types: vec![0],
                        samples_f64_bits: vec![],
                    },
                }],
            }],
            edges: vec![],
            output: GraphNodeId(1),
            original_pipeline: Some("inert source graph".into()),
        };
        state.source_items.push(SourceItem {
            owner: ItemRef::Layer(id),
            source: EditableSource::Text {
                content: "retained text".into(),
                markup: Some("<b>retained text</b>".into()),
                font: ResourceId(1),
                direction: 0,
                source_modified: true,
                configuration: vec![FilterArgument {
                    name: "nested".into(),
                    value: FilterValue::Config(vec![FilterArgument { name: "graph".into(), value: FilterValue::Graph(graph) }]),
                }],
                current_origin: None,
            },
            transform: None,
            cache: Some(SourceCache {
                blake3: blake3::hash(&bytes).to_hex().to_string(),
                bytes: bytes.into(),
                width: 1,
                height: 1,
                format: PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
            }),
        });
    }
    doc.xcf_state = Some(state);
    doc.validate_xcf_state().unwrap();
}

fn grouped_fixture() -> (Document, Arc<Retention>, Arc<Store>, BackingQuota, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, false);
    let mut leaf = doc.layers.pop().unwrap();
    leaf.mask = Some(photocraft_doc::LayerMask {
        surface: Arc::new(
            photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes())
                .unwrap(),
        ),
        enabled: true,
        linked: false,
        density: 0.75,
        feather: 1.5,
    });
    let leaf_id = leaf.id;
    let mut inner = Layer::group("inner retained group", vec![leaf]);
    inner.locks.pixels = true;
    let inner_id = inner.id;
    let mut outer = Layer::group("outer retained group", vec![inner]);
    outer.locks.position = true;
    let outer_id = outer.id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items[0].parent = Some(ItemRef::Layer(inner_id));
    for (id, name, parent) in [(inner_id, "inner retained group", Some(ItemRef::Layer(outer_id))), (outer_id, "outer retained group", None)] {
        state.items.push(ItemState {
            item: ItemRef::Layer(id),
            name: name.into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: None,
            mask: None,
            composition: None,
        });
    }
    doc.layers.push(outer);
    doc.validate_xcf_state().unwrap();
    (doc, guard, store, q, leaf_id, outer_id)
}
#[test]
fn nested_native_owner_admits_legal_xcf_groups_and_edits_leaf() {
    let (doc, guard, store, q, leaf, group) = grouped_fixture();
    let state = doc.xcf_state.clone();
    let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let changed = original.try_edit(AccountedEdit::LayerOpacity { layer: leaf, opacity: 0.375 }, &control()).unwrap();
    changed.document().validate_xcf_state().unwrap();
    assert_eq!(changed.document().xcf_state, state);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert!(matches!(
        changed.try_edit(AccountedEdit::NativePixels { layer: group, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 32] }, &control()),
        Err(BackingError::Unsupported)
    ));
    drop(changed);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

fn access(doc: &Document, id: photocraft_doc::LayerId) -> photocraft_doc::accounted_document::AccountedLayerAccess<'_> {
    photocraft_doc::accounted_document::accounted_layer_access(doc, id, &control()).unwrap().unwrap()
}
#[test]
fn group_child_vectors_names_masks_and_parent_relations_survive_charged_history() {
    use photocraft_doc::accounted_document::AccountedHistory;
    let (doc, guard, store, q, leaf, group) = grouped_fixture();
    let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let original_mask = access(original.document(), leaf).layer.mask.as_ref().unwrap().surface.clone();
    let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerName { layer: group, name: "renamed outer group" }, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerVisibility { layer: leaf, visible: false }, &control()).unwrap();
    let changed = history.current().unwrap().clone();
    changed.document().validate_xcf_state().unwrap();
    assert!(q.usage().unwrap().index > before.index);
    let current = access(changed.document(), leaf);
    assert!(!current.layer.visible);
    assert!(current.locks.pixels && current.locks.position);
    assert!(Arc::ptr_eq(&original_mask, &current.layer.mask.as_ref().unwrap().surface));
    assert_eq!(current.layer.mask.as_ref().unwrap().surface.default_bytes(), &(-0.0f64).to_ne_bytes());
    let old_state = original.document().xcf_state.as_ref().unwrap();
    let state = changed.document().xcf_state.as_ref().unwrap();
    assert_eq!(state.provenance, old_state.provenance);
    assert_eq!(state.opaque, old_state.opaque);
    assert_eq!(state.items.iter().map(|i| (i.item, i.parent)).collect::<Vec<_>>(), old_state.items.iter().map(|i| (i.item, i.parent)).collect::<Vec<_>>());
    assert_eq!(state.items.iter().find(|i| i.item == ItemRef::Layer(group)).unwrap().name, "renamed outer group");
    let LayerContent::Group(old_group) = &original.document().layers[0].content else { panic!() };
    let LayerContent::Group(new_group) = &changed.document().layers[0].content else { panic!() };
    assert_ne!(old_group.children.as_ptr(), new_group.children.as_ptr());
    assert_eq!(old_group.expanded, new_group.expanded);
    history.undo(&control()).unwrap();
    assert!(access(history.current().unwrap().document(), leaf).layer.visible);
    history.undo(&control()).unwrap();
    assert!(history.current().unwrap().shares_model(&original));
    history.redo(&control()).unwrap();
    history.redo(&control()).unwrap();
    assert!(history.current().unwrap().shares_model(&changed));
    let weak = original.downgrade();
    drop(history);
    drop(original);
    drop(store);
    drop(original_mask);
    assert!(weak.upgrade().is_none());
    assert!(q.usage().unwrap().resident > 0);
    drop(changed);
    assert_eq!(q.usage().unwrap().resident, q.usage().unwrap().index);
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn nested_native_edit_preserves_original_defaults_and_group_mask_owners() {
    let (doc, guard, store, q, leaf, _) = grouped_fixture();
    let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let exact = [0.125f64.to_bits(), (-0.0f64).to_bits(), 0x3fd0000000000001, 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    let current = original.try_edit(AccountedEdit::NativePixels { layer: leaf, rect: Rect::new(0, 0, 1, 1), bytes: &exact }, &control()).unwrap();
    let a = access(original.document(), leaf).layer;
    let b = access(current.document(), leaf).layer;
    assert!(Arc::ptr_eq(&a.mask.as_ref().unwrap().surface, &b.mask.as_ref().unwrap().surface));
    let LayerContent::BackedRaster(old) = &a.content else { panic!() };
    let LayerContent::BackedRaster(new) = &b.content else { panic!() };
    let mut oldbits = [0; 32];
    old.checked_view().read_pixel_native_into(0, 0, &mut oldbits, &control()).unwrap();
    let mut bits = [0; 32];
    new.checked_view().read_pixel_native_into(0, 0, &mut bits, &control()).unwrap();
    assert_eq!(bits.as_slice(), exact);
    assert_ne!(oldbits.as_slice(), exact);
    assert_eq!(old.descriptor().default_bytes(), new.descriptor().default_bytes());
    assert_eq!(original.document().xcf_state, current.document().xcf_state);
    drop(current);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn nested_copy_and_all_cancel_checkpoints_preserve_original_and_refund() {
    let (doc, guard, store, q, leaf, _) = grouped_fixture();
    let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let usage = q.usage().unwrap();
    let denial = q.reserve(QuotaKind::Index, (1 << 20) - usage.index).unwrap();
    assert!(matches!(original.try_edit(AccountedEdit::LayerName { layer: leaf, name: "changed child" }, &control()), Err(BackingError::Quota)));
    assert_eq!(access(original.document(), leaf).layer.name, "current layer");
    drop(denial);
    let calls = AtomicU64::new(0);
    let count = |_| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let measured =
        original.try_edit(AccountedEdit::LayerName { layer: leaf, name: "changed child" }, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(measured);
    for cancel_at in 0..calls.load(Ordering::Relaxed) {
        let step = AtomicU64::new(0);
        let cancel = |_| if step.fetch_add(1, Ordering::Relaxed) == cancel_at { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(matches!(
            original.try_edit(AccountedEdit::LayerName { layer: leaf, name: "changed child" }, &ReadControl { interrupt: &Interrupt::NONE, work: &cancel }),
            Err(BackingError::Cancelled)
        ));
        assert_eq!(q.usage().unwrap(), usage);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn bounded_lookup_and_admission_allow_depth100_and_refuse_depth101_or_count4097() {
    use photocraft_doc::accounted_document::accounted_layer_access;
    for depth in [100, 101] {
        let (mut doc, guard, store, q, _) = fixture();
        let leaf = doc.layers[0].id;
        for _ in 0..depth {
            let children = std::mem::take(&mut doc.layers);
            doc.layers.push(Layer::group("g", children));
        }
        if depth == 100 {
            let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
            assert_eq!(access(original.document(), leaf).layer.id, leaf);
            let before = q.usage().unwrap();
            let edited = original.try_edit(AccountedEdit::LayerVisibility { layer: leaf, visible: false }, &control()).unwrap();
            assert!(!access(edited.document(), leaf).layer.visible);
            // Each admitted Group child Vec owns a distinct copied Layer header.
            assert!(q.usage().unwrap().index - before.index >= (100 * std::mem::size_of::<Layer>()) as u64);
            drop(edited);
            drop(original);
        } else {
            assert!(matches!(accounted_layer_access(&doc, leaf, &control()), Err(BackingError::Length)));
            assert!(matches!(AccountedDocument::from_owned(doc, guard, q.clone(), &control()), Err(BackingError::Length)));
        }
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let (mut doc, guard, store, q, _) = fixture();
    let leaf = doc.layers[0].id;
    let template = doc.layers[0].clone();
    doc.layers.extend((0..4096).map(|_| template.clone()));
    drop(template);
    assert!(matches!(accounted_layer_access(&doc, leaf, &control()), Err(BackingError::Integrity)));
    assert!(matches!(AccountedDocument::from_owned(doc, guard, q.clone(), &control()), Err(BackingError::Length)));
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn canonical_lookup_refuses_parent_mismatch_missing_and_duplicate_item_without_allocating() {
    use photocraft_doc::accounted_document::accounted_layer_access;
    for corruption in 0..3 {
        let (mut doc, guard, store, q, leaf, _) = grouped_fixture();
        let state = doc.xcf_state.as_mut().unwrap();
        if corruption == 0 {
            state.items[0].parent = None;
        } else if corruption == 1 {
            state.items.remove(0);
        } else {
            state.items.push(state.items[0].clone());
        }
        let before = q.usage().unwrap();
        assert!(matches!(accounted_layer_access(&doc, leaf, &control()), Err(BackingError::Integrity)));
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        drop(doc);
        drop(guard);
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}

#[test]
fn canonical_ancestor_locks_union_without_sibling_contamination_and_poll_bounded_scans() {
    use photocraft_doc::accounted_document::accounted_layer_access;
    let (mut doc, guard, store, q, leaf, group) = grouped_fixture();
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|i| i.item == ItemRef::Layer(group)).unwrap().locks.visibility = true;
    state.items[0].locks.alpha = true;
    state.items[1].locks.content = true;
    let access = accounted_layer_access(&doc, leaf, &control()).unwrap().unwrap();
    assert!(access.imported_locks.visibility && access.imported_locks.alpha && access.imported_locks.content);
    assert!(!access.imported_locks.position);
    assert!(access.locks.position && access.locks.pixels);
    let outer = accounted_layer_access(&doc, group, &control()).unwrap().unwrap();
    assert!(outer.imported_locks.visibility);
    assert!(!outer.imported_locks.alpha && !outer.imported_locks.content);
    let before = q.usage().unwrap();
    let calls = AtomicU64::new(0);
    let counter = |_| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    accounted_layer_access(&doc, leaf, &ReadControl { interrupt: &Interrupt::NONE, work: &counter }).unwrap();
    for cancel_at in 0..calls.load(Ordering::Relaxed) {
        let step = AtomicU64::new(0);
        let cancel = |_| if step.fetch_add(1, Ordering::Relaxed) == cancel_at { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(matches!(accounted_layer_access(&doc, leaf, &ReadControl { interrupt: &Interrupt::NONE, work: &cancel }), Err(BackingError::Cancelled)));
        assert_eq!(q.usage().unwrap(), before);
    }
    let template = doc.xcf_state.as_ref().unwrap().items[0].clone();
    doc.xcf_state.as_mut().unwrap().items.resize(4097, template);
    assert!(matches!(accounted_layer_access(&doc, leaf, &control()), Err(BackingError::Length)));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(doc);
    drop(guard);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
