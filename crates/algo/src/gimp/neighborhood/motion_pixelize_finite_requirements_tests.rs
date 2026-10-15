//! UNRUN planner/legacy boundary drafts. Surface buffers are external harness
//! allocations; a prospective report is not an aggregate reservation receipt.
use super::super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};

fn source(sample: SampleType) -> Surface {
    let mut value = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
    for y in 0..2 {
        for x in 0..3 {
            value.try_write_pixel_f64(x, y, &[0.25, 0.75, 0.5, 0.5]).unwrap();
        }
    }
    value.try_write_pixel_f64(300, 300, &[-0.0, 0.123456789, 0.75, 0.0]).unwrap();
    value
}
fn limits(q: NeighborhoodRequirements) -> GaussianLimits {
    GaussianLimits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units }
}
fn gaussian() -> GaussianParams {
    GaussianParams { std_dev_x: f64::from(0.8f32), std_dev_y: f64::from(1.3f32), abyss: GaussianAbyss::Clamp, clip_extent: true }
}
#[test]
fn motion_report_drives_exact_limits_and_preserves_original_outside_roi() {
    let roi = Rect::new(0, 0, 3, 2);
    let params = MotionBlurParams { length: 2.5, angle_degrees: -35.0 };
    for sample in [SampleType::U16, SampleType::F64] {
        let src = source(sample);
        let before = src.clone();
        let q = motion_blur_requirements(&src, roi, roi, params, &Interrupt::NONE).unwrap();
        let result = try_motion_blur_linear(&src, roi, roi, params, limits(q), &Interrupt::NONE).unwrap();
        let generous = try_motion_blur_linear(&src, roi, roi, params, GaussianLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(result, generous);
        assert_eq!(src, before);
        assert_eq!(result.default_bytes(), src.default_bytes());
        assert_eq!(result.try_read_interleaved(Rect::new(300, 300, 301, 301)).unwrap(), src.try_read_interleaved(Rect::new(300, 300, 301, 301)).unwrap());
        for low in [
            GaussianLimits { resident_bytes: q.peak_resident_bytes - 1, work_units: q.work_units },
            GaussianLimits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units - 1 },
        ] {
            assert_eq!(try_motion_blur_linear(&src, roi, roi, params, low, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
        }
    }
}
#[test]
fn pixelize_report_has_exact_admission_and_empty_helper_prices_legacy_clone() {
    let src = source(SampleType::F64);
    let before = src.clone();
    let roi = Rect::new(0, 0, 3, 2);
    let params = PixelizeParams { size_x: 2, size_y: 2, offset_x: -1, offset_y: -1 };
    let q = pixelize_requirements(&src, roi, roi, params, &Interrupt::NONE).unwrap();
    let result = try_pixelize_square(&src, roi, roi, params, limits(q), &Interrupt::NONE).unwrap();
    assert_eq!(result, try_pixelize_square(&src, roi, roi, params, GaussianLimits::default(), &Interrupt::NONE).unwrap());
    assert_eq!(
        try_pixelize_square(&src, roi, roi, params, GaussianLimits { resident_bytes: q.peak_resident_bytes - 1, work_units: q.work_units }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(
        try_pixelize_square(&src, roi, roi, params, GaussianLimits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units - 1 }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    );
    let empty = Rect::new(0, 0, 0, 0);
    let clone = pixelize_requirements(&src, empty, roi, params, &Interrupt::NONE).unwrap();
    assert!(clone.peak_resident_bytes >= src.resident_payload_bytes().unwrap());
    assert!(clone.work_units > 0);
    assert_eq!(try_pixelize_square(&src, empty, roi, params, GaussianLimits { resident_bytes: 0, work_units: 0 }, &Interrupt::NONE).unwrap(), src);
    assert_eq!(src, before);
}
#[test]
fn finite_report_keeps_fir_iir_auto_and_noop_limit_policy() {
    let roi = Rect::new(0, 0, 3, 2);
    let src = source(SampleType::F64);
    let before = src.clone();
    for filter in [GaussianFilter::Auto, GaussianFilter::Fir, GaussianFilter::Iir] {
        let q = gaussian_finite_requirements(&src, roi, roi, gaussian(), filter, &Interrupt::NONE).unwrap();
        let result = try_gaussian_finite_clamp(&src, roi, roi, gaussian(), filter, limits(q), &Interrupt::NONE).unwrap();
        assert_eq!(result, try_gaussian_finite_clamp(&src, roi, roi, gaussian(), filter, GaussianLimits::default(), &Interrupt::NONE).unwrap());
        assert_eq!(
            try_gaussian_finite_clamp(
                &src,
                roi,
                roi,
                gaussian(),
                filter,
                GaussianLimits { resident_bytes: q.peak_resident_bytes - 1, work_units: q.work_units },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(
            try_gaussian_finite_clamp(
                &src,
                roi,
                roi,
                gaussian(),
                filter,
                GaussianLimits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units - 1 },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        );
    }
    let zero = GaussianParams { std_dev_x: 0.0, std_dev_y: 0.0, ..gaussian() };
    for (output, params) in [(roi, zero), (Rect::new(20, 20, 21, 21), gaussian())] {
        let q = gaussian_finite_requirements(&src, output, roi, params, GaussianFilter::Auto, &Interrupt::NONE).unwrap();
        assert!(q.peak_resident_bytes >= src.resident_payload_bytes().unwrap());
        assert!(q.work_units > 0);
        assert_eq!(
            try_gaussian_finite_clamp(&src, output, roi, params, GaussianFilter::Auto, GaussianLimits { resident_bytes: 0, work_units: 0 }, &Interrupt::NONE)
                .unwrap(),
            src
        );
    }
    assert_eq!(src, before);
}
#[test]
fn planner_refusals_and_cancel_leave_source_exact_without_running_kernel() {
    let src = source(SampleType::F64);
    let before = src.clone();
    let roi = Rect::new(0, 0, 3, 2);
    let motion = MotionBlurParams { length: 2., angle_degrees: 0. };
    let pixel = PixelizeParams { size_x: 2, size_y: 2, offset_x: 0, offset_y: 0 };
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert_eq!(motion_blur_requirements(&src, roi, roi, motion, &interrupt), Err(RasterInputError::Cancelled));
    assert_eq!(pixelize_requirements(&src, roi, roi, pixel, &interrupt), Err(RasterInputError::Cancelled));
    assert_eq!(gaussian_finite_requirements(&src, roi, roi, gaussian(), GaussianFilter::Auto, &interrupt), Err(RasterInputError::Cancelled));
    assert_eq!(
        motion_blur_requirements(&src, roi, roi, MotionBlurParams { length: f64::NAN, ..motion }, &Interrupt::NONE),
        Err(RasterInputError::InvalidSample)
    );
    assert_eq!(pixelize_requirements(&src, roi, roi, PixelizeParams { size_x: 0, ..pixel }, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
    assert_eq!(
        gaussian_finite_requirements(&src, roi, roi, GaussianParams { std_dev_x: -1., ..gaussian() }, GaussianFilter::Auto, &Interrupt::NONE),
        Err(RasterInputError::UnsupportedConversion)
    );
    assert_eq!(src, before);
}
