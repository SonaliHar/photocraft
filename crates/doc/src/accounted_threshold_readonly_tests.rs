//! UNRUN separate Histogram Threshold storage/copy/denial regression drafts.
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
#[test]
fn explicit_class_preserves_raw_storage_order_and_rejects_other_edit_classes() {
    let q = quota();
    let source = owned(&q);
    assert!(source.is_readonly_threshold_storage());
    assert!(!source.is_readonly_threshold_alpha_storage());
    source.validate_readonly_threshold_storage(&control()).unwrap();
    assert!(source.validate_readonly_threshold_alpha_storage(&control()).is_err());
    let layer = source.document().layers[0].id;
    assert!(source.try_edit(AccountedEdit::LayerMetadata { layer, name: Some("changed"), visible: None, opacity: None }, &control()).is_err());
    assert!(source.try_replace_selection(None, &control()).is_err());
    let state = source.document().xcf_state.as_ref().unwrap();
    let node = &state.filter_stacks[0].nodes[0];
    assert_eq!(node.arguments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["high", "channel", "low"]);
    assert_eq!(node.arguments[2].value, FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())));
    // Storage preserves even a nonempty physical mask: only a separate measured
    // render/source context may decide whether that mask is admissible there.
    assert_eq!(state.filter_masks[0].surface.default_bytes(), &(-0.0f64).to_ne_bytes());
    let mut sample = [0.0];
    state.filter_masks[0].surface.try_read_pixel_f64(0, 0, &mut sample).unwrap();
    assert_eq!(sample[0], 0.5 + 2f64.powi(-40));
    drop(source);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn title_copies_preserve_mask_tile_arcs_native_guard_and_flatten_repeated_lifetimes() {
    let q = quota();
    let original = owned(&q);
    let renamed = original.try_with_document_name("current.pcraft", &control()).unwrap();
    assert!(renamed.is_readonly_threshold_storage());
    assert_eq!(original.document().name, "old");
    assert_eq!(renamed.document().name, "current.pcraft");
    assert_eq!(renamed.document().xcf_state, original.document().xcf_state);
    let a = original.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.tiles().next().unwrap().1;
    let b = renamed.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.tiles().next().unwrap().1;
    assert!(Arc::ptr_eq(a, b));
    let unchanged = renamed.try_with_document_name("current.pcraft", &control()).unwrap();
    assert!(unchanged.shares_model(&renamed));
    drop(unchanged);
    let mut current = renamed;
    for i in 0..32 {
        current = current.try_with_document_name(&format!("saved-{i}.pcraft"), &control()).unwrap();
    }
    drop(original);
    current.validate_readonly_threshold_storage(&control()).unwrap();
    drop(current);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn closed_schema_mask_geometry_and_argument_domains_refuse_without_normalization() {
    for case in 0..11 {
        let q = quota();
        let (mut doc, guard) = fixture(&q, true);
        match case {
            0 => doc.size = Size::new(9, 6),
            1 => doc.sample_encoding = Some(crate::SampleEncoding::Linear),
            2 => doc.mode = ColorMode::Grayscale,
            3 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].operation.name = "gimp:threshold-alpha".into(),
            4 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[1].name = "low".into(),
            5 => {
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[2].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()))
            }
            6 => {
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[2].value = FilterValue::Number(ExactNumber::Float32Bits(f32::NAN.to_bits()))
            }
            7 => doc.xcf_state.as_mut().unwrap().filter_masks[0].bounds = Rect::new(1, 0, 9, 6),
            8 => doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().stored_operator = 0,
            9 => {
                doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].composition.as_mut().unwrap().compatibility_epoch =
                    "xcf-22-stock-threshold-alpha-precision-v1".into()
            }
            _ => doc.xcf_state.as_mut().unwrap().filter_masks.clear(),
        }
        assert!(AccountedDocument::new_readonly_threshold_storage(doc, guard, q.clone(), &control()).is_err());
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let q = quota();
    let (mut doc, guard) = fixture(&q, true);
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0.25f32.to_bits()));
    node.arguments[2].value = FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits()));
    node.enabled = false;
    let owner = AccountedDocument::new_readonly_threshold_storage(doc, guard, q.clone(), &control()).unwrap();
    owner.validate_readonly_threshold_storage(&control()).unwrap();
}
#[test]
fn requirements_and_real_quota_work_cancel_denials_preserve_original_owner() {
    let q = quota();
    let owner = owned(&q);
    assert_eq!(
        AccountedDocument::readonly_threshold_storage_requirements(owner.document(), &q, &control()).unwrap(),
        AccountedDocument::owned_model_reservation_bytes()
    );
    let baseline = q.usage().unwrap();
    let held = q.reserve(QuotaKind::Index, (16 << 20) - baseline.index).unwrap();
    assert!(owner.try_with_document_name("denied.pcraft", &control()).is_err());
    drop(held);
    assert_eq!(q.usage().unwrap(), baseline);
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(owner.try_with_document_name("denied.pcraft", &deny).is_err());
    let cancelled = Interrupt::cancel_only(&|| true);
    let cancelled = ReadControl { interrupt: &cancelled, work: &|_| Ok(()) };
    assert!(owner.validate_readonly_threshold_storage(&cancelled).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    assert_eq!(owner.document().name, "old");
}
#[test]
fn legacy_and_alpha_do_not_acquire_histogram_authority_from_serialized_name() {
    let q = quota();
    let (doc, guard) = fixture(&q, true);
    assert!(AccountedDocument::from_owned_readonly_threshold_alpha_storage(doc, guard, q.clone(), &control()).is_err());
    let (doc, guard) = fixture(&q, true);
    let legacy = AccountedDocument::from_owned(doc, guard, q.clone(), &control()).unwrap();
    assert!(!legacy.is_readonly_threshold_storage());
    assert!(legacy.validate_readonly_threshold_storage(&control()).is_err());
    assert!(legacy.try_with_document_name("new.pcraft", &control()).is_err());
}
