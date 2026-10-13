use photocraft_algo::gimp::registry::{WorkingEncoding, alpha_clip_finite_member};
#[test]
fn finite_typed_member_predicts_independent_crossed_alpha() {
    let (_, operation) = alpha_clip_finite_member(Some("0:0"), true, 0.875, true, 0.125, WorkingEncoding::Linear).unwrap();
    for (alpha, expected) in [(0.0, 0.875), (0.125, 0.875), (0.25, 0.125), (1.0, 0.125)] {
        let mut output = [0.0; 4];
        operation.try_eval_pixel(&[0.125, 0.375, 0.625, alpha], 3, &mut output).unwrap();
        assert_eq!(output, [0.125, 0.375, 0.625, expected]);
    }
}

#[test]
fn finite_typed_flags_types_ranges_and_native_identity_remain_distinct() {
    use photocraft_algo::gimp::{
        point::PointOperation,
        registry::{alpha_clip_member, point_member},
    };
    let input = [0.125 + 2f64.powi(-40), -0.0, 1.5, 0.625];
    for encoding in [WorkingEncoding::Linear, WorkingEncoding::Nonlinear] {
        let (member, operation) = alpha_clip_finite_member(Some("0:0"), false, -0.125, false, 1.125, encoding).unwrap();
        assert_eq!(member.encoding, encoding);
        assert_eq!(operation, PointOperation::AlphaClipFinitePrecision { clip_low: false, low_limit: -0.125, clip_high: false, high_limit: 1.125 });
        let mut output = [0.; 4];
        operation.try_eval_pixel(&input, 3, &mut output).unwrap();
        assert_eq!(output.map(f64::to_bits), input.map(f64::to_bits));
    }
    for version in [None, Some("0:1")] {
        assert!(alpha_clip_finite_member(version, true, 0.875, true, 0.125, WorkingEncoding::Linear).is_err());
    }
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(alpha_clip_finite_member(Some("0:0"), false, v, false, 1.125, WorkingEncoding::Linear).is_err());
        assert!(alpha_clip_finite_member(Some("0:0"), false, -0.125, false, v, WorkingEncoding::Linear).is_err());
    }
    assert!(alpha_clip_member(Some("0:0"), true, 0.875, true, 0.125, WorkingEncoding::Linear).is_err());
    assert!(point_member("gegl:alpha-clip", Some("0:0"), &[("clip-low", 1.), ("low-limit", 0.875), ("clip-high", 1.), ("high-limit", 0.125)]).is_err());
}
