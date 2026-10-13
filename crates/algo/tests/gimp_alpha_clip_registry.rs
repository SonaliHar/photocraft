use photocraft_algo::gimp::{
    point::PointOperation,
    registry::{WorkingEncoding, alpha_clip_member, point_member},
};
#[test]
fn typed_member_rejects_boolean_numeric_aliases_and_retains_float32_bits() {
    for encoding in [WorkingEncoding::Linear, WorkingEncoding::Nonlinear] {
        let (member, op) = alpha_clip_member(Some("0:0"), true, 0.25, false, 0.75, encoding).unwrap();
        assert_eq!(member.operation, "gegl:alpha-clip");
        assert_eq!(member.encoding, encoding);
        assert_eq!(op, PointOperation::AlphaClipPrecision { clip_low: true, low_limit: 0.25, clip_high: false, high_limit: 0.75 });
        let mut out = [0.; 4];
        op.try_eval_pixel(&[0.125, 0.375, 0.625, 0.125], 3, &mut out).unwrap();
        assert_eq!(out, [0.125, 0.375, 0.625, 0.25]);
    }
    assert!(point_member("gegl:alpha-clip", Some("0:0"), &[("clip-low", 1.), ("low-limit", 0.25), ("clip-high", 1.), ("high-limit", 0.75)]).is_err());
    for version in [None, Some("0:1"), Some("1:0")] {
        assert!(alpha_clip_member(version, true, 0.25, true, 0.75, WorkingEncoding::Linear).is_err());
    }
    for (low, high) in [(f32::NAN, 0.75), (0.25, f32::INFINITY), (-0.01, 0.75), (0.25, 1.01), (0.75, 0.25)] {
        assert!(alpha_clip_member(Some("0:0"), false, low, false, high, WorkingEncoding::Linear).is_err());
    }
}
