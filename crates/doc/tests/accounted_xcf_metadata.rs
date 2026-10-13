use photocraft_doc::{
    BackedLayer, ColorMode, Document, Layer, LayerContent, PixelFormat, SampleType, Size,
    accounted_document::{AccountedDocument, AccountedEdit, AccountedHistory},
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
fn pixel(doc: &Document) -> Vec<u8> {
    let LayerContent::BackedRaster(backed) = &doc.layers[0].content else { panic!("backing lost") };
    let mut bytes = vec![0; backed.format().bytes_per_pixel()];
    backed.checked_view().read_pixel_native_into(0, 0, &mut bytes, &control()).unwrap();
    bytes
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
#[test]
fn verified_project_state_current_controls_and_shared_profile_survive_charged_history() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    let id = doc.layers[0].id;
    let original_state = doc.xcf_state.clone().unwrap();
    let original_profile = doc.source_color_context.as_ref().unwrap().original_icc.clone().unwrap();
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let mut history = AccountedHistory::new(owner.clone(), 4, &control()).unwrap();
    history.try_commit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.25 }, &control()).unwrap();
    let changed = history.current().unwrap();
    assert_eq!(changed.document().xcf_state.as_ref().unwrap(), &original_state);
    assert!(Arc::ptr_eq(changed.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(), &original_profile));
    assert_eq!(changed.document().layers[0].opacity, 0.25);
    changed.document().validate_xcf_state().unwrap();
    history.try_commit(AccountedEdit::LayerName { layer: id, name: "current renamed" }, &control()).unwrap();
    assert_eq!(history.current().unwrap().document().xcf_state.as_ref().unwrap().items[0].name, "current renamed");
    assert_eq!(owner.document().xcf_state.as_ref().unwrap(), &original_state);
    assert!(history.undo(&control()).unwrap());
    assert_eq!(history.current().unwrap().document().layers[0].opacity, 0.25);
    assert!(history.redo(&control()).unwrap());
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(original_profile);
    drop(history);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn canonical_metadata_is_preserved_across_native_cow_without_source_rewrite() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, false);
    let id = doc.layers[0].id;
    let original = doc.xcf_state.clone();
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let bytes = [0.25000000000000006f64, -0.0, 0.75, 0.5].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let changed = owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &bytes }, &control()).unwrap();
    assert_eq!(changed.document().xcf_state, original);
    assert_eq!(pixel(changed.document()), bytes);
    assert_ne!(pixel(owner.document()), bytes);
    drop(changed);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn canonical_copy_denial_and_noop_preserve_source_without_reads() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let held = q.reserve(QuotaKind::Index, (1 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    let noop = owner.try_edit(AccountedEdit::LayerName { layer: id, name: "current layer" }, &control()).unwrap();
    assert!(owner.shares_model(&noop));
    assert_eq!(q.usage().unwrap(), full);
    assert_eq!(owner.try_edit(AccountedEdit::LayerName { layer: id, name: "changed" }, &control()).unwrap_err(), BackingError::Quota);
    assert_eq!(q.usage().unwrap(), full);
    assert_eq!(owner.document().xcf_state.as_ref().unwrap().items[0].name, "current layer");
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(held);
    drop(noop);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn every_canonical_census_copy_and_publish_checkpoint_is_cancellable_and_refunds() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let calls = AtomicU64::new(0);
    let probe = |_: u64| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let counted = ReadControl { interrupt: &Interrupt::NONE, work: &probe };
    let changed = owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &counted).unwrap();
    assert!(!changed.document().xcf_state.as_ref().unwrap().items[0].visible);
    changed.document().validate_xcf_state().unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let checkpoints = calls.load(Ordering::Relaxed);
    assert!(checkpoints > 50);
    for threshold in 1..=checkpoints {
        let calls = AtomicU64::new(0);
        let cancel = |_: u64| {
            if calls.fetch_add(1, Ordering::Relaxed) + 1 >= threshold { Err(BackingError::Cancelled) } else { Ok(()) }
        };
        let cancelled = ReadControl { interrupt: &Interrupt::NONE, work: &cancel };
        assert_eq!(
            owner.try_edit(AccountedEdit::LayerVisibility { layer: id, visible: false }, &cancelled).unwrap_err(),
            BackingError::Cancelled,
            "checkpoint {threshold}"
        );
        assert_eq!(q.usage().unwrap(), before, "checkpoint {threshold}");
        assert!(owner.document().layers[0].visible);
        assert!(owner.document().xcf_state.as_ref().unwrap().items[0].visible);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn owned_resource_bytes_keep_conservative_payload_charges_and_share_allocations() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    // This original physical payload is prepaid before allocation and remains
    // retained alongside the codec guard during ownership transfer.
    let original_payload = q.reserve(QuotaKind::Resident, 128 << 10).unwrap();
    let bytes = vec![0x42; 128 << 10];
    doc.xcf_state.as_mut().unwrap().resources.push(ResourceReference {
        id: ResourceId(2),
        kind: ResourceKind::Pattern,
        name: "retained pattern".into(),
        collection: None,
        internal: false,
        locator: None,
        content_hash: Some(blake3::hash(&bytes).to_hex().to_string()),
        owned_bytes: Some(bytes.into()),
        resolution: None,
    });
    doc.validate_xcf_state().unwrap();
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, Arc::new((guard, original_payload)), q.clone(), &control()).unwrap();
    let baseline = q.usage().unwrap();
    let changed = owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap();
    let current = q.usage().unwrap();
    // The existing conservative lease remains; it does not imply a second allocation.
    assert!(current.resident - baseline.resident >= 128 << 10);
    assert!(current.index - baseline.index < 32 << 10);
    let old = &owner.document().xcf_state.as_ref().unwrap().resources[1];
    let new = &changed.document().xcf_state.as_ref().unwrap().resources[1];
    assert_eq!(old, new);
    assert!(Arc::ptr_eq(old.owned_bytes.as_ref().unwrap(), new.owned_bytes.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        owner.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        changed.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    drop(changed);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn recursive_configuration_limits_and_source_pixel_refusal_precede_clone_or_reads() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    let id = doc.layers[0].id;
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert_eq!(
        owner.try_edit(AccountedEdit::NativePixels { layer: id, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 32] }, &control()).unwrap_err(),
        BackingError::Unsupported
    );
    assert_eq!(q.usage().unwrap(), before);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, true);
    let id = doc.layers[0].id;
    let mut value = FilterValue::Bool(false);
    for _ in 0..34 {
        value = FilterValue::Config(vec![FilterArgument { name: "nested".into(), value }]);
    }
    let EditableSource::Text { configuration, .. } = &mut doc.xcf_state.as_mut().unwrap().source_items[0].source else { panic!("text source lost") };
    *configuration = vec![FilterArgument { name: "root".into(), value }];
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let baseline = q.usage().unwrap();
    assert_eq!(owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap_err(), BackingError::Length);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn canonical_filter_mask_storage_is_explicitly_unsupported_before_copy() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, false);
    let id = doc.layers[0].id;
    doc.xcf_state.as_mut().unwrap().filter_masks.push(photocraft_doc::FilterMask {
        id: MaskId(1),
        owner: FilterId(1),
        surface: photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false)),
        bounds: Rect::new(0, 0, 1, 1),
    });
    // This ownership test intentionally makes no renderer/relationship claim.
    // The independent Surface storage must be refused even without reading it.
    let owner = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let baseline = q.usage().unwrap();
    assert_eq!(owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap_err(), BackingError::Unsupported);
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn embedded_profile_bound_is_enforced_before_model_copy() {
    let (mut doc, guard, store, q, _) = fixture();
    metadata(&mut doc, false);
    let id = doc.layers[0].id;
    let length = (16 << 20) + 1;
    let original_payload = q.reserve(QuotaKind::Resident, length as u64).unwrap();
    doc.xcf_state.as_mut().unwrap().resources.push(ResourceReference {
        id: ResourceId(1),
        kind: ResourceKind::ColorProfile,
        name: "oversized profile".into(),
        collection: None,
        internal: false,
        locator: None,
        content_hash: None,
        owned_bytes: Some(vec![0; length].into()),
        resolution: None,
    });
    // Ownership can retain a prepaid caller model; copy admission applies its
    // profile ceiling without reading/hash validation or cloning this payload.
    let owner = AccountedDocument::from_owned(doc, Arc::new((guard, original_payload)), q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    assert_eq!(owner.try_edit(AccountedEdit::LayerOpacity { layer: id, opacity: 0.5 }, &control()).unwrap_err(), BackingError::Length);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(owner);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
