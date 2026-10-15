//! UNRUN typed current Threshold Alpha patch / retention regression drafts.
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
fn patch(owner: &AccountedDocument, value_f32_bits: Option<u32>, enabled: Option<bool>) -> ThresholdAlphaPatch {
    let state = owner.document().xcf_state.as_ref().unwrap();
    let stack = &state.filter_stacks[0];
    ThresholdAlphaPatch { layer: owner.document().layers[0].id, filter: stack.nodes[0].id, expected_stack_revision: stack.revision, value_f32_bits, enabled }
}
#[test]
fn typed_patch_changes_only_current_words_revision_and_keeps_native_mask_owners() {
    let q = quota();
    let source = owned(&q, true);
    let before = source.document().clone();
    let edited = source.try_with_threshold_alpha(patch(&source, Some(0.25f32.to_bits()), Some(false)), &control()).unwrap();
    let mut expected = before.clone();
    let stack = &mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];
    stack.nodes[0].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits()));
    stack.nodes[0].enabled = false;
    stack.revision += 1;
    assert_eq!(edited.document(), &expected);
    assert_eq!(source.document(), &before);
    let a = &source.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    let b = &edited.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
    assert_eq!(a.default_bytes(), &(-0.0f64).to_ne_bytes());
    assert_eq!(a, b);
    let coord = photocraft_geom::TileCoord::new(0, 0);
    assert!(Arc::ptr_eq(a.tile(coord).unwrap(), b.tile(coord).unwrap()));
    edited.validate_readonly_threshold_alpha_storage(&control()).unwrap();
    assert!(edited.try_replace_selection(None, &control()).is_err());
    assert!(edited.try_edit(AccountedEdit::LayerVisibility { layer: edited.document().layers[0].id, visible: false }, &control()).is_err());
}
#[test]
fn empty_raw_equal_and_max_revision_noops_keep_exact_owner_even_with_exhausted_quota() {
    let q = quota();
    let (mut document, guard) = fixture(&q, true);
    document.xcf_state.as_mut().unwrap().filter_stacks[0].revision = u64::MAX;
    let source = AccountedDocument::from_owned_readonly_threshold_alpha_storage(document, guard, q.clone(), &control()).unwrap();
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Resident, (64 << 20) - before.resident).unwrap();
    let empty = source.try_with_threshold_alpha(patch(&source, None, None), &control()).unwrap();
    assert!(empty.shares_model(&source));
    let equal = source.try_with_threshold_alpha(patch(&source, Some((-0.0f32).to_bits()), Some(true)), &control()).unwrap();
    assert!(equal.shares_model(&source));
    assert!(matches!(source.try_with_threshold_alpha(patch(&source, Some(0.0f32.to_bits()), None), &control()), Err(BackingError::Length)));
    drop(empty);
    drop(equal);
    drop(blocker);
    assert_eq!(q.usage().unwrap(), before);
}
#[test]
fn raw_signed_zero_is_a_stored_change_and_stale_or_invalid_patches_refuse() {
    let q = quota();
    let source = owned(&q, true);
    let before = q.usage().unwrap();
    for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits(), (-0.1f32).to_bits(), 1.1f32.to_bits()] {
        assert!(source.try_with_threshold_alpha(patch(&source, Some(bits), None), &control()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let mut wrong = patch(&source, None, None);
    wrong.expected_stack_revision += 1;
    assert!(source.try_with_threshold_alpha(wrong, &control()).is_err());
    let mut wrong = patch(&source, None, None);
    wrong.filter = FilterId(999);
    assert!(source.try_with_threshold_alpha(wrong, &control()).is_err());
    let changed = source.try_with_threshold_alpha(patch(&source, Some(0.0f32.to_bits()), None), &control()).unwrap();
    assert!(!changed.shares_model(&source));
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
    let (document, guard) = fixture(&q, true);
    let legacy = AccountedDocument::from_owned(document, guard, q.clone(), &control()).unwrap();
    assert!(legacy.try_with_threshold_alpha(patch(&legacy, None, None), &control()).is_err());
}
#[test]
fn mixed_patch_and_title_copies_flatten_original_retention_without_recursive_current_chain() {
    let q = quota();
    let source = owned(&q, true);
    let original = source.downgrade();
    let first = source.try_with_threshold_alpha(patch(&source, Some(0.25f32.to_bits()), None), &control()).unwrap();
    let first_weak = first.downgrade();
    let named = first.try_with_document_name("current.pcraft", &control()).unwrap();
    let named_weak = named.downgrade();
    let last = named.try_with_threshold_alpha(patch(&named, None, Some(false)), &control()).unwrap();
    drop(first);
    drop(named);
    drop(source);
    assert!(first_weak.upgrade().is_none());
    assert!(named_weak.upgrade().is_none());
    assert!(original.upgrade().is_some());
    assert_eq!(last.document().name, "current.pcraft");
    assert_eq!(last.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 9);
    drop(last);
    assert!(original.upgrade().is_none());
    drop(original);
    drop(first_weak);
    drop(named_weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn work_cancel_and_real_allocation_denials_preserve_source_and_refund_private_copy_fees() {
    let q = quota();
    let source = owned(&q, true);
    let before = q.usage().unwrap();
    let saved = source.document().clone();
    let spent = AtomicU64::new(0);
    let deny = |n| {
        spent.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(source.try_with_threshold_alpha(patch(&source, Some(0.75f32.to_bits()), None), &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(spent.load(Ordering::Relaxed) > 0);
    assert_eq!(source.document(), &saved);
    assert_eq!(q.usage().unwrap(), before);
    let allocated_polls = AtomicU64::new(0);
    let late = |_n| {
        if q.usage().map_err(|_| BackingError::Quota)?.resident > before.resident && allocated_polls.fetch_add(1, Ordering::Relaxed) > 8 {
            Err(BackingError::Cancelled)
        } else {
            Ok(())
        }
    };
    assert!(matches!(
        source.try_with_threshold_alpha(patch(&source, Some(0.75f32.to_bits()), None), &ReadControl { interrupt: &Interrupt::NONE, work: &late }),
        Err(BackingError::Cancelled)
    ));
    assert!(allocated_polls.load(Ordering::Relaxed) > 8);
    assert_eq!(source.document(), &saved);
    assert_eq!(q.usage().unwrap(), before);
    let stop = || true;
    assert!(matches!(
        source.try_with_threshold_alpha(patch(&source, None, Some(false)), &ReadControl { interrupt: &Interrupt::cancel_only(&stop), work: &|_| Ok(()) }),
        Err(BackingError::Cancelled)
    ));
    let block = q.reserve(QuotaKind::Resident, (64 << 20) - q.usage().unwrap().resident).unwrap();
    assert!(source.try_with_threshold_alpha(patch(&source, None, Some(false)), &control()).is_err());
    drop(block);
    assert_eq!(source.document(), &saved);
    assert_eq!(q.usage().unwrap(), before);
}
