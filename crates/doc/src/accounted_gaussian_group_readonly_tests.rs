//! UNRUN structural-custody drafts. Opaque synthetic ICC bytes are not a CMS,
//! source or rendering witness. Expected Document clones are external oracle
//! buffers; fixture storage is genuinely prepaid before its allocations.
use super::*;
use crate::import_compositing::{GimpComposition, ResolvedGimpComposition};
use crate::item_identity::{FilterId, MaskId};
use crate::xcf_state::{ItemState, MaskState};
use crate::{BackedLayer, Group, Layer, LayerMask, Size, SourceColorContext, XcfState};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
pub(super) fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 32 << 20, scratch: 0 })
}
#[derive(Debug)]
pub(super) struct Guard {
    _model: QuotaLease,
    _payload: QuotaLease,
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    _guard: Arc<Guard>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        for (i, byte) in output.iter_mut().enumerate() {
            *byte = offset.wrapping_add(i as u64) as u8;
        }
        Ok(())
    }
}
fn leaf(name: &str, bounds: Rect, q: &BackingQuota, guard: &Arc<Guard>) -> Layer {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut default = [0; 16];
    default[..4].copy_from_slice(&(-0f32).to_ne_bytes());
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let raster = BackedRaster::new(Arc::new(Store { descriptor, _guard: guard.clone() }), q.clone(), 1, &ctl()).unwrap();
    let mut layer = Layer::new(name, LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
    layer.import_compositing =
        Some(crate::ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    layer
}
fn row(layer: &Layer, extent: Rect, parent: Option<ItemRef>, group: bool) -> ItemState {
    let multiply = layer.blend == crate::BlendMode::Multiply;
    ItemState {
        item: ItemRef::Layer(layer.id),
        name: layer.name.clone(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(extent),
        mask: None,
        composition: Some(GimpComposition {
            compatibility_epoch: if group {
                crate::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH
            } else if multiply {
                "xcf-22-authored-blend-auto"
            } else {
                "xcf-22-normal"
            }
            .into(),
            source_mode: if multiply { 30 } else { 28 },
            legacy: false,
            pass_through: false,
            stored_blend_space: if multiply { -1 } else { 0 },
            stored_composite_space: -1,
            stored_operator: if multiply { -2 } else { -1 },
            resolved: Some(ResolvedGimpComposition {
                blend_space: GimpSpace::LinearRgb,
                composite_space: GimpSpace::LinearRgb,
                operator: if multiply { GimpOperator::ClipToBackdrop } else { GimpOperator::Union },
            }),
        }),
    }
}
fn mask_row(id: MaskId, owner: LayerId, extent: Rect, physical: bool) -> ItemState {
    ItemState {
        item: ItemRef::Mask(id),
        name: if physical { "physical group mask" } else { "saved selection" }.into(),
        custom_name: false,
        visible: true,
        locks: Default::default(),
        parent: None,
        source_tattoo: None,
        source_flags: 0,
        raster_bounds: Some(extent),
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(owner),
            bounds: extent,
            offset: if physical { [extent.x0, extent.y0] } else { [0, 0] },
            outside_f64_bits: 0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: physical,
            linked: physical,
        }),
    }
}
pub(super) fn fixture(q: &BackingQuota, edited: bool, enabled: bool) -> (Document, Arc<Guard>) {
    let guard = Arc::new(Guard { _model: q.reserve(QuotaKind::Index, 8 << 20).unwrap(), _payload: q.reserve(QuotaKind::Resident, 8 << 20).unwrap() });
    let mut doc = Document::new("original.pcraft", Size::new(16, 12), ColorMode::Rgb, SampleType::F32);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.icc_profile = Some(Arc::new(vec![0x91; 132]));
    doc.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(doc.icc_profile.as_ref().unwrap());
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 650 });
    let base = leaf("base", doc.bounds(), q, &guard);
    let normal_bounds = Rect::from_xywh(0, 0, 11, 8);
    let multiply_bounds = if edited { Rect::from_xywh(-1, 2, 10, 9) } else { Rect::from_xywh(2, -1, 10, 9) };
    let extent = normal_bounds.union(&multiply_bounds);
    let mut multiply = leaf("multiply", multiply_bounds, q, &guard);
    multiply.blend = crate::BlendMode::Multiply;
    multiply.opacity = if edited { 0.4375 } else { 0.7125 };
    let normal = leaf("normal", normal_bounds, q, &guard);
    let mut group = Layer::new("isolated", LayerContent::Group(Group { children: vec![multiply, normal], expanded: true, artboard: None }));
    group.import_compositing =
        Some(crate::ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    surface.try_write_pixel_f64(extent.x0, extent.y0, &[0.25]).unwrap();
    group.mask = Some(LayerMask { surface: Arc::new(surface), enabled: true, linked: true, density: 1., feather: 0. });
    let LayerContent::Group(tree) = &group.content else { panic!("fixture") };
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    // Intentionally non-tree row order: associations must follow actual IDs.
    state.items = vec![
        row(&tree.children[1], normal_bounds, Some(ItemRef::Layer(group.id)), false),
        mask_row(MaskId(301), group.id, doc.bounds(), false),
        row(&base, doc.bounds(), None, false),
        row(&group, extent, None, true),
        mask_row(MaskId(300), group.id, extent, true),
        row(&tree.children[0], multiply_bounds, Some(ItemRef::Layer(group.id)), false),
    ];
    let arguments = vec![
        FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1.3f32.to_bits())) },
        FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
        FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::filter".into(), value: 0 } },
        FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.8f32.to_bits())) },
        FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "gegl:gaussian-blur::abyss-policy".into(), value: 1 } },
    ];
    let raw = if enabled { -643996976 } else { -1474592175 };
    let node = FilterNode {
        id: FilterId(400),
        operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: FILTER_EPOCH.into() },
        source_display_name: Some("original filter".into()),
        source_icon: None,
        arguments,
        enabled,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(u32::from(edited)),
        mask: Some(ItemRef::Mask(MaskId(301))),
        auxiliary: Vec::new(),
        composition: Some(GimpComposition {
            compatibility_epoch: FILTER_EPOCH.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: raw,
            stored_composite_space: raw,
            stored_operator: raw,
            resolved: None,
            pass_through: false,
        }),
    };
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(group.id), revision: 17, nodes: vec![node] });
    let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    // An allocated raw -0 word remains distinct from an omitted +0 tile. The
    // storage gate must not normalize it or claim this is an empty-mask proof.
    mask.try_write_pixel_f64(1, 1, &[-0.0]).unwrap();
    state.filter_masks.push(crate::FilterMask { id: MaskId(301), owner: FilterId(400), bounds: doc.bounds(), surface: mask });
    state.selection.active = Some(ItemRef::Layer(group.id));
    state.selection.selected.push(ItemRef::Layer(group.id));
    doc.xcf_state = Some(state);
    doc.layers = vec![base, group];
    (doc, guard)
}
pub(super) fn owned(q: &BackingQuota, edited: bool, enabled: bool) -> AccountedDocument {
    let (doc, guard) = fixture(q, edited, enabled);
    AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q.clone(), &ctl()).unwrap()
}

