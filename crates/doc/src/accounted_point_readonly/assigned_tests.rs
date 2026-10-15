//! UNRUN assigned custody drafts; no CMS/source or Box evaluation proof. Synthetic records below deliberately include
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
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 128 << 20, index: 16 << 20, scratch: 0 })
}
struct SourceGuard {
    _fee: photocraft_raster::backing::quota::QuotaLease,
    _index: photocraft_raster::backing::quota::QuotaLease,
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
    let guard = Arc::new(SourceGuard { _fee: q.reserve(QuotaKind::Resident, 8 << 20).unwrap(), _index: q.reserve(QuotaKind::Index, 8 << 20).unwrap() });
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

fn assigned(q: &BackingQuota) -> (Document, Arc<SourceGuard>) {
    let (mut doc, guard) = fixture(q, SampleType::F32, SampleEncoding::NonlinearProfile, 650);
    // Actual fixture preholds above precede these two independently allocated
    // opaque roles. Neither byte sequence asserts a valid or related ICC.
    let mut original = Vec::with_capacity(4096);
    original.extend_from_slice(&[1, 2, 3, 4]);
    let mut current = Vec::with_capacity(8192);
    current.extend_from_slice(&[5, 6, 7, 8]);
    doc.source_color_context.as_mut().unwrap().original_icc = Some(Arc::new(original));
    doc.icc_profile = Some(Arc::new(current));
    doc.working_profile_interpretation = crate::WorkingProfileInterpretation::Icc;
    (doc, guard)
}
pub(in crate::accounted_document) fn owner() -> AccountedDocument {
    owner_at_revision(u64::MAX)
}
pub(super) fn owner_at_revision(revision: u64) -> AccountedDocument {
    let q = quota();
    let (mut doc, guard) = assigned(&q);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = revision;
    AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q, &ctl()).unwrap()
}

