use super::{saturation_linear_float, saturation_linear_float_derived};
use crate::gimp::point_semi_flatten::SemiFlatten;

#[test]
#[ignore = "implementation-first: derived Float stage checks remain UNRUN"]
fn derived_semi_accepts_signed_hdr_without_widening_original_input_policy() {
    let operation = SemiFlatten { color: [0.25, 0.5, 0.75, 0.375] };
    let input = [-1.0, 2.0, 0.25, 0.5];
    let mut output = [91.0; 4];
    assert!(operation.try_eval_pixel(&input, 3, &mut output).is_err());
    assert_eq!(output, [91.0; 4]);
    operation.try_eval_derived_pixel(&input, 3, &mut output).unwrap();
    // Independent dyadic blend: half foreground plus half background.
    assert_eq!(output, [-0.375, 1.25, 0.5, 1.0]);
    for alpha in [0.0_f64, -0.0, 1.0] {
        let input = [-1.0, 2.0, 0.25, alpha];
        operation.try_eval_derived_pixel(&input, 3, &mut output).unwrap();
        assert_eq!(output.map(f64::to_bits), input.map(f64::to_bits));
    }
    for input in [[f64::NAN, 0.0, 0.0, 0.5], [f64::INFINITY, 0.0, 0.0, 0.5], [f64::MAX, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, -0.1], [0.0, 0.0, 0.0, 1.1]] {
        output = [91.0; 4];
        assert!(operation.try_eval_derived_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [91.0; 4]);
    }
    output = [91.0; 4];
    assert!(operation.try_eval_derived_pixel(&input, 1, &mut output).is_err());
    assert_eq!(output, [91.0; 4]);
    let invalid = SemiFlatten { color: [f32::NAN, 0.0, 0.0, 1.0] };
    assert!(invalid.try_eval_derived_pixel(&input, 3, &mut output).is_err());
    assert_eq!(output, [91.0; 4]);
}

#[test]
#[ignore = "implementation-first: derived Float stage checks remain UNRUN"]
fn derived_vibrance_saturation_retains_hdr_identity_alpha_and_finite_failure() {
    for alpha in [0.0_f32, -0.0, 0.375, 1.0] {
        let input = [-0.25, 1.5, 2.0, alpha];
        assert!(saturation_linear_float(input, 1.0).is_err());
        let identity = saturation_linear_float_derived(input, 1.0).unwrap();
        assert_eq!(identity.map(f32::to_bits), input.map(f32::to_bits));
        let gray = saturation_linear_float_derived(input, 0.0).unwrap();
        assert_eq!(gray[0].to_bits(), gray[1].to_bits());
        assert_eq!(gray[1].to_bits(), gray[2].to_bits());
        assert_eq!(gray[3].to_bits(), alpha.to_bits());
        assert!(gray.iter().all(|v| v.is_finite()));
    }
    for scale in [0.0, 0.125, 1.0, 1.25, 10.0] {
        let input = [0.125, 0.5, 0.875, -0.0];
        assert_eq!(saturation_linear_float(input, scale).unwrap().map(f32::to_bits), saturation_linear_float_derived(input, scale).unwrap().map(f32::to_bits));
    }
    for input in [[f32::NAN, 0.0, 0.0, 0.5], [0.0, f32::INFINITY, 0.0, 0.5], [0.0, 0.0, 0.0, f32::NAN], [0.0, 0.0, 0.0, -0.1], [0.0, 0.0, 0.0, 1.1]] {
        assert!(saturation_linear_float_derived(input, 1.0).is_err());
    }
    for scale in [-0.1, 10.1, f64::INFINITY, f64::NAN] {
        assert!(saturation_linear_float_derived([0.0; 4], scale).is_err());
    }
    assert!(saturation_linear_float_derived([f32::MAX, -f32::MAX, f32::MAX, 0.5], 10.0).is_err());
}
