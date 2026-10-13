use photocraft_algo::{
    resample::Resample,
    wide_geometry::GeometryLimits,
    wide_resample::{ResizeOptions, try_resize},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

fn options(filter: Resample, sx: f64, sy: f64) -> ResizeOptions {
    ResizeOptions { scale_x: sx, scale_y: sy, filter, clamp_edges: true }
}

#[test]
fn nearest_and_identity_keep_u32_nan_zero_and_default_bits() {
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let raw = if sample == SampleType::U32 {
            [16_777_216_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat()
        } else {
            [(-0.0_f64).to_ne_bytes(), 0x7ff8123456789abc_u64.to_ne_bytes()].concat()
        };
        let bounds = Rect::new(-2, 0, 0, 1);
        let mut source = Surface::try_with_default_bytes(format, &raw[..sample.bytes()]).unwrap();
        source.try_write_interleaved(bounds, &raw).unwrap();
        let history = source.clone();
        let (out, area) = try_resize(&source, bounds, options(Resample::Nearest, 2.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(area, Rect::new(-4, 0, 0, 1));
        let expected = [&raw[..sample.bytes()], &raw[..sample.bytes()], &raw[sample.bytes()..], &raw[sample.bytes()..]].concat();
        assert_eq!(out.try_read_interleaved(area).unwrap(), expected);
        assert_eq!(out.default_bytes(), source.default_bytes());
        for filter in [Resample::Nearest, Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
            let (out, area) = try_resize(&source, bounds, options(filter, 1.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(out.try_read_interleaved(area).unwrap(), raw);
        }
        assert_eq!(history, source);
    }
}

#[test]
fn bilinear_hdr_premultiplied_alpha_keeps_sub_f32_detail_and_tiny_alpha() {
    let bounds = Rect::new(0, 0, 2, 1);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    let detail = 2.0_f64.powi(-40);
    source.try_write_pixel_f64(0, 0, &[1.0 + detail, 4.0, -2.0, 0.25]).unwrap();
    source.try_write_pixel_f64(1, 0, &[3.0 + detail, -2.0, 8.0, 0.75]).unwrap();
    let history = source.clone();
    let (out, area) = try_resize(&source, bounds, options(Resample::Bilinear, 2.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
    let values = out.try_read_region_f64(area).unwrap();
    let expected = [1.0 + detail, 4.0, -2.0, 0.25, 2.0 + detail, 1.0, 3.0, 0.375, 2.8 + detail, -1.4, 7.0, 0.625, 3.0 + detail, -2.0, 8.0, 0.75];
    for (actual, want) in values.iter().zip(expected) {
        assert!((actual - want).abs() < 1e-14, "{actual} != {want}");
    }
    assert_eq!(source, history);
    source.try_write_pixel_f64(0, 0, &[1.0 + detail, 2.0, 3.0, 1e-15]).unwrap();
    source.try_write_pixel_f64(1, 0, &[1.0 + detail, 2.0, 3.0, 1e-15]).unwrap();
    let (out, area) = try_resize(&source, bounds, options(Resample::Bilinear, 2.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
    for pixel in out.try_read_region_f64(area).unwrap().as_chunks::<4>().0.iter() {
        assert!((pixel[0] - (1.0 + detail)).abs() < 1e-14);
        assert!((pixel[3] - 1e-15).abs() < 1e-29);
    }
}

#[test]
fn downsample_stretched_support_and_all_shared_filters_keep_f64_constants() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let bounds = Rect::new(0, 0, 4, 1);
    let source = Surface::try_from_interleaved(format, bounds, &[0.0_f64, 1.0, 2.0, 3.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    let (out, area) = try_resize(&source, bounds, options(Resample::Bilinear, 0.5, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(out.try_read_region_f64(area).unwrap(), [0.625, 2.375]);
    let value = 1.0 + 2.0_f64.powi(-40);
    let source = Surface::try_with_default_bytes(format, &value.to_ne_bytes()).unwrap();
    for filter in [Resample::Bilinear, Resample::Bicubic, Resample::Lanczos] {
        let (out, area) = try_resize(&source, bounds, options(filter, 1.75, 2.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert!(out.try_read_region_f64(area).unwrap().iter().all(|v| (*v - value).abs() < 1e-14));
        assert_eq!(out.default_bytes(), source.default_bytes());
    }
}

#[test]
fn resized_default_abyss_empty_bounds_and_explicit_unsupported_sharpening() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut source = Surface::try_with_default_bytes(format, &2.0_f64.to_ne_bytes()).unwrap();
    let bounds = Rect::new(0, 0, 1, 1);
    source.try_write_pixel_f64(0, 0, &[4.0]).unwrap();
    let mut params = options(Resample::Bilinear, 2.0, 1.0);
    params.clamp_edges = false;
    let (out, area) = try_resize(&source, bounds, params, GeometryLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(out.try_read_region_f64(area).unwrap(), [3.5, 3.5]);
    assert_eq!(out.default_bytes(), source.default_bytes());
    let (_, area) = try_resize(&source, Rect::new(3, 0, 3, 1), options(Resample::Lanczos, 0.5, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(area, Rect::EMPTY);
    assert_eq!(
        try_resize(&source, bounds, options(Resample::PreserveDetails, 2.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::UnsupportedConversion
    );
}

#[test]
fn checked_resampling_limits_cancellation_and_nonfinite_are_transactional() {
    let source = Surface::new(PixelFormat::RGBA64F);
    let bounds = Rect::new(0, 0, 70, 40);
    let history = source.clone();
    let params = options(Resample::Bicubic, 2.0, 1.0);
    let count = AtomicUsize::new(0);
    let poll = || count.fetch_add(1, Ordering::Relaxed) > 4;
    let interrupt = Interrupt::cancel_only(&poll);
    assert_eq!(try_resize(&source, bounds, params, GeometryLimits::default(), &interrupt).unwrap_err(), RasterInputError::Cancelled);
    assert_eq!(source, history);
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e30] {
        assert!(try_resize(&source, bounds, options(Resample::Bilinear, scale, 1.0), GeometryLimits::default(), &Interrupt::NONE).is_err());
    }
    for limits in [GeometryLimits { resident_bytes: 1, work_units: u64::MAX }, GeometryLimits { resident_bytes: u64::MAX, work_units: 1 }] {
        assert_eq!(try_resize(&source, bounds, params, limits, &Interrupt::NONE).unwrap_err(), RasterInputError::LimitExceeded);
    }
    assert_eq!(
        try_resize(&source, bounds, options(Resample::Lanczos, 1e-5, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::LimitExceeded
    );
    let mut bad = source.clone();
    bad.try_write_pixel_f64(0, 0, &[f64::NAN, 0.0, 0.0, 1.0]).unwrap();
    let before = bad.clone();
    assert_eq!(
        try_resize(&bad, Rect::new(0, 0, 1, 1), options(Resample::Bilinear, 2.0, 1.0), GeometryLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::InvalidSample
    );
    assert_eq!(bad, before);
}
