use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::wide::try_apply_normal_coverage;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn normal_coverage_arithmetic_keeps_f64_precision_and_history() {
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let rect = Rect::new(0, 0, 2, 1);
    let raw = [(1.0_f64 + f64::EPSILON).to_ne_bytes(), 0x7ff8_1234_5678_9abc_u64.to_ne_bytes()].concat();
    let mut target = Surface::try_from_interleaved(fmt, rect, &raw).unwrap();
    let history = target.clone();
    let paint = Surface::try_from_interleaved(fmt, rect, &[3.0_f64.to_ne_bytes(), 0.0_f64.to_ne_bytes()].concat()).unwrap();
    try_apply_normal_coverage(&mut target, &paint, rect, &[0.25, 0.0], 1.0, None, false, &Interrupt::NONE).unwrap();
    let result = target.try_read_region_f64(rect).unwrap();
    assert_eq!(result[0], (1.0_f64 + f64::EPSILON) * 0.75 + 3.0 * 0.25);
    assert_eq!(result[1].to_bits(), 0x7ff8_1234_5678_9abc);
    assert_eq!(history.try_read_interleaved(rect).unwrap(), raw);
}

#[test]
fn invalid_coverage_and_cancellation_never_commit() {
    let mut target = Surface::new(PixelFormat::RGBA64F);
    let paint = target.clone();
    let before = target.clone();
    let rect = Rect::new(0, 0, 1, 1);
    assert!(try_apply_normal_coverage(&mut target, &paint, rect, &[f64::NAN], 1.0, None, false, &Interrupt::NONE).is_err());
    assert_eq!(target, before);
    let cancel = Interrupt::cancel_only(&|| true);
    assert!(try_apply_normal_coverage(&mut target, &paint, rect, &[1.0], 1.0, None, false, &cancel).is_err());
    assert_eq!(target, before);
}

#[test]
fn rgba_source_over_and_transparency_lock_use_f64_and_keep_alpha_bits() {
    let rect = Rect::new(0, 0, 1, 1);
    let dst = [1.0_f64 + f64::EPSILON, -2.0, 4.5, 0.75];
    let src = [3.0_f64, 0.25, -1.0, 0.5];
    let raw: Vec<u8> = dst.into_iter().flat_map(f64::to_ne_bytes).collect();
    let mut target = Surface::try_from_interleaved(PixelFormat::RGBA64F, rect, &raw).unwrap();
    let paint = Surface::try_from_interleaved(PixelFormat::RGBA64F, rect, &src.into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
    try_apply_normal_coverage(&mut target, &paint, rect, &[0.5], 1.0, None, true, &Interrupt::NONE).unwrap();
    let values = target.try_read_region_f64(rect).unwrap();
    let oa = 0.25 + 0.75 * 0.75;
    for channel in 0..3 {
        assert_eq!(values[channel], (src[channel] * 0.25 + dst[channel] * 0.75 * 0.75) / oa);
    }
    assert_eq!(values[3].to_bits(), dst[3].to_bits());
}

#[test]
fn adjacent_u32_codes_survive_copy_paint_and_masked_pixels() {
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let rect = Rect::new(0, 0, 2, 1);
    let raw = [16_777_216_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat();
    let paint = Surface::try_from_interleaved(fmt, rect, &raw).unwrap();
    let mut target = Surface::try_from_interleaved(fmt, rect, &[0_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat()).unwrap();
    try_apply_normal_coverage(&mut target, &paint, rect, &[1.0, 0.0], 1.0, None, false, &Interrupt::NONE).unwrap();
    assert_eq!(target.try_read_interleaved(rect).unwrap(), raw);
}

#[test]
fn f64_paint_source_is_quantized_only_after_u32_coverage_arithmetic() {
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let mut target = Surface::new(fmt);
    let before = target.clone();
    let float = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let paint = Surface::try_with_default_bytes(float, &(0.6 / f64::from(u32::MAX)).to_ne_bytes()).unwrap();
    try_apply_normal_coverage(&mut target, &paint, Rect::new(0, 0, 1, 1), &[0.5], 1.0, None, false, &Interrupt::NONE).unwrap();
    assert_eq!(target, before); // 0.3 integer code rounds to zero, without creating a tile.
    let quantized = paint.try_convert(fmt).unwrap();
    try_apply_normal_coverage(&mut target, &quantized, Rect::new(0, 0, 1, 1), &[0.5], 1.0, None, false, &Interrupt::NONE).unwrap();
    assert_ne!(target, before); // Premature quantization rounds0.6→1, then0.5→1.
}
