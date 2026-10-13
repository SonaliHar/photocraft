use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_fir, try_gaussian_young};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

fn params() -> GaussianParams {
    GaussianParams { std_dev_x: 2.25, std_dev_y: 1.5, abyss: GaussianAbyss::Clamp, clip_extent: true }
}

#[test]
fn small_sigma_f32_negative_roi_matches_independent_two_dimensional_premultiplied_sum() {
    let extent = Rect::new(-2, -1, 1, 1);
    let roi = Rect::new(-1, -1, 0, 1);
    let pixels =
        [[-0.0_f32, 1.25, -0.5, 0.25], [0.3, 0.4, 0.5, 0.75], [99.0, 88.0, 77.0, 0.0], [2.0, -0.25, 0.0, 1.0], [0.2, 0.6, 0.8, 0.5], [0.4, 0.1, 0.7, 0.125]];
    let raw = pixels.into_iter().flatten().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(PixelFormat::RGBA32F, extent, &raw).unwrap();
    let parameters = GaussianParams { std_dev_x: f64::from(0.8_f32), std_dev_y: f64::from(1.3_f32), ..params() };
    let result = try_gaussian_fir(&source, roi, extent, parameters, Default::default(), &Interrupt::NONE).unwrap();
    let rx = (4.0 * parameters.std_dev_x).ceil() as i32;
    let ry = (4.0 * parameters.std_dev_y).ceil() as i32;
    let weights = |radius: i32, sigma: f64| (-radius..=radius).map(|distance| (-0.5 * (f64::from(distance) / sigma).powi(2)).exp()).collect::<Vec<_>>();
    let wx = weights(rx, parameters.std_dev_x);
    let wy = weights(ry, parameters.std_dev_y);
    let total = wx.iter().sum::<f64>() * wy.iter().sum::<f64>();
    for y in roi.y0..roi.y1 {
        let mut expected = [0.0; 4];
        for (j, weight_y) in wy.iter().enumerate() {
            for (i, weight_x) in wx.iter().enumerate() {
                let x = (-1 + i as i32 - rx).clamp(extent.x0, extent.x1 - 1);
                let y = (y + j as i32 - ry).clamp(extent.y0, extent.y1 - 1);
                let pixel = pixels[((y - extent.y0) * 3 + x - extent.x0) as usize].map(f64::from);
                let weight = weight_x * weight_y / total;
                for channel in 0..3 {
                    expected[channel] += pixel[channel] * pixel[3] * weight;
                }
                expected[3] += pixel[3] * weight;
            }
        }
        for channel in 0..3 {
            expected[channel] /= expected[3];
        }
        let mut actual = [0.0; 4];
        result.try_read_pixel_f64(-1, y, &mut actual).unwrap();
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 6e-8, "final F32 rounding only: actual{actual}, expected{expected}");
        }
    }
    assert_eq!(source.try_read_interleaved(extent).unwrap(), raw);
    for x in [-2, 0] {
        let outside = Rect::new(x, -1, x + 1, 1);
        assert_eq!(result.try_read_interleaved(outside).unwrap(), source.try_read_interleaved(outside).unwrap());
    }
    assert_eq!(result.default_bytes(), source.default_bytes());
}

#[test]
fn gaussian_anisotropic_impulse_matches_independent_normal_weights() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let extent = Rect::new(-20, -20, 21, 21);
    let mut source = Surface::new(format);
    source.try_write_pixel_f64(0, 0, &[1.0]).unwrap();
    let result = try_gaussian_fir(&source, extent, extent, params(), GaussianLimits::default(), &Interrupt::NONE).unwrap();
    let sx = (-9..=9).map(|x| (-0.5 * (f64::from(x) / 2.25).powi(2)).exp()).sum::<f64>();
    let sy = (-6..=6).map(|y| (-0.5 * (f64::from(y) / 1.5).powi(2)).exp()).sum::<f64>();
    let mut value = [0.0];
    for (x, y) in [(0, 0), (3, 2), (-7, 5)] {
        result.try_read_pixel_f64(x, y, &mut value).unwrap();
        let expected = (-0.5 * (f64::from(x) / 2.25).powi(2)).exp() / sx * (-0.5 * (f64::from(y) / 1.5).powi(2)).exp() / sy;
        assert!((value[0] - expected).abs() < 1e-17);
    }
}

