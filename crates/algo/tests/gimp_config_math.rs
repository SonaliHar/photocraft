use photocraft_algo::gimp::{
    curves::{FreeCurve, FreeCurvesConfiguration, OwnedPreparedCurveTables},
    levels::{LevelsChannel, LevelsConfiguration},
};

#[test]
fn levels_order_negative_gamma_alpha_and_atomic_rejection() {
    let mut channels = [LevelsChannel::default(); 5];
    channels[0].low_output = 0.1;
    channels[1].high_output = 0.5;
    channels[4].gamma = 2.0;
    let config = LevelsConfiguration { channels, clamp_input: false, clamp_output: false };
    let mut out = [99.0; 4];
    config.try_eval_pixel(&[0.4, -0.25, 2.0, 0.25], 3, &mut out).unwrap();
    assert!((out[0] - 0.28).abs() < 1e-14);
    assert!((out[1] + 0.125).abs() < 1e-14);
    assert!((out[2] - 1.9).abs() < 1e-14);
    assert_eq!(out[3], 0.5);
    config.try_eval_pixel(&[0.0, 0.0, 0.0, -0.25], 3, &mut out).unwrap();
    assert_eq!(out[3], -0.25);
    let previous = out;
    assert!(config.try_eval_pixel(&[0.0, f64::INFINITY, 0.0, 0.5], 3, &mut out).is_err());
    assert_eq!(out, previous);
}

#[test]
fn identity_levels_preserves_untouched_nan_and_signed_zero_bits() {
    let config = LevelsConfiguration { channels: [LevelsChannel::default(); 5], clamp_input: false, clamp_output: false };
    let input = [f64::from_bits(0x7ff8000000000042), -0.0, 3.5, -0.0];
    let mut output = [0.0; 4];
    config.try_eval_pixel(&input, 3, &mut output).unwrap();
    assert_eq!(input.map(f64::to_bits), output.map(f64::to_bits));
}

#[test]
fn free_curve_interpolation_endpoints_and_atomic_validation() {
    let values: Vec<f64> = (0..256).map(|i| (f64::from(i) / 255.0).powi(2)).collect();
    let curve = FreeCurve { samples: &values };
    let config = FreeCurvesConfiguration { channels: [curve; 5] };
    let mut out = [0.0; 4];
    config.try_eval_pixel(&[-1.0, 0.5, 2.0, 0.5], 3, &mut out).unwrap();
    let mut prepared_out = [0.0; 4];
    config.prepare().unwrap().try_eval_pixel(&[-1.0, 0.5, 2.0, 0.5], 3, &mut prepared_out).unwrap();
    assert_eq!(out.map(f64::to_bits), prepared_out.map(f64::to_bits));
    let owned = OwnedPreparedCurveTables::try_new(std::array::from_fn(|_| values.clone())).unwrap();
    let mut owned_out = [0.0; 4];
    owned.try_eval_pixel(&[-1.0, 0.5, 2.0, 0.5], 3, &mut owned_out).unwrap();
    assert_eq!(owned_out.map(f64::to_bits), out.map(f64::to_bits));
    assert_eq!(owned.payload_bytes(), 10240);
    assert_eq!(out[0], 0.0);
    assert_eq!(out[2], 1.0);
    assert!((out[3] - 0.25000384467512493).abs() < 1e-14);
    let before = out;
    assert!(config.try_eval_pixel(&[0.0, f64::NAN, 0.0, 0.5], 3, &mut out).is_err());
    assert_eq!(before, out);
    let invalid = FreeCurve { samples: &values[..255] };
    assert!(FreeCurvesConfiguration { channels: [invalid; 5] }.try_eval_pixel(&[0.0; 4], 3, &mut out).is_err());
    assert_eq!(before, out);
}
