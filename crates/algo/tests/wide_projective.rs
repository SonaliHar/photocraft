use photocraft_algo::{
    transform::{Homography, Interp, try_warp_surface},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn bilinear_keeps_adjacent_u32_codes_and_f64_low_bits_at_negative_origin() {
    let bounds = Rect::new(-2, -1, 0, 0);
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
        let raw: Vec<u8> = if sample == SampleType::U32 {
            [1u32, u32::MAX, 3, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            [0.5 + 2f64.powi(-40), 1.0, 0.5 + 3.0 * 2f64.powi(-40), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
        let before = source.clone();
        let h = Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]);
        let out = try_warp_surface(&source, bounds, &h, Interp::Bilinear).unwrap();
        let mut p = [0.; 2];
        out.try_read_pixel_f64(-1, -1, &mut p).unwrap();
        let expected = if sample == SampleType::U32 { 2.0 / f64::from(u32::MAX) } else { 0.5 + 2.0 * 2f64.powi(-40) };
        assert_eq!(p, [expected, 1.0]);
        out.try_read_pixel_f64(-2, -1, &mut p).unwrap();
        let half_alpha = if sample == SampleType::U32 { f64::from(0x80000000u32) / f64::from(u32::MAX) } else { 0.5 };
        assert_eq!(p[1], half_alpha, "transparent finite-ROI edge, rounded once at native depth");
        assert_eq!(source, before);
    }
}

#[test]
fn genuine_projective_inverse_matches_independent_bilinear_weights() {
    let bounds = Rect::new(0, 0, 2, 2);
    let source = Surface::try_from_interleaved(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
        bounds,
        &[0.0f64, 0.5, 0.25, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let out = try_warp_surface(&source, bounds, &Homography([1., 0., 0., 0., 1., 0., 0.1, 0., 1.]), Interp::Bilinear).unwrap();
    let mut p = [0.; 2];
    out.try_read_pixel_f64(0, 0, &mut p).unwrap();
    let f = 1.0 / 38.0;
    assert!((p[0] - (0.75 * f + 0.25 * f * f)).abs() < 1e-16);
    assert_eq!(p[1], 1.0);
    let placed = photocraft_algo::warp::try_place_source_projective_with_interp(
        &source,
        bounds,
        &Homography([1., 0., 0., 0., 1., 0., 0.1, 0., 1.]),
        None,
        Interp::Bilinear,
    )
    .unwrap();
    assert_eq!(placed, out);
}

#[test]
fn raw_integer_translation_preserves_exact_nonzero_default_bits() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
    let raw: Vec<u8> = [-0.0f64, 0.5 + 2f64.powi(-40)].into_iter().flat_map(f64::to_ne_bytes).collect();
    let source = Surface::try_with_default_bytes(format, &raw).unwrap();
    let out = try_warp_surface(&source, Rect::new(-3, -2, 0, 0), &Homography([1., 0., 7., 0., 1., -4., 0., 0., 1.]), Interp::Bilinear).unwrap();
    assert_eq!(out.default_bytes(), raw);
    assert_eq!(out.try_read_interleaved(Rect::new(4, -6, 5, -5)).unwrap(), raw);
    assert_eq!(source.default_bytes(), raw);
}

#[test]
fn bilinear_premultiplies_alpha_without_hidden_color_bleed() {
    let raw: Vec<u8> = [1.0f64, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    let source = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), Rect::new(0, 0, 2, 1), &raw).unwrap();
    let out = try_warp_surface(&source, Rect::new(0, 0, 2, 1), &Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]), Interp::Bilinear).unwrap();
    let mut pixel = [0.; 4];
    out.try_read_pixel_f64(1, 0, &mut pixel).unwrap();
    assert_eq!(pixel, [1., 0., 0., 0.5]);
    assert_eq!(source.try_read_interleaved(Rect::new(0, 0, 2, 1)).unwrap(), raw);
}

