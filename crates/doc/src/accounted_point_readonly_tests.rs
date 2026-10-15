//! UNRUN generic storage drafts. Synthetic records below deliberately include
//! unqualified operation identities: L1 custody is not source/evaluator proof.
use super::*;
use crate::{
    BackedLayer, ColorMode, Layer, PixelFormat, SampleEncoding, SampleType, Size,
    item_identity::{FilterId, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_geom::TileCoord;
use photocraft_raster::{
    Interrupt,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 0 })
}
struct SourceGuard {
    _fee: photocraft_raster::backing::quota::QuotaLease,
}
struct Store {
    descriptor: NativePlaneDescriptor,
    _guard: Arc<SourceGuard>,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("descriptor", &self.descriptor).finish_non_exhaustive()
    }
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, sample: SampleType, encoding: SampleEncoding, source_precision: u32) -> (Document, Arc<SourceGuard>) {
    // Actual source/model/mask/default/profile fixture storage is prepaid before
    // allocations. A store retains this same original guard; it contains no Doc.
    let guard = Arc::new(SourceGuard { _fee: q.reserve(QuotaKind::Resident, 8 << 20).unwrap() });
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let bounds = Rect::new(-13, 13, 244, 142);
    let mut default = vec![0; format.bytes_per_pixel()];
    if sample == SampleType::F64 {
        default[..8].copy_from_slice(&(-0f64).to_ne_bytes());
    }
    let descriptor = NativePlaneDescriptor::new(format, bounds, &default).unwrap();
    let backed = BackedRaster::new(Arc::new(Store { descriptor, _guard: guard.clone() }), q.clone(), 1, &ctl()).unwrap();
    let layer = Layer::new("original layer", LayerContent::BackedRaster(BackedLayer::new(backed, descriptor, &ctl()).unwrap()));
    let layer_id = layer.id;
    let mut doc = Document::new("original title", Size::new(295, 160), ColorMode::Rgb, sample);
    doc.layers.push(layer);
    doc.sample_encoding = Some(encoding);
    doc.source_color_context = Some(crate::SourceColorContext { original_icc: None, source_precision });
    doc.icc_profile = Some(Arc::new(vec![1, 2, 3, 0])); // opaque custody, no ICC validity assertion
    let canvas = doc.bounds();
    let filter = FilterId(100);
    let mask = MaskId(101);
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    state.provenance.original_locator = Some("never-read://original".into());
    let arguments = (0..10)
        .map(|i| FilterArgument {
            name: format!("raw-{i}"),
            value: match i % 3 {
                0 => FilterValue::Number(ExactNumber::Float32Bits(if i == 0 { (-0f32).to_bits() } else { 0x7fc01234 })),
                1 => FilterValue::Bool(i % 2 == 0),
                _ => FilterValue::Enum { domain: "opaque-stored-enum".into(), value: i },
            },
        })
        .collect();
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(layer_id),
        revision: u64::MAX,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity {
                name: "unqualified:raw-point".into(),
                stored_version: "opaque-version".into(),
                schema_epoch: "opaque-schema".into(),
            },
            source_display_name: Some("raw source name".into()),
            source_icon: Some("raw source icon".into()),
            arguments,
            enabled: false,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(crate::import_compositing::GimpComposition {
                compatibility_epoch: "opaque-unresolved-epoch".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: i32::MIN,
                stored_composite_space: 17,
                stored_operator: -91,
                resolved: None,
                pass_through: false,
            }),
        }],
    });
    let mask_format = PixelFormat::new(ColorMode::Grayscale, sample, false);
    let mut mask_default = vec![0; mask_format.bytes_per_pixel()];
    match sample {
        SampleType::F32 => mask_default.copy_from_slice(&(-0f32).to_ne_bytes()),
        SampleType::F64 => mask_default.copy_from_slice(&(-0f64).to_ne_bytes()),
        _ => {}
    }
    let mut surface = Surface::try_with_default_bytes(mask_format, &mask_default).unwrap();
    // The right-edge tile has raw off-canvas padding. A storage clone must never
    // prune/normalize it, even though a later semantic mask gate may refuse it.
    let tile = surface.try_tile_mut(TileCoord::new(1, 0)).unwrap();
    *tile.bytes_mut().last_mut().unwrap() = 0x91;
    state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds: canvas, surface });
    state.items.push(ItemState {
        item: ItemRef::Mask(mask),
        name: "raw effect mask".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(17),
        source_flags: 0,
        raster_bounds: Some(canvas),
        composition: None,
        mask: Some(MaskState {
            owner: ItemRef::Layer(layer_id),
            bounds: canvas,
            offset: [0, 0],
            outside_f64_bits: 0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: false,
        }),
    });
    state.items.push(ItemState {
        item: ItemRef::Layer(layer_id),
        name: "original layer".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(18),
        source_flags: 0,
        raster_bounds: Some(bounds),
        composition: None,
        mask: None,
    });
    state.selection.selected.push(ItemRef::Layer(layer_id));
    state.selection.active = Some(ItemRef::Layer(layer_id));
    doc.xcf_state = Some(state);
    (doc, guard)
}
fn owned(sample: SampleType, encoding: SampleEncoding, precision: u32) -> AccountedDocument {
    let q = quota();
    let (doc, guard) = fixture(&q, sample, encoding, precision);
    AccountedDocument::new_readonly_point_storage(doc, guard, q, &ctl()).unwrap()
}
#[test]
fn all_canonical_depths_and_encodings_keep_literal_half_correspondence_and_distinct_bounds() {
    for (sample, precisions) in [
        (SampleType::U8, [100, 150, 175]),
        (SampleType::U16, [200, 250, 275]),
        (SampleType::U32, [300, 350, 375]),
        (SampleType::F32, [600, 650, 675]),
        (SampleType::F64, [700, 750, 775]),
        (SampleType::F32, [500, 550, 575]),
    ] {
        for (encoding, precision) in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual].into_iter().zip(precisions) {
            let owner = owned(sample, encoding, precision);
            assert!(owner.is_readonly_point_storage());
            owner.validate_readonly_point_storage(&ctl()).unwrap();
            let LayerContent::BackedRaster(backed) = &owner.document().layers[0].content else { panic!() };
            assert_eq!(backed.operative_bounds(), Rect::new(-13, 13, 244, 142));
            assert_ne!(backed.operative_bounds(), owner.document().bounds());
            assert_eq!(owner.document().xcf_state.as_ref().unwrap().filter_masks[0].bounds, owner.document().bounds());
            assert_eq!(owner.document().source_color_context.as_ref().unwrap().source_precision, precision);
        }
    }
}
#[test]
fn title_copy_preserves_all_raw_fields_padding_native_profile_and_actual_original_guard() {
    let q = quota();
    let (doc, guard) = fixture(&q, SampleType::F64, SampleEncoding::NonlinearProfile, 750);
    let weak = Arc::downgrade(&guard);
    let before = doc.clone();
    let old = AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    let new = old.try_with_document_name("new title", &ctl()).unwrap();
    let mut expected = before.clone();
    expected.name = "new title".into();
    assert_eq!(new.document(), &expected);
    assert_eq!(old.document(), &before);
    assert!(new.quota().shares_ledger(&q));
    let a = &old.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &new.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert!(Arc::ptr_eq(a.tile(TileCoord::new(1, 0)).unwrap(), b.tile(TileCoord::new(1, 0)).unwrap()));
    assert_eq!(a.tile(TileCoord::new(1, 0)).unwrap().bytes().last(), Some(&0x91));
    assert_eq!(a.default_bytes(), b.default_bytes());
    assert!(Arc::ptr_eq(old.document().icc_profile.as_ref().unwrap(), new.document().icc_profile.as_ref().unwrap()));
    drop(before);
    drop(expected);
    drop(old);
    assert!(weak.upgrade().is_some());
    drop(new);
    assert!(weak.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
fn requirements_are_not_a_receipt_noop_is_shared_and_generic_mutations_stay_closed() {
    let q = quota();
    let (doc, guard) = fixture(&q, SampleType::U32, SampleEncoding::Perceptual, 375);
    let before = q.usage().unwrap();
    let required = AccountedDocument::readonly_point_storage_requirements(&doc, &q, &ctl()).unwrap();
    assert_eq!(required, AccountedDocument::owned_model_reservation_bytes());
    assert_eq!(q.usage().unwrap(), before);
    let owner = AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    let baseline = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    let full = q.usage().unwrap();
    let noop = owner.try_with_document_name("original title", &ctl()).unwrap();
    assert!(noop.shares_model(&owner));
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let id = owner.document().layers[0].id;
    assert!(owner.try_edit(AccountedEdit::LayerName { layer: id, name: "generic" }, &ctl()).is_err());
    assert!(owner.try_replace_selection(None, &ctl()).is_err());
    assert!(owner.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: "not yet" }, &ctl()).is_err());
    assert!(
        owner
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: id, name: None, visible: Some(false), opacity_f32_bits: None }, &ctl())
            .is_err()
    );
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn wrong_quota_layout_precision_mask_ids_and_nested_or_oversized_arguments_refuse() {
    for case in 0..9 {
        let q = quota();
        let _outside = q.reserve(QuotaKind::Index, 256).unwrap();
        let original_baseline = q.usage().unwrap();
        let (mut doc, guard) = fixture(&q, SampleType::U8, SampleEncoding::NonlinearProfile, 150);
        let canvas = doc.bounds();
        match case {
            0 => doc.source_color_context.as_mut().unwrap().source_precision = 250,
            1 => doc.xcf_state.as_mut().unwrap().items[1].raster_bounds = Some(canvas),
            2 => doc.xcf_state.as_mut().unwrap().filter_masks[0].bounds = Rect::new(0, 0, 8, 6),
            3 => doc.xcf_state.as_mut().unwrap().filter_masks[0].owner = FilterId(0),
            4 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0]
                .arguments
                .push(FilterArgument { name: "eleventh".into(), value: FilterValue::Bool(true) }),
            5 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Config(Vec::new()),
            6 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
            7 => doc.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_tile_mut(TileCoord::new(2, 0)).map(|_| ()).unwrap(),
            _ => {}
        }
        let supplied = if case == 8 { quota() } else { q.clone() };
        assert!(AccountedDocument::new_readonly_point_storage(doc, guard, supplied.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), original_baseline);
        if case == 8 {
            assert_eq!(supplied.usage().unwrap(), Default::default());
        }
    }
}
#[test]
fn title_work_cancel_and_early_or_after_fees_fail_privately_and_refund() {
    let owner = owned(SampleType::F32, SampleEncoding::Linear, 500);
    let q = owner.quota().clone();
    let before = q.usage().unwrap();
    let failed = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_document_name("new", &failed), Err(BackingError::Work)));
    let counter = AtomicU64::new(0);
    let cancelled = || counter.fetch_add(1, Ordering::Relaxed) > 12;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(owner.try_with_document_name("new", &c), Err(BackingError::Cancelled)));
    let after_fees = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&after_fees);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(owner.try_with_document_name("new", &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "original title");
}
#[test]
fn repeated_titles_keep_flat_original_retention_without_enrolling_an_operation() {
    let original = owned(SampleType::F64, SampleEncoding::Perceptual, 775);
    let q = original.quota().clone();
    let mut current = original.clone();
    let mut previous = None;
    for i in 0..32 {
        current = current.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &ctl()).unwrap();
        let usage = q.usage().unwrap();
        if let Some(previous) = previous {
            assert_eq!(usage, previous);
        }
        previous = Some(usage);
        let root = current.owner.retention.downcast_ref::<OriginalRetention>().unwrap();
        assert!(root.original.shares_model(&original));
    }
    assert_eq!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].operation.name, "unqualified:raw-point");
    assert_eq!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, u64::MAX);
}
