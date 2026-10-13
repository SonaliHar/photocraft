use photocraft_algo::transform::{Homography, try_warp_bilinear_hidden_surface_with_limits};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
#[test]
fn zero_alpha_linear_keeps_weighted_hidden_color_at_transparent_edge() {
    let source = Surface::try_from_interleaved(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
        Rect::new(0, 0, 2, 1),
        &[0.2f64, 0., 0.6, 0.].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let before = source.clone();
    let out = try_warp_bilinear_hidden_surface_with_limits(
        &source,
        Rect::new(0, 0, 2, 1),
        &Homography([1., 0., 0.25, 0., 1., 0.75, 0., 0., 1.]),
        Default::default(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut p = [0.; 2];
    out.try_read_pixel_f64(0, 0, &mut p).unwrap();
    assert_eq!(p, [0.2 * 0.75 * 0.25, 0.]);
    assert_eq!(source, before);
}

#[test]
fn hidden_linear_is_distinct_from_ordinary_bilinear_and_failures_are_atomic() {
    use photocraft_algo::{
        transform::{Interp, try_warp_surface_with_limits},
        wide_geometry::GeometryLimits,
    };
    use photocraft_raster::RasterInputError;
    let b = Rect::new(-2, -1, 0, 0);
    let source = Surface::try_from_interleaved(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
        b,
        &[0.2f64, 0., 0.6, 0.].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let before = source.clone();
    let h = Homography([1., 0., 0.25, 0., 1., 0.75, 0., 0., 1.]);
    let ordinary = try_warp_surface_with_limits(&source, b, &h, Interp::Bilinear, Default::default(), &Interrupt::NONE).unwrap();
    let mut p = [0.; 2];
    ordinary.try_read_pixel_f64(-2, -1, &mut p).unwrap();
    assert_eq!(p, [0., 0.]);
    assert!(matches!(
        try_warp_bilinear_hidden_surface_with_limits(&source, b, &h, GeometryLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
        Err(RasterInputError::LimitExceeded)
    ));
    assert!(matches!(
        try_warp_bilinear_hidden_surface_with_limits(&source, b, &h, Default::default(), &Interrupt::cancel_only(&|| true)),
        Err(RasterInputError::Cancelled)
    ));
    assert_eq!(source, before);
}

#[test]
fn associated_alpha_floor_applies_uniformly_below_at_and_above_boundary() {
    let floor = 2f64.powi(-16);
    for alpha in [0., 2f64.powi(-20), f64::from(f32::from_bits(0x377fffff)), floor, f64::from(f32::from_bits(0x37800001)), floor * 2., 0.25] {
        let b = Rect::new(0, 0, 1, 1);
        let source = Surface::try_from_interleaved(
            PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
            b,
            &[0.6f64, alpha].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
        )
        .unwrap();
        let before = source.clone();
        let output = try_warp_bilinear_hidden_surface_with_limits(
            &source,
            b,
            &Homography([1., 0., 0.25, 0., 1., 0.75, 0., 0., 1.]),
            Default::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let mut p = [0.; 2];
        output.try_read_pixel_f64(0, 0, &mut p).unwrap();
        let expected_alpha = alpha * 0.1875;
        let expected_color = 0.6 * alpha.max(floor) * 0.1875 / expected_alpha.max(floor);
        assert!((p[0] - expected_color).abs() < 1e-15, "alpha {alpha}: {p:?}");
        assert_eq!(p[1].to_bits(), expected_alpha.to_bits());
        assert_eq!(source, before);
    }
}

#[test]
fn association_floor_does_not_clamp_its_mathematically_generated_color() {
    let b = Rect::new(0, 0, 2, 1);
    let source = Surface::try_from_interleaved(
        PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true),
        b,
        &[1f64, 0., 1., 1.].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
    )
    .unwrap();
    let out =
        try_warp_bilinear_hidden_surface_with_limits(&source, b, &Homography([1., 0., 0.5, 0., 1., 0., 0., 0., 1.]), Default::default(), &Interrupt::NONE)
            .unwrap();
    let mut p = [0.; 2];
    out.try_read_pixel_f64(1, 0, &mut p).unwrap();
    assert_eq!(p, [1. + 2f64.powi(-16), 0.5]);
}