#[test]
fn nonunit_arithmetic_rejects_and_mid_operation_cancel_keeps_source() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let bounds = Rect::new(0, 0, 1, 1);
    let source = Surface::try_from_interleaved(format, bounds, &1.125f64.to_ne_bytes()).unwrap();
    let before = source.clone();
    let h = Homography([1., 0., 0.25, 0., 1., 0., 0., 0., 1.]);
    assert!(try_warp_surface(&source, bounds, &h, Interp::Bilinear).is_err());
    assert_eq!(source, before);
    let source = Surface::new(format);
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 4;
    assert!(
        photocraft_algo::transform::try_warp_surface_with_limits(
            &source,
            Rect::new(0, 0, 512, 512),
            &h,
            Interp::Bilinear,
            GeometryLimits::default(),
            &Interrupt::cancel_only(&cancelled)
        )
        .is_err()
    );
    assert!(counter.load(std::sync::atomic::Ordering::Relaxed) > 4);
    assert_eq!(source.tile_count(), 0);
}

#[test]
fn identity_keeps_hidden_signed_zero_and_unsupported_sampling_is_atomic() {
    let bounds = Rect::new(-1, -1, 0, 0);
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let raw: Vec<u8> = [-0.0f64, 0.5 + 2f64.powi(-40), 1.0, 0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
    let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    let out = try_warp_surface(&source, bounds, &Homography::IDENTITY, Interp::Bilinear).unwrap();
    assert_eq!(out.try_read_interleaved(bounds).unwrap(), raw);
    assert!(try_warp_surface(&source, bounds, &Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]), Interp::Bicubic).is_err());
}

#[test]
fn checked_wide_projective_cancel_and_budget_are_bounded() {
    let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
    let bounds = Rect::new(0, 0, 10, 10);
    let h = Homography([1., 0., 0.25, 0., 1., 0., 0., 0., 1.]);
    let result = photocraft_algo::transform::try_warp_surface_with_limits(
        &source,
        bounds,
        &h,
        Interp::Bilinear,
        GeometryLimits { resident_bytes: 0, work_units: 0 },
        &Interrupt::NONE,
    );
    assert!(result.is_err());
    assert!(
        photocraft_algo::transform::try_warp_surface_with_limits(
            &source,
            bounds,
            &h,
            Interp::Bilinear,
            GeometryLimits::default(),
            &Interrupt::cancel_only(&|| true)
        )
        .is_err()
    );
    assert_eq!(source.tile_count(), 0);
}

#[test]
fn explicit_cubic_retains_f64_alpha_overshoot_and_low_bit_raw_identity() {
    let bounds = Rect::new(0, 0, 8, 7);
    let source = Surface::try_with_default_bytes(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
        &[0.5 + 2f64.powi(-40), 1.].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let before = source.clone();
    let translated = photocraft_algo::transform::try_warp_cubic_surface_with_limits(
        &source,
        bounds,
        &Homography([1., 0., 0.5, 0., 1., 0.375, 0., 0., 1.]),
        GeometryLimits::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut pixel = [0.; 2];
    translated.try_read_pixel_f64(1, 1, &mut pixel).unwrap();
    assert!((pixel[0] - (0.5 + 2f64.powi(-40))).abs() < 1e-15);
    assert!(pixel[1] > 1., "negative cubic lobes yield a finite overshoot, not a silently clipped F64 alpha");
    let identity =
        photocraft_algo::transform::try_warp_cubic_surface_with_limits(&source, bounds, &Homography::IDENTITY, GeometryLimits::default(), &Interrupt::NONE)
            .unwrap();
    assert_eq!(identity.default_bytes(), source.default_bytes());
    assert_eq!(source, before);
}

#[test]
fn explicit_cubic_rejects_nonunit_input_and_bounded_work_before_publication() {
    use photocraft_algo::transform::try_warp_cubic_surface_with_limits;
    let bounds = Rect::new(0, 0, 4, 4);
    let source = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &1.2f64.to_ne_bytes()).unwrap();
    let before = source.clone();
    let h = Homography([1., 0., 0.25, 0., 1., 0.125, 0., 0., 1.]);
    assert_eq!(
        try_warp_cubic_surface_with_limits(&source, bounds, &h, Default::default(), &Interrupt::NONE),
        Err(photocraft_raster::RasterInputError::UnsupportedConversion)
    );
    let limits = GeometryLimits { work_units: 0, ..Default::default() };
    assert_eq!(try_warp_cubic_surface_with_limits(&source, bounds, &h, limits, &Interrupt::NONE), Err(photocraft_raster::RasterInputError::LimitExceeded));
    assert_eq!(
        try_warp_cubic_surface_with_limits(&source, bounds, &h, Default::default(), &Interrupt::cancel_only(&|| true)),
        Err(photocraft_raster::RasterInputError::Cancelled)
    );
    assert_eq!(source, before);
}

#[test]
fn explicit_cubic_quantizes_adjacent_u32_codes_once() {
    let bounds = Rect::new(-2, -1, 0, 0);
    let raw = [1u32, u32::MAX, 3, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, true), bounds, &raw).unwrap();
    let before = source.clone();
    let output = photocraft_algo::transform::try_warp_cubic_surface_with_limits(
        &source,
        bounds,
        &Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]),
        Default::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut pixel = [0.; 2];
    output.try_read_pixel_f64(-1, -1, &mut pixel).unwrap();
    assert_eq!(pixel[0], 2. / f64::from(u32::MAX));
    assert_eq!(source, before);
}

