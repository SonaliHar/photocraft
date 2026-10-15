//! UNRUN synthetic CURRENT configured-stage drafts. No original source or
//! reference-render qualification is inferred from these paid native fixtures.
use super::tests::{adopt, ctl, fixture};
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, OperationIdentity};

fn scalar(name: &str, value: f64) -> FilterArgument {
    FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) }
}
fn configured(doc: &mut Document, kind: u8, stored: SampleEncoding) {
    let (name, epoch, raw, args) = match kind {
        0 | 1 => (
            "gimp:color-balance",
            if kind == 0 { crate::gimp_color_balance::EPOCH } else { crate::gimp_color_balance::DEPTH_EPOCH },
            if kind == 0 { -113999279 } else { -1215462831 },
            vec![
                FilterArgument { name: "preserve-luminosity".into(), value: FilterValue::Bool(true) },
                FilterArgument {
                    name: "config".into(),
                    value: FilterValue::Config(
                        ["shadows", "midtones", "highlights"]
                            .map(|name| FilterArgument {
                                name: name.into(),
                                value: FilterValue::Config([scalar("cyan-red", 0.125), scalar("magenta-green", -0.25), scalar("yellow-blue", 0.0625)].into()),
                            })
                            .into(),
                    ),
                },
            ],
        ),
        2 => (
            "gimp:levels",
            "xcf-22-stock-levels-replace",
            -1080065455,
            vec![
                FilterArgument {
                    name: "trc".into(),
                    value: FilterValue::Enum { domain: "gimp:levels::trc".into(), value: if stored == SampleEncoding::Linear { 0 } else { 1 } },
                },
                FilterArgument { name: "clamp-input".into(), value: FilterValue::Bool(false) },
                FilterArgument { name: "clamp-output".into(), value: FilterValue::Bool(false) },
                FilterArgument {
                    name: "config".into(),
                    value: FilterValue::Config(
                        ["value", "red", "green", "blue", "alpha"]
                            .map(|name| FilterArgument {
                                name: name.into(),
                                value: FilterValue::Config(
                                    [
                                        scalar("low-input", 0.),
                                        scalar("high-input", 1.),
                                        scalar("gamma", 1.),
                                        scalar("low-output", 0.),
                                        scalar("high-output", 1.),
                                    ]
                                    .into(),
                                ),
                            })
                            .into(),
                    ),
                },
            ],
        ),
        3 => (
            "gimp:curves",
            "xcf-22-stock-curves-replace",
            -1080065455,
            vec![
                FilterArgument { name: "trc".into(), value: FilterValue::Enum { domain: "gimp:curves::trc".into(), value: 1 } },
                FilterArgument {
                    name: "config".into(),
                    value: FilterValue::Config(
                        ["value", "red", "green", "blue", "alpha"]
                            .map(|name| FilterArgument {
                                name: name.into(),
                                value: FilterValue::Curve {
                                    free: true,
                                    points_f64_bits: vec![],
                                    point_types: vec![],
                                    samples_f64_bits: (0..256)
                                        .map(|i| (if name == "alpha" { 0.25 + f64::from(i) / 510. } else { f64::from(i) / 255. }).to_bits())
                                        .collect(),
                                },
                            })
                            .into(),
                    ),
                },
            ],
        ),
        _ => panic!(),
    };
    let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = args;
    let policy = node.composition.as_mut().unwrap();
    policy.compatibility_epoch = epoch.into();
    policy.stored_blend_space = raw;
    policy.stored_composite_space = raw;
    policy.stored_operator = raw;
}
fn compare(owner: &AccountedDocument, profile: &Profile) {
    let doc = owner.document();
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!() };
    let state = doc.xcf_state.as_ref().unwrap();
    let context = PreparedBackedStockPointContext::new(owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    context.validate_operative(&ctl()).unwrap();
    let output = context.render_wide_band(layer.operative_bounds(), &ctl()).unwrap();
    let old = OrderedStack::new_with_interrupt(
        &state.filter_stacks[0],
        profile,
        doc.sample_encoding.unwrap(),
        22,
        layer.operative_bounds(),
        [false, true, true],
        &Interrupt::NONE,
    )
    .unwrap()
    .with_filter_output_precision(state.current_filter_output_precision);
    let mut raw = [0; 40];
    let mut i = 0;
    for y in layer.operative_bounds().y0..layer.operative_bounds().y1 {
        for x in layer.operative_bounds().x0..layer.operative_bounds().x1 {
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
            let pixel = std::array::from_fn(|c| try_read_sample_f64(&raw, doc.depth, c).unwrap());
            assert_eq!(output.pixels()[i].map(f64::to_bits), old.point_at(pixel, (x, y), &mut |_| Ok(())).unwrap().map(f64::to_bits));
            i += 1;
        }
    }
}
#[test]
fn configured_families_match_existing_ordered_math_and_disabled_identity() {
    for (kind, sample, stored) in [
        (0, SampleType::F64, SampleEncoding::NonlinearProfile),
        (1, SampleType::U8, SampleEncoding::Perceptual),
        (2, SampleType::F32, SampleEncoding::Linear),
        (2, SampleType::F32, SampleEncoding::NonlinearProfile),
        (3, SampleType::U8, SampleEncoding::NonlinearProfile),
    ] {
        for enabled in [false, true] {
            let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, sample, stored);
            configured(&mut doc, kind, stored);
            if kind >= 2 {
                super::general_tests::reframe_coverage(&mut doc, &q, photocraft_doc::Size::new(12, 9), Rect::new(0, 0, 12, 9), false);
            }
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
            let owner = adopt(doc, q, guard);
            compare(&owner, profile);
        }
    }
}
#[test]
fn structured_levels_keeps_finite_hdr_and_explicit_alpha_coverage_boundary() {
    let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F32, SampleEncoding::Linear);
    configured(&mut doc, 2, SampleEncoding::Linear);
    super::general_tests::reframe_coverage(&mut doc, &q, photocraft_doc::Size::new(12, 9), Rect::new(0, 0, 12, 9), false);
    let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    let raw = [-3f32, 2., 1.25, 0.125].map(f32::to_ne_bytes).concat();
    assert_eq!(pipeline.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap(), [-3., 2., 1.25, 0.125]);
    let FilterValue::Config(channels) = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].arguments[3].value else { panic!() };
    let FilterValue::Config(fields) = &mut channels[4].value else { panic!() };
    fields[3] = scalar("low-output", -1.);
    fields[4] = scalar("high-output", 2.);
    let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    assert_eq!(pipeline.evaluate_native(&raw, doc.pixel_format(), &ctl()).unwrap(), [-3., 2., 1.25, 0.]);
}
#[test]
fn configured_schema_profile_and_work_refusals_precede_native_reads() {
    for wrong in 0..4 {
        let (mut doc, q, guard, reads, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
        configured(&mut doc, 3, SampleEncoding::NonlinearProfile);
        super::general_tests::reframe_coverage(&mut doc, &q, photocraft_doc::Size::new(12, 9), Rect::new(0, 0, 12, 9), false);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        node.enabled = false;
        if wrong == 0 {
            node.composition.as_mut().unwrap().stored_operator = 0;
        }
        if wrong == 1 {
            let FilterValue::Config(channels) = &mut node.arguments[1].value else { panic!() };
            let FilterValue::Curve { free, .. } = &mut channels[0].value else { panic!() };
            *free = false;
        }
        if wrong == 2 {
            node.operation.schema_epoch = gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
        }
        let owner = adopt(doc, q, guard);
        let before = reads.load(std::sync::atomic::Ordering::SeqCst);
        if wrong == 3 {
            let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
            assert!(PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &control).is_err());
        } else {
            assert!(PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).is_err());
        }
        assert_eq!(before, reads.load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[test]
fn balance_depth_half_requires_exact_widened_input_even_disabled() {
    let (mut doc, q, _guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F32, SampleEncoding::Linear);
    doc.source_color_context.as_mut().unwrap().source_precision = 500;
    configured(&mut doc, 1, SampleEncoding::Linear);
    doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = false;
    let pipeline = PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).unwrap();
    let exact = [0.125f32, 0.5, 0.75, 0.25].map(f32::to_ne_bytes).concat();
    assert_eq!(pipeline.evaluate_native(&exact, doc.pixel_format(), &ctl()).unwrap(), [0.125, 0.5, 0.75, 0.25]);
    let inexact = [0.1f32, 0.5, 0.75, 0.25].map(f32::to_ne_bytes).concat();
    assert!(pipeline.evaluate_native(&inexact, doc.pixel_format(), &ctl()).is_err());
    doc.source_color_context.as_mut().unwrap().source_precision = 600;
    assert!(PreparedStockPointPipeline::prepare(&doc, profile, &q, &ctl()).is_err());
}
