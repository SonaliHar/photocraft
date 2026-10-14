//! Synthetic declared-model drafts, UNRUN; not GIMP source/render qualification.
use super::*;
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
}
fn parameters() -> Parameters {
    Parameters { time_bits: Some(0), ranges: [[0; 3]; 7], overlap_bits: 0 }
}
fn prepare(format: PixelFormat, params: Parameters) -> PreparedHueSaturation {
    PreparedHueSaturation::prepare(format, context(), params, Limits::default(), &Interrupt::NONE).unwrap()
}
fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() <= 2e-7, "{a} versus {b}");
    }
}
#[test]
fn exclusive_dynamic_recipes_match_declared_partial_color_equations() {
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
fn all23_raw_words_remain_stored_and_unobserved_recipe_composition_refuses() {
    let mut signed = parameters();
    signed.ranges[5][2] = (-0f64).to_bits();
    signed.ranges[0][1] = (-0f64).to_bits();
    signed.overlap_bits = (-0f64).to_bits();
    assert_eq!(prepare(PixelFormat::RGBA32F, signed).parameters(), signed);
    let mut bad = Vec::new();
    let mut mixed = parameters();
    mixed.ranges[0][0] = 0.125f64.to_bits();
    mixed.ranges[0][1] = (-0.25f64).to_bits();
    bad.push(mixed);
    let mut other = parameters();
    other.ranges[2][0] = 0.125f64.to_bits();
    bad.push(other);
    let mut red = parameters();
    red.ranges[1][1] = (-0.25f64).to_bits();
    bad.push(red);
    let mut positive = parameters();
    positive.ranges[0][1] = 0.25f64.to_bits();
    bad.push(positive);
    let mut negative = parameters();
    negative.ranges[0][2] = (-0.25f64).to_bits();
    bad.push(negative);
    for params in bad {
        assert!(matches!(
            PreparedHueSaturation::prepare(PixelFormat::RGBA32F, context(), params, Limits::default(), &Interrupt::NONE),
            Err(Error::UnsupportedConfiguration)
        ));
    }
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
    assert!(matches!(PreparedHueSaturation::prepare(PixelFormat::RGBA32F, context(), overlap, Limits::default(), &Interrupt::NONE), Err(Error::Parameters)));
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
