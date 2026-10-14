// Draft regression controls; UNRUN in the implementation-only cohort.
use super::*;
fn context() -> WorkingContext {
    WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
}
fn source(values: &[[f32; 4]]) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    let bytes: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_ne_bytes()).collect();
    s.try_write_interleaved_with(Rect::new(0, 0, values.len() as i32, 1), &bytes, &Interrupt::NONE).unwrap();
    s
}
fn prepare(s: &Surface) -> PreparedStretchHsv<'_> {
    PreparedStretchHsv::prepare(s, Rect::new(0, 0, 3, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat, Limits::default(), &Interrupt::NONE).unwrap()
}
#[test]
fn global_alpha_zero_extrema_split_rois_and_raw_off_roi_alpha() {
    let s = source(&[[0.25, 0.25, 0.25, 0.], [0.5, 0.25, 0.25, f32::from_bits(0x7fc01234)], [1., 0., 0., 0.5]]);
    let before = s.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    let p = prepare(&s);
    assert_eq!(p.extrema(), ([0., 0.25], [1., 0.75]));
    let whole = p.apply_with_limits(Rect::new(0, 0, 3, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    let left = p.apply_with_limits(Rect::new(0, 0, 1, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    let middle = p.apply_with_limits(Rect::new(1, 0, 2, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    let full = whole.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    assert_eq!(&left.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap()[..16], &full[..16]);
    let bytes = middle.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap();
    assert_eq!(&bytes[16..32], &full[16..32]);
    assert_eq!(&bytes[..16], &before[..16]);
    assert_eq!(&bytes[32..], &before[32..]);
    assert_eq!(&bytes[28..32], &before[28..32]);
    assert_eq!(middle.default_bytes(), s.default_bytes());
    let result = p.try_eval_float_color([0.5, 0.25, 0.25]).unwrap();
    assert_eq!(result, [1.0f32 / 3., 1.0f32 / 6., 1.0f32 / 6.]);
    assert_eq!(s.try_read_interleaved(Rect::new(0, 0, 3, 1)).unwrap(), before);
}
#[test]
fn exact_degenerate_saturation_and_value_keep_the_component() {
    for (values, expected) in [
        ([[0.25, 0.125, 0.125, 1.], [0.5, 0.25, 0.25, 0.]], [[0., 0., 0.], [1., 0.5, 0.5]]),
        ([[0.5, 0.5, 0.5, 1.], [0.5, 0., 0., 0.]], [[0.5, 0.5, 0.5], [0.5, 0., 0.]]),
    ] {
        let s = source(&values);
        let p = PreparedStretchHsv::prepare(
            &s,
            Rect::new(0, 0, 2, 1),
            context(),
            NumericPolicy::DeclaredBinary64HsvFinalFloat,
            Limits::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        for (input, want) in values.into_iter().zip(expected) {
            assert_eq!(p.try_eval_float_color([input[0], input[1], input[2]]).unwrap(), want);
        }
    }
}
#[test]
fn actual_source_outside_extent_is_not_interpreted_but_default_is_preserved() {
    let s = source(&[[f32::NAN, 0., 0., 1.], [0.25, 0.25, 0.25, 1.], [0.75, 0., 0., 1.]]);
    let p =
        PreparedStretchHsv::prepare(&s, Rect::new(1, 0, 3, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat, Limits::default(), &Interrupt::NONE)
            .unwrap();
    let out = p.apply_with_limits(Rect::new(1, 0, 2, 1), context(), Limits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(out.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), s.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap());
    assert_eq!(out.default_bytes(), s.default_bytes());
}
#[test]
fn closed_format_context_and_nonfinite_unit_source_refusals() {
    let s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true));
    assert!(preparation_requirements(&s, Rect::new(0, 0, 1, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat).is_err());
    let s = source(&[[0., 0., 0., 1.]]);
    let mut c = context();
    c.profile_identity = [0; 32];
    assert!(preparation_requirements(&s, Rect::new(0, 0, 1, 1), c, NumericPolicy::DeclaredBinary64HsvFinalFloat).is_err());
    c = context();
    c.encoding = SampleEncoding::Linear;
    assert!(preparation_requirements(&s, Rect::new(0, 0, 1, 1), c, NumericPolicy::DeclaredBinary64HsvFinalFloat).is_err());
    for x in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        let s = source(&[[x, 0., 0., 0.]]);
        assert!(
            PreparedStretchHsv::prepare(
                &s,
                Rect::new(0, 0, 1, 1),
                context(),
                NumericPolicy::DeclaredBinary64HsvFinalFloat,
                Limits::default(),
                &Interrupt::NONE
            )
            .is_err()
        );
    }
}
#[test]
fn one_pixel_native_tile_work_and_cumulative_no_reset_no_refund() {
    let s = source(&[[0.25, 0.25, 0.25, 0.], [0.5, 0.25, 0.25, 1.], [1., 0., 0., 1.]]);
    let p = prepare(&s);
    let roi = Rect::new(1, 0, 2, 1);
    let needs = p.application_requirements(roi, context(), &Interrupt::NONE).unwrap();
    assert!(needs.work_units >= 2 * 65536 * 16);
    let req = preparation_requirements(&s, Rect::new(0, 0, 3, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat).unwrap();
    let limits = Limits { resident_bytes: Limits::default().resident_bytes, work_units: req.work_units + needs.work_units };
    let p = PreparedStretchHsv::prepare(&s, Rect::new(0, 0, 3, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat, limits, &Interrupt::NONE).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 2;
    assert!(matches!(p.apply_with_limits(roi, context(), limits, &Interrupt::cancel_only(&cancelled)), Err(Error::Raster(RasterInputError::Cancelled))));
    assert_eq!(p.remaining_work(), 0);
    assert!(p.apply_with_limits(roi, context(), limits, &Interrupt::NONE).is_err());
}

#[test]
fn tiny_unit_colors_and_gray_black_degenerate_stats_remain_finite() {
    let tiny = f32::from_bits(1);
    let s = source(&[[tiny, 0., 0., 0.], [0., 0., tiny, 1.]]);
    let p =
        PreparedStretchHsv::prepare(&s, Rect::new(0, 0, 2, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat, Limits::default(), &Interrupt::NONE)
            .unwrap();
    assert_eq!(p.try_eval_float_color([tiny, 0., 0.]).unwrap(), [tiny, 0., 0.]);
    assert_eq!(p.try_eval_float_color([0., 0., tiny]).unwrap(), [0., 0., tiny]);
    let s = source(&[[0., 0., 0., 0.], [0., 0., 0., 1.]]);
    let p =
        PreparedStretchHsv::prepare(&s, Rect::new(0, 0, 2, 1), context(), NumericPolicy::DeclaredBinary64HsvFinalFloat, Limits::default(), &Interrupt::NONE)
            .unwrap();
    assert_eq!(p.extrema(), ([0., 0.], [0., 0.]));
    assert_eq!(p.try_eval_float_color([0., 0., 0.]).unwrap(), [0., 0., 0.]);
}
