//! Ignored UNRUN/uncompiled structural hierarchy contracts. Procedural paid
//! CURRENT data is neither a GIMP Source fixture nor a profile/math oracle.
use super::*;
use crate::{
    BackedLayer, ColorMode, Group, Layer, PixelFormat, SampleEncoding, SampleType, Size, SourceColorContext,
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{FilterId, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    xcf_provenance::OpaqueRecord,
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { index: 64 << 20, resident: 128 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Hold {
    _index: QuotaLease,
    _resident: QuotaLease,
}
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    q: BackingQuota,
    _hold: Arc<Hold>,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.q.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(out.len() as u64)?;
        if offset.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        out.fill(0);
        Ok(())
    }
}
fn composition() -> GimpComposition {
    GimpComposition {
        compatibility_epoch: "xcf-22-normal".into(),
        source_mode: 28,
        legacy: false,
        stored_blend_space: 0,
        stored_composite_space: -1,
        stored_operator: -1,
        resolved: Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union }),
        pass_through: false,
    }
}
fn fixture(q: &BackingQuota, sample: SampleType) -> (Document, Arc<Hold>) {
    // Real held bounds precede all model/vector/ICC/default/mask maps and data.
    let hold = Arc::new(Hold { _index: q.reserve(QuotaKind::Index, 16 << 20).unwrap(), _resident: q.reserve(QuotaKind::Resident, 16 << 20).unwrap() });
    let mut d = Document::new("tree", Size::new(8, 6), ColorMode::Rgb, sample);
    d.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    d.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if sample == SampleType::F32 { 650 } else { 750 } });
    let icc = vec![1, 2, 3, 4];
    d.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&icc);
    d.icc_profile = Some(Arc::new(icc)); // data-only actual marker, not parsed CMS
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let canvas = d.bounds();
    let leaf = |id: u64| {
        let mut default = vec![0; format.bytes_per_pixel()];
        default[0] = 0x91;
        let descriptor = NativePlaneDescriptor::new(format, canvas, &default).unwrap();
        let native = BackedRaster::new(Arc::new(Store { descriptor, q: q.clone(), _hold: hold.clone() }), q.clone(), 1, &ctl()).unwrap();
        let mut l = Layer::new(format!("leaf{id}"), LayerContent::BackedRaster(BackedLayer::new(native, descriptor, &ctl()).unwrap()));
        l.id = LayerId(id);
        l.opacity = 0.375;
        l
    };
    let mut empty = Layer::new("empty", LayerContent::Group(Group { children: vec![], expanded: false, artboard: None }));
    empty.id = LayerId(12);
    let mut inner = Layer::new("inner", LayerContent::Group(Group { children: vec![leaf(2), empty], expanded: false, artboard: None }));
    inner.id = LayerId(11);
    inner.opacity = 0.5;
    let mut outer = Layer::new("outer", LayerContent::Group(Group { children: vec![leaf(1), inner], expanded: false, artboard: None }));
    outer.id = LayerId(10);
    d.layers = vec![outer, leaf(3)];
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    fn rows(layers: &[Layer], parent: Option<ItemRef>, canvas: Rect, out: &mut Vec<ItemState>) {
        for l in layers {
            out.push(ItemState {
                item: ItemRef::Layer(l.id),
                name: l.name.clone(),
                custom_name: false,
                visible: l.visible,
                locks: ItemLocks::default(),
                parent,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: if l.is_group() { None } else { Some(canvas) },
                composition: Some(composition()),
                mask: None,
            });
            if let LayerContent::Group(g) = &l.content {
                rows(&g.children, Some(ItemRef::Layer(l.id)), canvas, out);
            }
        }
    }
    rows(&d.layers, None, canvas, &mut state.items);
    let mut nodes = Vec::new();
    for i in 0..2u64 {
        let fid = FilterId(100 + i);
        let mid = MaskId(200 + i);
        nodes.push(FilterNode {
            id: fid,
            operation: OperationIdentity { name: "unqualified:primitive".into(), stored_version: "0:0".into(), schema_epoch: "data-only".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument { name: "scalar".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits())) }],
            enabled: i == 0,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mid)),
            auxiliary: vec![],
            composition: Some(GimpComposition {
                compatibility_epoch: "data-only".into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: i32::MIN,
                stored_composite_space: 17,
                stored_operator: -91,
                resolved: None,
                pass_through: false,
            }),
        });
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        surface.try_tile_mut(photocraft_geom::TileCoord::new(0, 0)).unwrap();
        state.filter_masks.push(FilterMask { id: mid, owner: fid, surface, bounds: canvas });
        state.items.push(ItemState {
            item: ItemRef::Mask(mid),
            name: "filter mask".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(canvas),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(LayerId(2)),
                bounds: canvas,
                offset: [0, 0],
                outside_f64_bits: 0,
                enabled: true,
                shown: false,
                editing: false,
                linked: false,
            }),
        });
        let bytes = Arc::new(vec![0, 0x91, i as u8, 255]);
        state.opaque.push(OpaqueRecord {
            owner: Some(ItemRef::Mask(mid)),
            namespace: "gimp/xcf/source-property".into(),
            identity: format!("raw/μ/{i}"),
            source_version: 22,
            order: i as u32,
            flags: 7,
            blake3: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
            rendering_required: false,
        });
    }
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(LayerId(2)), nodes, revision: 7 });
    state.selection.selected = vec![ItemRef::Layer(LayerId(11)), ItemRef::Layer(LayerId(2))];
    state.selection.active = Some(ItemRef::Layer(LayerId(2)));
    d.xcf_state = Some(state);
    (d, hold)
}
fn owned(q: &BackingQuota, s: SampleType) -> AccountedDocument {
    let (d, h) = fixture(q, s);
    AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, h, q.clone(), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN/uncompiled: real same-q tree/native custody; empty group None bounds and paid ordinals, no producer/profile/math grant"]
fn actual_tree_index_canonical_none_and_direct_retention() {
    for s in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let baseline = q.usage().unwrap();
        let owner = owned(&q, s);
        let index = owner.checked_readonly_normal_hierarchy_index(&ctl()).unwrap();
        assert_eq!(index.len(), 6);
        assert_eq!(index.group_count(), 3);
        assert_eq!(index.leaf_count(), 3);
        assert_eq!(index.max_depth(), 3);
        assert_eq!(index.row(3).unwrap().id, LayerId(2));
        assert_eq!(index.row(3).unwrap().parent, Some(2));
        assert_eq!(index.row(4).unwrap().operative_bounds, Rect::new(0, 0, 0, 0));
        assert_eq!(index.row(0).unwrap().subtree_end, 5);
        assert!(index.matches_document(owner.document()));
        assert!(!owner.is_readonly_filtered_flat_point_storage());
        assert!(!owner.is_readonly_masked_filtered_flat_point_storage());
        assert!(!owner.is_readonly_point_storage());
        drop(index);
        let renamed = owner.try_with_document_name("renamed", &ctl()).unwrap();
        assert_eq!(renamed.document().layers, owner.document().layers);
        assert!(Arc::ptr_eq(renamed.document().icc_profile.as_ref().unwrap(), owner.document().icc_profile.as_ref().unwrap()));
        drop(owner);
        renamed.validate_readonly_normal_hierarchy_storage(&ctl()).unwrap();
        drop(renamed);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: nonfirst nested stable target; compound one revision, ordered move/remove archive and empty continuation"]
fn nonfirst_nested_edits_preserve_source_native_and_order() {
    let q = quota();
    let owner = owned(&q, SampleType::F64);
    let original = owner.document().clone(); // external comparison clone covered by fixture prehold
    let args = [PointArgumentPatch { name: "scalar", value: PointArgumentValue::Float32Bits(0.75f32.to_bits()) }];
    let next = owner
        .try_with_readonly_normal_hierarchy_filter_with_opacity(
            PointFilterPatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 7, arguments: &args, enabled: Some(true) },
            Some((-0f64).to_bits()),
            &ctl(),
        )
        .unwrap();
    let s = next.document().xcf_state.as_ref().unwrap();
    assert_eq!(s.filter_stacks[0].revision, 8);
    assert_eq!(s.filter_stacks[0].nodes[0], original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]);
    assert_eq!(s.opaque, original.xcf_state.as_ref().unwrap().opaque);
    let moved = next
        .try_move_readonly_normal_hierarchy_filter(
            PointFilterMovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 8, index: 0 },
            &ctl(),
        )
        .unwrap();
    assert_eq!(moved.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id, FilterId(101));
    let removed = moved
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(101), expected_stack_revision: 9 }, &ctl())
        .unwrap();
    let empty = removed
        .try_remove_readonly_normal_hierarchy_filter(PointFilterRemovePatch { layer: LayerId(2), filter: FilterId(100), expected_stack_revision: 10 }, &ctl())
        .unwrap();
    let s = empty.document().xcf_state.as_ref().unwrap();
    assert!(s.filter_stacks[0].nodes.is_empty());
    assert!(s.filter_masks.is_empty());
    assert_eq!(s.filter_stacks[0].revision, 11);
    assert!(s.opaque.iter().all(|r| r.owner.is_none()));
    for (a, b) in s.opaque.iter().zip(&original.xcf_state.as_ref().unwrap().opaque) {
        assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
        assert_eq!((a.blake3.clone(), a.order, a.flags), (b.blake3.clone(), b.order, b.flags));
    }
    empty.validate_readonly_normal_hierarchy_storage(&ctl()).unwrap();
    assert_eq!(empty.document().layers, original.layers);
}
#[test]
#[ignore = "UNRUN/uncompiled: whole-tree malformed prestate refuses equal/repair, physical BYTEzero default/padding strict and no generic grants"]
fn whole_tree_prestate_before_noop_and_repair() {
    for variant in 0..5 {
        let q = quota();
        let (mut d, h) = fixture(&q, SampleType::F32);
        match variant {
            0 => d.xcf_state.as_mut().unwrap().items[2].parent = None,
            1 => d.xcf_state.as_mut().unwrap().items[4].raster_bounds = Some(d.bounds()),
            2 => {
                if let LayerContent::Group(g) = &mut d.layers[0].content {
                    g.children[0].opacity = f32::NAN
                }
            }
            3 => {
                *d.xcf_state.as_mut().unwrap().filter_masks[0]
                    .surface
                    .try_tile_mut(photocraft_geom::TileCoord::new(0, 0))
                    .unwrap()
                    .bytes_mut()
                    .last_mut()
                    .unwrap() = 1
            }
            _ => d.xcf_state.as_mut().unwrap().filter_stacks[0].owner = ItemRef::Layer(LayerId(10)),
        };
        assert!(AccountedDocument::new_readonly_normal_hierarchy_current_storage(d, h, q, &ctl()).is_err());
    }
    let q = quota();
    let owner = owned(&q, SampleType::F32);
    assert!(owner.try_edit(AccountedEdit::LayerVisibility { layer: LayerId(2), visible: false }, &ctl()).is_err());
    assert!(
        owner
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: LayerId(2), name: None, visible: None, opacity_f32_bits: None }, &ctl())
            .is_err()
    );
    let changed = owner
        .try_edit(AccountedEdit::LayerMetadata { layer: LayerId(11), name: Some("group renamed"), visible: Some(false), opacity: Some(-0.) }, &ctl())
        .unwrap();
    assert_eq!(layer_at(changed.document(), 2, &ctl()).unwrap().name, "group renamed");
    let expanded = changed.try_with_readonly_normal_hierarchy_group_expanded(None, true, true, &ctl()).unwrap();
    for row in expanded.checked_readonly_normal_hierarchy_index(&ctl()).unwrap().rows() {
        if let LayerContent::Group(g) = &row.layer.content {
            assert!(g.expanded)
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: prospective generic Source Index lease, actual L header, no allocation before caps and original local work/cancel"]
fn unique_source_index_receipt_and_lowered_limits() {
    struct Receipt {
        lease: QuotaLease,
        _padding: [u8; 2048],
    }
    impl crate::normal_hierarchy_index::NormalHierarchyIndexLease for Receipt {
        fn bytes(&self) -> u64 {
            self.lease.bytes()
        }
    }
    let q = quota();
    let (d, _h) = fixture(&q, SampleType::F64);
    let req = d.checked_normal_hierarchy_index_requirements(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(())).unwrap();
    let larger = d.checked_normal_hierarchy_index_requirements_for_lease::<Receipt>(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(())).unwrap();
    assert!(larger.index_bytes >= req.index_bytes + 2000);
    let before = q.usage().unwrap();
    let index = d
        .checked_normal_hierarchy_index_with_reservation(NormalHierarchyLimits::STANDARD, &Interrupt::NONE, &|_| Ok(()), |bytes| {
            Ok(Receipt { lease: q.reserve(QuotaKind::Index, bytes).unwrap(), _padding: [0; 2048] })
        })
        .unwrap();
    assert_eq!(index.len(), 6);
    drop(index);
    assert_eq!(q.usage().unwrap(), before);
    let requested = std::cell::Cell::new(false);
    let low = NormalHierarchyLimits::try_lowered(5, 32, 4096, 64 << 20).unwrap();
    assert!(
        d.checked_normal_hierarchy_index_with_reservation(low, &Interrupt::NONE, &|_| Ok(()), |n| {
            requested.set(true);
            q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
        })
        .is_err()
    );
    assert!(!requested.get());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(d.checked_normal_hierarchy_index_with(NormalHierarchyLimits::STANDARD, &q, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert!(
        d.checked_normal_hierarchy_index_with(
            NormalHierarchyLimits::STANDARD,
            &q,
            &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }
        )
        .is_err()
    );
    assert_eq!(q.usage().unwrap(), before);
    // Non-Sync Cell callback observes a REAL acquired index receipt, so dynamic
    // source cancellation is not replaced by a captured deadline/initial poll.
    let q2 = quota();
    let initial = q2.usage().unwrap();
    let (d, h) = fixture(&q2, SampleType::F32);
    let held = q2.usage().unwrap();
    let calls = std::cell::Cell::new(0u64);
    let local = |_: u64| {
        calls.set(calls.get() + 1);
        if q2.usage().unwrap().index > held.index { Err(BackingError::Cancelled) } else { Ok(()) }
    };
    assert!(matches!(AccountedDocument::new_readonly_normal_hierarchy_storage_with_local_work(d, h, q2.clone(), &ctl(), &local), Err(BackingError::Cancelled)));
    assert!(calls.get() > 1);
    assert_eq!(q2.usage().unwrap(), initial);
}
#[test]
#[ignore = "UNRUN/uncompiled: real quota/late-work/cancel copy refunds/full-Index refusal and structural no-op sharing with validation headroom"]
fn changed_copy_fences_and_shared_noop() {
    let q = quota();
    let owner = owned(&q, SampleType::F32);
    let before = q.usage().unwrap();
    let available = (64u64 << 20) - before.index;
    let block = q.reserve(QuotaKind::Index, available).unwrap();
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    drop(block);
    assert_eq!(q.usage().unwrap(), before);
    let noop = owner.try_with_document_name("tree", &ctl()).unwrap();
    assert!(noop.shares_model(&owner));
    drop(noop);
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let changed = owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(changed);
    assert_eq!(q.usage().unwrap(), before);
    let left = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| left.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "tree");
}

#[path="accounted_normal_hierarchy_mask_tests.rs"]
mod masked_current_tests;

#[path="accounted_normal_hierarchy_plain_tests.rs"]
mod plain_quantized_tests;

#[path="accounted_normal_hierarchy_u32_tests.rs"]
mod u32_plain_tests;

#[path="accounted_normal_hierarchy_native_tests.rs"]
mod native_edit_tests;
