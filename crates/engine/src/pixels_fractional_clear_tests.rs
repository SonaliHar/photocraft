//! External harness Surfaces and references do not assert document quota receipts.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_raster::Interrupt;
fn raw(f: PixelFormat, alpha: f64) -> Vec<u8> {
    let mut v = vec![0; f.bytes_per_pixel()];
    for i in 0..f.mode.color_channels() {
        try_write_sample_f64(&mut v, f.sample, i, 0.625).unwrap();
    }
    try_write_sample_f64(&mut v, f.sample, f.mode.color_channels(), alpha).unwrap();
    v
}
#[test]
#[ignore = "UNRUN/uncompiled: cross-tile exact U32/F64 and Gray8 selection, sparse default, independent native alpha reference, preflight-invalid and cancellation atomicity; ordinary/None continuity"]
fn fractional_clear_surface_preflights_and_preserves_native_bits() {
    let area = Rect::new(255, 0, 258, 1);
    for depth in [SampleType::U32, SampleType::F64] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let f = PixelFormat::new(mode, depth, true);
            let one = raw(f, 0.5 + 2f64.powi(-40));
            let native = one.repeat(3);
            for selection_depth in [SampleType::U8, SampleType::U32, SampleType::F64] {
                let sf = PixelFormat::new(ColorMode::Grayscale, selection_depth, false);
                let mut default = vec![0; sf.bytes_per_pixel()];
                try_write_sample_f64(&mut default, selection_depth, 0, 0.5 + 2f64.powi(-42)).unwrap();
                let mut selection = Surface::try_with_default_bytes(sf, &default).unwrap();
                selection.try_write_pixel_f64(255, 0, &[0.0]).unwrap();
                selection.try_write_pixel_f64(257, 0, &[1.0]).unwrap();
                let mut s = Surface::try_from_interleaved(f, area, &native).unwrap();
                let mut expected = native.clone();
                let weight = try_read_sample_f64(&default, selection_depth, 0).unwrap();
                let alpha = try_read_sample_f64(&one, depth, mode.color_channels()).unwrap();
                try_write_sample_f64(&mut expected[f.bytes_per_pixel()..2 * f.bytes_per_pixel()], depth, mode.color_channels(), alpha * (1.0 - weight))
                    .unwrap();
                expected[2 * f.bytes_per_pixel()..].fill(0);
                try_clear_surface(&mut s, area, Some(&selection), &Interrupt::NONE).unwrap();
                assert_eq!(s.try_read_interleaved(area).unwrap(), expected);
                let mut full = Surface::try_from_interleaved(f, area, &native).unwrap();
                try_clear_surface(&mut full, area, None, &Interrupt::NONE).unwrap();
                assert!(full.try_read_interleaved(area).unwrap().iter().all(|v| *v == 0));
            }
        }
    }
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let mut bytes = raw(f, 0.5).repeat(3);
    let bad = 2 * f.bytes_per_pixel() + 24;
    bytes[bad..bad + 8].copy_from_slice(&f64::NAN.to_ne_bytes());
    let mut target = Surface::try_from_interleaved(f, area, &bytes).unwrap();
    let selection = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &0.375f64.to_ne_bytes()).unwrap();
    assert!(preflight_clear_surface(&target, area, Some(&selection), &Interrupt::NONE).is_err());
    assert!(try_clear_surface(&mut target, area, Some(&selection), &Interrupt::NONE).is_err());
    assert_eq!(target.try_read_interleaved(area).unwrap(), bytes);
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(try_clear_surface(&mut target, area, None, &cancelled).is_err());
    assert_eq!(target.try_read_interleaved(area).unwrap(), bytes);
    // Full coverage does not acquire a new NaN-alpha validity requirement.
    try_clear_surface(&mut target, area, None, &Interrupt::NONE).unwrap();
    assert!(target.try_read_interleaved(area).unwrap().iter().all(|v| *v == 0));
}
