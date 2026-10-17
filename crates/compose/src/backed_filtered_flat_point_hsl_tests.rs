//! Ignored UNRUN typed HSL integration contracts; synthetic compatibility, not GIMP oracle.
use super::*;
fn set_hsl(node: &mut photocraft_doc::xcf_filters::FilterNode) {
    set_stock(node, 3);
    node.operation.schema_epoch = crate::ALIEN_MAP_HSL_MIXED_EPOCH.into();
    node.composition.as_mut().unwrap().compatibility_epoch = crate::ALIEN_MAP_HSL_MIXED_EPOCH.into();
    node.arguments[0].value = photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 };
}
fn hsl_fixture(sample: SampleType, amount: f64, size: Size) -> Fixture {
    let mut f = stock_fixture(sample, 3, amount, size);
    set_hsl(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0]);
    f
}
// Public strict old HSL kernel selected independently for these unit inputs.
// It is harness work, not a production fee or exact producer-format oracle.
fn hsl_unit(p: [f64; 4]) -> [f64; 4] {
    let operation = photocraft_algo::gimp::point::PointOperation::AlienMapHsl { frequency: [1., 2.5, 0.], phase: [0., 91., 360.], keep: [false, true, false] }
        .prepare()
        .unwrap();
    let mut out = [0.; 4];
    operation.try_eval_pixel(&p, 3, &mut out).unwrap();
    out
}
#[test]
#[ignore = "UNRUN exactmodel1 pairedHSL solepaidroot, actualSource/SourceOpacity+Current bothdepth/fullorigin, widealpha/rawarguments/propertygroups/nativecustody and existingNormal reference"]
fn source_current_and_wide_alpha_custody() {
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for size in [Size::new(8, 6), Size::new(130, 65)] {
            for amount in [1., 0., -0., 0.375] {
                let mut f = hsl_fixture(sample, amount, size);
                groups(&mut f);
                let nodes = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone();
                let raw = f.document.xcf_state.as_ref().unwrap().opaque.clone();
                let q = f.quota.clone();
                let baseline = q.usage().unwrap();
                let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                if amount == 1. {
                    drop(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                    assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
                } else {
                    drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                    assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
                }
                assert_eq!(q.usage().unwrap(), baseline);
                let (o, _, _, _) = adopt(f);
                let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                c.validate_operative(&ctl()).unwrap();
                for x in 0u32..5 {
                    let p = input(0, x, 0, sample);
                    let expected = mix_reference(p, hsl_unit(p), amount);
                    assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                    if amount == 1. {
                        assert_eq!(expected[3].to_bits(), p[3].to_bits());
                    }
                    let normal = working
                        .normal(working.normal([0.; 4], expected, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                        .unwrap();
                    assert_eq!(c.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits), normal.map(f64::to_bits));
                }
                assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, nodes);
                assert_eq!(o.document().xcf_state.as_ref().unwrap().opaque, raw);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN namedHSL/Sepia order usesindependentfixedsequences notrenderertraversal, laterunitcheckedderived path, enabledzero fullkernel/bridge versusdisabled widepass"]
fn named_order_and_enabled_zero_boundaries() {
    for sample in [SampleType::F32, SampleType::F64] {
        for hsl_first in [true, false] {
            let mut f = ordered(sample, &[&[0, 0], &[]], true);
            let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
            if hsl_first {
                set_stock(&mut nodes[0], 2);
                set_hsl(&mut nodes[1]);
            } else {
                set_hsl(&mut nodes[0]);
                set_stock(&mut nodes[1], 2);
            }
            let (o, _, _, _) = adopt(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let hsl_then_sepia = stock(2, bridge(hsl_unit(p)));
                let sepia_then_hsl = hsl_unit(bridge(stock(2, p)));
                assert_ne!(hsl_then_sepia[..3], sepia_then_hsl[..3]);
                let expected = if hsl_first { hsl_then_sepia } else { sepia_then_hsl };
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        for enabled in [true, false] {
            let mut f = ordered(sample, &[&[0, 0, 0], &[]], true);
            let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
            set_stock(&mut nodes[0], 2);
            nodes[0].opacity_f64_bits = 0.375f64.to_bits();
            set_hsl(&mut nodes[1]);
            nodes[1].opacity_f64_bits = (-0f64).to_bits();
            nodes[1].enabled = enabled;
            set_kind(&mut nodes[2], 5);
            let (o, _, _, _) = current(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let invert = [1. - p[0], 1. - p[1], 1. - p[2], p[3]];
                let before = if enabled {
                    let p = bridge(invert);
                    let _full = hsl_unit(p);
                    bridge(p.map(|v| f64::from(v as f32)))
                } else {
                    bridge(invert)
                };
                let expected = mix_reference(before, stock(2, before), 0.375);
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN exactHSL/RGB crossedmodel/epoch andtyped Source refusal, allinactive native/mask/profile proof, actual4480path work/cancel/coldlaterroot provider cleanup"]
fn exact_models_inactive_proof_and_controls() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    for fault in 0..5 {
        let mut f = hsl_fixture(SampleType::F64, 0.375, Size::new(8, 6));
        let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match fault {
            0 => node.arguments[0].value = FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 },
            1 => {
                node.operation.schema_epoch = crate::ALIEN_MAP_RGB_MIXED_EPOCH.into();
                node.composition.as_mut().unwrap().compatibility_epoch = crate::ALIEN_MAP_RGB_MIXED_EPOCH.into();
            }
            2 => node.arguments[7].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
            3 => node.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits())),
            _ => node.composition.as_mut().unwrap().compatibility_epoch = "xcf-22-stock-alien-map-hsl-precision-v1".into(),
        }
        groups(&mut f);
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let reads = f.reads.load(Ordering::Relaxed);
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
        assert_eq!(f.reads.load(Ordering::Relaxed), reads);
    }
    for fault in 0..3 {
        let mut f = hsl_fixture(SampleType::F64, -0., Size::new(8, 6));
        if fault == 2 {
            f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
            let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
            set_hsl(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0]);
        }
        f.document.layers[0].visible = false;
        f.document.layers[0].opacity = 0.;
        let id = f.document.layers[0].id;
        let state = f.document.xcf_state.as_mut().unwrap();
        state.items.iter_mut().find(|r| r.item == ItemRef::Layer(id)).unwrap().visible = false;
        state.filter_stacks[0].nodes[0].enabled = false;
        match fault {
            0 => state.filter_masks[0].surface.try_write_interleaved(Rect::new(200, 200, 201, 201), &f64::NAN.to_ne_bytes()).unwrap(),
            1 => f.document.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb { icc_blake3: "0".repeat(64) },
            _ => {}
        }
        let (o, _, _, _) = current(f);
        let result = Context::new(&o, Default::default(), &ctl());
        if fault < 2 {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().validate_operative(&ctl()).is_err());
        }
    }
    let (o, rasters, fails, _) = current(hsl_fixture(SampleType::F64, 0.375, Size::new(8, 6)));
    let q = o.quota().clone();
    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
    c.validate_operative(&ctl()).unwrap();
    let r = Rect::new(0, 0, 2, 1);
    drop(c.render_band(r, &ctl()).unwrap());
    let baseline = q.usage().unwrap();
    let total = AtomicU64::new(0);
    let count = |n| {
        total.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    drop(c.render_band(r, &ReadControl { interrupt: &Interrupt::NONE, work: &count }).unwrap());
    let limit = total.load(Ordering::Relaxed) - 1;
    let used = AtomicU64::new(0);
    let deny = |n| if used.fetch_add(n, Ordering::Relaxed).saturating_add(n) > limit { Err(BackingError::Work) } else { Ok(()) };
    assert!(matches!(c.render_band(r, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }), Err(E::Backing(BackingError::Work))));
    assert_eq!(q.usage().unwrap(), baseline);
    let polls = AtomicU64::new(0);
    let observe = || {
        polls.fetch_add(1, Ordering::Relaxed);
        false
    };
    let i = Interrupt::cancel_only(&observe);
    drop(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }).unwrap());
    let accepted = polls.load(Ordering::Relaxed);
    let polls = AtomicU64::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= accepted - 1;
    let i = Interrupt::cancel_only(&cancel);
    assert!(matches!(c.render_band(r, &ReadControl { interrupt: &i, work: &|_| Ok(()) }), Err(E::Backing(BackingError::Cancelled))));
    assert_eq!(q.usage().unwrap(), baseline);
    rasters[1].clear_cache().unwrap();
    let before = q.usage().unwrap();
    fails[1].store(true, Ordering::Relaxed);
    assert!(matches!(c.render_band(r, &ctl()), Err(E::View(photocraft_raster::checked_view::RasterViewError::Backed(BackingError::Io)))));
    assert_eq!(q.usage().unwrap(), before);
    fails[1].store(false, Ordering::Relaxed);
}
