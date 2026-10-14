//! Shared no-clip regression drafts. Tests are authored, not executed in this wave.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};

fn operations() -> [PointOperation; 2] {
    [
        PointOperation::AlphaClipPrecision { clip_low: false, low_limit: 0.25, clip_high: false, high_limit: 0.75 },
        PointOperation::AlphaClipFinitePrecision { clip_low: false, low_limit: 0.875, clip_high: false, high_limit: 0.125 },
    ]
}
#[test]
fn neither_clip_shared_applier_preserves_wide_words_default_and_off_roi() {
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Rgb, sample, true);
        let raw = if sample == SampleType::U32 {
            [0x01000001u32, 0x80000003, u32::MAX, 0x7fffffff].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        } else {
            [f64::from_bits(0x7ff8000000001234), -0.0, f64::from_bits(1), 0.5].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>()
        };
        let mut source = Surface::try_with_default_bytes(format, &raw).unwrap();
        source.try_write_interleaved(Rect::new(-1, -1, 0, 0), &raw).unwrap();
        let before = source.clone();
        for operation in operations() {
            let result = try_apply(&source, Rect::new(0, 0, 1, 1), operation, MAX_BYTES, 1_000_000, &Interrupt::NONE).unwrap();
            assert_eq!(result, before);
            assert_eq!(result.default_bytes(), raw);
            assert_eq!(result.try_read_interleaved(Rect::new(-1, -1, 0, 0)).unwrap(), raw);
            assert_eq!(source, before);
        }
    }
}
const MAX_BYTES: u64 = photocraft_raster::MAX_CHECKED_RASTER_BYTES;
#[test]
fn identity_still_validates_parameters_quota_and_cancel_before_return() {
    let source = Surface::new(PixelFormat::RGBA32F);
    let roi = Rect::new(0, 0, 1, 1);
    for operation in operations() {
        assert!(try_apply(&source, roi, operation, MAX_BYTES, 0, &Interrupt::NONE).is_err());
        assert!(matches!(try_apply(&source, roi, operation, MAX_BYTES, 1_000_000, &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
    }
    let malformed = PointOperation::AlphaClipFinitePrecision { clip_low: false, low_limit: f32::NAN, clip_high: false, high_limit: 0.5 };
    assert!(try_apply(&source, roi, malformed, MAX_BYTES, 1_000_000, &Interrupt::NONE).is_err());
    let invalid_unit = PointOperation::AlphaClipPrecision { clip_low: false, low_limit: 0.75, clip_high: false, high_limit: 0.25 };
    assert!(try_apply(&source, roi, invalid_unit, MAX_BYTES, 1_000_000, &Interrupt::NONE).is_err());
}
