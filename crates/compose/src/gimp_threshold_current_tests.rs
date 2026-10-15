//! UNRUN exact current typed recipe tests; no source/mask/profile authority.
use super::*;
use photocraft_doc::{
    import_compositing::{GimpComposition, GimpOperator, GimpSpace, ResolvedGimpComposition},
    item_identity::{FilterId, ItemRef},
    xcf_filters::{FilterArgument, OperationIdentity},
};
fn node() -> FilterNode {
    let epoch = crate::gimp_filters::THRESHOLD_PRECISION_EPOCH;
    FilterNode {
        id: FilterId(17),
        operation: OperationIdentity { name: "gimp:threshold".into(), stored_version: "0:0".into(), schema_epoch: epoch.into() },
        arguments: vec![
            FilterArgument { name: "channel".into(), value: FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 0 } },
            FilterArgument { name: "low".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits())) },
            FilterArgument { name: "high".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.75f32.to_bits())) },
        ],
        source_display_name: Some("retained display".into()),
        source_icon: None,
        enabled: true,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: Some(0),
        source_region: Some(0),
        mask: None,
        auxiliary: Vec::new(),
        composition: Some(GimpComposition {
            compatibility_epoch: epoch.into(),
            source_mode: 62,
            legacy: false,
            stored_blend_space: -1565818287,
            stored_composite_space: -1565818287,
            stored_operator: -1565818287,
            resolved: None,
            pass_through: false,
        }),
    }
}
fn check(node: &FilterNode) -> Result<(Threshold, bool)> {
    validate(std::slice::from_ref(node), ColorMode::Rgb, SampleEncoding::NonlinearProfile, 22)
}
#[test]
fn every_channel_and_disabled_state_preserve_argument_order_signed_zero_and_original_node() {
    for channel in 0..=5 {
        for enabled in [false, true] {
            for rotate in 0..3 {
                let mut n = node();
                n.enabled = enabled;
                n.arguments[0].value = FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: channel };
                n.arguments.rotate_left(rotate);
                let before = n.clone();
                let (op, active) = check(&n).unwrap();
                assert_eq!(op.channel, channel as u8);
                assert_eq!(op.low.to_bits(), (-0f64).to_bits());
                assert_eq!(op.high, 0.75);
                assert_eq!(active, enabled);
                assert_eq!(n, before);
            }
        }
    }
    let mut reversed = node();
    reversed.arguments[1].value = FilterValue::Number(ExactNumber::Float32Bits(0.875f32.to_bits()));
    reversed.arguments[2].value = FilterValue::Number(ExactNumber::Float32Bits(0.125f32.to_bits()));
    let (op, _) = check(&reversed).unwrap();
    let mut output = [0.5; 4];
    op.try_eval_pixel(&[0.5; 4], 3, &mut output).unwrap();
    assert_eq!(output, [0., 0., 0., 0.5]);
}
#[test]
fn context_and_current_identity_outer_fields_refuse_individually() {
    let original = node();
    for mode in [ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        assert!(validate(std::slice::from_ref(&original), mode, SampleEncoding::NonlinearProfile, 22).is_err());
    }
    for encoding in [SampleEncoding::Linear, SampleEncoding::Perceptual] {
        assert!(validate(std::slice::from_ref(&original), ColorMode::Rgb, encoding, 22).is_err());
    }
    for version in [0, 21, 23] {
        assert!(validate(std::slice::from_ref(&original), ColorMode::Rgb, SampleEncoding::NonlinearProfile, version).is_err());
    }
    assert!(validate(&[], ColorMode::Rgb, SampleEncoding::NonlinearProfile, 22).is_err());
    assert!(validate(&[original.clone(), original.clone()], ColorMode::Rgb, SampleEncoding::NonlinearProfile, 22).is_err());
    for field in 0..17 {
        let mut n = original.clone();
        match field {
            0 => n.id = FilterId(0),
            1 => n.operation.name = "gimp:threshold-alpha".into(),
            2 => n.operation.stored_version = "0:1".into(),
            3 => n.operation.schema_epoch = "other".into(),
            4 => n.composition = None,
            5 => n.composition.as_mut().unwrap().compatibility_epoch = "other".into(),
            6 => n.composition.as_mut().unwrap().source_mode = 28,
            7 => n.composition.as_mut().unwrap().legacy = true,
            8 => n.composition.as_mut().unwrap().pass_through = true,
            9 => {
                n.composition.as_mut().unwrap().resolved =
                    Some(ResolvedGimpComposition { blend_space: GimpSpace::LinearRgb, composite_space: GimpSpace::LinearRgb, operator: GimpOperator::Union })
            }
            10 => n.composition.as_mut().unwrap().stored_blend_space = 0,
            11 => n.composition.as_mut().unwrap().stored_composite_space = 0,
            12 => n.composition.as_mut().unwrap().stored_operator = 0,
            13 => n.opacity_f64_bits = 0.5f64.to_bits(),
            14 => n.source_clipping = None,
            15 => n.source_region = Some(1),
            _ => n.auxiliary.push(("aux".into(), ItemRef::Layer(photocraft_doc::LayerId(33)))),
        }
        assert!(check(&n).is_err(), "field{field}");
        n.enabled = false;
        assert!(check(&n).is_err(), "disabled does not hide invalid field{field}");
    }
    assert_eq!(original, node());
}
#[test]
fn exact_typed_argument_count_names_duplicates_domains_widths_and_unit_values_are_closed() {
    for bad in 0..9 {
        let mut n = node();
        match bad {
            0 => {
                n.arguments.pop();
            }
            1 => n.arguments.push(n.arguments[0].clone()),
            2 => n.arguments[1] = n.arguments[0].clone(),
            3 => n.arguments[2] = n.arguments[1].clone(),
            4 => n.arguments[0].value = FilterValue::Enum { domain: "Other".into(), value: 0 },
            5 => n.arguments[0].value = FilterValue::Enum { domain: "GimpHistogramChannel".into(), value: 6 },
            6 => n.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(0)),
            7 => n.arguments[1].value = FilterValue::Number(ExactNumber::Float64Bits(0f64.to_bits())),
            _ => n.arguments[1].name = "unknown".into(),
        }
        assert!(check(&n).is_err(), "argument{bad}");
    }
    for position in [1, 2] {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.001, 1.001] {
            let mut n = node();
            n.arguments[position].value = FilterValue::Number(ExactNumber::Float32Bits(value.to_bits()));
            assert!(check(&n).is_err());
        }
    }
}
#[test]
fn successful_typed_node_returns_no_mask_profile_or_source_token() {
    let n = node();
    assert!(n.mask.is_none());
    assert!(check(&n).is_ok(), "caller separately validates required physical mask and genuine owner");
    // Only a typed numerical operation and activation bit escape. No source,
    // retained profile, storage class or namespace correspondence is fabricated.
    let (op, enabled) = check(&n).unwrap();
    assert!(enabled);
    let mut output = [0.; 4];
    op.try_eval_pixel(&[0.25, 0.5, 0.75, -0.0], 3, &mut output).unwrap();
    assert_eq!(output[3].to_bits(), (-0f64).to_bits());
}
