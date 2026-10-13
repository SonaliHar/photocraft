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
    metadata(&mut doc, true);
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
fn typed_mask_controls_preserve_physical_surface_and_exact_canonical_history() {
    use photocraft_doc::accounted_document::AccountedHistory;
    let (original, store, q, id) = masked_owner();
    assert_eq!(AccountedEdit::MaskEnabled { layer: id, enabled: false }.layer_id(), id);
    let original_state = original.document().xcf_state.clone();
    let mut history = AccountedHistory::new(original.clone(), 4, &control()).unwrap();
    let unchanged = original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: true }, &control()).unwrap();
    assert!(unchanged.shares_model(&original));
    drop(unchanged);
    history.try_commit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()).unwrap();
    history.try_commit(AccountedEdit::MaskLinked { layer: id, linked: true }, &control()).unwrap();
    let current = history.current().unwrap().clone();
    current.document().validate_xcf_state().unwrap();
    let a = original.document().layers[0].mask.as_ref().unwrap();
    let b = current.document().layers[0].mask.as_ref().unwrap();
    assert!(Arc::ptr_eq(&a.surface, &b.surface));
    assert!(!b.enabled && b.linked);
    assert_eq!((a.density, a.feather), (b.density, b.feather));
    let mut raw = [0; 8];
    b.surface.try_read_pixel_bytes(0, 0, &mut raw).unwrap();
    assert_eq!(raw, 0.25000000000000006f64.to_ne_bytes());
    let mut expected = original_state.clone().unwrap();
    let mask = expected.items.iter_mut().find_map(|i| i.mask.as_mut()).unwrap();
    mask.enabled = false;
    mask.linked = true;
    assert_eq!(current.document().xcf_state.as_ref().unwrap(), &expected);
    history.undo(&control()).unwrap();
    assert!(!history.current().unwrap().document().layers[0].mask.as_ref().unwrap().linked);
    history.undo(&control()).unwrap();
    assert!(history.current().unwrap().shares_model(&original));
    history.redo(&control()).unwrap();
    history.redo(&control()).unwrap();
    assert!(history.current().unwrap().shares_model(&current));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(current);
    drop(history);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn malformed_mask_owner_duplicate_missing_and_flag_divergence_refuse_without_copy() {
    for corruption in 0..6 {
        let (owner, store, q, id) = masked_owner();
        let mut doc = owner.document().clone();
        let state = doc.xcf_state.as_mut().unwrap();
        let index = state.items.iter().position(|i| i.mask.is_some()).unwrap();
        match corruption {
            0 => state.items[index].mask.as_mut().unwrap().owner = ItemRef::Layer(photocraft_doc::LayerId(u64::MAX)),
            1 => state.items.push(state.items[index].clone()),
            2 => {
                state.items.remove(index);
            }
            3 => state.items[index].mask.as_mut().unwrap().enabled = false,
            4 => state.items[index].item = ItemRef::Channel(ChannelId(1)),
            _ => state.items[index].mask.as_mut().unwrap().linked = true,
        }
        // This intentionally malformed model is prepaid by its retained test owner.
        let retained = Arc::new(owner);
        let malformed = AccountedDocument::from_owned(doc, retained, q.clone(), &control()).unwrap();
        let before = q.usage().unwrap();
        assert!(matches!(malformed.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()), Err(BackingError::Integrity)));
        assert!(matches!(malformed.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: true }, &control()), Err(BackingError::Integrity)));
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        drop(malformed);
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn mask_control_quota_denial_and_every_cancellation_fence_preserve_original() {
    let (original, store, q, id) = masked_owner();
    let before = q.usage().unwrap();
    let denial = q.reserve(QuotaKind::Index, (1 << 20) - before.index).unwrap();
    assert!(matches!(original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()), Err(BackingError::Quota)));
    assert!(original.try_edit(AccountedEdit::MaskLinked { layer: id, linked: false }, &control()).unwrap().shares_model(&original));
    drop(denial);
    let calls = AtomicU64::new(0);
    let count = |_| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    let changed = original.try_edit(AccountedEdit::MaskLinked { layer: id, linked: true }, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(changed);
    for cancel_at in 0..calls.load(Ordering::Relaxed) {
        let step = AtomicU64::new(0);
        let cancel = |_| if step.fetch_add(1, Ordering::Relaxed) == cancel_at { Err(BackingError::Cancelled) } else { Ok(()) };
        assert!(matches!(
            original.try_edit(AccountedEdit::MaskLinked { layer: id, linked: true }, &ReadControl { interrupt: &Interrupt::NONE, work: &cancel }),
            Err(BackingError::Cancelled)
        ));
        assert_eq!(q.usage().unwrap(), before);
        assert!(!original.document().layers[0].mask.as_ref().unwrap().linked);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn flag_only_mask_edits_preserve_all_layer_and_mask_locks_without_pixel_or_offset_changes() {
    let (owner, store, q, id) = masked_owner();
    let mut doc = owner.document().clone();
    doc.layers[0].locks = photocraft_doc::Locks { transparency: true, pixels: true, position: true, artboard: true, all: true };
    let state = doc.xcf_state.as_mut().unwrap();
    for item in &mut state.items {
        item.locks = ItemLocks { content: true, position: true, alpha: true, visibility: true };
    }
    let retained = Arc::new(owner);
    let original = AccountedDocument::from_owned(doc, retained, q.clone(), &control()).unwrap();
    let expected_locks = original.document().layers[0].locks;
    let original_state = original.document().xcf_state.clone().unwrap();
    let disabled = original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()).unwrap();
    let linked = disabled.try_edit(AccountedEdit::MaskLinked { layer: id, linked: true }, &control()).unwrap();
    assert_eq!(linked.document().layers[0].locks, expected_locks);
    let mut expected = original_state.clone();
    let mask = expected.items.iter_mut().find_map(|item| item.mask.as_mut()).unwrap();
    mask.enabled = false;
    mask.linked = true;
    assert_eq!(linked.document().xcf_state.as_ref().unwrap(), &expected);
    assert!(Arc::ptr_eq(&original.document().layers[0].mask.as_ref().unwrap().surface, &linked.document().layers[0].mask.as_ref().unwrap().surface));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(linked);
    drop(disabled);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn stateless_flag_controls_preserve_runtime_mask_sample_formats_without_reads() {
    for (sample, raw) in
        [(SampleType::U8, vec![173]), (SampleType::U16, 0xabcd_u16.to_ne_bytes().to_vec()), (SampleType::F32, (-0.0f32).to_ne_bytes().to_vec())]
    {
        let (mut doc, guard, store, q, _) = fixture();
        let id = doc.layers[0].id;
        let surface = photocraft_raster::Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, sample, false), &raw).unwrap();
        doc.layers[0].mask = Some(photocraft_doc::LayerMask { surface: Arc::new(surface), enabled: true, linked: false, density: 0.625, feather: 2.25 });
        let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
        let disabled = original.try_edit(AccountedEdit::MaskEnabled { layer: id, enabled: false }, &control()).unwrap();
        let linked = disabled.try_edit(AccountedEdit::MaskLinked { layer: id, linked: true }, &control()).unwrap();
        let mask = linked.document().layers[0].mask.as_ref().unwrap();
        assert!(!mask.enabled && mask.linked);
        assert_eq!(mask.surface.format().sample, sample);
        assert_eq!(mask.surface.default_bytes(), raw);
        assert!(linked.document().xcf_state.is_none());
        assert_eq!(store.reads.load(Ordering::Relaxed), 0);
        drop(linked);
        drop(disabled);
        drop(original);
        drop(store);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn absent_physical_mask_refuses_explicitly_before_copy_and_preserves_quota() {
    let (doc, guard, store, q, _) = fixture();
    let id = doc.layers[0].id;
    let original = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    for edit in [AccountedEdit::MaskEnabled { layer: id, enabled: false }, AccountedEdit::MaskLinked { layer: id, linked: true }] {
        assert!(matches!(original.try_edit(edit, &control()), Err(BackingError::Unsupported)));
        assert_eq!(q.usage().unwrap(), before);
    }
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    drop(original);
    drop(store);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
