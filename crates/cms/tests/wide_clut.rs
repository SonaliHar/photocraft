//! Wide interpolation checks with independent analytic oracles and original ICC bytes.
use photocraft_cms::pipeline::{Pipeline, Stage};
use photocraft_cms::profile::{Lut, LutKind};
use photocraft_cms::xcf_color::NonFinitePolicy;
use photocraft_cms::{Builtin, Clut, Curve, Intent, Profile, Transform, TransformOptions, WideSampleKind, WideWork};

#[test]
fn gray_and_bilinear_coordinates_keep_below_f32_detail() {
    let x = 0.5 + 2.0_f64.powi(-40);
    let gray = Clut::try_new_f64(vec![2], 1, vec![0.0, 1.0]).unwrap();
    let mut output = [0.0];
    gray.try_eval_f64(&[x], &mut output).unwrap();
    assert_eq!(output, [x]);
    let xy = Clut::try_new_f64(vec![2, 2], 1, vec![0.0, 0.0, 0.0, 1.0]).unwrap();
    xy.try_eval_f64(&[x, 0.75], &mut output).unwrap();
    assert_eq!(output, [x * 0.75]);
    assert_ne!(output[0], f64::from(x as f32) * 0.75);
}

#[test]
fn six_tetrahedra_match_independent_minimum_function() {
    // Only the far corner is one: tetrahedral interpolation gives min(x,y,z).
    let mut nodes = vec![0.0; 8];
    nodes[7] = 1.0;
    let table = Clut::try_new_f64(vec![2; 3], 1, nodes).unwrap();
    let a = 0.2 + 2.0_f64.powi(-40);
    for input in [[a, 0.5, 0.8], [a, 0.8, 0.5], [0.5, a, 0.8], [0.5, 0.8, a], [0.8, a, 0.5], [0.8, 0.5, a]] {
        let mut out = [0.0];
        table.try_eval_f64(&input, &mut out).unwrap();
        assert!((out[0] - a).abs() < 1e-16, "{input:?} => {out:?}");
    }
}

#[test]
fn four_dimensional_and_general_multilinear_match_product_oracles() {
    for dimensions in [4, 5] {
        let mut nodes = vec![0.0; 1 << dimensions];
        *nodes.last_mut().unwrap() = 1.0;
        let table = Clut::try_new_f64(vec![2; dimensions], 1, nodes).unwrap();
        let input = [0.3 + 2.0_f64.powi(-40), 0.2, 0.6, 0.9, 0.8];
        let expected = if dimensions == 4 { input[0] * input[1..4].iter().copied().fold(1.0_f64, f64::min) } else { input[..dimensions].iter().product() };
        let mut out = [0.0];
        table.try_eval_f64(&input[..dimensions], &mut out).unwrap();
        assert!((out[0] - expected).abs() < 1e-16);
    }
}

#[test]
fn degenerate_dimensions_and_finite_outside_cube_are_defined() {
    let table = Clut::try_new_f64(vec![1, 2, 1], 1, vec![0.25, 0.75]).unwrap();
    let mut out = [0.0];
    table.try_eval_f64(&[-8.0, 0.5, 10.0], &mut out).unwrap();
    assert_eq!(out, [0.5]);
    table.try_eval_f64(&[0.0, -1.0, 0.0], &mut out).unwrap();
    assert_eq!(out, [0.25]);
    table.try_eval_f64(&[0.0, 2.0, 0.0], &mut out).unwrap();
    assert_eq!(out, [0.75]);
}

#[test]
fn invalid_layouts_nonfinite_and_bounds_fail_atomically() {
    let table = Clut::try_new_f64(vec![2; 3], 1, vec![0.0; 8]).unwrap();
    for input in [[f64::NAN, 0.5, 0.5], [f64::INFINITY, 0.5, 0.5]] {
        let mut out = [17.0];
        assert!(table.try_eval_f64(&input, &mut out).is_err());
        assert_eq!(out, [17.0]);
    }
    for mutation in 0..5 {
        let mut table = table.clone();
        match mutation {
            0 => table.grid[0] = 0,
            1 => table.grid[0] = usize::MAX,
            2 => table.data.clear(),
            3 => table.outputs = 0,
            _ => table.data[0] = f32::NAN,
        }
        let mut out = [17.0];
        assert!(table.try_eval_f64(&[0.5; 3], &mut out).is_err());
        assert_eq!(out, [17.0]);
    }
    assert!(Clut::try_new_f64(vec![usize::MAX; 3], 3, vec![]).is_err());
    assert!(Clut::try_new_f64(vec![2], 1, vec![0.0, f64::NAN]).is_err());
}

