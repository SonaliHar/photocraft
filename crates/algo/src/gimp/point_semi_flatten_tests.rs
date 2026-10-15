//! UNRUN kernel compatibility-policy tests, not a GIMP numerical oracle.
use super::*;

#[test]
#[ignore = "UNRUN partial coverage becomes opaque and parameter alpha is retained but ignored"]
fn partial_alpha_and_color_alpha_are_distinct() {
    let source = [0.25, 0.5, 0.75, 0.5];
    for color_alpha in [0.0, -0.0, 0.375, 1.0] {
        let operation = SemiFlatten { color: [0.75, 0.25, 0.125, color_alpha] };
        let mut output = [99.0; 4];
        operation.try_eval_pixel(&source, 3, &mut output).unwrap();
        assert_eq!(output, [0.5, 0.375, 0.4375, 1.0]);
        assert_eq!(operation.color[3].to_bits(), color_alpha.to_bits());
    }
}

#[test]
#[ignore = "UNRUN Float alpha staging precedes branch; signed zero/full branches remain working samples"]
fn alpha_boundary_is_staged_before_branch() {
    let operation = SemiFlatten { color: [0.875, 0.25, 0.5, 1.0] };
    let low_bit_color = f64::from_bits(0x3fd0_0000_0000_0001);
    for alpha in [0.0f64, -0.0, 1.0, f64::from_bits(1.0f64.to_bits() - 1)] {
        let mut output = [99.0; 4];
        operation.try_eval_pixel(&[low_bit_color, 0.5, 0.75, alpha], 3, &mut output).unwrap();
        assert_eq!(output[0], f64::from(low_bit_color as f32));
        assert_eq!(output[3].to_bits(), f64::from(alpha as f32).to_bits());
    }
    // Largest Float below one remains partial, even though the nearest Double
    // below one above rounds to full coverage at the declared Float boundary.
    let alpha = f64::from(f32::from_bits(1.0f32.to_bits() - 1));
    let mut output = [99.0; 4];
    operation.try_eval_pixel(&[0.0, 0.0, 0.0, alpha], 3, &mut output).unwrap();
    assert!(output[0] > 0.0);
    assert_eq!(output[3], 1.0);
}

#[test]
#[ignore = "UNRUN invalid shape/domain never publishes partial output or changes inputs"]
fn invalid_input_and_color_are_atomic() {
    let valid = SemiFlatten { color: [0.75, 0.25, 0.125, 1.0] };
    for input in [
        [f64::NAN, 0.5, 0.5, 0.5],
        [0.5, f64::INFINITY, 0.5, 0.5],
        [-0.01, 0.5, 0.5, 0.5],
        [0.5, 1.01, 0.5, 0.5],
        [0.5, 0.5, 0.5, -0.01],
        [0.5, 0.5, 0.5, 1.01],
    ] {
        let mut output = [91.0, 92.0, 93.0, 94.0];
        assert!(valid.try_eval_pixel(&input, 3, &mut output).is_err());
        assert_eq!(output, [91.0, 92.0, 93.0, 94.0]);
    }
    for bad in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        for channel in 0..4 {
            let mut operation = valid;
            operation.color[channel] = bad;
            let mut output = [99.0; 4];
            assert!(operation.validate().is_err());
            assert!(operation.try_eval_pixel(&[0.5; 4], 3, &mut output).is_err());
            assert_eq!(output, [99.0; 4]);
        }
    }
    for channels in [0, 1, 2, 4, 255] {
        let mut output = [99.0; 4];
        assert!(valid.try_eval_pixel(&[0.5; 4], channels, &mut output).is_err());
        assert_eq!(output, [99.0; 4]);
    }
    let mut short_output = [99.0; 3];
    assert!(valid.try_eval_pixel(&[0.5; 4], 3, &mut short_output).is_err());
    assert_eq!(short_output, [99.0; 3]);
    let mut output = [99.0; 4];
    assert!(valid.try_eval_pixel(&[0.5; 3], 3, &mut output).is_err());
    assert_eq!(output, [99.0; 4]);
}
