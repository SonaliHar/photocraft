use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_young, try_gaussian_young_recursive};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn direct_recurrence_matches_impulse_oracle_hdr_alpha_abyss_edges_and_tiles() {
    let mut worst = 0.0_f64;
    for extent in [Rect::new(-2, -3, 7, 4), Rect::new(250, -2, 266, 3), Rect::new(0, 0, 1, 1)] {
        let mut source = Surface::new(PixelFormat::RGBA64F);
        for y in extent.y0..extent.y1 {
            for x in extent.x0..extent.x1 {
                let n = f64::from((x * 7 + y * 13).rem_euclid(31));
                source.try_write_pixel_f64(x, y, &[n / 5.0 - 1.0, 0.25, n / 31.0, 0.1 + n / 40.0]).unwrap();
            }
        }
        let before = source.try_read_interleaved(extent).unwrap();
        for (sx, sy) in [(0.0, 0.5), (0.5, 1.5), (2.25, 1.5), (3.75, 1.5), (8.0, 2.0)] {
            for abyss in [GaussianAbyss::Clamp, GaussianAbyss::None] {
                let params = GaussianParams { std_dev_x: sx, std_dev_y: sy, abyss, clip_extent: false };
                let roi = Rect::new(extent.x0 - 1, extent.y0 - 1, extent.x1 + 1, extent.y1 + 1);
                let a =
                    try_gaussian_young(&source, roi, extent, params, GaussianLimits::default(), &Interrupt::NONE).unwrap().try_read_region_f64(roi).unwrap();
                let b = try_gaussian_young_recursive(&source, roi, extent, params, GaussianLimits::default(), &Interrupt::NONE)
                    .unwrap()
                    .try_read_region_f64(roi)
                    .unwrap();
                let max = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).fold(0.0_f64, f64::max);
                worst = worst.max(max);
                assert!(max < 1e-9, "{extent:?}, sigma{sx},{sy}, abyss{abyss:?}: maxf64={max}");
            }
        }
        assert_eq!(source.try_read_interleaved(extent).unwrap(), before);
    }
    println!("direct recurrence vs finite impulse convolution: max f64 difference {worst}");
}

#[test]
fn recursive_limits_cancel_raw_identity_and_untouched_region() {
    let extent = Rect::new(0, 0, 32, 32);
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[1.0, 2.0, 3.0, 0.25]).unwrap();
    source.try_write_pixel_f64(100, 100, &[f64::from_bits(0x7ff8000000000042), -0.0, 1.0, -0.0]).unwrap();
    let outside = Rect::new(100, 100, 101, 101);
    let original = source.try_read_interleaved(outside).unwrap();
    let params = GaussianParams { std_dev_x: 2.25, std_dev_y: 1.5, abyss: GaussianAbyss::Clamp, clip_extent: true };
    let result = try_gaussian_young_recursive(&source, extent, extent, params, GaussianLimits::default(), &Interrupt::NONE).unwrap();
    assert_eq!(result.try_read_interleaved(outside).unwrap(), original);
    let identity = GaussianParams { std_dev_x: 0.0, std_dev_y: 0.0, ..params };
    assert_eq!(
        try_gaussian_young_recursive(&source, outside, outside, identity, GaussianLimits::default(), &Interrupt::NONE)
            .unwrap()
            .try_read_interleaved(outside)
            .unwrap(),
        original
    );
    let limits = GaussianLimits { work_units: 1, resident_bytes: 1 };
    assert_eq!(try_gaussian_young_recursive(&source, extent, extent, params, limits, &Interrupt::NONE).unwrap_err(), RasterInputError::LimitExceeded);
    let count = AtomicUsize::new(0);
    let cancel = || count.fetch_add(1, Ordering::Relaxed) > 12;
    assert_eq!(
        try_gaussian_young_recursive(&source, extent, extent, params, GaussianLimits::default(), &Interrupt::cancel_only(&cancel)).unwrap_err(),
        RasterInputError::Cancelled
    );
    assert_eq!(source.try_read_interleaved(outside).unwrap(), original);
}
