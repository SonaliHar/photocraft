//! Deferred native Clear contracts. All buffers are external harness allocations.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
fn pixel(f: PixelFormat, alpha: f64) -> Vec<u8> {
    let mut raw = vec![0; f.bytes_per_pixel()];
    for i in 0..f.mode.color_channels() {
        try_write_sample_f64(&mut raw, f.sample, i, 0.625).unwrap();
    }
    try_write_sample_f64(&mut raw, f.sample, f.mode.color_channels(), alpha).unwrap();
    raw
}
#[test]
#[ignore = "UNRUN/uncompiled: exact native fractional alpha association, raw endpoints, signed zero and computed-positive encoded-zero distinction; no producer claim"]
fn native_fractional_endpoints_and_alpha_only_precision() {
    for depth in [SampleType::U32, SampleType::F64] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let f = PixelFormat::new(mode, depth, true);
            let alpha = if depth == SampleType::U32 { f64::from(0x80000001u32) / f64::from(u32::MAX) } else { 0.5 + 2f64.powi(-40) };
            let original = pixel(f, alpha);
            for weight in [0.0, -0.0, 1.0, 0.375, 0.5 + 2f64.powi(-42), f64::from(0x80000001u32) / f64::from(u32::MAX)] {
                let mut actual = original.clone();
                let mut expected = original.clone();
                if weight == 1.0 {
                    expected.fill(0);
                } else if weight != 0.0 {
                    try_write_sample_f64(&mut expected, depth, mode.color_channels(), alpha * (1.0 - weight)).unwrap();
                }
                try_clear_native_pixel(&mut actual, f, weight).unwrap();
                assert_eq!(actual, expected);
            }
            for alpha in [0.0, -0.0] {
                let mut raw = pixel(f, alpha);
                try_clear_native_pixel(&mut raw, f, 0.375).unwrap();
                assert!(raw.iter().all(|v| *v == 0));
            }
            if depth == SampleType::U32 {
                let mut raw = pixel(f, 1.0 / f64::from(u32::MAX));
                let colors = raw[..mode.color_channels() * 4].to_vec();
                try_clear_native_pixel(&mut raw, f, 0.75).unwrap();
                assert_eq!(&raw[..colors.len()], colors);
                assert_eq!(try_read_sample_f64(&raw, depth, mode.color_channels()).unwrap(), 0.0);
            }
        }
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: NaN colors untouched, alpha-only finite refusal atomic, full invalid-alpha clear and zero raw skip; no source permission"]
fn invalid_alpha_and_raw_color_endpoints_are_atomic() {
    let f = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
    let mut raw = pixel(f, 0.75);
    raw[..8].copy_from_slice(&f64::NAN.to_ne_bytes());
    let colors = raw[..24].to_vec();
    try_clear_native_pixel(&mut raw, f, 0.375).unwrap();
    assert_eq!(&raw[..24], colors);
    for alpha in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut invalid = pixel(f, alpha);
        let before = invalid.clone();
        assert!(try_clear_native_pixel(&mut invalid, f, 0.375).is_err());
        assert_eq!(invalid, before);
        try_clear_native_pixel(&mut invalid, f, -0.0).unwrap();
        assert_eq!(invalid, before);
        try_clear_native_pixel(&mut invalid, f, 1.0).unwrap();
        assert!(invalid.iter().all(|v| *v == 0));
    }
    for weight in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut actual = raw.clone();
        assert!(try_clear_native_pixel(&mut actual, f, weight).is_err());
        assert_eq!(actual, raw);
    }
}
