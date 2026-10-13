//! Source interpretation is independent of display precision and preserves original ICC data.
use photocraft_cms::xcf_color::{Encoding, effective_profile};
use photocraft_cms::xcf_color::{
    NonFinitePolicy, SamplePrecision, SourceColorContext, convert_encoding_f64, parse_bounded_profile, validate_profile, validate_sample,
};
use photocraft_cms::{Builtin, ColorSpace, Curve, Profile};

fn unequal_profile() -> Profile {
    let mut p = Builtin::DisplayP3.profile().clone();
    p.trc = Some([Curve::Gamma(1.8), Curve::Table(vec![0.0, 0.1, 0.45, 1.0]), photocraft_cms::curve::srgb_trc()]);
    p.with_encoded_bytes()
}

#[test]
fn nonlinear_profile_accepts_valid_unequal_table_and_parametric_curves() {
    let bytes = unequal_profile().to_bytes();
    let p = effective_profile(Some(&bytes), ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
    assert_eq!(p.to_bytes().as_slice(), bytes.as_slice());
    assert_eq!(p.trc, Profile::parse(&bytes).unwrap().trc);
}

#[test]
fn table_curve_f64_does_not_narrow_input_to_f32() {
    let curve = Curve::Table(vec![0.0, 1.0]);
    let x = 0.5 + 2.0_f64.powi(-40);
    assert_eq!(curve.eval64(x).to_bits(), x.to_bits());
    assert_eq!(curve.eval_inverse64(x).to_bits(), x.to_bits());
}

#[test]
fn context_retains_assignment_absence_and_all_original_profile_bytes() {
    let source = unequal_profile().to_bytes();
    let working = Builtin::Srgb.profile().to_bytes();
    for assigned in [None, Some(source.as_slice())] {
        let context = SourceColorContext::new(
            assigned,
            &source,
            &working,
            ColorSpace::Rgb,
            Encoding::Linear,
            Encoding::Perceptual,
            SamplePrecision::F64,
            NonFinitePolicy::PreserveBits,
        )
        .unwrap();
        assert_eq!(context.assigned_bytes().as_deref().map(Vec::as_slice), assigned);
        assert_eq!(context.source_bytes().as_slice(), source.as_slice());
        assert_eq!(context.working_bytes().as_slice(), working.as_slice());
        assert_eq!(context.source_profile().matrix, unequal_profile().matrix);
        assert!(context.source_profile().trc.as_ref().unwrap().iter().all(Curve::is_identity));
        assert_ne!(context.source_profile().to_bytes().as_slice(), source.as_slice());
        assert_ne!(context.working_profile().to_bytes().as_slice(), working.as_slice());
    }
}

#[test]
fn transfer_is_channel_specific_and_retains_f64_detail() {
    let p = unequal_profile();
    let input = [0.5 + 2.0_f64.powi(-40), 0.6, 0.8];
    let mut out = [0.0; 3];
    convert_encoding_f64(&p, Encoding::NonlinearProfile, Encoding::Linear, &input, &mut out).unwrap();
    let curves = p.trc.as_ref().unwrap();
    for i in 0..3 {
        assert_eq!(out[i], curves[i].eval64(input[i]));
    }
    let mut back = [0.0; 3];
    convert_encoding_f64(&p, Encoding::Linear, Encoding::NonlinearProfile, &out, &mut back).unwrap();
    for i in 0..3 {
        assert!((back[i] - input[i]).abs() < 1e-12);
    }
    assert_ne!(out[0], curves[0].eval64(f64::from(input[0] as f32)));
}

#[test]
fn gray_table_and_all_icc_parametric_forms_are_supported() {
    let mut gray = Builtin::SGray.profile().clone();
    gray.gray_trc = Some(Curve::Table(vec![0.0, 0.05, 0.25, 0.6, 1.0]));
    let bytes = gray.with_encoded_bytes().to_bytes();
    let gray = effective_profile(Some(&bytes), ColorSpace::Gray, Encoding::NonlinearProfile).unwrap();
    let mut out = [0.0];
    convert_encoding_f64(&gray, Encoding::NonlinearProfile, Encoding::Linear, &[0.625], &mut out).unwrap();
    assert!((out[0] - 0.425).abs() < 2e-5);
    for (kind, p) in [
        (0, [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        (1, [2.0, 0.9, 0.1, 0.0, 0.0, 0.0, 0.0]),
        (2, [2.0, 0.9, 0.1, 0.05, 0.0, 0.0, 0.0]),
        (3, [2.0, 1.0, 0.0, 0.1, 0.1, 0.0, 0.0]),
        (4, [2.0, 1.0, 0.0, 0.1, 0.1, 0.02, 0.0]),
    ] {
        let mut profile = Builtin::Srgb.profile().clone();
        profile.trc = Some(std::array::from_fn(|_| Curve::Parametric { kind, p }));
        let bytes = profile.with_encoded_bytes().to_bytes();
        assert!(effective_profile(Some(&bytes), ColorSpace::Rgb, Encoding::NonlinearProfile).is_ok(), "kind {kind}");
    }
}

#[test]
fn hdr_samples_are_not_clamped_and_nonfinite_storage_is_explicit() {
    for precision in [SamplePrecision::F16, SamplePrecision::F32, SamplePrecision::F64] {
        for value in [-4.0, -0.0, 8.0] {
            validate_sample(value, precision, NonFinitePolicy::Reject).unwrap();
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(validate_sample(value, precision, NonFinitePolicy::Reject).is_err());
            validate_sample(value, precision, NonFinitePolicy::PreserveBits).unwrap();
        }
    }
    for precision in [SamplePrecision::U8, SamplePrecision::U16, SamplePrecision::U32] {
        for value in [-0.1, 1.1, f64::NAN] {
            assert!(validate_sample(value, precision, NonFinitePolicy::PreserveBits).is_err());
        }
    }
    let mut p = Builtin::Srgb.profile().clone();
    p.trc = Some([Curve::Gamma(2.0), Curve::Gamma(2.0), Curve::Gamma(2.0)]);
    let mut out = [0.0; 3];
    convert_encoding_f64(&p, Encoding::NonlinearProfile, Encoding::Linear, &[-2.0, 4.0, -0.0], &mut out).unwrap();
    assert_eq!(out[0], -4.0);
    assert_eq!(out[1], 16.0);
    assert_eq!(out[2].to_bits(), (-0.0_f64).to_bits());
    let original = [17.0; 3];
    out = original;
    assert!(convert_encoding_f64(&p, Encoding::Linear, Encoding::NonlinearProfile, &[0.0, f64::NAN, 1.0], &mut out).is_err());
    assert_eq!(out, original);
}

#[test]
fn wide_transform_uses_f64_and_commits_only_successful_output() {
    use photocraft_cms::pipeline::{Pipeline, Stage};
    use photocraft_cms::{Transform, TransformOptions};
    let transform = Transform::from_pipeline(
        Pipeline::new(3, vec![Stage::Curves(vec![Curve::Identity; 3]), Stage::matrix3(&photocraft_cms::math::IDENTITY)]),
        TransformOptions::default(),
    );
    let input = [0.5 + 2.0_f64.powi(-40), -3.0, 9.0];
    let mut out = [0.0; 3];
    transform.try_eval_f64(&input, &mut out).unwrap();
    assert_eq!(out, input);
    let original = out;
    assert!(transform.try_eval_f64(&[f64::NAN, 1.0, 1.0], &mut out).is_err());
    assert_eq!(out, original);
    assert!(transform.try_eval_f64(&[0.0], &mut out).is_err());
    let invalid = Transform::from_pipeline(
        Pipeline::new(3, vec![Stage::Matrix { rows: 3, cols: 3, m: vec![0.0; 8], offset: vec![0.0; 3] }]),
        TransformOptions::default(),
    );
    assert!(invalid.try_eval_f64(&input, &mut out).is_err());
    assert_eq!(out, original);
}

#[test]
fn wide_context_conversion_agrees_with_independent_matrix_calculation() {
    let source = unequal_profile().to_bytes();
    let working = Builtin::LinearSrgb.profile().to_bytes();
    let context = SourceColorContext::new(
        Some(&source),
        &source,
        &working,
        ColorSpace::Rgb,
        Encoding::NonlinearProfile,
        Encoding::Linear,
        SamplePrecision::F64,
        NonFinitePolicy::Reject,
    )
    .unwrap();
    let input = [0.55 + 2.0_f64.powi(-40), 0.7, 1.5];
    let source_profile = Profile::parse(&source).unwrap();
    let working_profile = Profile::parse(&working).unwrap();
    let curves = source_profile.trc.as_ref().unwrap();
    let linear = std::array::from_fn(|i| curves[i].eval64(input[i]));
    let xyz = photocraft_cms::math::apply(source_profile.matrix.as_ref().unwrap(), linear);
    let expected = photocraft_cms::math::apply(&photocraft_cms::math::invert(working_profile.matrix.as_ref().unwrap()).unwrap(), xyz);
    let mut out = [0.0; 3];
    context.to_working_f64(&input, &mut out).unwrap();
    for i in 0..3 {
        assert!((out[i] - expected[i]).abs() < 1e-12, "{out:?} / {expected:?}");
    }
}

#[test]
fn malformed_icc_and_invalid_interpretations_fail_without_fallback() {
    let bytes = Builtin::Srgb.profile().to_bytes();
    for length in 0..132 {
        assert!(parse_bounded_profile(&bytes[..length], ColorSpace::Rgb).is_err());
    }
    assert!(parse_bounded_profile(&bytes, ColorSpace::Gray).is_err());
    let mut broken = bytes.to_vec();
    broken[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse_bounded_profile(&broken, ColorSpace::Rgb).is_err());
    let mut broken = bytes.to_vec();
    broken[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(parse_bounded_profile(&broken, ColorSpace::Rgb).is_err());
    for curve in [
        Curve::Gamma(0.0),
        Curve::Gamma(f64::NAN),
        Curve::Table(vec![0.0, 1.0, 0.5]),
        Curve::Table(vec![0.0, f32::NAN]),
        Curve::Parametric { kind: 5, p: [1.0; 7] },
    ] {
        let mut p = Builtin::Srgb.profile().clone();
        p.trc = Some(std::array::from_fn(|_| curve.clone()));
        assert!(validate_profile(&p).is_err());
    }
    let mut p = Builtin::Srgb.profile().clone();
    p.matrix = Some([[0.0; 3]; 3]);
    assert!(validate_profile(&p).is_err());
}

#[test]
fn wide_byte_u32_identity_preserves_adjacent_codes_and_extras() {
    use photocraft_cms::pipeline::Pipeline;
    use photocraft_cms::{Transform, TransformOptions, WideSampleKind, WideWork};
    let transform = Transform::from_pipeline(Pipeline::new(3, vec![]), TransformOptions::default());
    let codes = [0, 1, u32::MAX - 1, u32::MAX, 2147483647, 2147483648, 2147483649, 17];
    let src: Vec<u8> = codes.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut out = vec![19; src.len()];
    let mut events = vec![];
    transform
        .try_convert_bytes_wide(WideSampleKind::U32, &src, 4, &mut out, 4, true, NonFinitePolicy::Reject, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
    assert_eq!(out, src);
    assert_eq!(events.iter().filter(|e| matches!(e, WideWork::ReserveBytes(_))).count(), 1);
    assert_eq!(events.iter().filter(|e| matches!(e, WideWork::ReleaseBytes(_))).count(), 1);
}

#[test]
fn wide_byte_f64_keeps_hdr_detail_and_extra_nan_payload_bits() {
    use photocraft_cms::pipeline::Pipeline;
    use photocraft_cms::{Transform, TransformOptions, WideSampleKind};
    let transform = Transform::from_pipeline(Pipeline::new(3, vec![]), TransformOptions::default());
    let samples = [0.5 + 2.0_f64.powi(-40), -3.0, 5.0, f64::from_bits(0x7ff8_0000_0000_4321)];
    let src: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut out = vec![0; src.len()];
    transform.try_convert_bytes_wide(WideSampleKind::F64, &src, 4, &mut out, 4, true, NonFinitePolicy::PreserveBits, &mut |_| Ok(())).unwrap();
    assert_eq!(out, src);
    let before = out.clone();
    assert!(transform.try_convert_bytes_wide(WideSampleKind::F64, &src, 4, &mut out, 4, true, NonFinitePolicy::Reject, &mut |_| Ok(())).is_err());
    assert_eq!(out, before);
}

#[test]
fn wide_byte_cancellation_denial_and_bad_strides_are_atomic() {
    use photocraft_cms::pipeline::Pipeline;
    use photocraft_cms::{CmsError, Transform, TransformOptions, WideSampleKind, WideWork};
    let transform = Transform::from_pipeline(Pipeline::new(3, vec![]), TransformOptions::default());
    let src: Vec<u8> = [0.2_f64; 12].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut out = vec![17; src.len()];
    let before = out.clone();
    let mut visited = 0;
    let mut released = 0;
    assert!(
        transform
            .try_convert_bytes_wide(WideSampleKind::F64, &src, 3, &mut out, 3, false, NonFinitePolicy::Reject, &mut |event| {
                if matches!(event, WideWork::ReleaseBytes(_)) {
                    released += 1;
                }
                if matches!(event, WideWork::SampleUnits(units) if units > 0) {
                    visited += 1;
                    if visited == 3 {
                        return Err(CmsError::Invalid("cancelled".into()));
                    }
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(released, 1);
    assert_eq!(out, before);
    assert!(
        transform
            .try_convert_bytes_wide(WideSampleKind::F64, &src, 3, &mut out, 3, false, NonFinitePolicy::Reject, &mut |event| if matches!(
                event,
                WideWork::ReserveBytes(_)
            ) {
                Err(CmsError::Invalid("resident budget".into()))
            } else {
                Ok(())
            })
            .is_err()
    );
    assert_eq!(out, before);
    for stride in [0, 1, usize::MAX] {
        assert!(transform.try_convert_bytes_wide(WideSampleKind::F64, &src, stride, &mut out, 3, false, NonFinitePolicy::Reject, &mut |_| Ok(())).is_err());
        assert_eq!(out, before);
    }
    assert!(
        transform.try_convert_bytes_wide(WideSampleKind::F64, &src[..src.len() - 1], 3, &mut out, 3, false, NonFinitePolicy::Reject, &mut |_| Ok(())).is_err()
    );
    assert_eq!(out, before);
}

#[test]
fn wide_gray_rgb_conversion_changes_channels_and_copies_alpha_bits() {
    use photocraft_cms::{Intent, Transform, WideSampleKind};
    let gray = Builtin::SGray.profile();
    let rgb = Builtin::Srgb.profile();
    let parsed_gray = Profile::parse(&gray.to_bytes()).unwrap();
    let parsed_rgb = Profile::parse(&rgb.to_bytes()).unwrap();
    let transform = Transform::new(&parsed_gray, &parsed_rgb, Intent::RelativeColorimetric, false).unwrap();
    let samples = [0.3_f64 + 2.0_f64.powi(-40), 0.8];
    let source: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut target = vec![0; 4 * 8];
    transform.try_convert_bytes_wide(WideSampleKind::F64, &source, 2, &mut target, 4, true, NonFinitePolicy::Reject, &mut |_| Ok(())).unwrap();
    let decoded: Vec<f64> = target.as_chunks::<8>().0.iter().map(|v| f64::from_ne_bytes(*v)).collect();
    for value in &decoded[..3] {
        assert!((*value - samples[0]).abs() < 1e-4);
    }
    assert_eq!(decoded[3].to_bits(), samples[1].to_bits());
    let context = SourceColorContext::new(
        None,
        &gray.to_bytes(),
        &rgb.to_bytes(),
        ColorSpace::Gray,
        Encoding::NonlinearProfile,
        Encoding::NonlinearProfile,
        SamplePrecision::F64,
        NonFinitePolicy::Reject,
    )
    .unwrap();
    let mut out = [0.0; 3];
    context.to_working_f64(&samples[..1], &mut out).unwrap();
    assert_eq!(out, decoded[..3]);
}

#[test]
fn wide_byte_exact_slice_limit_and_release_failure_remain_atomic() {
    use photocraft_cms::pipeline::Pipeline;
    use photocraft_cms::{CmsError, Transform, TransformOptions, WideSampleKind, WideWork};
    let transform = Transform::from_pipeline(Pipeline::new(1, vec![]), TransformOptions::default());
    let src = vec![0; 65536];
    let mut dst = vec![17; src.len()];
    transform.try_convert_bytes_wide(WideSampleKind::U32, &src, 1, &mut dst, 1, false, NonFinitePolicy::Reject, &mut |_| Ok(())).unwrap();
    assert_eq!(dst, src);
    dst.fill(17);
    let before = dst.clone();
    assert!(
        transform
            .try_convert_bytes_wide(WideSampleKind::U32, &src, 1, &mut dst, 1, false, NonFinitePolicy::Reject, &mut |event| if matches!(
                event,
                WideWork::ReleaseBytes(_)
            ) {
                Err(CmsError::Invalid("cancelled at final checkpoint".into()))
            } else {
                Ok(())
            })
            .is_err()
    );
    assert_eq!(dst, before);
    let src = vec![0; 65540];
    let mut dst = vec![17; src.len()];
    let before = dst.clone();
    assert!(transform.try_convert_bytes_wide(WideSampleKind::U32, &src, 1, &mut dst, 1, false, NonFinitePolicy::Reject, &mut |_| Ok(())).is_err());
    assert_eq!(dst, before);
}

#[test]
fn wide_clut_converts_and_integer_hdr_conversion_fails_explicitly() {
    use photocraft_cms::pipeline::{Pipeline, Stage};
    use photocraft_cms::{Clut, Transform, TransformOptions, WideSampleKind};
    let clut = Clut::new(vec![2], 1, vec![0.0, 1.0]);
    let transform = Transform::from_pipeline(Pipeline::new(1, vec![Stage::Clut(clut)]), TransformOptions::default());
    let mut out = [19.0];
    let input = 0.5 + 2.0_f64.powi(-40);
    transform.try_eval_f64(&[input], &mut out).unwrap();
    assert_eq!(out, [input]);
    let transform =
        Transform::from_pipeline(Pipeline::new(1, vec![Stage::Matrix { rows: 1, cols: 1, m: vec![2.0], offset: vec![0.0] }]), TransformOptions::default());
    let mut out = [17_u8; 4];
    assert!(
        transform
            .try_convert_bytes_wide(WideSampleKind::U32, &u32::MAX.to_ne_bytes(), 1, &mut out, 1, false, NonFinitePolicy::Reject, &mut |_| Ok(()))
            .is_err()
    );
    assert_eq!(out, [17; 4]);
}

#[test]
fn checked_preview_transfer_rejects_overflow_without_committing_output() {
    use photocraft_cms::xcf_color::try_convert_encoding_f32;
    let mut p = Builtin::Srgb.profile().clone();
    p.trc = Some([Curve::Gamma(2.0), Curve::Gamma(2.0), Curve::Gamma(2.0)]);
    let mut output = [17.0; 3];
    assert!(try_convert_encoding_f32(&p, Encoding::NonlinearProfile, Encoding::Linear, &[f32::MAX, 0.5, 0.5], &mut output).is_err());
    assert_eq!(output, [17.0; 3]);
    try_convert_encoding_f32(&p, Encoding::NonlinearProfile, Encoding::Linear, &[0.5; 3], &mut output).unwrap();
    assert_eq!(output, [0.25; 3]);
}

#[test]
fn identical_builtin_profile_preserves_authoritative_wide_bits() {
    use photocraft_cms::{Intent, Transform, WideSampleKind};
    let profile = Builtin::LinearSrgb.profile();
    let transform = Transform::new(profile, profile, Intent::RelativeColorimetric, false).unwrap();
    assert!(transform.is_identity());
    let parsed = photocraft_cms::Profile::parse(&profile.to_bytes()).unwrap();
    let hashed = parsed.clone();
    let _ = hashed.content_hash();
    assert!(Transform::new(&parsed, &hashed, Intent::RelativeColorimetric, false).unwrap().is_identity());
    let values = [-0.0_f64, 3.25, 0.5 + 2.0_f64.powi(-40), f64::from_bits(0x7ff8000000000042)];
    let bytes: Vec<_> = values.iter().flat_map(|value| value.to_ne_bytes()).collect();
    let mut output = vec![17; bytes.len()];
    transform.try_convert_bytes_wide(WideSampleKind::F64, &bytes, 4, &mut output, 4, true, NonFinitePolicy::PreserveBits, &mut |_| Ok(())).unwrap();
    assert_eq!(bytes, output);
    let codes = [0x80000000_u32, 0x80000001, 0xffffffff, 0xfffffffe];
    let bytes: Vec<_> = codes.iter().flat_map(|value| value.to_ne_bytes()).collect();
    transform.try_convert_bytes_wide(WideSampleKind::U32, &bytes, 4, &mut output[..bytes.len()], 4, true, NonFinitePolicy::Reject, &mut |_| Ok(())).unwrap();
    assert_eq!(&output[..bytes.len()], bytes);
    let nan_color: Vec<_> = [f64::NAN, 0.5, 0.5, 1.0].iter().flat_map(|value| value.to_ne_bytes()).collect();
    let before = output.clone();
    assert!(
        transform.try_convert_bytes_wide(WideSampleKind::F64, &nan_color, 4, &mut output, 4, true, NonFinitePolicy::PreserveBits, &mut |_| Ok(())).is_err()
    );
    assert_eq!(output, before);
}
