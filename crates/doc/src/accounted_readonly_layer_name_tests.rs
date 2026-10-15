//! UNRUN name-only storage drafts; fixtures grant no source/CMS/render proof.
use super::*;
use crate::{
    BackedLayer, ColorMode, FilterMask, Layer, PixelFormat, SampleType, Size,
    item_identity::{FilterId, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState, XcfState},
};
use photocraft_raster::{
    Interrupt, Surface,
    backing::{BackedRaster, NativePlaneDescriptor, NativePlaneStore, quota::QuotaLimits},
};
use std::sync::atomic::{AtomicU64, Ordering};
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 16 << 20, scratch: 0 })
}
#[derive(Debug)]
struct Store(NativePlaneDescriptor);
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.0
    }
    fn read_exact_at(&self, _: u64, output: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(output.len() as u64)?;
        output.fill(0);
        Ok(())
    }
}
fn fixture(q: &BackingQuota, histogram: bool, layer_name: &str, item_name: &str, custom: bool) -> AccountedDocument {
    let guard: Arc<dyn Any + Send + Sync> = Arc::new(q.reserve(QuotaKind::Resident, 2 << 20).unwrap());
    let bounds = Rect::new(0, 0, 8, 6);
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let descriptor = NativePlaneDescriptor::new(format, bounds, &[0; 32]).unwrap();
    let backing = BackedRaster::new(Arc::new(Store(descriptor)), q.clone(), 1, &control()).unwrap();
    let layer = Layer::new(layer_name, LayerContent::BackedRaster(BackedLayer::new(backing, descriptor, &control()).unwrap()));
    let id = layer.id;
    let mut doc = Document::new("saved title", Size::new(8, 6), ColorMode::Rgb, SampleType::F64);
    doc.layers.push(layer);
    doc.sample_encoding = Some(crate::SampleEncoding::NonlinearProfile);
    doc.source_color_context = Some(crate::SourceColorContext { original_icc: None, source_precision: 750 });
    // Opaque current bytes test Arc custody only, without profile qualification.
    doc.icc_profile = Some(Arc::new(vec![1, 2, 3, 0]));
    let mut state = XcfState::default();
    state.provenance.format = "xcf".into();
    state.provenance.format_version = 22;
    state.provenance.source_hash = Some("a".repeat(64));
    let epoch = if histogram { "xcf-22-stock-threshold-precision-v1" } else { "xcf-22-stock-threshold-alpha-precision-v1" };
    let filter = FilterId(100);
    let mask = MaskId(101);
    let arguments = if histogram {
        vec![
            FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
            FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 5 } },
            FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) },
        ]
    } else {
        vec![FilterArgument { name: "value".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0.0f32).to_bits())) }]
    };
    state.filter_stacks.push(FilterStack {
        owner: ItemRef::Layer(id),
        revision: u64::MAX,
        nodes: vec![FilterNode {
            id: filter,
            operation: OperationIdentity {
                name: if histogram { "gimp:threshold" } else { "gimp:threshold-alpha" }.into(),
                stored_version: "0:0".into(),
                schema_epoch: epoch.into(),
            },
            source_display_name: Some("Original filter".into()),
            source_icon: None,
            arguments,
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(if histogram { 0 } else { 2 }),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(mask)),
            auxiliary: Vec::new(),
            composition: histogram.then(|| crate::import_compositing::GimpComposition {
                compatibility_epoch: epoch.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: -1_565_818_287,
                stored_composite_space: -1_565_818_287,
                stored_operator: -1_565_818_287,
                resolved: None,
                pass_through: false,
            }),
        }],
    });
    let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &(-0.0f64).to_ne_bytes()).unwrap();
    surface.try_write_pixel_f64(0, 0, &[0.5 + 2f64.powi(-40)]).unwrap();
    state.filter_masks.push(FilterMask { id: mask, owner: filter, bounds, surface });
    state.items.push(ItemState {
        item: ItemRef::Mask(mask),
        name: "raw selection".into(),
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
    state.items.push(ItemState {
        item: ItemRef::Layer(id),
        name: item_name.into(),
        custom_name: custom,
        visible: true,
        locks: ItemLocks::default(),
        parent: None,
        source_tattoo: Some(18),
        source_flags: 0,
        raster_bounds: Some(bounds),
        composition: None,
        mask: None,
    });
    state.selection.selected.push(ItemRef::Layer(id));
    state.selection.active = Some(ItemRef::Layer(id));
    doc.xcf_state = Some(state);
    if histogram {
        AccountedDocument::new_readonly_threshold_storage(doc, guard, q.clone(), &control()).unwrap()
    } else {
        AccountedDocument::from_owned_readonly_threshold_alpha_storage(doc, guard, q.clone(), &control()).unwrap()
    }
}
fn patch<'a>(owner: &AccountedDocument, name: &'a str) -> ReadonlyLayerNamePatch<'a> {
    ReadonlyLayerNamePatch { layer: owner.document().layers[0].id, name }
}
#[test]
fn both_classes_copy_only_names_and_keep_native_mask_profile_source_and_max_revision() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "old", "old", false);
        let before = old.document().clone();
        let new = old.try_with_readonly_layer_name(patch(&old, " exact 名称 "), &control()).unwrap();
        let mut expected = before.clone();
        expected.layers[0].name = " exact 名称 ".into();
        let item = &mut expected.xcf_state.as_mut().unwrap().items[1];
        item.name = " exact 名称 ".into();
        item.custom_name = true;
        assert_eq!(new.document(), &expected);
        assert_eq!(old.document(), &before);
        assert!(new.quota().shares_ledger(&q));
        assert!(Arc::ptr_eq(old.document().icc_profile.as_ref().unwrap(), new.document().icc_profile.as_ref().unwrap()));
        let a = &old.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        let b = &new.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert_eq!(a.default_bytes(), &(-0.0f64).to_ne_bytes());
        assert!(Arc::ptr_eq(a.tiles().next().unwrap().1, b.tiles().next().unwrap().1));
        assert!(new.try_edit(AccountedEdit::LayerName { layer: new.document().layers[0].id, name: "generic" }, &control()).is_err());
    }
}
#[test]
fn equal_names_keep_false_custom_and_owner_without_free_quota() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "same", "same", false);
        let blocker = q.reserve(QuotaKind::Index, (16 << 20) - q.usage().unwrap().index).unwrap();
        let full = q.usage().unwrap();
        let same = old.try_with_readonly_layer_name(patch(&old, "same"), &control()).unwrap();
        assert!(same.shares_model(&old));
        assert!(!same.document().xcf_state.as_ref().unwrap().items[1].custom_name);
        assert!(old.try_with_readonly_layer_name(patch(&old, "other"), &control()).is_err());
        assert_eq!(q.usage().unwrap(), full);
        drop(blocker);
    }
}
#[test]
fn canonical_divergence_and_custom_flag_follow_upstream_reconciliation() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "target", "stale", false);
        let new = old.try_with_readonly_layer_name(patch(&old, "target"), &control()).unwrap();
        assert!(!new.shares_model(&old));
        assert!(new.document().xcf_state.as_ref().unwrap().items[1].custom_name);
        let old = fixture(&q, histogram, "stale", "target", false);
        let new = old.try_with_readonly_layer_name(patch(&old, "target"), &control()).unwrap();
        assert!(!new.document().xcf_state.as_ref().unwrap().items[1].custom_name);
    }
}
#[test]
fn exact_empty_spaces_and_existing_name_bound_are_not_trimmed_or_lowered() {
    let q = quota();
    let old = fixture(&q, true, "old", "old", false);
    let empty = old.try_with_readonly_layer_name(patch(&old, ""), &control()).unwrap();
    assert_eq!(empty.document().layers[0].name, "");
    let spaced = empty.try_with_readonly_layer_name(patch(&empty, "  名  "), &control()).unwrap();
    assert_eq!(spaced.document().layers[0].name, "  名  ");
    let name = "x".repeat(1 << 20);
    let bounded = spaced.try_with_readonly_layer_name(patch(&spaced, &name), &control()).unwrap();
    assert_eq!(bounded.document().layers[0].name, name);
}
#[test]
fn hostile_name_target_work_and_cancel_fail_without_publication_or_fees() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "old", "old", false);
        let before = q.usage().unwrap();
        for name in ["nul\0name".to_string(), "x".repeat((1 << 20) + 1)] {
            assert!(old.try_with_readonly_layer_name(patch(&old, &name), &control()).is_err());
        }
        assert!(old.try_with_readonly_layer_name(ReadonlyLayerNamePatch { layer: LayerId(0), name: "bad" }, &control()).is_err());
        let zero = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(old.try_with_readonly_layer_name(patch(&old, "new"), &zero), Err(BackingError::Work)));
        let counter = AtomicU64::new(0);
        let cancelled = || counter.fetch_add(1, Ordering::Relaxed) > 12;
        let interrupt = Interrupt::cancel_only(&cancelled);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(old.try_with_readonly_layer_name(patch(&old, "new"), &ctl), Err(BackingError::Cancelled)));
        let after_fees = || q.usage().unwrap().index > before.index;
        let interrupt = Interrupt::cancel_only(&after_fees);
        let ctl = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(old.try_with_readonly_layer_name(patch(&old, "new"), &ctl), Err(BackingError::Cancelled)));
        assert_eq!(old.document().layers[0].name, "old");
        assert_eq!(q.usage().unwrap(), before);
    }
}
#[test]
fn repeated_title_and_layer_copies_keep_bounded_flat_original_retention() {
    for histogram in [false, true] {
        let q = quota();
        let old = fixture(&q, histogram, "old", "old", false);
        let mut current = old.try_with_readonly_layer_name(patch(&old, "first"), &control()).unwrap();
        let mut last_usage = None;
        for i in 0..32 {
            current = current.try_with_document_name(if i % 2 == 0 { "title A" } else { "title B" }, &control()).unwrap();
            current = current.try_with_readonly_layer_name(patch(&current, if i % 2 == 0 { "layer A" } else { "layer B" }), &control()).unwrap();
            let usage = q.usage().unwrap();
            if let Some(before) = last_usage {
                assert_eq!(usage, before, "equal-capacity successors must not retain each predecessor");
            }
            last_usage = Some(usage);
        }
        assert_eq!(current.document().xcf_state.as_ref().unwrap().filter_stacks[0].revision, u64::MAX);
        let mask = &old.document().xcf_state.as_ref().unwrap().filter_masks[0].surface;
        assert!(Arc::ptr_eq(mask.tiles().next().unwrap().1, current.document().xcf_state.as_ref().unwrap().filter_masks[0].surface.tiles().next().unwrap().1));
    }
}