#[test]
fn fractional_nearest_copies_low_bits_hidden_hdr_and_signed_zero_exactly() {
    let bounds = Rect::new(-2, -1, 0, 0);
    let raw = [-0.0f64, 0.0, 1.0 + 2f64.powi(-40), 0.75].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true), bounds, &raw).unwrap();
    let out = try_warp_surface(&source, bounds, &Homography([1., 0., 0.25, 0., 1., 0.75, 0., 0., 1.]), Interp::Nearest).unwrap();
    assert_eq!(out.try_read_interleaved(Rect::new(-2, 0, 0, 1)).unwrap(), raw);
    assert_eq!(out.try_read_interleaved(Rect::new(-2, -1, 1, 0)).unwrap(), vec![0; 48]);
    assert_eq!(source.try_read_interleaved(bounds).unwrap(), raw);
}

#[test]
fn nearest_projective_default_noalpha_and_bounded_cancel_preserve_source() {
    use photocraft_algo::transform::try_warp_surface_with_limits;
    use photocraft_raster::RasterInputError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let source = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false), &1u32.to_ne_bytes()).unwrap();
    let bounds = Rect::new(-2, -1, 1, 1);
    let h = Homography([1., 0., 0., 0., 1., 0., 0.1, 0., 1.]);
    let out = try_warp_surface(&source, bounds, &h, Interp::Nearest).unwrap();
    let mut p = [0; 8];
    out.try_read_pixel_bytes(0, 0, &mut p).unwrap();
    assert_eq!(p, [1u32, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>().as_slice());
    let limit = GeometryLimits { work_units: 0, ..Default::default() };
    assert_eq!(try_warp_surface_with_limits(&source, bounds, &h, Interp::Nearest, limit, &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 4;
    let result = try_warp_surface_with_limits(
        &source,
        Rect::new(-10, -10, 1024, 1024),
        &Homography([1., 0., 0.25, 0., 1., 0.75, 0., 0., 1.]),
        Interp::Nearest,
        Default::default(),
        &Interrupt::cancel_only(&cancel),
    );
    assert_eq!(result, Err(RasterInputError::Cancelled));
    assert_eq!(source.tile_count(), 0);
    assert_eq!(source.default_bytes(), 1u32.to_ne_bytes());
}

#[test]
fn cubic_signed_alpha_preserves_straight_color_without_clamping() {
    let bounds = Rect::new(0, 0, 4, 4);
    let raw = [0.5f64, 1.].into_iter().cycle().take(32).flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
    let source = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true), bounds, &raw).unwrap();
    let h = Homography([0.8660254037844386, -0.5, 0., 0.5, 0.8660254037844386, 0., 0., 0., 1.]);
    let out = photocraft_algo::transform::try_warp_cubic_surface_with_limits(&source, bounds, &h, Default::default(), &Interrupt::NONE).unwrap();
    let area = photocraft_algo::transform::try_warp_bounds(bounds, &h).unwrap();
    let mut negative = 0;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let mut p = [0.; 2];
            out.try_read_pixel_f64(x, y, &mut p).unwrap();
            if p[1] < 0. {
                negative += 1;
                assert!((p[0] - 0.5).abs() < 1e-15);
            }
        }
    }
    assert!(negative > 0);
    assert_eq!(source.try_read_interleaved(bounds).unwrap(), raw);
}
