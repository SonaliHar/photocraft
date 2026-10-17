//! UNRUN observed-source compatibility policy controls. Generated inputs are
//! paid structural harness fixtures, not additional GIMP/ICC numerical evidence.
use super::*;

fn activity(doc: &mut Document, visible: bool, opacity: f32, enabled: bool) {
    let layer = &mut doc.layers[0];
    layer.visible = visible;
    layer.opacity = opacity;
    let id = layer.id;
    let state = doc.xcf_state.as_mut().unwrap();
    state.items.iter_mut().find(|item| item.item == ItemRef::Layer(id)).unwrap().visible = visible;
    state.filter_stacks[0].nodes[0].enabled = enabled;
}
#[test]
#[ignore = "UNRUN all three observed roles generic activity cross-product with unchanged full proof cost"]
fn source_activity_cross_product_keeps_profile_native_and_space_preparation() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for radius in [0, 4] {
            let (d, q, g, _) = observed_role(sample, assigned, s, radius);
            let mut input = resident_source(d, &q, g, s);
            let mut proof_work = None;
            for visible in [false, true] {
                for opacity in [-0.0, 0.0, 0.375, f32::from_bits(1), 1.0] {
                    for enabled in [false, true] {
                        activity(&mut input.document, visible, opacity, enabled);
                        let raw = input.document.clone(); // External expected metadata only.
                        let before = q.usage().unwrap();
                        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
                        let prepared = LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).unwrap();
                        assert!(q.usage().unwrap().resident > before.resident);
                        let consumed = u64::MAX - budget.work;
                        if let Some(expected) = proof_work {
                            assert_eq!(consumed, expected);
                        } else {
                            proof_work = Some(consumed);
                        }
                        assert_eq!(input.document, raw);
                        assert_eq!(input.document.layers[0].opacity.to_bits(), opacity.to_bits());
                        drop(prepared);
                        assert_eq!(q.usage().unwrap(), before);
                    }
                }
            }
            // The observed bool policy does not enroll the older literal tuple
            // constructor. Exact old disabled source still refuses there.
            let mut old = input.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].clone();
            old.enabled = false;
            old.operation.schema_epoch = crate::gimp_neighborhood::box_blur::EPOCH.into();
            let c = old.composition.as_mut().unwrap();
            c.compatibility_epoch = crate::gimp_neighborhood::box_blur::EPOCH.into();
            c.stored_blend_space = -1474592175;
            c.stored_composite_space = -1474592175;
            c.stored_operator = -1474592175;
            assert!(crate::gimp_neighborhood::box_blur::radius(&old, 22, Rect::new(0, 0, 16, 12)).is_err());
        }
    }
}
#[test]
#[ignore = "UNRUN hidden zero disabled source still rejects malformed physical mask/native/profile and filter metadata"]
fn inactive_source_never_waives_full_inputs_or_effective_opacity_validation() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        for defect in 0..10 {
            let (d, q, g, _) = observed_role(sample, assigned, s, 4);
            let mut input = resident_source(d, &q, g, s);
            activity(&mut input.document, false, 0.0, false);
            match defect {
                0 => input.document.layers[0].opacity = f32::NAN,
                1 => input.document.layers[0].opacity = 1.0001,
                2 => {
                    let id = input.document.layers[0].id;
                    input.document.xcf_state.as_mut().unwrap().items.iter_mut().find(|v| v.item == ItemRef::Layer(id)).unwrap().visible = true;
                }
                3 => input.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = 0.5f64.to_bits(),
                4 => {
                    let pixel: Vec<u8> = [f64::NAN, 0., 0., 1.]
                        .into_iter()
                        .flat_map(|v| if sample == SampleType::F32 { (v as f32).to_ne_bytes().to_vec() } else { v.to_ne_bytes().to_vec() })
                        .collect();
                    let LayerContent::Raster(v) = &mut input.document.layers[0].content else { panic!() };
                    // Off-operative retained words still participate in source proof.
                    v.try_write_interleaved(Rect::new(255, 255, 256, 256), &pixel).unwrap();
                }
                5 => {
                    let word = if sample == SampleType::F32 { f32::from_bits(1).to_ne_bytes().to_vec() } else { f64::from_bits(1).to_ne_bytes().to_vec() };
                    input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface.try_write_interleaved(Rect::new(255, 255, 256, 256), &word).unwrap();
                }
                6 => input.document.icc_profile = Some(Arc::new(vec![0; 640])),
                7 => input.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].source_region = Some(1),
                8 => {
                    let pixel: Vec<u8> = [f64::NAN, 0., 0., 0.]
                        .into_iter()
                        .flat_map(|v| if sample == SampleType::F32 { (v as f32).to_ne_bytes().to_vec() } else { v.to_ne_bytes().to_vec() })
                        .collect();
                    input.document.layers[0].content = LayerContent::Raster(Surface::try_with_default_bytes(input.document.pixel_format(), &pixel).unwrap());
                }
                _ => {
                    let word = if sample == SampleType::F32 { 1f32.to_ne_bytes().to_vec() } else { 1f64.to_ne_bytes().to_vec() };
                    let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false);
                    input.document.xcf_state.as_mut().unwrap().filter_masks[0].surface = Surface::try_with_default_bytes(format, &word).unwrap();
                }
            }
            let before = q.usage().unwrap();
            let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
            assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut budget, &input).is_err());
            assert_eq!(q.usage().unwrap(), before);
        }
    }
}
#[test]
#[ignore = "UNRUN inactive source retains genuine profile/space fees and cumulative work/cancellation failure"]
fn inactive_source_has_no_fee_or_work_discount_and_failure_refunds() {
    let s = Rect::new(-3, 5, 2, 8);
    for (sample, assigned) in [(SampleType::F32, false), (SampleType::F64, false), (SampleType::F32, true)] {
        let (d, q, g, _) = observed_role(sample, assigned, s, 0);
        let mut input = resident_source(d, &q, g, s);
        activity(&mut input.document, false, -0.0, false);
        let before = q.usage().unwrap();
        let polls = AtomicU64::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let interrupt = Interrupt::cancel_only(&count);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        drop(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input).unwrap());
        let consumed = u64::MAX - budget.work;
        assert!(consumed > 0);
        let total = polls.load(Ordering::Relaxed);
        assert!(total > 2);
        let mut lower = Budget { q: q.clone(), work: consumed - 1, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut lower, &input).is_err());
        let mut quota = Budget { q: q.clone(), work: u64::MAX, deny: true };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &Interrupt::NONE, &mut quota, &input).is_err());
        let seen = AtomicU64::new(0);
        let cancel = || seen.fetch_add(1, Ordering::Relaxed) >= total - 1;
        let interrupt = Interrupt::cancel_only(&cancel);
        let mut budget = Budget { q: q.clone(), work: u64::MAX, deny: false };
        assert!(LeasedStreamedBoxPipeline::new_geometry_budgeted(&input.document, &interrupt, &mut budget, &input).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
}
