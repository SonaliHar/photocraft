//! Synthetic declared-model drafts, UNRUN; not GIMP source/render qualification.
use super::*;
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
}
fn parameters() -> Parameters {
    Parameters { time_bits: Some(0), ranges: [[0; 3]; 7], overlap_bits: 0 }
}
fn prepare(format: PixelFormat, params: Parameters) -> PreparedMixedHueSaturation {
    PreparedMixedHueSaturation::prepare(format, context(), params, NumericPolicy::OriginalHslF32SixPhase, Limits::default(), &Interrupt::NONE).unwrap()
}
fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() <= 2e-7, "{a} versus {b}");
    }
}
#[test]
fn previous_direction_recipes_match_declared_partial_color_equations() {
    let mut sat = parameters();
    sat.ranges[0][1] = (-0.25f64).to_bits();
    let sat = prepare(PixelFormat::RGBA32F, sat);
    near(sat.try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [0.725, 0.425, 0.275]);
    near(sat.try_eval_float_rgb([0.4; 3]).unwrap(), [0.4; 3]);
    let mut light = parameters();
    light.ranges[0][2] = 0.25f64.to_bits();
    let light = prepare(PixelFormat::RGBA32F, light);
    near(light.try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [0.85, 0.55, 0.4]);
    near(light.try_eval_float_rgb([0.4; 3]).unwrap(), [0.55; 3]);
    let mut hue = parameters();
    hue.ranges[0][0] = 0.125f64.to_bits();
    hue.ranges[1][0] = (-0.125f64).to_bits();
    hue.overlap_bits = 0.25f64.to_bits();
    near(prepare(PixelFormat::RGBA32F, hue).try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [0.8, 0.4, 0.2]);
    hue.overlap_bits = 0.75f64.to_bits();
    near(prepare(PixelFormat::RGBA32F, hue).try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [0.8, 0.4625, 0.2]);
}
#[test]
fn all23_raw_words_policy_and_unknown_sectors_remain_separate_from_evaluation() {
    let mut signed = parameters();
    signed.ranges[5][2] = (-0f64).to_bits();
    signed.ranges[0][1] = (-0f64).to_bits();
    signed.overlap_bits = (-0f64).to_bits();
    let prepared = prepare(PixelFormat::RGBA32F, signed);
    assert_eq!(prepared.parameters(), signed);
    assert_eq!(prepared.policy(), NumericPolicy::OriginalHslF32SixPhase);
    let mut other = parameters();
    other.ranges[2][0] = 0.125f64.to_bits();
    assert!(matches!(
        PreparedMixedHueSaturation::prepare(PixelFormat::RGBA32F, context(), other, NumericPolicy::OriginalHslF32SixPhase, Limits::default(), &Interrupt::NONE),
        Err(Error::UnsupportedConfiguration)
    ));
    let mut negative = parameters();
    negative.ranges[0][1] = (-1f64).to_bits();
    negative.ranges[1][1] = (-1f64).to_bits();
    assert!(matches!(prepare(PixelFormat::RGBA32F, negative).try_eval_float_rgb([0.8, 0.4, 0.2]), Err(Error::UnsupportedNegativeSaturation)));
}
#[test]
fn mixed_sum_uses_original_hue_weight_and_achromatic_local_skip() {
    let mut p = parameters();
    p.ranges[0] = [0.125f64.to_bits(), (-0.125f64).to_bits(), 0.25f64.to_bits()];
    p.ranges[1] = [(-0.125f64).to_bits(), 0.125f64.to_bits(), (-0.125f64).to_bits()];
    p.overlap_bits = 0.5f64.to_bits();
    let candidate = prepare(PixelFormat::RGBA32F, p);
    // Retained GIMP endpoint words support this declared model within residual,
    // not source-exact/internal precision or overall rendering admission.
    near(candidate.try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [0.82382816, 0.520_643_5, 0.3220052]);
    near(candidate.try_eval_float_rgb([0.4; 3]).unwrap(), [0.55; 3]);
}
#[test]
fn explicit_float_six_phase_includes_the_observed_red_hard_boundary() {
    let mut p = parameters();
    p.ranges[1][1] = (-0.25f64).to_bits();
    let rgb = [0.65_f32, 0.35, 0.5];
    let output = prepare(PixelFormat::RGBA32F, p).try_eval_float_rgb(rgb).unwrap();
    near(output, [0.6125, 0.3875, 0.5]);
    assert!(output[0] < rgb[0]);
    assert!(output[1] > rgb[1]);
}
#[test]
fn result_s_upper_ceiling_is_distinct_from_parameter_and_lightness_clamps() {
    let mut p = parameters();
    p.ranges[0][1] = 1f64.to_bits();
    p.overlap_bits = 0.5f64.to_bits();
    near(prepare(PixelFormat::RGBA32F, p).try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap(), [1., 1. / 3., 0.]);
    p.ranges[1][1] = 1f64.to_bits();
    near(prepare(PixelFormat::RGBA32F, p).try_eval_float_rgb([0.65, 0.35, 0.5]).unwrap(), [0.875, 0.125, 0.5]);
    let mut light = parameters();
    light.ranges[0][2] = 1f64.to_bits();
    light.ranges[1][2] = 1f64.to_bits();
    light.overlap_bits = 0.5f64.to_bits();
    let hdr = prepare(PixelFormat::RGBA32F, light).try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap();
    near(hdr, [1.1666666, 1.5, 1.6666666]);
    assert!(hdr.iter().all(|v| *v > 1.));
    light.ranges[0][2] = (-1f64).to_bits();
    light.ranges[1][2] = (-1f64).to_bits();
    let negative = prepare(PixelFormat::RGBA32F, light).try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap();
    near(negative, [-0.6666667, -0.33333334, -0.16666669]);
    assert!(negative.iter().all(|v| *v < 0.));
}
#[test]
fn tiny_unit_float_colors_neutral_and_negative_saturation_do_not_divide_by_zero() {
    let tiny = f32::from_bits(1);
    let neutral = prepare(PixelFormat::RGBA32F, parameters());
    assert_eq!(neutral.try_eval_float_rgb([tiny, 0., 0.]).unwrap(), [tiny, 0., 0.]);
    let mut p = parameters();
    p.ranges[0][1] = (-0.25f64).to_bits();
    let candidate = prepare(PixelFormat::RGBA32F, p);
    assert!(candidate.try_eval_float_rgb([tiny, 0., 0.]).unwrap().iter().all(|v| v.is_finite()));
    let mut overlap = parameters();
    overlap.ranges[0][0] = 0.125f64.to_bits();
    overlap.overlap_bits = 1;
    assert!(matches!(
        PreparedMixedHueSaturation::prepare(
            PixelFormat::RGBA32F,
            context(),
            overlap,
            NumericPolicy::OriginalHslF32SixPhase,
            Limits::default(),
            &Interrupt::NONE
        ),
        Err(Error::Parameters)
    ));
}
#[test]
fn double_float_ingress_preserves_raw_alpha_default_and_unselected_nonfinite_words() {
    let format = PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F64, alpha: true };
    let alpha = 0xfff8_0000_0000_0042u64;
    let outside = [0x7ff8_1234_5678_9abcu64, (-10f64).to_bits(), f64::INFINITY.to_bits(), alpha];
    let selected = [0.8f64.to_bits() + 1, 0.4f64.to_bits(), 0.2f64.to_bits(), alpha];
    let default = outside.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    let extent = Rect::new(-1, 0, 1, 1);
    let roi = Rect::new(0, 0, 1, 1);
    let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
    let raw = outside.into_iter().chain(selected).flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    source.try_write_interleaved_with(extent, &raw, &Interrupt::NONE).unwrap();
    let before = source.clone();
    let mut p = parameters();
    p.ranges[0][2] = 0.25f64.to_bits();
    let candidate = prepare(format, p);
    let expected = candidate.try_eval_float_rgb([0.8, 0.4, 0.2]).unwrap();
    let output = candidate.apply(&source, roi, context(), Limits::default(), &Interrupt::NONE).unwrap();
    let expected = expected.into_iter().map(|v| f64::from(v).to_bits()).chain([alpha]).flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    assert_eq!(output.try_read_interleaved(roi).unwrap(), expected);
    assert_eq!(output.default_bytes(), default);
    assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), source.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap());
    assert_eq!(source, before);
}
#[test]
fn split_roi_has_same_coordinate_local_words_and_work_cancellation_precedes_publication() {
    let format = PixelFormat::RGBA32F;
    let extent = Rect::new(0, 0, 2, 1);
    let raw = [0.8f32, 0.4, 0.2, 0.25, 0.4, 0.4, 0.4, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
    let before = source.clone();
    let mut p = parameters();
    p.ranges[0][1] = (-0.25f64).to_bits();
    let candidate = prepare(format, p);
    let full = candidate.apply(&source, extent, context(), Limits::default(), &Interrupt::NONE).unwrap();
    for roi in [Rect::new(0, 0, 1, 1), Rect::new(1, 0, 2, 1)] {
        let partial = candidate.apply(&source, roi, context(), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(partial.try_read_interleaved(roi).unwrap(), full.try_read_interleaved(roi).unwrap());
    }
    let initial = candidate.remaining_work();
    assert!(matches!(
        candidate.apply(&source, Rect::new(0, 0, 1, 1), context(), Limits { work_units: 5000, ..Limits::default() }, &Interrupt::NONE),
        Err(Error::Raster(RasterInputError::LimitExceeded))
    ));
    assert_eq!(candidate.remaining_work(), initial);
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
    assert!(matches!(
        candidate.apply(&source, extent, context(), Limits::default(), &Interrupt::cancel_only(&cancelled)),
        Err(Error::Raster(RasterInputError::Cancelled))
    ));
    assert!(candidate.remaining_work() < initial);
    assert_eq!(source, before);
}
#[test]
fn unit_native_ingress_refuses_before_float_rounding_can_hide_out_of_domain_values() {
    let format = PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F64, alpha: false };
    let roi = Rect::new(0, 0, 1, 1);
    let raw = [f64::from_bits(1f64.to_bits() + 1), 0.4, 0.2].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(format, roi, &raw).unwrap();
    let before = source.clone();
    assert!(matches!(prepare(format, parameters()).apply(&source, roi, context(), Limits::default(), &Interrupt::NONE), Err(Error::OutsideUnit)));
    assert_eq!(source, before);
}