#[test]
fn title_copy_preserves_raw_tree_masks_native_defaults_and_flat_original_custody() {
    for edited in [false, true] {
        for enabled in [false, true] {
            let q = quota();
            let initial = q.usage().unwrap();
            let source = owned(&q, edited, enabled);
            let changed = source.try_with_document_name("continued.pcraft", &ctl()).unwrap();
            let again = changed.try_with_document_name("again.pcraft", &ctl()).unwrap();
            let mut expected = source.document().clone();
            expected.name = "again.pcraft".into();
            assert_eq!(again.document(), &expected);
            again.validate_gaussian_group_storage(&ctl()).unwrap();
            assert!(again.is_readonly_gaussian_group_storage());
            assert!(!again.is_readonly_point_storage());
            assert!(!again.is_readonly_threshold_alpha_storage());
            assert!(Arc::ptr_eq(&source.document().layers[1].mask.as_ref().unwrap().surface, &again.document().layers[1].mask.as_ref().unwrap().surface));
            assert!(Arc::ptr_eq(source.document().icc_profile.as_ref().unwrap(), again.document().icc_profile.as_ref().unwrap()));
            let a = &source.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
            let b = &again.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
            assert_eq!(a.default_bytes(), b.default_bytes());
            for ((ca, ta), (cb, tb)) in a.tiles().zip(b.tiles()) {
                assert_eq!(ca, cb);
                assert!(Arc::ptr_eq(ta, tb));
            }
            let retained = again.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
            assert!(retained.original.shares_model(&source));
            drop(expected); // external oracle also holds native/store Arcs
            drop(source);
            drop(changed);
            again.validate_gaussian_group_storage(&ctl()).unwrap();
            drop(again);
            assert_eq!(q.usage().unwrap(), initial);
        }
    }
}
#[test]
fn exact_recipe_and_associations_refuse_nearby_models_without_owner_publication() {
    for fault in 0..13 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, false, true);
        match fault {
            0 => doc.layers[1].blend = crate::BlendMode::PassThrough,
            1 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.schema_epoch = "other".into(),
            2 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch = "observed-group-v022".into(),
            3 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].value = FilterValue::Bool(false),
            4 => doc.xcf_state.as_mut().unwrap().items[0].parent = None,
            5 => doc.xcf_state.as_mut().unwrap().items[4].mask.as_mut().unwrap().editing = false,
            6 => doc.xcf_state.as_mut().unwrap().filter_masks[0].owner = FilterId(999),
            7 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].mask = Some(ItemRef::Mask(MaskId(300))),
            8 => doc.source_color_context.as_mut().unwrap().source_precision = 600,
            9 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_blend_space = -1474592175,
            10 => doc.layers[1].visible = false,
            11 => {
                let id = doc.xcf_state.as_ref().unwrap().items[4].item;
                doc.xcf_state.as_mut().unwrap().items[1].item = id;
            }
            _ => doc.xcf_state.as_mut().unwrap().current_filter_output_precision = crate::FilterOutputPrecision::Half,
        }
        let before = q.usage().unwrap();
        assert!(AccountedDocument::from_owned_readonly_gaussian_group(doc, guard.clone(), q.clone(), &ctl()).is_err());
        // Failed adoption may free native backing caches/headers; it cannot
        // retain additional owner reservations or refund the still-live guard.
        let after = q.usage().unwrap();
        assert!(after.resident <= before.resident);
        assert!(after.index <= before.index);
        assert!(after.resident >= 16 << 20);
        drop(guard);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}
