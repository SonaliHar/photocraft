use super::*;

fn kinds() -> [PointOperation; 4] {
    [
        PointOperation::ColorTemperature { original: 6500.0, intended: 5500.0 },
        PointOperation::Sepia { scale: 0.625, srgb: false },
        PointOperation::Sepia { scale: 0.625, srgb: true },
        PointOperation::AlienMap { frequency: [1.0, 2.5, 0.0], phase: [0.0, 91.0, 360.0], keep: [false, true, false] },
    ]
}

#[test]
#[ignore = "TEMP candidate contract; UNRUN and uncompiled"]
fn strict_unit_schedule_and_wide_alpha_match_existing() {
    for op in kinds() {
        let prepared = op.prepare().unwrap();
        for alpha in [-0.0, 0.0, 0.375 + f64::EPSILON, f64::from_bits(1.0f64.to_bits() - 1), 1.0] {
            for v in [0.0, 0.125, 0.5 + 2.0f64.powi(-45), 1.0] {
                let input = [v, 0.25, 0.875, alpha];
                let mut old = [0.0; 4];
                let mut derived = [0.0; 4];
                prepared.try_eval_pixel(&input, 3, &mut old).unwrap();
                prepared.try_eval_derived_rgb_pixel(&input, &mut derived).unwrap();
                assert_eq!(old.map(f64::to_bits), derived.map(f64::to_bits));
                assert_eq!(derived[3].to_bits(), alpha.to_bits());
            }
        }
    }
}

#[test]
#[ignore = "TEMP candidate contract; UNRUN and uncompiled"]
fn derived_hdr_exact_float_expressions_and_kept_wide_words() {
    let input = [-0.25, 1.25 + 2.0f64.powi(-44), 2.0, -0.0];
    for op in kinds() {
        let prepared = op.prepare().unwrap();
        let mut out = [0.0; 4];
        assert!(prepared.try_eval_pixel(&input, 3, &mut out).is_err());
        prepared.try_eval_derived_rgb_pixel(&input, &mut out).unwrap();
        let mut expected = input;
        match op {
            PointOperation::ColorTemperature { .. } => {
                // Cached gains are the existing constructor result, not a new table.
                for (v, gain) in expected[..3].iter_mut().zip(prepared.temperature_gains.unwrap()) {
                    *v = f64::from((*v as f32) * (gain as f32));
                }
            }
            PointOperation::Sepia { scale, .. } => {
                let matrix = [[0x3ec9374c, 0x3f44dd2f, 0x3e418937], [0x3eb2b021, 0x3f2f9db2, 0x3e2c0831], [0x3e8b4396, 0x3f08b439, 0x3e0624dd]];
                let rgb = [input[0] as f32, input[1] as f32, input[2] as f32];
                for (i, row) in matrix.into_iter().enumerate() {
                    let transformed = rgb.into_iter().zip(row).map(|(v, bits)| v * f32::from_bits(bits)).sum::<f32>();
                    expected[i] = f64::from(rgb[i] * (1.0 - scale as f32) + transformed * scale as f32);
                }
            }
            PointOperation::AlienMap { frequency, phase, keep } => {
                for i in 0..3 {
                    if !keep[i] {
                        let angle = (2.0 * f64::from(input[i] as f32) - 1.0) * std::f64::consts::PI * frequency[i] + phase[i].to_radians();
                        expected[i] = f64::from(((1.0 + angle.sin()) * 0.5) as f32);
                    }
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(out.map(f64::to_bits), expected.map(f64::to_bits));
    }
    let all_keep = PointOperation::AlienMap { frequency: [20.0; 3], phase: [360.0; 3], keep: [true; 3] }.prepare().unwrap();
    let wide = [f64::MAX, -f64::MAX, -0.0, -0.0];
    let mut out = [0.0; 4];
    all_keep.try_eval_derived_rgb_pixel(&wide, &mut out).unwrap();
    assert_eq!(out.map(f64::to_bits), wide.map(f64::to_bits));
}

#[test]
#[ignore = "TEMP candidate contract; UNRUN and uncompiled"]
fn invalid_shape_nonfinite_overflow_and_hsl_are_atomic_refusals() {
    let sentinel = [7.0, 8.0, 9.0, -0.0];
    for op in kinds() {
        let prepared = op.prepare().unwrap();
        for input in [[f64::NAN, 0.0, 0.0, 1.0], [0.0, f64::INFINITY, 0.0, 1.0], [0.0, 0.0, 0.0, 1.001]] {
            let mut out = sentinel;
            assert!(prepared.try_eval_derived_rgb_pixel(&input, &mut out).is_err());
            assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
        }
        let mut out = sentinel;
        assert!(prepared.try_eval_derived_rgb_pixel(&[0.0; 3], &mut out).is_err());
        assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
    }
    for op in [
        PointOperation::ColorTemperature { original: 6500.0, intended: 5500.0 },
        PointOperation::Sepia { scale: 0.0, srgb: true },
        PointOperation::AlienMap { frequency: [0.0; 3], phase: [0.0; 3], keep: [false; 3] },
    ] {
        let mut out = sentinel;
        assert!(op.prepare().unwrap().try_eval_derived_rgb_pixel(&[f64::MAX, 0.0, 0.0, 1.0], &mut out).is_err());
        assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
    }
    let overflowing = PointOperation::Sepia { scale: 1.0, srgb: false }.prepare().unwrap();
    let mut out = sentinel;
    assert!(overflowing.try_eval_derived_rgb_pixel(&[f64::from(f32::MAX), f64::from(f32::MAX), f64::from(f32::MAX), 1.0], &mut out).is_err());
    assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
    let hsl = PointOperation::AlienMapHsl { frequency: [1.0; 3], phase: [0.0; 3], keep: [true; 3] }.prepare().unwrap();
    assert!(hsl.try_eval_derived_rgb_pixel(&[-1.0, 0.0, 1.0, 1.0], &mut out).is_err());
    assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
}
