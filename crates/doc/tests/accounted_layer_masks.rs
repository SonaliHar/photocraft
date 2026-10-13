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

fn masked_owner() -> (AccountedDocument, Arc<Store>, BackingQuota, photocraft_doc::LayerId) {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, false);
    let id = doc.layers[0].id;
    let original_mask_payload = q.reserve(QuotaKind::Resident, (256 * 256 * 8) + 4096).unwrap();
    let mut surface =
        photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
    let exact = 0.25000000000000006f64.to_ne_bytes();
    surface.try_write_interleaved_with(Rect::new(0, 0, 1, 1), &exact, &Interrupt::NONE).unwrap();
    doc.layers[0].mask = Some(photocraft_doc::LayerMask { surface: std::sync::Arc::new(surface), enabled: true, linked: false, density: 0.75, feather: 1.5 });
    doc.xcf_state.as_mut().unwrap().items.push(ItemState {
        item: ItemRef::Mask(MaskId(1)),
        name: "original layer mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: None,
        composition: None,
        mask: Some(photocraft_doc::xcf_state::MaskState {
            owner: ItemRef::Layer(id),
            bounds: Rect::new(-1, -1, 3, 2),
            offset: [0, 0],
            outside_f64_bits: (-0.0f64).to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    });
    doc.validate_xcf_state().unwrap();
    let owner = AccountedDocument::from_owned(doc, Arc::new((guard, original_mask_payload)), q.clone(), &control()).unwrap();
    (owner, store, q, id)
}

#[test]
fn current_layer_mask_and_original_xcf_mask_survive_accounted_history_without_tile_copy() {
    let (owner, store, q, id) = masked_owner();
    let exact = 0.25000000000000006f64.to_ne_bytes();
    let before = q.usage().unwrap();
    let changed = owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap();
    let after = q.usage().unwrap();
    assert!(after.resident - before.resident < 256 * 256 * 8, "shared tile payload must not be cloned/charged twice");
    let old = owner.document().layers[0].mask.as_ref().unwrap();
    let new = changed.document().layers[0].mask.as_ref().unwrap();
    assert_eq!(old, new);
    assert!(Arc::ptr_eq(&old.surface, &new.surface));
    assert!(Arc::ptr_eq(old.surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap(), new.surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap()));
    let mut raw = [0; 8];
    new.surface.try_read_pixel_bytes(0, 0, &mut raw).unwrap();
    assert_eq!(raw, exact);
    new.surface.try_read_pixel_bytes(-1, -1, &mut raw).unwrap();
    assert_eq!(raw, (-0.0f64).to_ne_bytes());
    assert_eq!(changed.document().xcf_state, owner.document().xcf_state);
    changed.document().validate_xcf_state().unwrap();
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn mask_source_retention_survives_current_name_visibility_history_and_weak_drop() {
    let (owner, store, q, id) = masked_owner();
    let weak = owner.downgrade();
    let original_mask_state = owner.document().xcf_state.as_ref().unwrap().items[1].clone();
    let mut history = photocraft_doc::accounted_document::AccountedHistory::new(owner.clone(), 4, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerName { layer: id, name: "current mask owner" }, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap();
    let source = owner.document().layers[0].mask.as_ref().unwrap();
    let current = history.current().unwrap().document().layers[0].mask.as_ref().unwrap();
    assert!(Arc::ptr_eq(&source.surface, &current.surface));
    assert_eq!(history.current().unwrap().document().xcf_state.as_ref().unwrap().items[1], original_mask_state);
    assert!(history.undo(&control()).unwrap());
    assert_eq!(history.current().unwrap().document().layers[0].name, "current mask owner");
    assert!(history.redo(&control()).unwrap());
    let retained = history.current().unwrap().clone();
    drop(history);
    drop(owner);
    assert!(weak.upgrade().is_none());
    // Current immutable mask/source guard outlives the original model/history.
    let mut bytes = [0; 8];
    retained.document().layers[0].mask.as_ref().unwrap().surface.try_read_pixel_bytes(0, 0, &mut bytes).unwrap();
    assert_eq!(bytes, 0.25000000000000006f64.to_ne_bytes());
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(retained);
    drop(store);
    let weak_only = q.usage().unwrap();
    assert_eq!(weak_only.resident, weak_only.index);
    assert!(weak_only.index > 0);
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn mask_copy_quota_denial_and_all_cancellation_checkpoints_preserve_old_peak() {
    let (owner, store, q, id) = masked_owner();
    let before = q.usage().unwrap();
    let held = q.reserve(QuotaKind::Index, (1 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    assert_eq!(owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &control()).unwrap_err(), BackingError::Quota);
    assert_eq!(q.usage().unwrap(), full);
    let same = owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: true }, &control()).unwrap();
    assert!(same.shares_model(&owner));
    drop(same);
    drop(held);
    let calls = AtomicU64::new(0);
    let count = |_: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let probe = ReadControl { interrupt: &Interrupt::NONE, work: &count };
    let changed = owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &probe).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let checkpoints = calls.load(Ordering::Relaxed);
    assert!(checkpoints > 20);
    for threshold in 1..=checkpoints {
        let calls = AtomicU64::new(0);
        let cancel = |_: u64| {
            if calls.fetch_add(1, Ordering::Relaxed) + 1 >= threshold { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let controlled = ReadControl { interrupt: &Interrupt::NONE, work: &cancel };
        assert_eq!(owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &controlled).unwrap_err(), BackingError::Cancelled);
        assert_eq!(q.usage().unwrap(), before);
        assert!(owner.document().layers[0].visible);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn native_raster_cow_keeps_shared_mask_native_bits_and_source_metadata() {
    let (owner, store, q, id) = masked_owner();
    let original = owner.document().xcf_state.clone();
    // Establish the shared native tile cache before measuring COW delta.
    // Its separately charged immutable source cache remains with the old owner.
    let LayerContent::BackedRaster(initial) = &owner.document().layers[0].content else { panic!("native storage lost") };
    let mut original_raw = [0; 32];
    initial.checked_view().read_pixel_native_into(0, 0, &mut original_raw, &control()).unwrap();
    let before = q.usage().unwrap();
    let raw = [0.5000000000000001f64, -0.0, 0.25000000000000006, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let changed = owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &raw }, &control()).unwrap();
    assert!(Arc::ptr_eq(&owner.document().layers[0].mask.as_ref().unwrap().surface, &changed.document().layers[0].mask.as_ref().unwrap().surface));
    let LayerContent::BackedRaster(current) = &changed.document().layers[0].content else { panic!("native storage lost") };
    let mut current_raw = [0; 32];
    current.checked_view().read_pixel_native_into(0, 0, &mut current_raw, &control()).unwrap();
    assert_eq!(current_raw.as_slice(), raw);
    let mut mask_raw = [0; 8];
    changed.document().layers[0].mask.as_ref().unwrap().surface.try_read_pixel_bytes(0, 0, &mut mask_raw).unwrap();
    assert_eq!(mask_raw, 0.25000000000000006f64.to_ne_bytes());
    assert_eq!(changed.document().xcf_state, original);
    assert!(q.usage().unwrap().resident > before.resident);
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn malformed_mask_tile_length_and_hostile_origins_refuse_before_model_copy() {
    for (coordinate, expected_error) in
        [(photocraft_geom::TileCoord::new(0, 0), BackingError::Integrity), (photocraft_geom::TileCoord::new(i32::MAX, 0), BackingError::Bounds)]
    {
        let (mut doc, guard, store, q, _) = fixture();
        let id = doc.layers[0].id;
        let payload = q.reserve(QuotaKind::Resident, (256 * 256) + 4096).unwrap();
        let mut encoded_u8 = photocraft_raster::Surface::new(PixelFormat::GRAY8);
        encoded_u8.tile_mut(photocraft_geom::TileCoord::new(0, 0));
        let tile = encoded_u8.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap().clone();
        let mut declared_f64 =
            photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &[0; 8]).unwrap();
        declared_f64.put_tiles([(coordinate, tile)]);
        drop(encoded_u8);
        doc.layers[0].mask = Some(photocraft_doc::LayerMask { surface: Arc::new(declared_f64), enabled: true, linked: true, density: 1.0, feather: 0.0 });
        let owner = AccountedDocument::from_owned(doc, Arc::new((guard, payload)), q.clone(), &control()).unwrap();
        let before = q.usage().unwrap();
        assert_eq!(owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap_err(), expected_error);
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        drop(owner);
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