#[test]
fn full_quota_title_noop_and_all_generic_edit_selection_routes_stay_closed() {
    let q = quota();
    let source = owned(&q, false, true);
    let used = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (128 << 20) - used.resident).unwrap();
    assert!(source.try_with_document_name(&source.document().name, &ctl()).unwrap().shares_model(&source));
    assert!(source.try_with_document_name("changed", &ctl()).is_err());
    let id = source.document().layers[1].id;
    for edit in [
        AccountedEdit::LayerName { layer: id, name: "new" },
        AccountedEdit::MaskLinked { layer: id, linked: false },
        AccountedEdit::NativePixels { layer: id, rect: Rect::from_xywh(0, 0, 1, 1), bytes: &[0; 16] },
    ] {
        assert!(source.try_edit(edit, &ctl()).is_err());
    }
    assert!(source.try_replace_selection(None, &ctl()).is_err());
    assert!(source.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: "new" }, &ctl()).is_err());
    assert_eq!(q.usage().unwrap().resident, 128 << 20);
    drop(blocker);
}
#[test]
fn title_copy_work_cancel_and_failed_private_map_assembly_refund_every_new_charge() {
    let q = quota();
    let source = owned(&q, true, false);
    let before = q.usage().unwrap();
    let work = AtomicU64::new(0);
    let meter = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    source.validate_gaussian_group_storage(&ReadControl { interrupt: &Interrupt::NONE, work: &meter }).unwrap();
    assert!(work.load(Ordering::Relaxed) > 0);
    let denied = |_: u64| Err(BackingError::Quota);
    assert!(source.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &denied }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    // Refuse after actual prospective copy fees become live, leaving source
    // and redo-compatible shared model identity intact. This is an early
    // private-copy rollback case, not an allocator fault or rendering proof.
    let late = |_: u64| if q.usage().unwrap().index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(source.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &late }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(source.document().name, "original.pcraft");
}

