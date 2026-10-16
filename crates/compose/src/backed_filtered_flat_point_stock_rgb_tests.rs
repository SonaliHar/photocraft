//! Ignored UNRUN synthetic paid contracts; no GIMP projection/display oracle.
use super::*;
fn stock_kind(variant: u8) -> crate::StockRgbKind {
    match variant {
        0 => crate::StockRgbKind::ColorTemperature,
        1 | 2 => crate::StockRgbKind::Sepia,
        _ => crate::StockRgbKind::AlienRgb,
    }
}
fn bool_arg(name: &str, value: bool) -> photocraft_doc::xcf_filters::FilterArgument {
    photocraft_doc::xcf_filters::FilterArgument { name: name.into(), value: photocraft_doc::xcf_filters::FilterValue::Bool(value) }
}
fn set_stock(n: &mut photocraft_doc::xcf_filters::FilterNode, variant: u8) {
    let kind = stock_kind(variant);
    n.operation = OperationIdentity { name: kind.operation_name().into(), stored_version: kind.version().into(), schema_epoch: kind.epoch().into() };
    n.composition.as_mut().unwrap().compatibility_epoch = kind.epoch().into();
    n.arguments = match variant {
        0 => vec![number("original-temperature", 6500.), number("intended-temperature", 4000.)],
        1 | 2 => vec![number("scale", 0.625), bool_arg("srgb", variant == 2)],
        _ => vec![
            photocraft_doc::xcf_filters::FilterArgument {
                name: "color-model".into(),
                value: photocraft_doc::xcf_filters::FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 0 },
            },
            number("cpn-1-frequency", 1.),
            number("cpn-2-frequency", 2.5),
            number("cpn-3-frequency", 0.),
            number("cpn-1-phaseshift", 0.),
            number("cpn-2-phaseshift", 91.),
            number("cpn-3-phaseshift", 360.),
            bool_arg("cpn-1-keep", false),
            bool_arg("cpn-2-keep", true),
            bool_arg("cpn-3-keep", false),
        ],
    };
}
fn stock_fixture(sample: SampleType, variant: u8, amount: f64, size: Size) -> Fixture {
    let mut f = if size == Size::new(8, 6) {
        ordered(sample, &[&[0], &[]], true)
    } else {
        let mut f = fixture(sample, size, &[Some(0), None], None);
        let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
        let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
        f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
        f
    };
    let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    set_stock(node, variant);
    node.opacity_f64_bits = amount.to_bits();
    f
}
// Harness-only public curve and old public coefficient extraction. No quota,
// independent-producer or exact-format claim is attached to these references.
fn stock(variant: u8, p: [f64; 4]) -> [f64; 4] {
    use photocraft_algo::gimp::point::PointOperation;
    let curve = photocraft_cms::srgb_trc();
    let linear = variant < 2;
    let mut w = p;
    if linear {
        for v in &mut w[..3] {
            *v = curve.eval64(*v);
        }
    }
    let mut out = w;
    match variant {
        0 => {
            let mut gain = [0.; 4];
            PointOperation::ColorTemperature { original: 6500., intended: 4000. }.prepare().unwrap().try_eval_pixel(&[1.; 4], 3, &mut gain).unwrap();
            for c in 0..3 {
                out[c] = f64::from((w[c] as f32) * (gain[c] as f32));
            }
        }
        1 | 2 => {
            let rows = [[0x3ec9374c, 0x3f44dd2f, 0x3e418937], [0x3eb2b021, 0x3f2f9db2, 0x3e2c0831], [0x3e8b4396, 0x3f08b439, 0x3e0624dd]];
            let rgb = [w[0] as f32, w[1] as f32, w[2] as f32];
            for (i, row) in rows.into_iter().enumerate() {
                let t = rgb.into_iter().zip(row).map(|(v, b)| v * f32::from_bits(b)).sum::<f32>();
                out[i] = f64::from(rgb[i] * (1. - 0.625f32) + t * 0.625f32);
            }
        }
        _ => {
            let freq = [1., 2.5, 0.];
            let phase = [0f64, 91., 360.];
            for c in [0, 2] {
                let angle = (2. * f64::from(w[c] as f32) - 1.) * std::f64::consts::PI * freq[c] + phase[c].to_radians();
                out[c] = f64::from(((1. + angle.sin()) * 0.5) as f32);
            }
        }
    }
    if linear {
        for v in &mut out[..3] {
            *v = curve.eval_inverse64(*v);
        }
    }
    out
}
#[test]
#[ignore = "UNRUN exactthree kinds/bothSepia spaces/bothdepths/fullorigin actualpaidSource andSourceOpacity rawgroups/soleStage/Normal reference, synthetic declaredcrossproduct notproduceroracle"]
fn source_current_and_solo_stage_custody() {
    let working =
        WorkingSpace::new_with_interrupt(photocraft_cms::Builtin::Srgb.profile(), SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, &Interrupt::NONE)
            .unwrap();
    for sample in [SampleType::F32, SampleType::F64] {
        for variant in 0..4 {
            for size in [Size::new(8, 6), Size::new(130, 65)] {
                for amount in [1., 0., -0., 0.375] {
                    let mut f = stock_fixture(sample, variant, amount, size);
                    groups(&mut f);
                    let nodes = f.document.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.clone();
                    let raw = f.document.xcf_state.as_ref().unwrap().opaque.clone();
                    let q = f.quota.clone();
                    let before = q.usage().unwrap();
                    let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
                    if amount == 1. {
                        drop(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                        assert!(
                            LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err()
                        );
                    } else {
                        drop(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).unwrap());
                        assert!(LeasedFilteredFlatPointPipeline::new_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
                    }
                    assert_eq!(q.usage().unwrap(), before);
                    let (o, _, _, _) = adopt(f);
                    let c = Context::new(&o, Default::default(), &ctl()).unwrap();
                    c.validate_operative(&ctl()).unwrap();
                    for x in 0u32..5 {
                        let p = input(0, x, 0, sample);
                        let expected = mix_reference(p, stock(variant, p), amount);
                        assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
                        let normal = working
                            .normal(working.normal([0.; 4], expected, 1., GimpOperator::Union).unwrap(), input(1, x, 0, sample), 0.375, GimpOperator::Union)
                            .unwrap();
                        assert_eq!(
                            c.render_band(Rect::new(x as i32, 0, x as i32 + 1, 1), &ctl()).unwrap().pixels()[0].map(f64::to_bits),
                            normal.map(f64::to_bits)
                        );
                    }
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes, nodes);
                    assert_eq!(o.document().xcf_state.as_ref().unwrap().opaque, raw);
                }
            }
        }
    }
}
#[test]
#[ignore = "UNRUN independentnamed Temperature-Sepia/Alien noncommutingorder, existingenabledNLFloatbridge and opacityzero versusdisabled, actualderivedworking/literalwidealphatails"]
fn ordered_named_sequences_and_enabled_zero() {
    for sample in [SampleType::F32, SampleType::F64] {
        for order in [[0, 1], [1, 0], [3, 2], [2, 3]] {
            let mut f = ordered(sample, &[&[0, 0], &[]], true);
            for (n, v) in f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes.iter_mut().zip(order) {
                set_stock(n, v);
            }
            let (o, _, _, _) = adopt(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let temp_then_sepia = stock(1, bridge(stock(0, p)));
                let sepia_then_temp = stock(0, bridge(stock(1, p)));
                let alien_then_sepia = stock(2, bridge(stock(3, p)));
                let sepia_then_alien = stock(3, bridge(stock(2, p)));
                assert_ne!(temp_then_sepia[..3], sepia_then_temp[..3]);
                assert_ne!(alien_then_sepia[..3], sepia_then_alien[..3]);
                let expected = match order {
                    [1, 0] => temp_then_sepia,
                    [0, 1] => sepia_then_temp,
                    [2, 3] => alien_then_sepia,
                    _ => sepia_then_alien,
                };
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
        for enabled in [true, false] {
            let mut f = ordered(sample, &[&[0, 0, 0], &[]], true);
            let nodes = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes;
            set_stock(&mut nodes[0], 3);
            nodes[0].opacity_f64_bits = 0.375f64.to_bits();
            set_stock(&mut nodes[1], 1);
            nodes[1].opacity_f64_bits = (-0f64).to_bits();
            nodes[1].enabled = enabled;
            set_stock(&mut nodes[2], 0);
            let (o, _, _, _) = current(f);
            let c = Context::new(&o, Default::default(), &ctl()).unwrap();
            c.validate_operative(&ctl()).unwrap();
            for x in 0u32..5 {
                let p = input(0, x, 0, sample);
                let t = stock(0, p);
                let before = if enabled {
                    let p = bridge(t);
                    let _full = stock(1, p);
                    bridge(p.map(|v| f64::from(v as f32)))
                } else {
                    bridge(t)
                };
                let expected = mix_reference(before, stock(3, before), 0.375);
                assert_eq!(root_value(&c, 0, x as i32, 0).map(f64::to_bits), expected.map(f64::to_bits));
            }
        }
    }
}
#[test]
#[ignore = "UNRUN oldsingleton/HSL/wrongtyped/invalidbound fullsourcefinalrefusal, fullinactive native/mask/profile proof and actualwarmWork/cancel/coldlaterrootIocleanup"]
fn exact_role_refusal_and_unconditional_control() {
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    for fault in 0..6 {
        let mut f = stock_fixture(SampleType::F64, 3, 0.375, Size::new(8, 6));
        let node = &mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        match fault {
            0 => {
                node.operation.schema_epoch = "xcf-22-stock-alien-map-precision-v1".into();
                node.composition.as_mut().unwrap().compatibility_epoch = node.operation.schema_epoch.clone();
            }
            1 => node.arguments[0].value = FilterValue::Enum { domain: "gegl:alien-map/color-model".into(), value: 1 },
            2 => node.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits())),
            3 => node.arguments[7].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
            4 => {
                node.operation.stored_version = "1:0".into();
            }
            _ => node.arguments[1] = number("cpn-1-frequency", 21.),
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
    for fault in 0..4 {
        let mut f = stock_fixture(SampleType::F64, 0, -0., Size::new(8, 6));
        if fault == 3 {
            f = fixture(SampleType::F64, Size::new(8, 6), &[Some(0), None], Some(0));
            let index = f.quota.reserve(QuotaKind::Index, 8 << 20).unwrap();
            let resident = f.quota.reserve(QuotaKind::Resident, 8 << 20).unwrap();
            f.retention = Arc::new(Extra { _prior: f.retention, _index: index, _resident: resident });
            set_stock(&mut f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0], 0);
            f.document.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits = (-0f64).to_bits();
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
            2 => state.filter_stacks[0].nodes[0].source_clipping = Some(1),
            _ => {}
        }
        groups(&mut f);
        if fault == 3 {
            // Source BackedRaster payload proof belongs to mandatory CURRENT promotion.
            let (owner, _, _, _) = current(f);
            let context = Context::new(&owner, Default::default(), &ctl()).unwrap();
            assert!(context.validate_operative(&ctl()).is_err());
            continue;
        }
        let q = f.quota.clone();
        let before = q.usage().unwrap();
        let mut b = Budget { q: q.clone(), used: 0, limit: u64::MAX };
        assert!(LeasedFilteredFlatPointPipeline::new_opacity_source_budgeted(&f.document, &Interrupt::NONE, &mut b, f.retention.clone()).is_err());
        assert_eq!(q.usage().unwrap(), before);
    }
    for variant in 0..4 {
        let (o, rasters, fails, _) = current(stock_fixture(SampleType::F64, variant, 0.375, Size::new(8, 6)));
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
        assert!(matches!(c.render_band(r, &ctl()), Err(E::Backing(BackingError::Io))));
        assert_eq!(q.usage().unwrap(), before);
        fails[1].store(false, Ordering::Relaxed);
    }
}

#[path = "backed_filtered_flat_point_hsl_tests.rs"]
mod hsl;
