//! UNRUN CURRENT-only drafts. Synthetic paid models establish no original
//! source qualification or numerical parity with a reference application.
use super::tests::{adopt, ctl, fixture};
use super::*;
use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, OperationIdentity};

fn configure(doc: &mut Document, family: u8, choice: i32) {
    let (name, epoch, raw, arguments) = match family {
        0 => (
            "gimp:desaturate",
            gimp_filters::DESATURATE_PRECISION_EPOCH,
            1789100625,
            vec![FilterArgument { name: "mode".into(), value: FilterValue::Enum { domain: "GimpDesaturateMode".into(), value: choice } }],
        ),
        1..=3 => (
            "gegl:saturation",
            match family {
                1 => gimp_filters::SATURATION_PRECISION_EPOCH,
                2 => gimp_filters::SATURATION_GRAY_PRECISION_EPOCH,
                _ => gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH,
            },
            -1581350319,
            vec![
                FilterArgument { name: "scale".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
                FilterArgument { name: "colorspace".into(), value: FilterValue::Enum { domain: "GeglSaturationType".into(), value: choice } },
            ],
        ),
        4 => ("gegl:value-invert", gimp_filters::VALUE_INVERT_PRECISION_EPOCH, -1395293615, vec![]),
        5 => (
            "gimp:posterize",
            "xcf-22-stock-point-replace",
            -261651887,
            vec![FilterArgument { name: "levels".into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(choice) }) }],
        ),
        _ => panic!(),
    };
    let state = doc.xcf_state.as_mut().unwrap();
    let node = &mut state.filter_stacks[0].nodes[0];
    node.operation = OperationIdentity { name: name.into(), stored_version: "0:0".into(), schema_epoch: epoch.into() };
    node.arguments = arguments;
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
    let current = PreparedBackedStockPointContext::new(owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    current.validate_operative(&ctl()).unwrap();
    let output = current.render_wide_band(layer.operative_bounds(), &ctl()).unwrap();
    let legacy = OrderedStack::new_with_interrupt(
        &state.filter_stacks[0],
        profile,
        doc.sample_encoding.unwrap(),
        22,
        layer.operative_bounds(),
        [false; 3],
        &Interrupt::NONE,
    )
    .unwrap()
    .with_current_point_storage(Some(doc.depth))
    .with_filter_output_precision(state.current_filter_output_precision);
    let mut raw = [0u8; 40];
    let mut index = 0;
    for y in layer.operative_bounds().y0..layer.operative_bounds().y1 {
        for x in layer.operative_bounds().x0..layer.operative_bounds().x1 {
            layer.checked_view().read_logical_pixel_native_into(x, y, &mut raw[..layer.format().bytes_per_pixel()], &ctl()).unwrap();
            let read = |i| try_read_sample_f64(&raw, doc.depth, i).unwrap();
            let rgb = if doc.mode == ColorMode::Grayscale { [read(0); 3] } else { [read(0), read(1), read(2)] };
            let expected = legacy.point_at([rgb[0], rgb[1], rgb[2], read(doc.mode.color_channels())], (x, y), &mut |_| Ok(())).unwrap();
            assert_eq!(output.pixels()[index].map(f64::to_bits), expected.map(f64::to_bits));
            index += 1;
        }
    }
}

#[test]
fn residual_precision_families_preserve_exact_upstream_schedule_and_distinct_epochs() {
    for family in 0..=4 {
        for sample in [SampleType::U16, SampleType::F32, SampleType::F64] {
            for stored in [SampleEncoding::Linear, SampleEncoding::NonlinearProfile] {
                if family == 3 && sample == SampleType::F64 {
                    continue;
                }
                if family == 4
                    && !matches!(
                        (sample, stored),
                        (SampleType::U16, SampleEncoding::NonlinearProfile)
                            | (SampleType::F32, SampleEncoding::Linear)
                            | (SampleType::F64, SampleEncoding::NonlinearProfile)
                    )
                {
                    continue;
                }
                let mode = if family == 2 { ColorMode::Grayscale } else { ColorMode::Rgb };
                let (mut doc, q, guard, _, profile, _) = fixture(0, mode, sample, stored);
                configure(&mut doc, family, if family == 3 || family == 2 { 2 } else { 0 });
                let owner = adopt(doc, q, guard);
                compare(&owner, profile);
            }
        }
    }
}

#[test]
fn posterize_signed_levels_use_opaque_native_u8_and_disabled_schema_is_still_checked() {
    for enabled in [false, true] {
        for levels in [2, 7, 256] {
            let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
            super::general_tests::reframe_coverage(&mut doc, &q, photocraft_doc::Size::new(32, 32), Rect::new(0, 0, 32, 32), true);
            configure(&mut doc, 5, levels);
            doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0].enabled = enabled;
            let owner = adopt(doc, q, guard);
            compare(&owner, profile);
        }
    }
    let (mut doc, q, guard, _, profile, _) = fixture(0, ColorMode::Rgb, SampleType::U8, SampleEncoding::NonlinearProfile);
    super::general_tests::reframe_coverage(&mut doc, &q, photocraft_doc::Size::new(32, 32), Rect::new(0, 0, 32, 32), false);
    configure(&mut doc, 5, 7);
    let owner = adopt(doc, q, guard);
    let current = PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).unwrap();
    assert!(current.validate_operative(&ctl()).is_err());
}

#[test]
fn residual_domains_refuse_wrong_epoch_raw_precision_and_cancel_before_native_reads() {
    for wrong in 0..4 {
        let (mut doc, q, guard, reads, profile, _) = fixture(0, ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
        configure(&mut doc, if wrong == 0 { 3 } else { 0 }, 0);
        let node = &mut doc.xcf_state.as_mut().unwrap().filter_stacks[0].nodes[0];
        if wrong == 1 {
            node.operation.schema_epoch = gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into();
        }
        if wrong == 2 {
            node.composition.as_mut().unwrap().stored_operator = 0;
            node.enabled = false;
        }
        let owner = adopt(doc, q, guard);
        let before = reads.load(std::sync::atomic::Ordering::SeqCst);
        if wrong == 3 {
            let cancel = || true;
            let interrupt = Interrupt::cancel_only(&cancel);
            let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
            assert!(PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &control).is_err());
        } else {
            assert!(PreparedBackedStockPointContext::new(&owner, profile, NormalBandLimits::default(), &ctl()).is_err());
        }
        assert_eq!(before, reads.load(std::sync::atomic::Ordering::SeqCst));
    }
}
