use super::*;
fn operation(keep: [bool; 3], frequency: [f64; 3], phase: [f64; 3]) -> PreparedPointOperation {
    PointOperation::AlienMapHsl { frequency, phase, keep }.prepare().unwrap()
}
#[test]
#[ignore = "TEMP UNRUN/uncompiled: checked unit subset retains exact old Float HSL order and wide alpha"]
fn unit_lattice_and_keep_modes_match_old_map() {
    for bits in 0u8..8 {
        let keep = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        for (frequency, phase) in [([1.; 3], [0.; 3]), ([0., 2.5, 20.], [360., 91., 0.])] {
            let prepared = operation(keep, frequency, phase);
            for r in [0., 0.125, 0.5, 1.] {
                for g in [0., 0.25, 1.] {
                    for b in [0., 0.75, 1.] {
                        let rgb = [r, g, b];
                        let old = super::super::alien_hsl::map(rgb, frequency, phase, keep);
                        assert_eq!(try_map_derived(rgb, frequency, phase, keep).unwrap().map(f64::to_bits), old.map(f64::to_bits));
                        for alpha in [-0., 0., 0.375 + f64::EPSILON, f64::from_bits(1f64.to_bits() - 1), 1.] {
                            let mut out = [7.; 4];
                            prepared.try_eval_derived_hsl_pixel(&[r, g, b, alpha], &mut out).unwrap();
                            assert_eq!(out[..3], old);
                            assert_eq!(out[3].to_bits(), alpha.to_bits());
                        }
                    }
                }
            }
        }
    }
}
#[test]
#[ignore = "TEMP UNRUN/uncompiled: benign signed/HDR partial domain, achromatic literal branch, allkeep still Float HSL roundtrip"]
fn finite_signed_and_achromatic_cases_keep_existing_arithmetic() {
    for bits in 0u8..8 {
        let keep = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        for rgb in [[-0.25, 0.5, 1.25], [-2., -1., -0.5], [-1.; 3], [2.; 3], [-0.; 3]] {
            let expected = super::super::alien_hsl::map(rgb, [1.; 3], [0.; 3], keep);
            assert!(expected.iter().all(|v| v.is_finite()));
            let actual = try_map_derived(rgb, [1.; 3], [0.; 3], keep).unwrap();
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
        }
    }
    let all_keep = operation([true; 3], [1.; 3], [0.; 3]);
    let input = [-0.25, 0.5 + 2f64.powi(-45), 1.25, -0.];
    let mut out = [0.; 4];
    all_keep.try_eval_derived_hsl_pixel(&input, &mut out).unwrap();
    assert_ne!([out[0].to_bits(), out[1].to_bits(), out[2].to_bits()], [input[0].to_bits(), input[1].to_bits(), input[2].to_bits()]);
    assert_eq!(out[3].to_bits(), input[3].to_bits());
    // Existing strict unit API remains a separate refusing boundary.
    assert!(all_keep.try_eval_pixel(&input, 3, &mut [0.; 4]).is_err());
}
#[test]
#[ignore = "TEMP UNRUN/uncompiled: exact zero denominators/Float overflows/nonfinite/shape/wrongkind reject atomically without epsilon or fallback"]
fn singularities_and_overflows_are_atomic_refusals() {
    let sentinel = [7., 8., 9., -0.];
    let maximum = f64::from(f32::MAX);
    for bits in 0u8..8 {
        let keep = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        let p = operation(keep, [1.; 3], [0.; 3]);
        for rgb in [[-1., 0., 1.], [0., 1., 2.], [maximum; 3], [maximum, -maximum, 0.], [f64::MAX, 0., 0.], [f64::NAN, 0., 1.]] {
            let mut out = sentinel;
            assert!(p.try_eval_derived_hsl_pixel(&[rgb[0], rgb[1], rgb[2], 1.], &mut out).is_err());
            assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
        }
    }
    for (frequency, phase) in [([f64::NAN, 1., 1.], [0.; 3]), ([21.; 3], [0.; 3]), ([1.; 3], [361.; 3])] {
        assert!(try_map_derived([0.25, 0.5, 0.75], frequency, phase, [true; 3]).is_err());
    }
    let p = operation([true; 3], [1.; 3], [0.; 3]);
    for input in [[0., 0., 0., f64::NAN], [0., 0., 0., 1.001]] {
        let mut out = sentinel;
        assert!(p.try_eval_derived_hsl_pixel(&input, &mut out).is_err());
        assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
    }
    let mut out = sentinel;
    assert!(p.try_eval_derived_hsl_pixel(&[0.; 3], &mut out).is_err());
    assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
    assert!(
        PointOperation::AlienMap { frequency: [1.; 3], phase: [0.; 3], keep: [true; 3] }
            .prepare()
            .unwrap()
            .try_eval_derived_hsl_pixel(&[0.; 4], &mut out)
            .is_err()
    );
    assert_eq!(out.map(f64::to_bits), sentinel.map(f64::to_bits));
}
