//! Ignored UNRUN storage-only synthetic regressions. No source/evaluator/ICC
//! semantic or larger GIMP geometry evidence is supplied by these fixtures.
use super::*;
use crate::{
    BackedLayer, ColorMode, Layer, PixelFormat, SampleEncoding, SampleType, Size, SourceColorContext,
    item_identity::{FilterId, MaskId},
    xcf_filters::ExactNumber,
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_geom::TileCoord;
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaLease, QuotaLimits},
    },
};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 64 << 20, scratch: 0 })
}
struct Guard {
    _index: QuotaLease,
    _payload: QuotaLease,
}
struct Store {
    descriptor: NativePlaneDescriptor,
    _guard: Arc<Guard>,
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
    fn read_exact_at(&self, offset: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).ok_or(BackingError::Length)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Bounds);
        }
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, sample: SampleType) -> (Document, Arc<Guard>) {
    // Real preholds precede every model/profile/mask allocation and cover spare
    // capacities and the bounded external malformed clones in these drafts.
    let guard = Arc::new(Guard { _index: q.reserve(QuotaKind::Index, 16 << 20).unwrap(), _payload: q.reserve(QuotaKind::Resident, 16 << 20).unwrap() });
    let mut doc = Document::new("initial", Size::new(257, 259), ColorMode::Rgb, sample);
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: if sample == SampleType::F32 { 650 } else { 750 } });
    doc.working_profile_interpretation = WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "a".repeat(64) };
    let mut icc = Vec::with_capacity(8192);
    icc.extend_from_slice(&[1, 2, 3, 4]);
    doc.icc_profile = Some(Arc::new(icc)); // opaque custody only
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    let canvas = doc.bounds();
    for index in 0..3 {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut default = vec![0; format.bytes_per_pixel()];
        default[0] = 0x91;
        let descriptor = NativePlaneDescriptor::new(format, canvas, &default).unwrap();
        let raster = BackedRaster::new(Arc::new(Store { descriptor, _guard: guard.clone() }), q.clone(), 1, &ctl()).unwrap();
        let mut layer = Layer::new(format!("root{index}"), LayerContent::BackedRaster(BackedLayer::new(raster, descriptor, &ctl()).unwrap()));
        layer.id = LayerId(index + 1); // Deliberate cross-namespace numeric equality.
        layer.visible = index != 1;
        layer.opacity = if index == 1 { -0.0 } else { 0.375 };
        state.items.push(ItemState {
            item: ItemRef::Layer(layer.id),
            name: layer.name.clone(),
            custom_name: false,
            visible: layer.visible,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(canvas),
            composition: None,
            mask: None,
        });
        if index != 0 {
            // Filter/Mask numeric IDs may coincide with Layer IDs: namespaces are typed.
            let filter = FilterId(index);
            let mask = MaskId(index);
            state.filter_stacks.push(FilterStack {
                owner: ItemRef::Layer(layer.id),
                revision: u64::MAX,
                nodes: vec![FilterNode {
                    id: filter,
                    operation: OperationIdentity { name: "unqualified:raw-point".into(), stored_version: "opaque".into(), schema_epoch: "opaque".into() },
                    source_display_name: Some("stored".into()),
                    source_icon: None,
                    arguments: vec![FilterArgument { name: "raw".into(), value: FilterValue::Number(ExactNumber::Float64Bits(0x7ff8000000000091)) }],
                    enabled: false,
                    opacity_f64_bits: 1f64.to_bits(),
                    source_clipping: Some(0),
                    source_region: Some(0),
                    mask: Some(ItemRef::Mask(mask)),
                    auxiliary: Vec::new(),
                    composition: Some(crate::import_compositing::GimpComposition {
                        compatibility_epoch: "unqualified".into(),
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
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let zero = if sample == SampleType::F32 { (-0f32).to_ne_bytes().to_vec() } else { (-0f64).to_ne_bytes().to_vec() };
            let mut surface = Surface::try_with_default_bytes(format, &zero).unwrap();
            // Native physical padding is retained without semantic zero qualification.
            *surface.try_tile_mut(TileCoord::new(1, 1)).unwrap().bytes_mut().last_mut().unwrap() = 0x91;
            state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds: canvas, surface });
            state.items.push(ItemState {
                item: ItemRef::Mask(mask),
                name: "effect mask".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(canvas),
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(layer.id),
                    bounds: canvas,
                    offset: [0, 0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: false,
                }),
            });
        }
        state.selection.selected.push(ItemRef::Layer(layer.id));
        doc.layers.push(layer);
    }
    state.selection.active = state.selection.selected.last().copied();
    doc.xcf_state = Some(state);
    (doc, guard)
}
fn owned(q: &BackingQuota, s: SampleType) -> AccountedDocument {
    let (d, g) = fixture(q, s);
    AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q.clone(), &ctl()).unwrap()
}
#[test]
#[ignore = "UNRUN: actual incoming capacity/custody plus additional owner header only; not an ICC/source grant"]
fn constructor_counts_actual_capacity_and_retains_typed_namespaces() {
    for sample in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let baseline = q.usage().unwrap();
        let (d, g) = fixture(&q, sample);
        let before = q.usage().unwrap();
        assert_eq!(
            AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()).unwrap(),
            AccountedDocument::owned_model_reservation_bytes()
        );
        assert_eq!(q.usage().unwrap(), before);
        let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q.clone(), &ctl()).unwrap();
        assert_eq!(q.usage().unwrap().index - before.index, AccountedDocument::owned_model_reservation_bytes());
        owner.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
        assert!(owner.is_readonly_filtered_flat_point_storage());
        assert!(!owner.is_readonly_point_storage());
        assert!(!owner.is_readonly_flat_composite_storage());
        assert_eq!(owner.document().icc_profile.as_ref().unwrap().capacity(), 8192);
        assert!(measured(owner.document(), &q, &ctl()).unwrap().index >= 8192);
        drop(owner);
        assert_eq!(q.usage().unwrap(), baseline);
    }
    // More canonical rows than the legacy borrowed accessor admits. All native
    // stores remain genuine same-q clones; model/map metadata is within the
    // fixture's preheld16MiB envelope, not a new physical pixel allocation.
    let q = quota();
    let (mut d, g) = fixture(&q, SampleType::F32);
    let layer = d.layers[1].clone();
    let state = d.xcf_state.as_ref().unwrap();
    let stack = state.filter_stacks[0].clone();
    let mask = state.filter_masks[0].clone();
    let layer_item = state.items.iter().find(|i| i.item == ItemRef::Layer(layer.id)).unwrap().clone();
    let mask_item = state.items.iter().find(|i| i.item == ItemRef::Mask(mask.id)).unwrap().clone();
    for index in 3..2049u64 {
        let id = LayerId(index + 20000);
        let fid = FilterId(index + 20000);
        let mid = MaskId(index + 20000);
        let mut next = layer.clone();
        next.id = id;
        d.layers.push(next);
        let state = d.xcf_state.as_mut().unwrap();
        let mut next = stack.clone();
        next.owner = ItemRef::Layer(id);
        next.nodes[0].id = fid;
        next.nodes[0].mask = Some(ItemRef::Mask(mid));
        state.filter_stacks.push(next);
        let mut next = mask.clone();
        next.id = mid;
        next.owner = fid;
        state.filter_masks.push(next);
        let mut row = layer_item.clone();
        row.item = ItemRef::Layer(id);
        state.items.push(row);
        let mut row = mask_item.clone();
        row.item = ItemRef::Mask(mid);
        row.mask.as_mut().unwrap().owner = ItemRef::Layer(id);
        state.items.push(row);
    }
    assert_eq!(d.xcf_state.as_ref().unwrap().items.len(), 4097);
    assert!(measured(&d, &q, &ctl()).unwrap().index <= 16 << 20);
    let owner = AccountedDocument::new_readonly_filtered_flat_point_storage(d, g, q, &ctl()).unwrap();
    let id = owner.document().layers[0].id;
    assert!(super::super::accounted_layer_access(owner.document(), id, &ctl()).is_err());
    assert_eq!(owner.accounted_layer_access(id, &ctl()).unwrap().unwrap().layer.id, id);
}
#[test]
#[ignore = "UNRUN: full paid title copies preserve all roots/native/default/mask/profile bits and direct codec lifetime"]
fn repeated_title_copy_keeps_class_raw_owners_and_flat_retention() {
    for sample in [SampleType::F32, SampleType::F64] {
        let q = quota();
        let baseline = q.usage().unwrap();
        let original = owned(&q, sample);
        let mut current = original.clone();
        for title in ["one.pcraft", "two.pcraft", "three.pcraft"] {
            let next = current.try_with_document_name(title, &ctl()).unwrap();
            let mut expected = original.document().clone(); // external harness comparison
            expected.name = title.into();
            assert_eq!(next.document(), &expected);
            assert!(next.is_readonly_filtered_flat_point_storage());
            assert!(Arc::ptr_eq(&next.owner.retention, &original.owner.retention));
            assert!(Arc::ptr_eq(next.document().icc_profile.as_ref().unwrap(), original.document().icc_profile.as_ref().unwrap()));
            let a = &original.document().xcf_state.as_ref().unwrap().filter_masks;
            let b = &next.document().xcf_state.as_ref().unwrap().filter_masks;
            for (a, b) in a.iter().zip(b) {
                assert_eq!(a.surface.default_bytes(), b.surface.default_bytes());
                for ((ca, ta), (cb, tb)) in a.surface.tiles().zip(b.surface.tiles()) {
                    assert_eq!(ca, cb);
                    assert!(Arc::ptr_eq(ta, tb));
                }
            }
            current = next;
        }
        drop(original);
        current.validate_readonly_filtered_flat_point_storage(&ctl()).unwrap();
        drop(current);
        assert_eq!(q.usage().unwrap(), baseline);
    }
}
#[test]
#[ignore = "UNRUN: global root/filter/mask/selection relationships and precision/profile boundaries reject malformed current data"]
fn malformed_global_relations_and_profile_pairs_refuse() {
    for variant in 0..15 {
        let q = quota();
        let (mut d, _g) = fixture(&q, SampleType::F64);
        let state = d.xcf_state.as_mut().unwrap();
        match variant {
            0 => d.layers[1].id = d.layers[0].id,
            1 => state.filter_stacks[1].nodes[0].id = state.filter_stacks[0].nodes[0].id,
            2 => state.filter_masks[0].owner = FilterId(9999),
            3 => state.filter_stacks[0].owner = ItemRef::Layer(LayerId(9999)),
            4 => state.selection.active = Some(ItemRef::Layer(LayerId(9999))),
            5 => state.selection.selected[1] = state.selection.selected[0],
            6 => state.items[0].name.push('x'),
            7 => state.filter_masks[0].bounds = Rect::new(0, 0, 256, 259),
            8 => d.source_color_context.as_mut().unwrap().source_precision = 650,
            9 => d.working_profile_interpretation = WorkingProfileInterpretation::Icc,
            10 => d.layers[1].opacity = f32::NAN,
            11 => state.filter_stacks[0].nodes[0].source_display_name = Some("bad\0text".into()),
            12 => state.current_filter_output_precision = crate::FilterOutputPrecision::Half,
            13 => d.metadata.text = vec![(String::new(), String::new()); 4097],
            _ => {
                d.physical_resolution = Some(crate::physical_resolution::PhysicalResolution {
                    x_dpi: 72.,
                    y_dpi: 72.,
                    unit: crate::physical_resolution::PhysicalUnit::Custom(crate::physical_resolution::CustomUnit {
                        name: "unit".into(),
                        factor: 1.,
                        digits: 2,
                        symbol: "bad\0symbol".into(),
                        abbreviation: "u".into(),
                    }),
                })
            }
        }
        assert!(AccountedDocument::readonly_filtered_flat_point_storage_requirements(&d, &q, &ctl()).is_err());
    }
}
#[test]
#[ignore = "UNRUN: genuine class remains closed to generic edit/noop/history/selection and singleton property APIs"]
fn generic_and_singleton_mutations_remain_closed_even_noop() {
    let q = quota();
    let owner = owned(&q, SampleType::F32);
    let id = owner.document().layers[0].id;
    let edit = AccountedEdit::LayerMetadata { layer: id, name: None, visible: None, opacity: None };
    assert!(owner.validate_imported_metadata_edit(edit, &ctl()).is_err());
    assert!(owner.try_edit(edit, &ctl()).is_err());
    assert!(owner.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: id, name: &owner.document().layers[0].name }, &ctl()).is_err());
    assert!(
        owner
            .try_with_readonly_layer_properties(ReadonlyLayerPropertiesPatch { layer: id, name: None, visible: None, opacity_f32_bits: None }, &ctl())
            .is_err()
    );
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    assert!(history.try_commit(edit, &ctl()).is_err());
    assert_eq!(history.len(), 1);
    assert!(owner.try_replace_selection(None, &ctl()).is_err());
    assert!(owner.try_with_imported_layer_translation(id, 0, 0, &ctl()).is_err());
}
#[test]
#[ignore = "UNRUN: validated title noop at full Index and private-copy Work/cancel/quota rollback keep incoming owners intact"]
fn noop_and_copy_failures_refund_without_publishing() {
    let q = quota();
    let owner = owned(&q, SampleType::F64);
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (64 << 20) - before.index).unwrap();
    assert!(owner.try_with_document_name("initial", &ctl()).unwrap().shares_model(&owner));
    assert!(owner.try_with_document_name("changed", &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
    let remaining = AtomicU64::new(256);
    let work = |n| remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    // Calibrate against the same complete title operation, then deny the final
    // unit on the same work ledger; no elapsed/performance claim is made.
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let temporary = owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap();
    drop(temporary);
    assert_eq!(q.usage().unwrap(), before);
    let remaining = AtomicU64::new(total.load(Ordering::Relaxed) - 1);
    let work = |n| remaining.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_sub(n)).map(|_| ()).map_err(|_| BackingError::Work);
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    // This control observes actual live copy leases, not an invented phase count.
    let work = |_| if q.usage().map_err(|_| BackingError::Quota)?.index > before.index { Err(BackingError::Cancelled) } else { Ok(()) };
    assert!(owner.try_with_document_name("changed", &ReadControl { interrupt: &Interrupt::NONE, work: &work }).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "initial");
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(owner.validate_readonly_filtered_flat_point_storage(&ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), before);
}