#[test]
#[ignore = "UNRUN assigned Point custody; not source/profile qualification"]
fn two_actual_profile_capacities_are_retained_and_requirements_only_add_header() {
    let q = quota();
    let (doc, guard) = assigned(&q);
    let original = doc.source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap();
    let current = doc.icc_profile.as_ref().unwrap();
    assert!(!Arc::ptr_eq(original, current));
    assert!(original.capacity() >= 4096 && current.capacity() >= 8192);
    let measured = measured_assigned(&doc, &q, &ctl()).unwrap();
    assert!(measured.total().unwrap() >= (original.capacity() + current.capacity()) as u64);
    let before = q.usage().unwrap();
    let extra = AccountedDocument::readonly_assigned_point_storage_requirements(&doc, &q, &ctl()).unwrap();
    assert_eq!(extra, AccountedDocument::owned_model_reservation_bytes());
    assert_eq!(q.usage().unwrap(), before);
    let owner = AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).unwrap();
    assert_eq!(q.usage().unwrap().index - before.index, extra);
    assert!(owner.is_readonly_assigned_point_storage());
    assert!(!owner.is_readonly_point_storage());
    assert!(owner.validate_readonly_point_storage(&ctl()).is_err());
    owner.validate_readonly_assigned_point_storage(&ctl()).unwrap();
    drop(owner);
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
#[ignore = "UNRUN old rejection and closed metadata shape"]
fn old_implicit_rejects_and_assigned_roles_model_marker_and_precision_are_closed() {
    for case in 0..8 {
        let q = quota();
        let (mut doc, guard) = assigned(&q);
        assert!(AccountedDocument::readonly_point_storage_requirements(&doc, &q, &ctl()).is_err());
        match case {
            0 => doc.source_color_context.as_mut().unwrap().original_icc = None,
            1 => doc.icc_profile = None,
            2 => doc.icc_profile = Some(Arc::new(Vec::new())),
            3 => doc.working_profile_interpretation = crate::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            4 => doc.source_color_context.as_mut().unwrap().source_precision = 550,
            5 => doc.sample_encoding = Some(SampleEncoding::Linear),
            6 => doc.mode = ColorMode::Grayscale,
            _ => doc.depth = SampleType::F64,
        }
        assert!(AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q.clone(), &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), Default::default());
    }
    let q = quota();
    let (doc, guard) = assigned(&q);
    assert!(AccountedDocument::new_readonly_point_storage(doc, guard, q.clone(), &ctl()).is_err());
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
#[ignore = "UNRUN paid native/profile/mask lifetime and flat title copies"]
fn title_copy_preserves_class_raw_owners_and_original_codec_without_model_chains() {
    let original = owner();
    let q = original.quota().clone();
    let original_model = Arc::downgrade(&original.owner);
    let codec = Arc::downgrade(&original.owner.retention);
    let first = original.try_with_document_name("first", &ctl()).unwrap();
    let first_model = Arc::downgrade(&first.owner);
    let second = first.try_with_document_name("second", &ctl()).unwrap();
    assert!(second.is_readonly_assigned_point_storage());
    assert!(!second.is_readonly_point_storage());
    assert!(Arc::ptr_eq(original.document().icc_profile.as_ref().unwrap(), second.document().icc_profile.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        original.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap(),
        second.document().source_color_context.as_ref().unwrap().original_icc.as_ref().unwrap()
    ));
    let a = &original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &second.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert_eq!(a.default_bytes(), b.default_bytes());
    for ((ca, ta), (cb, tb)) in a.tiles().zip(b.tiles()) {
        assert_eq!(ca, cb);
        assert!(Arc::ptr_eq(ta, tb));
    }
    let LayerContent::BackedRaster(a) = &original.document().layers[0].content else { panic!() };
    let LayerContent::BackedRaster(b) = &second.document().layers[0].content else { panic!() };
    assert_eq!(a, b);
    assert!(second.try_with_document_name("second", &ctl()).unwrap().shares_model(&second));
    drop(first);
    assert!(first_model.upgrade().is_none());
    drop(original);
    assert!(original_model.upgrade().is_some());
    assert!(codec.upgrade().is_some());
    drop(second);
    assert!(original_model.upgrade().is_none());
    assert!(codec.upgrade().is_none());
    assert_eq!(q.usage().unwrap(), Default::default());
}
#[test]
#[ignore = "UNRUN assigned class generic edit/history no-op closure"]
fn generic_noop_history_layer_selection_and_typed_implicit_routes_refuse() {
    let owner = owner();
    let layer = owner.document().layers[0].id;
    let edit = AccountedEdit::LayerVisibility { layer, visible: true };
    assert!(owner.try_edit(edit, &ctl()).is_err());
    assert!(owner.validate_imported_metadata_edit(edit, &ctl()).is_err());
    assert!(owner.try_replace_selection(None, &ctl()).is_err());
    assert!(owner.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer, name: "original layer" }, &ctl()).is_err());
    let mut history = AccountedHistory::new(owner.clone(), 4, &ctl()).unwrap();
    assert!(history.try_commit(edit, &ctl()).is_err());
    assert_eq!(history.len(), 1);
    assert!(history.current().unwrap().shares_model(&owner));
    assert!(
        current_copy(
            &owner,
            measured_assigned(owner.document(), owner.quota(), &ctl()).unwrap(),
            CopyKind::LayerName(ReadonlyLayerNamePatch { layer, name: "other" }),
            &ctl()
        )
        .is_err()
    );
}
#[test]
#[ignore = "UNRUN quota/work/cancel rollback and full-quota title no-op"]
fn control_and_actual_index_failure_leave_owner_and_ledger_unchanged() {
    let owner = owner();
    let q = owner.quota().clone();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let full = q.usage().unwrap();
    assert!(owner.try_with_document_name("original title", &ctl()).unwrap().shares_model(&owner));
    assert!(matches!(owner.try_with_document_name("changed", &ctl()), Err(BackingError::Quota)));
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let failed = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(owner.try_with_document_name("changed", &failed), Err(BackingError::Work)));
    let cancel_after_fee = || q.usage().unwrap().index > before.index;
    let interrupt = Interrupt::cancel_only(&cancel_after_fee);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(matches!(owner.try_with_document_name("changed", &c), Err(BackingError::Cancelled)));
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(owner.document().name, "original title");
    let q2 = quota();
    let (doc, guard) = assigned(&q2);
    assert!(matches!(AccountedDocument::new_readonly_assigned_point_storage(doc, guard, q2.clone(), &failed), Err(BackingError::Work)));
    assert_eq!(q2.usage().unwrap(), Default::default());
}
