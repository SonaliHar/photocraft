//! Bounded analytic profile admission; these constraints do not change Curve math.
use photocraft_cms::{Builtin, curve::Curve, xcf_color::qualify_analytic_profile};

#[test]
fn generic_analytic_class_accepts_distinct_definitions_without_hash_identity() {
    for gamma in [0.75, 1.25, 1.9, 2.6, 3.5] {
        let mut profile = Builtin::Srgb.profile().clone();
        profile.trc = Some([Curve::Gamma(gamma), Curve::Gamma(2.1), Curve::Identity]);
        let encoded = profile.with_encoded_bytes().to_bytes();
        let parsed = photocraft_cms::Profile::parse(&encoded).unwrap();
        qualify_analytic_profile(&parsed).unwrap();
        assert_eq!(parsed.to_bytes().as_slice(), encoded.as_slice());
    }
    qualify_analytic_profile(Builtin::SGray.profile()).unwrap();
}

#[test]
fn generic_analytic_class_rejects_unsampled_branch_defects_and_unstable_primaries() {
    let base = Builtin::Srgb.profile().clone();
    let curve = |c, d, e, f| Curve::Parametric { kind: 4, p: [2.0, 1.0, 0.0, c, d, e, f] };
    for invalid in [
        curve(0.0, 0.001, 0.0, 0.0),
        curve(0.1, 0.001, 0.0, 0.0),
        curve(0.01, 0.001, 0.0, 0.02),
        Curve::Gamma(f64::NAN),
        Curve::Gamma(0.00001),
        Curve::Table64(vec![0.0, 1.0]),
    ] {
        let mut profile = base.clone();
        profile.trc = Some([invalid, Curve::Identity, Curve::Identity]);
        assert!(qualify_analytic_profile(&profile).is_err());
    }
    for matrix in [[[0.3, 0.3, 0.3642], [0.3, 0.3, 0.4], [0.3, 0.3, 0.2249]], [[-0.1, 0.5, 0.5642], [0.2, 0.6, 0.2], [0.1, 0.1, 0.6249]]] {
        let mut profile = base.clone();
        profile.matrix = Some(matrix);
        assert!(qualify_analytic_profile(&profile).is_err());
    }
    let mut white = base;
    white.white_point = [1.0, 1.0, 1.0];
    assert!(qualify_analytic_profile(&white).is_err());
}

#[test]
fn analytic_class_checks_real_branch_boundaries_and_retains_definition() {
    for kind in [3, 4] {
        for gamma in [0.75_f64, 1.25, 2.15, 3.5] {
            for d in [1.0_f64 / 65536.0, 0.04, 0.25] {
                let g = (gamma * 65536.0).round() / 65536.0;
                let d = (d * 65536.0).round() / 65536.0;
                let c = (d.powf(g - 1.0) * 65536.0).floor() / 65536.0;
                if c == 0.0 || c > 4.0 {
                    continue;
                }
                let curve = Curve::Parametric { kind, p: [g, 1.0, 0.0, c, d, 0.0, 0.0] };
                let mut gray = Builtin::SGray.profile().clone();
                gray.gray_trc = Some(curve.clone());
                let before = gray.clone();
                qualify_analytic_profile(&gray).unwrap();
                assert_eq!(gray, before);
                for x in [0.0, d * 0.5, d, d + 1.0 / 65536.0, 0.5, 1.0] {
                    let y = curve.eval64(x);
                    assert!(y.is_finite());
                    assert!((curve.eval_inverse64(y) - x).abs() < 1e-10);
                }
            }
        }
    }
    let mut rgb = Builtin::Srgb.profile().clone();
    rgb.pcs = photocraft_cms::profile::Pcs::Lab;
    assert!(qualify_analytic_profile(&rgb).is_err());
    assert!(qualify_analytic_profile(Builtin::CoatedCmyk.profile()).is_err());
    let mut gray = Builtin::SGray.profile().clone();
    gray.trc = Some([Curve::Gamma(f64::NAN), Curve::Identity, Curve::Identity]);
    qualify_analytic_profile(&gray).unwrap(); // unrelated RGB tags never replace kTRC
    gray.gray_trc = Some(Curve::Gamma(f64::NAN));
    assert!(qualify_analytic_profile(&gray).is_err());
    let mut bytes = Builtin::Srgb.profile().to_bytes().as_slice().to_vec();
    bytes.get_mut(..4).unwrap().copy_from_slice(&u32::MAX.to_be_bytes());
    // The general ICC parser deliberately tolerates declared/truncated sizes;
    // the new admission class preserves that parser behavior but rejects this.
    let malformed = photocraft_cms::Profile::parse(&bytes).unwrap();
    assert!(qualify_analytic_profile(&malformed).is_err());
    let mut bytes = Builtin::Srgb.profile().to_bytes().as_slice().to_vec();
    bytes.get_mut(20..24).unwrap().copy_from_slice(b"BAD ");
    let fallback = photocraft_cms::Profile::parse(&bytes).unwrap();
    assert!(qualify_analytic_profile(&fallback).is_err());
}