#[test]
fn gaussian_alpha_is_premultiplied_and_outside_roi_bits_remain_exact() {
    let extent = Rect::new(0, 0, 3, 1);
    let source_values = [4.0_f64, 0.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.5];
    let raw: Vec<u8> = source_values.into_iter().flat_map(f64::to_ne_bytes).collect();
    let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, extent, &raw).unwrap();
    let roi = Rect::new(1, 0, 2, 1);
    let result = try_gaussian_fir(&source, roi, extent, params(), GaussianLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(result.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw[..32]);
    let mut value = [0.0; 4];
    result.try_read_pixel_f64(1, 0, &mut value).unwrap();
    assert_eq!(value[1], 0.0, "fully transparent green must not bleed into the blur");
    assert!(value[0] > 1.9 && value[0] < 2.1);
    assert!(value[3] > 0.0 && value[3] < 1.0);
    assert_eq!(source.try_read_interleaved(extent).unwrap(), raw);
}

#[test]
fn gaussian_limits_invalid_inputs_and_internal_cancellation_are_checked() {
    let extent = Rect::new(0, 0, 16, 16);
    let source = Surface::new(PixelFormat::RGBA64F);
    assert_eq!(
        try_gaussian_fir(&source, extent, extent, params(), GaussianLimits { resident_bytes: 1024, work_units: 1 }, &Interrupt::NONE).unwrap_err(),
        RasterInputError::LimitExceeded
    );
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 4;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert_eq!(try_gaussian_fir(&source, extent, extent, params(), GaussianLimits::default(), &interrupt).unwrap_err(), RasterInputError::Cancelled);
    let invalid = GaussianParams { std_dev_x: f64::NAN, ..params() };
    assert_eq!(try_gaussian_fir(&source, extent, extent, invalid, GaussianLimits::default(), &Interrupt::NONE).unwrap_err(), RasterInputError::InvalidSample);
}

#[test]
fn zero_sigma_is_raw_identity_and_operation_clip_retains_outside_bytes() {
    let extent = Rect::new(0, 0, 2, 1);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let raw = [0x8000_0000_0000_0000_u64.to_ne_bytes(), 0x7ff8_1234_5678_9abc_u64.to_ne_bytes()].concat();
    let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
    let identity = GaussianParams { std_dev_x: 0.0, std_dev_y: 0.0, ..params() };
    let result = try_gaussian_fir(&source, extent, extent, identity, GaussianLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(result.try_read_interleaved(extent).unwrap(), raw);
    let outside = Rect::new(20, 20, 21, 21);
    let clipped = try_gaussian_fir(&source, outside, extent, params(), GaussianLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(clipped, source);
}

#[test]
fn recursive_gaussian_has_same_premultiplied_hdr_roi_and_cancel_guarantees() {
    let extent = Rect::new(0, 0, 3, 1);
    let values = [4.0_f64, 0.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.5];
    let raw: Vec<u8> = values.into_iter().flat_map(f64::to_ne_bytes).collect();
    let source = Surface::try_from_interleaved(PixelFormat::RGBA64F, extent, &raw).unwrap();
    let roi = Rect::new(1, 0, 2, 1);
    let result = try_gaussian_young(&source, roi, extent, params(), GaussianLimits::default(), &Interrupt::NONE).unwrap();
    let mut output = [0.0; 4];
    result.try_read_pixel_f64(1, 0, &mut output).unwrap();
    assert_eq!(output[1], 0.0);
    assert!((output[0] - 2.0).abs() < 1e-12);
    assert_eq!(result.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw[..32]);
    let cancel = Interrupt::cancel_only(&|| true);
    assert_eq!(try_gaussian_young(&source, roi, extent, params(), GaussianLimits::default(), &cancel).unwrap_err(), RasterInputError::Cancelled);
    let unqualified = GaussianParams { std_dev_x: 0.25, ..params() };
    assert_eq!(
        try_gaussian_young(&source, roi, extent, unqualified, GaussianLimits::default(), &Interrupt::NONE).unwrap_err(),
        RasterInputError::UnsupportedConversion
    );
    assert_eq!(source.try_read_interleaved(extent).unwrap(), raw);
}

#[test]
fn recursive_gaussian_cancels_during_sample_work_after_kernel_creation() {
    let extent = Rect::new(0, 0, 32, 32);
    let source = Surface::new(PixelFormat::RGBA64F);
    let history = source.clone();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) > 20;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert_eq!(try_gaussian_young(&source, extent, extent, params(), GaussianLimits::default(), &interrupt).unwrap_err(), RasterInputError::Cancelled);
    assert!(calls.load(Ordering::Relaxed) > 20);
    assert_eq!(source, history);
}
