//! UNRUN immutable filter-storage adoption/name-copy regression drafts. These
//! fixtures establish storage ownership only, never source/profile/render proof.
use super::*;
use crate::{
    BackedLayer, ColorMode, Layer, PixelFormat, SampleType, Size,
    item_identity::{FilterId, MaskId},
    xcf_filters::{FilterArgument, FilterNode, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaLimits, QuotaUsage},
    },
};
use std::sync::atomic::{AtomicU64, Ordering};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, scratch: 0, index: 16 << 20 })
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, masked: bool) -> (Document, Arc<dyn Any + Send + Sync>) {
    // Prospectively pay source model/default/map/native fixture construction.
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(q.reserve(QuotaKind::Resident, 2 << 20).unwrap());
    let bounds = Rect::new(0, 0, 8, 6);
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 32]).unwrap();
    let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new("retained layer", LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut document = Document::new("old", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    document.layers.push(layer);
    document.sample_encoding = Some(crate::SampleEncoding::NonlinearProfile);
    document.source_color_context = Some(crate::SourceColorContext { original_icc: None, source_precision: 750 });
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    let filter = FilterId(100);
    let mask = MaskId(101);
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: 7,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity { name: "gimp:threshold-alpha".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: Some("Original threshold".into()),
            source_icon: Some("retained-icon".into()),
            arguments: vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) }],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(2),
            source_region: Some(0),
            mask: masked.then_some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: None,
        }],
    });
    if masked {
        let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
        surface.try_write_pixel_f64(0, 0, &[0.5 + 2f64.powi(-40)]).unwrap();
        state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds, surface });
        // Actual shared mapper can emit mask metadata before the layer metadata.
        state.items.push(ItemState {
            item: ItemRef::Mask(mask),
            name: "original selection".into(),
            custom_name: false,
            visible: true,
            locks: ItemLocks::default(),
            parent: None,
            source_tattoo: Some(17),
            source_flags: 0,
            raster_bounds: Some(bounds),
            composition: None,
            mask: Some(MaskState {
                owner: ItemRef::Layer(id),
                bounds,
                offset: [0, 0],
                outside_f64_bits: 0f64.to_bits(),
                enabled: true,
                shown: false,
                editing: false,
                linked: false,
            }),
        });
    }
    state.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: "retained layer".into(),
        custom_name: false,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(18),
        source_flags: 0,
        raster_bounds: Some(bounds),
        mask: None,
        composition: None,
    });
    state.selection.selected.push(ItemRef::Layer(id));
    state.selection.active = Some(ItemRef::Layer(id));
    document.xcf_state = Some(state);
    (document, guard)
}
fn owned(q: &BackingQuota, masked: bool) -> AccountedDocument {
    let (document, guard) = fixture(q, masked);
    AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &control()).unwrap()
}
#[test]
fn explicit_storage_adoption_is_distinct_from_legacy_and_never_enables_edits() {
    let q = quota();
    let (document, guard) = fixture(&q, true);
    let legacy = AccountedDocument::from_owned(document, guard, q.clone(), &control()).unwrap();
    assert!(legacy.validate_readonly_threshold_alpha_storage(&control()).is_err());
    assert!(legacy.try_with_document_name("native.pcraft", &control()).is_err());
    let source = owned(&q, true);
    source.validate_readonly_threshold_alpha_storage(&control()).unwrap();
    let layer = source.document().layers[0].id;
    for edit in [
        AccountedEdit::LayerVisibility { layer, visible: false },
        AccountedEdit::LayerMetadata { layer, name: Some("new"), visible: Some(false), opacity: Some(0.5) },
        AccountedEdit::NativePixels { layer, rect: Rect::new(0, 0, 1, 1), bytes: &[0; 32] },
    ] {
        assert!(source.try_edit(edit, &control()).is_err());
    }
    assert!(source.validate_selection_replacement(&control()).is_err());
    assert!(source.try_replace_selection(None, &control()).is_err());
    assert!(source.validate_imported_metadata_edit(AccountedEdit::LayerName { layer, name: "new" }, &control()).is_err());
}
#[test]
fn paid_name_copy_preserves_full_mask_metadata_default_raw_lowbits_and_tile_identity() {
    let q = quota();
    let source = owned(&q, true);
    let renamed = source.try_with_document_name("current.pcraft", &control()).unwrap();
    assert_eq!(renamed.document().name, "current.pcraft");
    assert_eq!(source.document().name, "old");
    let a = source.document().xcf_state.as_ref().unwrap();
    let b = renamed.document().xcf_state.as_ref().unwrap();
    assert_eq!(a, b);
    assert_eq!(a.filter_masks[0].surface.default_bytes(), &(-0.0f64).to_ne_bytes());
    let coordinate = photocraft_geom::TileCoord::new(0, 0);
    assert!(Arc::ptr_eq(a.filter_masks[0].surface.tile(coordinate).unwrap(), b.filter_masks[0].surface.tile(coordinate).unwrap()));
    assert!(!std::ptr::eq(&a.filter_masks[0].surface, &b.filter_masks[0].surface));
    let again = renamed.try_with_document_name("current.pcraft", &control()).unwrap();
    assert!(again.shares_model(&renamed));
    drop(again);
    drop(renamed);
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn repeat_title_copies_keep_original_guard_but_not_recursive_intermediate_model_chain() {
    let q = quota();
    let source = owned(&q, true);
    let original = source.downgrade();
    let first = source.try_with_document_name("first.pcraft", &control()).unwrap();
    let intermediate = first.downgrade();
    let second = first.try_with_document_name("second.pcraft", &control()).unwrap();
    drop(first);
    drop(source);
    assert!(intermediate.upgrade().is_none());
    assert!(original.upgrade().is_some());
    second.validate_readonly_threshold_alpha_storage(&control()).unwrap();
    drop(second);
    assert!(original.upgrade().is_none());
    drop(original);
    drop(intermediate);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn forged_mask_relationship_coordinate_sample_type_and_scalar_storage_refuse() {
    for alteration in 0..5 {
        let q = quota();
        let (mut document, guard) = fixture(&q, true);
        let state = document.xcf_state.as_mut().unwrap();
        match alteration {
            0 => state.filter_masks[0].owner = FilterId(999),
            1 => {
                let surface = &mut state.filter_masks[0].surface;
                let tile = surface.tile(photocraft_geom::TileCoord::new(0, 0)).unwrap().clone();
                surface.put_tiles(std::iter::once((photocraft_geom::TileCoord::new(1, 0), tile)));
            }
            2 => state.filter_stacks[0].nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits())),
            3 => state.items[0].mask.as_mut().unwrap().outside_f64_bits = (-0.0f64).to_bits(),
            _ => state.items[0].raster_bounds = Some(Rect::new(1, 0, 9, 6)),
        }
        assert!(AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &control()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
}
#[test]
fn failed_name_copy_preserves_owner_native_storage_and_cumulative_work() {
    let q = quota();
    let source = owned(&q, true);
    let before = q.usage().unwrap();
    let consumed = AtomicU64::new(0);
    let charge = |n| {
        consumed.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    assert!(source.try_with_document_name("failed.pcraft", &denied).is_err());
    assert_eq!(source.document().name, "old");
    assert!(consumed.load(Ordering::Relaxed) > 0);
    assert_eq!(q.usage().unwrap(), before);
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert_eq!(source.validate_readonly_threshold_alpha_storage(&c), Err(BackingError::Cancelled));
}