#[test]
fn retained_metadata_rejects_unselected_active_nul_and_invalid_hash_without_adoption() {
    for fault in 0..16 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, false, true);
        let base = doc.layers[0].id;
        let state = doc.xcf_state.as_mut().unwrap();
        match fault {
            0 => state.selection.active = Some(ItemRef::Layer(base)),
            1 => state.selection.selected.clear(),
            2 => state.items[0].name = "bad\0item".into(),
            3 => state.filter_stacks[0].nodes[0].source_display_name = Some("bad\0display".into()),
            4 => state.filter_stacks[0].nodes[0].source_icon = Some("bad\0icon".into()),
            5 => state.provenance.container = "bad\0container".into(),
            6 => state.provenance.producer = Some("bad\0producer".into()),
            7 => state.provenance.release_id = Some("bad\0release".into()),
            8 => state.provenance.source_label = "bad\0label".into(),
            9 => state.provenance.original_locator = Some("inert\0locator".into()),
            10 => state.provenance.source_hash = Some("z".repeat(64)),
            11 => state.provenance.protected_source_fingerprints.push("a".repeat(63)),
            12 => state.provenance.source_label = "x".repeat((1 << 20) + 1),
            _ => {
                let bytes = Arc::new(vec![0x17; 128]);
                let digest = blake3::hash(&bytes).to_hex().to_string();
                state.opaque.push(crate::xcf_provenance::OpaqueRecord {
                    owner: Some(ItemRef::Layer(base)),
                    namespace: if fault == 13 { "bad\0namespace" } else { "source-property" }.into(),
                    identity: if fault == 14 { "bad\0identity" } else { "retained" }.into(),
                    source_version: 22,
                    order: 0,
                    flags: 0,
                    bytes,
                    blake3: if fault == 15 { "0".repeat(64) } else { digest },
                    rendering_required: false,
                });
            }
        }
        let before = q.usage().unwrap();
        assert!(AccountedDocument::from_owned_readonly_gaussian_group(doc, guard.clone(), q.clone(), &ctl()).is_err(), "fault {fault}");
        let after = q.usage().unwrap();
        assert!(after.index <= before.index && after.resident <= before.resident);
        drop(guard);
        assert_eq!(q.usage().unwrap(), Default::default());
    }
}

#[test]
fn opaque_digest_validation_is_exact_chunk_metered_and_keeps_valid_inert_evidence_through_title() {
    let q = quota();
    let (mut doc, guard) = fixture(&q, true, false);
    let state = doc.xcf_state.as_mut().unwrap();
    let bytes = Arc::new(vec![0x57; (64 << 10) + 17]);
    let expected = blake3::hash(&bytes).to_hex().to_string();
    // Provenance syntax intentionally accepts uppercase, as the old validator
    // did; opaque digest equality itself must remain exact lowercase.
    state.provenance.source_hash = Some("A".repeat(64));
    state.provenance.protected_source_fingerprints.push("B".repeat(64));
    state.opaque.push(crate::xcf_provenance::OpaqueRecord {
        owner: Some(ItemRef::Layer(doc.layers[0].id)),
        namespace: "source-property".into(),
        identity: "retained-exact-bytes".into(),
        source_version: 22,
        order: 7,
        flags: 0,
        bytes: bytes.clone(),
        blake3: expected.clone(),
        rendering_required: false,
    });
    let owner = AccountedDocument::from_owned_readonly_gaussian_group(doc, guard, q.clone(), &ctl()).unwrap();
    let changed = owner.try_with_document_name("evidence.pcraft", &ctl()).unwrap();
    let raw = &changed.document().xcf_state.as_ref().unwrap().opaque[0];
    assert!(Arc::ptr_eq(&raw.bytes, &bytes));
    assert_eq!(raw.blake3, expected);
    assert_eq!(
        changed.document().xcf_state.as_ref().unwrap().provenance.source_hash.as_deref(),
        Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    );
    assert!(payload_digest(&bytes, &expected.to_uppercase(), &ctl()).is_err());
    let mut corrupted = bytes.as_ref().clone(); // external test-oracle buffer
    corrupted[1] ^= 1;
    assert!(payload_digest(&corrupted, &expected, &ctl()).is_err());
    let charged = AtomicU64::new(0);
    let work = |n| {
        charged.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    payload_digest(&bytes, &expected, &ReadControl { interrupt: &Interrupt::NONE, work: &work }).unwrap();
    assert!(charged.load(Ordering::Relaxed) >= bytes.len() as u64);
    let remaining = AtomicU64::new((32 << 10) + std::mem::size_of::<blake3::Hasher>() as u64 + 512);
    let bounded = |n| remaining.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Quota);
    assert!(payload_digest(&bytes, &expected, &ReadControl { interrupt: &Interrupt::NONE, work: &bounded }).is_err());
    let before = q.usage().unwrap();
    let cancel_on_chunk = |n| if n == (32 << 10) + 1 { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_with_document_name("cancelled", &ReadControl { interrupt: &Interrupt::NONE, work: &cancel_on_chunk }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "original.pcraft");
    // External borrowed/Arc evidence must not outlive the genuine model guard.
    drop(corrupted);
    drop(bytes);
    drop(changed);
    drop(owner);
    assert_eq!(q.usage().unwrap(), Default::default());
}
