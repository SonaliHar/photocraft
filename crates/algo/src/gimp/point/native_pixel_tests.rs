//! UNRUN shared native boundary regression drafts, no source qualification.
use super::*;
use photocraft_color::SampleType;
#[test]
fn color_only_keeps_nan_alpha_and_native_depth_words_while_identity_keeps_all_raw() {
    for sample in SampleType::ALL {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut input = vec![0u8; format.bytes_per_pixel()];
        for c in 0..3 {
            try_write_sample_f64(&mut input, sample, c, [0.25, 0.5, 0.75][c]).unwrap();
        }
        try_write_sample_f64(&mut input, sample, 3, 0.125).unwrap();
        if sample == SampleType::F64 {
            input[24..].copy_from_slice(&0x7ff8123456789012u64.to_ne_bytes());
        }
        if sample == SampleType::F32 {
            input[12..].copy_from_slice(&0x7fc12345u32.to_ne_bytes());
        }
        let mut actual = vec![0u8; input.len()];
        PointOperation::Invert.prepare().unwrap().try_eval_native_pixel(&input, format, &mut actual).unwrap();
        let mut expected = input.clone();
        for c in 0..3 {
            try_write_sample_f64(&mut expected, sample, c, 1. - try_read_sample_f64(&input, sample, c).unwrap()).unwrap();
        }
        assert_eq!(actual, expected);
        PointOperation::Exposure { stops: 0., black_level: 0. }.prepare().unwrap().try_eval_native_pixel(&input, format, &mut actual).unwrap();
        assert_eq!(actual, input);
    }
}
#[test]
fn alpha_only_retains_all_color_bytes_and_native_failure_is_transactional() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let values = [0.25 + 2f64.powi(-45), -0.0, 0.75, 0.75];
    let input: Vec<_> = values.into_iter().flat_map(f64::to_ne_bytes).collect();
    let mut out = vec![0x55; input.len()];
    PointOperation::ThresholdAlpha { value: 0.5 }.prepare().unwrap().try_eval_native_pixel(&input, format, &mut out).unwrap();
    assert_eq!(&out[..24], &input[..24]);
    assert_eq!(&out[24..], &1f64.to_ne_bytes());
    let mut invalid = input.clone();
    invalid[..8].copy_from_slice(&f64::NAN.to_ne_bytes());
    let original = out.clone();
    assert!(PointOperation::Invert.prepare().unwrap().try_eval_native_pixel(&invalid, format, &mut out).is_err());
    assert_eq!(out, original);
    assert!(PointOperation::Invert.prepare().unwrap().try_eval_native_pixel(&input[..8], format, &mut out).is_err());
    assert_eq!(out, original);
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    assert!(PointOperation::ThresholdAlpha { value: 0.5 }.prepare().unwrap().validate_native_format(gray).is_err());
}
#[test]
fn coupled_color_and_alpha_matches_existing_typed_tuple_boundary() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let values = [0.25f32, 0.5, 0.75, 0.375];
    let input: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
    let operation = PointOperation::ColorOverlay { color: [0.8, 0.2, 0.1, 0.6] };
    let prepared = operation.prepare().unwrap();
    let mut wide = [0.; 4];
    prepared.try_eval_pixel(&values.map(f64::from), 3, &mut wide).unwrap();
    let mut expected = input.clone();
    for (c, v) in wide.into_iter().enumerate() {
        try_write_sample_f64(&mut expected, SampleType::F32, c, v).unwrap();
    }
    let mut output = input.clone();
    prepared.try_eval_native_pixel(&input, format, &mut output).unwrap();
    assert_eq!(output, expected);
}
