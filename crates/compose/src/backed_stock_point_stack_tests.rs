//! UNRUN paid synthetic CURRENT stack drafts. Optional-mask cases exercise
//! existing policy semantics; they claim no independently observed source file.
use super::tests::{adopt, ctl, fixture};
use super::*;
use photocraft_doc::{
    import_compositing::GimpSpace,
    item_identity::MaskId,
    xcf_filters::{ExactNumber, FilterArgument},
};

fn configure(doc: &mut Document, count: usize, masks: usize, reverse: bool, disabled: bool, half: bool) {
    let layer_id = doc.layers[0].id;
    doc.layers[0].import_compositing.as_mut().unwrap().composite_encoding = SampleEncoding::NonlinearProfile;
    let state = doc.xcf_state.as_mut().unwrap();
    let item = state.items.iter_mut().find(|item| item.item == ItemRef::Layer(layer_id)).unwrap();
    let composition = item.composition.as_mut().unwrap();
    composition.stored_composite_space = 2;
    composition.stored_operator = 1;
    composition.resolved.as_mut().unwrap().composite_space = GimpSpace::ProfileRgb;
    let template = state.filter_stacks[0].nodes[0].clone();
    let mask_template = state.filter_masks[0].clone();
    let item_template = state.items.iter().find(|item| matches!(item.item, ItemRef::Mask(_))).unwrap().clone();
    state.items.retain(|item| item.item == ItemRef::Layer(layer_id));
    state.filter_masks.clear();
    state.filter_stacks[0].nodes.clear();
    for i in 0..count {
        let mut node = template.clone();
        node.id = FilterId(layer_id.0.checked_add(1000 + i as u64).unwrap());
        let invert = (i == 0) != reverse;
        node.operation.name = if invert { "gegl:invert-linear" } else { "gegl:exposure" }.into();
        node.operation.stored_version = if invert { "0:0" } else { "1:0" }.into();
        node.operation.schema_epoch = gimp_filters::STACK_EDITS_EPOCH.into();
        node.arguments = if invert {
            vec![]
        } else {
            [("exposure", 0.5f32), ("black-level", 0.)]
                .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float32Bits(v.to_bits())) })
                .into()
        };
        node.opacity_f64_bits = if invert && half { 0.5f64 } else { 1.0 }.to_bits();
        node.enabled = !(disabled && i == 0);
        let policy = node.composition.as_mut().unwrap();
        policy.compatibility_epoch = gimp_filters::STACK_EDITS_EPOCH.into();
        policy.stored_blend_space = -1178762671;
        policy.stored_composite_space = -1178762671;
        policy.stored_operator = -1178762671;
        node.mask = None;
        if i < masks {
            let id = MaskId(layer_id.0.checked_add(1100 + i as u64).unwrap());
            node.mask = Some(ItemRef::Mask(id));
            let mut mask = mask_template.clone();
            mask.id = id;
            mask.owner = node.id;
            state.filter_masks.push(mask);
            let mut item = item_template.clone();
            item.item = ItemRef::Mask(id);
            state.items.push(item);
        }
        state.filter_stacks[0].nodes.push(node);
    }
}
fn compare(owner: &AccountedDocument, profile: &Profile) -> Vec<[u64; 4]> {
    let doc = owner.document();
    let state = doc.xcf_state.as_ref().unwrap();
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
    let context = PreparedBackedStockPointContext::new(owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    assert_eq!(context.node_ids().collect::<Vec<_>>(), state.filter_stacks[0].nodes.iter().map(|node| node.id).collect::<Vec<_>>());
    context.validate_operative(&ctl()).unwrap();
    let band = context.render_wide_band(doc.bounds(), &ctl()).unwrap();
    let legacy =
        OrderedStack::new_with_interrupt(&state.filter_stacks[0], profile, SampleEncoding::NonlinearProfile, 22, doc.bounds(), [false; 3], &Interrupt::NONE)
            .unwrap();
    let mut raw = [0; 4];
    let mut i = 0;
    for y in 0..6 {
        for x in 0..8 {
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw, &ctl()).unwrap();
            let pixel = raw.map(|v| f64::from(v) / 255.);
            assert_eq!(band.pixels()[i].map(f64::to_bits), legacy.point_at(pixel, (x, y), &mut |_| Ok(())).unwrap().map(f64::to_bits));
            i += 1;
        }
    }
    band.pixels().iter().map(|pixel| pixel.map(f64::to_bits)).collect()
}
#[test]
fn witnessed_order_disable_half_opacity_and_optional_masks_match_whole_legacy_stack() {
    let mut different = Vec::new();
    for reverse in [false, true] {
        for disabled in [false, true] {
            for half in [false, true] {
                for masks in 0..=2 {
                    let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
                    configure(&mut doc, 2, masks, reverse, disabled, half);
                    let owner = adopt(doc, q, guard);
                    let output = compare(&owner, profile);
                    if !disabled && !half && masks == 0 {
                        different.push(output);
                    }
                }
            }
        }
    }
    assert_ne!(different[0], different[1], "saved node reversal must change operative ordering");
}
#[test]
fn removed_single_node_keeps_stack_epoch_and_no_new_u8_intermediate_quantizer() {
    for reverse in [false, true] {
        for masks in 0..=1 {
            let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
            configure(&mut doc, 1, masks, reverse, false, true);
            let owner = adopt(doc, q, guard);
            let output = compare(&owner, profile);
            assert!(output.iter().any(|pixel| pixel[..3].iter().any(|bits| {
                let v = f64::from_bits(*bits);
                v * 255.0 != (v * 255.0).round()
            })));
        }
    }
}
#[test]
fn stack_recipe_refuses_mixed_epochs_shared_masks_wrong_owner_and_unwitnessed_values() {
    for wrong in 0..5 {
        let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
        configure(&mut doc, 2, 2, false, false, true);
        let state = doc.xcf_state.as_mut().unwrap();
        match wrong {
            0 => state.filter_stacks[0].nodes[1].operation.schema_epoch = gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into(),
            1 => state.filter_stacks[0].nodes[1].mask = state.filter_stacks[0].nodes[0].mask,
            2 => state.items[0].composition.as_mut().unwrap().stored_composite_space = -1,
            3 => state.filter_stacks[0].nodes[0].opacity_f64_bits = 0.25f64.to_bits(),
            _ => state.filter_stacks[0].nodes[1].arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())),
        }
        assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
    }
}
#[test]
fn stack_preparation_shares_work_and_releases_real_private_fees_on_refusal() {
    let (mut doc, q, guard, reads, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
    configure(&mut doc, 2, 2, false, false, true);
    let owner = adopt(doc, q.clone(), guard);
    let before = q.usage().unwrap();
    let used = std::sync::atomic::AtomicU64::new(0);
    let work = |units| {
        let previous = used.fetch_add(units, std::sync::atomic::Ordering::SeqCst);
        if previous.saturating_add(units) > 8192 { Err(BackingError::Work) } else { Ok(()) }
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let initial_reads = reads.load(std::sync::atomic::Ordering::SeqCst);
    assert!(PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &control).is_err());
    assert!(used.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(initial_reads, reads.load(std::sync::atomic::Ordering::SeqCst));
}
