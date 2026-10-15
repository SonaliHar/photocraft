//! UNRUN typed Histogram Threshold current patch and retention drafts.
//! Synthetic storage ownership does not establish source/profile/render proof.
use super::*;
use crate::{
    BackedLayer, ColorMode, Layer, PixelFormat, SampleType, Size,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{FilterArgument, FilterNode, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{
        BackedRaster, NativePlaneDescriptor, NativePlaneStore,
        quota::{QuotaLimits, QuotaUsage},
    },
};
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
            operation: OperationIdentity { name: "gimp:threshold".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: Some("Original threshold".into()),
            source_icon: Some("retained-icon".into()),
            arguments: vec![
                FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
                FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 5 } },
                FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) },
            ],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: masked.then_some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: Some(crate::import_compositing::GimpComposition {
                compatibility_epoch: EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: OPAQUE,
                stored_composite_space: OPAQUE,
                stored_operator: OPAQUE,
                resolved: None,
                pass_through: false,
            }),
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
fn owned(q: &BackingQuota) -> AccountedDocument {
    let (document, guard) = fixture(q, true);
    AccountedDocument::new_readonly_threshold_storage(document, guard, q.clone(), &control()).unwrap()
}
fn patch(owner: &AccountedDocument) -> ThresholdPatch {
    let state = owner.document().xcf_state.as_ref().unwrap();
    let stack = &state.filter_stacks[0];
    ThresholdPatch {
        layer: owner.document().layers[0].id,
        filter: stack.nodes[0].id,
        expected_stack_revision: stack.revision,
        low_f32_bits: None,
        high_f32_bits: None,
        channel: None,
        enabled: None,
    }
}
#[test]
fn compound_patch_preserves_original_argument_order_type_raw_bits_and_source_native_owner() {
    let q = quota();
    let old = owned(&q);
    let old_state = old.document().xcf_state.as_ref().unwrap();
    let before = old_state.clone();
    let edit = ThresholdPatch {
        low_f32_bits: Some(0.875f32.to_bits()),
        high_f32_bits: Some((-0f32).to_bits()),
        channel: Some(4),
        enabled: Some(false),
        ..patch(&old)
    };
    let current = old.try_with_threshold(edit, &control()).unwrap();
    let state = current.document().xcf_state.as_ref().unwrap();
    let node = &state.filter_stacks[0].nodes[0];
    assert_eq!(state.filter_stacks[0].revision, 8);
    assert_eq!(node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["high", "channel", "low"]);
    assert_eq!(node.arguments[0].value, FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits())));
    assert_eq!(node.arguments[1].value, FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 4 });
    assert_eq!(node.arguments[2].value, FilterValue::Number(ExactNumber::Float32Bits(0.875f32.to_bits())));
    assert!(!node.enabled);
    assert_eq!(old.document().xcf_state.as_ref().unwrap(), &before);
    assert_eq!(state.opaque, before.opaque);
    assert_eq!(state.provenance, before.provenance);
    assert_eq!(state.items, before.items);
    assert_eq!(state.filter_masks, before.filter_masks);
    assert!(Arc::ptr_eq(old_state.filter_masks[0].surface.tiles().next().unwrap().1, state.filter_masks[0].surface.tiles().next().unwrap().1));
    let LayerContent::BackedRaster(a) = &old.document().layers[0].content else { panic!() };
    let LayerContent::BackedRaster(b) = &current.document().layers[0].content else { panic!() };
    assert_eq!(a.descriptor(), b.descriptor());
    assert!(a.quota().shares_ledger(b.quota()));
    current.validate_readonly_threshold_storage(&control()).unwrap();
    assert_eq!(current.document().source_color_context, old.document().source_color_context);
}
#[test]
fn exact_noop_needs_no_quota_and_signed_zero_change_is_a_real_revision() {
    let q = quota();
    let old = owned(&q);
    let before = q.usage().unwrap();
    let blocker = q.reserve(QuotaKind::Index, (16 << 20) - before.index).unwrap();
    let noop = old
        .try_with_threshold(
            ThresholdPatch {
                low_f32_bits: Some((-0f32).to_bits()),
                high_f32_bits: Some(0.75f32.to_bits()),
                channel: Some(5),
                enabled: Some(true),
                ..patch(&old)
            },
            &control(),
        )
        .unwrap();
    assert!(noop.shares_model(&old));
    let full = q.usage().unwrap();
    assert!(old.try_with_threshold(ThresholdPatch { low_f32_bits: Some(0f32.to_bits()), ..patch(&old) }, &control()).is_err());
    assert_eq!(q.usage().unwrap(), full);
    drop(blocker);
    let changed = old.try_with_threshold(ThresholdPatch { low_f32_bits: Some(0f32.to_bits()), ..patch(&old) }, &control()).unwrap();
    assert!(!changed.shares_model(&old));
    assert_eq!(changed.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 8);
    assert_eq!(old.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
}
#[test]
fn malformed_target_numbers_and_revision_overflow_refuse_without_allocated_output() {
    let q = quota();
    let old = owned(&q);
    let before = q.usage().unwrap();
    for case in 0..9 {
        let mut edit = patch(&old);
        match case {
            0 => edit.layer = LayerId(0),
            1 => edit.filter = FilterId(0),
            2 => edit.expected_stack_revision += 1,
            3 => edit.low_f32_bits = Some(f32::NAN.to_bits()),
            4 => edit.high_f32_bits = Some(f32::INFINITY.to_bits()),
            5 => edit.low_f32_bits = Some((-0.1f32).to_bits()),
            6 => edit.high_f32_bits = Some(1.1f32.to_bits()),
            7 => edit.channel = Some(-1),
            _ => edit.channel = Some(6),
        }
        assert!(old.try_with_threshold(edit, &control()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    let (mut doc, guard) = fixture(&q, true);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].revision = u64::MAX;
    let maximum = AccountedDocument::new_readonly_threshold_storage(doc, guard, q.clone(), &control()).unwrap();
    assert!(maximum.try_with_threshold(patch(&maximum), &control()).unwrap().shares_model(&maximum));
    assert!(maximum.try_with_threshold(ThresholdPatch { enabled: Some(false), ..patch(&maximum) }, &control()).is_err());
}
#[test]
fn cancellation_and_cumulative_work_failure_leave_original_and_quota_intact() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let q = quota();
    let old = owned(&q);
    let before = q.usage().unwrap();
    let edit = ThresholdPatch { enabled: Some(false), channel: Some(0), ..patch(&old) };
    let cancelled = Interrupt::cancel_only(&|| true);
    let ctl = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert!(old.try_with_threshold(edit, &ctl).is_err());
    for allowance in [0u64, 1, 64, 4096, 1 << 20] {
        let used = AtomicU64::new(0);
        let charge = |n| {
            used.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(n).filter(|v| *v <= allowance)).map_err(|_| BackingError::Work)?;
            Ok(())
        };
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
        let result = old.try_with_threshold(edit, &ctl);
        if allowance == 0 {
            assert!(result.is_err());
        }
        drop(result);
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(old.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, 7);
        assert!(old.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
    }
}
#[test]
fn alternating_patch_and_title_copies_keep_class_and_flat_original_guard() {
    let q = quota();
    let root = owned(&q);
    let weak = root.downgrade();
    let mut current = root.clone();
    for i in 0..32 {
        let edit = ThresholdPatch { channel: Some(i % 6), enabled: Some(i % 2 == 0), ..patch(&current) };
        current = current.try_with_threshold(edit, &control()).unwrap();
        current = current.try_with_document_name(&format!("current-{i}.pcraft"), &control()).unwrap();
        assert!(current.is_readonly_threshold_storage());
    }
    drop(root);
    assert!(weak.upgrade().is_some());
    assert_eq!(current.document().name, "current-31.pcraft");
    drop(current);
    assert!(weak.upgrade().is_none());
    drop(weak);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