#[test]
fn wide_nodes_remain_exact_and_public_edits_replace_original_nodes() {
    let exact = 12345.0 / 65535.0;
    let mut table = Clut::try_new_f64(vec![2], 1, vec![exact, 1.0]).unwrap();
    let mut out = [0.0];
    table.try_eval_f64(&[0.0], &mut out).unwrap();
    assert_eq!(out, [exact]);
    assert_ne!(out[0], f64::from(exact as f32));
    table.data[0] = 0.25;
    table.try_eval_f64(&[0.0], &mut out).unwrap();
    assert_eq!(out, [0.25]);
}

#[test]
fn parsed_rgb_lut_retains_bytes_and_exact_integer_node_interpretation() {
    let mut source = Builtin::Srgb.profile().clone();
    let value = 12345.0 / 65535.0;
    let nodes = vec![value; 8 * 3];
    source.a2b[0] = Some(Lut {
        kind: LutKind::Ab,
        inputs: 3,
        outputs: 3,
        stages: vec![
            Stage::Curves(vec![Curve::Identity; 3]),
            Stage::Clut(Clut::try_new_f64(vec![2; 3], 3, nodes).unwrap()),
            Stage::Curves(vec![Curve::Identity; 3]),
        ],
    });
    let bytes = source.with_encoded_bytes().to_bytes();
    let source = Profile::parse(&bytes).unwrap();
    assert_eq!(source.to_bytes(), bytes);
    let (stages, _) = source.device_to_pcs(Intent::RelativeColorimetric).unwrap();
    let transform = Transform::from_pipeline(Pipeline::new(3, stages), TransformOptions::default());
    let mut out = [0.0; 3];
    transform.try_eval_f64(&[0.3 + 2.0_f64.powi(-40); 3], &mut out).unwrap();
    let expected = value * (65535.0 / 32768.0);
    for value in out {
        assert!((value - expected).abs() < 1e-16, "{value} / {expected}");
    }
}

