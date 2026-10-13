use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianParams, try_gaussian_small_auto};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

fn parameters() -> GaussianParams {
    GaussianParams { std_dev_x: f64::from(0.8_f32), std_dev_y: f64::from(1.3_f32), abyss: GaussianAbyss::Clamp, clip_extent: true }
}

#[test]
fn finite_constant_hdr_roi_keeps_source_and_unwritten_raw_bits() {
    let extent = Rect::new(-3, -2, 5, 9);
    let roi = Rect::new(-1, 0, 4, 5);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    for y in extent.y0..extent.y1 {
        for x in extent.x0..extent.x1 {
            source.try_write_pixel_f64(x, y, &[1.25, -0.75, 0.5, 0.625]).unwrap();
        }
    }
    source.try_write_pixel_f64(100, 100, &[-0., f64::from_bits(0x3fe0000000000001), 2., 0.]).unwrap();
    let before = source.clone();
    let result = try_gaussian_small_auto(&source, roi, extent, parameters(), Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(source, before);
    for y in roi.y0..roi.y1 {
        for x in roi.x0..roi.x1 {
            let mut value = [0.; 4];
            result.try_read_pixel_f64(x, y, &mut value).unwrap();
            for (a, b) in value.into_iter().zip([1.25, -0.75, 0.5, 0.625]) {
                assert!((a - b).abs() < 1e-12);
            }
        }
    }
    assert_eq!(result.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap(), source.try_read_interleaved(Rect::new(100, 100, 101, 101)).unwrap());
    assert_eq!(result.try_read_interleaved(Rect::new(-3, -2, -1, 0)).unwrap(), source.try_read_interleaved(Rect::new(-3, -2, -1, 0)).unwrap());
    assert_eq!(result.default_bytes(), source.default_bytes());
}

#[test]
fn finite_scope_budget_and_cancellation_fail_atomically() {
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[1., 0.5, 0.25, 1.]).unwrap();
    let before = source.clone();
    let extent = Rect::new(0, 0, 3, 9);
    let cancelled = Interrupt::new(&|| true, &|_| {});
    assert_eq!(try_gaussian_small_auto(&source, extent, extent, parameters(), Default::default(), &cancelled), Err(RasterInputError::Cancelled));
    assert_eq!(
        try_gaussian_small_auto(&source, extent, Rect::new(0, 0, 257, 9), parameters(), Default::default(), &Interrupt::NONE),
        Err(RasterInputError::UnsupportedConversion)
    );
    assert_eq!(
        try_gaussian_small_auto(&source, extent, extent, GaussianParams { std_dev_y: 1.5, ..parameters() }, Default::default(), &Interrupt::NONE),
        Err(RasterInputError::UnsupportedConversion)
    );
    assert_eq!(
        try_gaussian_small_auto(
            &source,
            extent,
            extent,
            parameters(),
            photocraft_algo::gimp::neighborhood::GaussianLimits { resident_bytes: 1, work_units: 1 },
            &Interrupt::NONE
        ),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(source, before);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let check = || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2;
    let middle = Interrupt::new(&check, &|_| {});
    let large = Rect::new(0, 0, 80, 80);
    assert_eq!(try_gaussian_small_auto(&source, large, large, parameters(), Default::default(), &middle), Err(RasterInputError::Cancelled));
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    assert_eq!(source, before);
}
