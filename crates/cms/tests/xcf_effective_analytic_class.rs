//! Current assigned ICC interpretation is independent of immutable source provenance.
use photocraft_cms::{
    Builtin, Profile,
    curve::{Curve, srgb_trc},
    xcf_color::{Encoding, effective_profile_from_definition, qualify_analytic_profile, qualify_effective_analytic_profile},
};
#[test]
fn active_current_transfer_mismatch_is_rejected_without_source_reinterpretation() {
    for builtin in [Builtin::Srgb, Builtin::SGray] {
        let mut source = builtin.profile().clone();
        if source.color_space == photocraft_cms::ColorSpace::Rgb {
            source.trc = Some([Curve::Gamma(1.25), Curve::Gamma(2.0), Curve::Gamma(3.5)]);
        } else {
            source.gray_trc = Some(Curve::Gamma(2.0));
        }
        qualify_analytic_profile(&source).unwrap();
        assert!(qualify_effective_analytic_profile(&source, Encoding::Linear).is_err());
        assert!(qualify_effective_analytic_profile(&source, Encoding::Perceptual).is_err());
        qualify_effective_analytic_profile(&source, Encoding::NonlinearProfile).unwrap();
        for encoding in [Encoding::Linear, Encoding::Perceptual] {
            let effective = effective_profile_from_definition(&source, encoding).unwrap();
            let before = effective.clone();
            qualify_effective_analytic_profile(&effective, encoding).unwrap();
            let encoded = effective.to_bytes();
            let parsed = Profile::parse(&encoded).unwrap();
            qualify_effective_analytic_profile(&parsed, encoding).unwrap();
            assert_eq!(effective, before);
            assert_eq!(parsed.to_bytes().as_slice(), encoded.as_slice());
        }
    }
}
#[test]
fn perceptual_accepts_only_canonical_or_exact_icc_coefficient_rounding() {
    let mut source = Builtin::Srgb.profile().clone();
    source.trc = Some(std::array::from_fn(|_| srgb_trc()));
    qualify_effective_analytic_profile(&source, Encoding::Perceptual).unwrap();
    let Curve::Parametric { kind, p } = srgb_trc() else {
        panic!("literal curve");
    };
    let rounded = p.map(|v| (v * 65536.0).round() / 65536.0);
    source.trc = Some(std::array::from_fn(|_| Curve::Parametric { kind, p: rounded }));
    qualify_effective_analytic_profile(&source, Encoding::Perceptual).unwrap();
    let mut wrong = rounded;
    wrong[0] += 1.0 / 65536.0;
    source.trc = Some(std::array::from_fn(|_| Curve::Parametric { kind, p: wrong }));
    qualify_analytic_profile(&source).unwrap();
    assert!(qualify_effective_analytic_profile(&source, Encoding::Perceptual).is_err());
    source.trc = Some([Curve::Identity, Curve::Gamma(1.0), Curve::Parametric { kind: 0, p: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }]);
    qualify_effective_analytic_profile(&source, Encoding::Linear).unwrap();
    let mut gray = Builtin::SGray.profile().clone();
    gray.gray_trc = Some(Curve::Identity);
    gray.trc = Some([Curve::Gamma(f64::NAN), Curve::Identity, Curve::Identity]);
    qualify_effective_analytic_profile(&gray, Encoding::Linear).unwrap();
    gray.gray_trc = Some(Curve::Gamma(1.00001));
    assert!(qualify_effective_analytic_profile(&gray, Encoding::Linear).is_err());
}