#[test]
fn wide_bulk_lut_accounts_taps_and_preserves_copied_alpha_bits() {
    let table = Clut::try_new_f64(vec![2; 3], 3, (0..8).flat_map(|i| [f64::from((i >> 2) & 1), f64::from((i >> 1) & 1), f64::from(i & 1)]).collect()).unwrap();
    let transform = Transform::from_pipeline(Pipeline::new(3, vec![Stage::Clut(table)]), TransformOptions::default());
    let input = [0.5 + 2.0_f64.powi(-40), 0.25, 0.75, f64::from_bits(0x7ff8_0000_0000_1298)];
    let src: Vec<u8> = input.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut out = vec![0; src.len()];
    let mut work = 0;
    transform
        .try_convert_bytes_wide(WideSampleKind::F64, &src, 4, &mut out, 4, true, NonFinitePolicy::PreserveBits, &mut |event| {
            if let WideWork::SampleUnits(units) = event {
                work += units;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(out, src);
    assert!(work >= 3 + 4 * 3);
}

#[test]
fn parsed_gray_lut_and_curve_preserve_exact_integer_values() {
    let mut gray = Builtin::SGray.profile().clone();
    let codes = [12345_u16, 23456, 34567];
    gray.a2b[0] = Some(Lut {
        kind: LutKind::Ab,
        inputs: 1,
        outputs: 3,
        stages: vec![
            Stage::Curves(vec![Curve::Identity]),
            Stage::Clut(Clut::try_new_f64(vec![2], 3, codes.iter().chain(codes.iter()).map(|v| f64::from(*v) / 65535.0).collect()).unwrap()),
            Stage::Curves(vec![Curve::Identity; 3]),
        ],
    });
    gray.gray_trc = Some(Curve::Table64(vec![0.0, 12345.0 / 65535.0, 1.0]));
    let bytes = gray.with_encoded_bytes().to_bytes();
    let parsed = Profile::parse(&bytes).unwrap();
    assert_eq!(parsed.to_bytes(), bytes);
    assert_eq!(parsed.gray_trc.as_ref().unwrap().eval64(0.5), 12345.0 / 65535.0);
    let (stages, _) = parsed.device_to_pcs(Intent::RelativeColorimetric).unwrap();
    let transform = Transform::from_pipeline(Pipeline::new(1, stages), TransformOptions::default());
    let mut out = [0.0; 3];
    transform.try_eval_f64(&[0.5 + 2.0_f64.powi(-40)], &mut out).unwrap();
    for i in 0..3 {
        assert!((out[i] - f64::from(codes[i]) / 32768.0).abs() < 2e-16);
    }
}

#[test]
fn bulk_lut_work_denial_does_not_commit_partial_conversion() {
    let table = Clut::try_new_f64(vec![2; 3], 3, vec![0.5; 24]).unwrap();
    let transform = Transform::from_pipeline(Pipeline::new(3, vec![Stage::Clut(table)]), TransformOptions::default());
    let src: Vec<u8> = [0.5_f64; 6].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut dst = vec![19; src.len()];
    let before = dst.clone();
    let mut work = 0;
    let mut released = false;
    assert!(
        transform
            .try_convert_bytes_wide(WideSampleKind::F64, &src, 3, &mut dst, 3, false, NonFinitePolicy::Reject, &mut |event| {
                if let WideWork::SampleUnits(units) = event {
                    work += units;
                    if work > 18 {
                        return Err(photocraft_cms::CmsError::Invalid("sample work limit".into()));
                    }
                }
                if matches!(event, WideWork::ReleaseBytes(_)) {
                    released = true;
                }
                Ok(())
            })
            .is_err()
    );
    assert_eq!(dst, before);
    assert!(released);
}

#[test]
fn lut_only_context_preserves_nonlinear_pipeline_and_rejects_transfer_override() {
    use photocraft_cms::ColorSpace;
    use photocraft_cms::xcf_color::{Encoding, SamplePrecision, SourceColorContext, convert_encoding_f64, effective_profile, try_convert_encoding_f32};
    for (space, mut source, channels, working) in [
        (ColorSpace::Rgb, Builtin::Srgb.profile().clone(), 3, Builtin::LinearSrgb.profile()),
        (ColorSpace::Gray, Builtin::SGray.profile().clone(), 1, Builtin::Srgb.profile()),
    ] {
        source.matrix = None;
        source.trc = None;
        source.gray_trc = None;
        source.a2b[0] = Some(Lut {
            kind: LutKind::Ab,
            inputs: channels,
            outputs: 3,
            stages: vec![
                Stage::Curves(vec![Curve::Identity; channels]),
                Stage::Clut(Clut::try_new_f64(vec![2; channels], 3, vec![0.2; (1 << channels) * 3]).unwrap()),
                Stage::Curves(vec![Curve::Identity; 3]),
            ],
        });
        let bytes = source.with_encoded_bytes().to_bytes();
        let profile = effective_profile(Some(&bytes), space, Encoding::NonlinearProfile).unwrap();
        assert_eq!(profile.to_bytes(), bytes);
        let context = SourceColorContext::new(
            Some(&bytes),
            &bytes,
            &working.to_bytes(),
            space,
            Encoding::NonlinearProfile,
            Encoding::NonlinearProfile,
            photocraft_cms::xcf_color::SamplePrecision::F64,
            NonFinitePolicy::Reject,
        )
        .unwrap();
        assert_eq!(context.assigned_bytes().unwrap(), bytes);
        assert_eq!(context.source_bytes(), bytes);
        assert_eq!(context.source_profile().to_bytes(), bytes);
        let identity = SourceColorContext::new(
            Some(&bytes),
            &bytes,
            &bytes,
            space,
            Encoding::NonlinearProfile,
            Encoding::NonlinearProfile,
            SamplePrecision::F64,
            NonFinitePolicy::Reject,
        )
        .unwrap();
        let detail = vec![0.5 + 2.0_f64.powi(-40); channels];
        let mut identity_out = vec![0.0; channels];
        identity.to_working_f64(&detail, &mut identity_out).unwrap();
        assert_eq!(identity_out, detail);
        let mut out = [0.0; 3];
        context.to_working_f64(&vec![0.5 + 2.0_f64.powi(-40); channels], &mut out).unwrap();
        assert!(out.iter().all(|v| v.is_finite()));
        for override_encoding in [Encoding::Linear, Encoding::Perceptual] {
            let error = effective_profile(Some(&bytes), space, override_encoding).unwrap_err();
            assert!(error.to_string().contains("retain profile-defined nonlinear encoding"));
            assert!(
                SourceColorContext::new(
                    Some(&bytes),
                    &bytes,
                    &working.to_bytes(),
                    space,
                    override_encoding,
                    Encoding::NonlinearProfile,
                    SamplePrecision::F64,
                    NonFinitePolicy::Reject
                )
                .is_err()
            );
        }
        let input = vec![0.5; channels];
        let mut identity = vec![0.0; channels];
        convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::NonlinearProfile, &input, &mut identity).unwrap();
        assert_eq!(identity, input);
        assert!(convert_encoding_f64(&profile, Encoding::NonlinearProfile, Encoding::Linear, &input, &mut identity).is_err());
        assert_eq!(identity, input);
        let mut preview = vec![17.0_f32; channels];
        assert!(try_convert_encoding_f32(&profile, Encoding::NonlinearProfile, Encoding::Linear, &vec![0.5; channels], &mut preview).is_err());
        assert_eq!(preview, vec![17.0; channels]);
    }
}
