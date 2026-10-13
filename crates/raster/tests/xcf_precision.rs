use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::Arc;

#[test]
fn exact_wide_default_and_untouched_cow_bits_survive_edits() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let default = 0x7ff8_1234_5678_9abc_u64.to_ne_bytes();
    let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
    assert_eq!(surface.try_read_interleaved(Rect::new(-1, -1, 1, 1)).unwrap(), default.repeat(4));
    let original = (1.0_f64 + f64::EPSILON).to_ne_bytes();
    surface.try_write_interleaved(Rect::new(0, 0, 1, 1), &original).unwrap();
    let history = surface.clone();
    assert!(Arc::ptr_eq(surface.tile(TileCoord::new(0, 0)).unwrap(), history.tile(TileCoord::new(0, 0)).unwrap()));
    surface.try_write_pixel_f64(1, 0, &[4.25]).unwrap();
    assert_eq!(surface.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), original);
    assert_eq!(history.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), default);
    assert_eq!(surface.default_bytes(), default);
}

#[test]
fn checked_wide_regions_keep_adjacent_u32_and_f64_precision() {
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let raw = if sample == SampleType::U32 {
            [16_777_216_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat()
        } else {
            [1.0_f64.to_ne_bytes(), (1.0_f64 + f64::EPSILON).to_ne_bytes()].concat()
        };
        let bounds = Rect::new(255, -1, 257, 0);
        let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
        let values = source.try_read_region_f64(bounds).unwrap();
        assert_ne!(values[0], values[1]);
        let mut edited = Surface::new(format);
        edited.try_write_region_f64(bounds, &values).unwrap();
        assert_eq!(edited.try_read_interleaved(bounds).unwrap(), raw);
    }
}

#[test]
fn checked_bounds_lengths_and_cancellation_do_not_commit() {
    let mut source = Surface::new(PixelFormat::RGBA64F);
    let before = source.clone();
    assert_eq!(source.try_write_region_f64(Rect::new(0, 0, 1, 1), &[1.0]), Err(RasterInputError::LengthMismatch));
    assert!(source.try_read_region_f64(Rect::new(i32::MIN, 0, i32::MAX, 1)).is_err());
    assert!(source.try_read_interleaved(Rect::new(1, 1, 0, 0)).is_err());
    let cancel = Interrupt::cancel_only(&|| true);
    assert_eq!(source.try_write_region_f64_with(Rect::new(0, 0, 1, 1), &[0.0; 4], &cancel), Err(RasterInputError::Cancelled));
    assert_eq!(source, before);
}

#[test]
fn original_index_and_alpha_bytes_survive_raw_storage_without_palette_normalization() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let bytes = [0, 0, 1, 255, 2, 128, 3, 255];
    let bounds = Rect::new(0, 0, 4, 1);
    let surface = Surface::try_from_interleaved(format, bounds, &bytes).unwrap();
    assert_eq!(surface.try_read_interleaved(bounds).unwrap(), bytes);
}

#[test]
fn same_format_conversion_and_alpha_changes_keep_authoritative_bits() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let raw = [0x7ff8_1234_5678_9abc_u64.to_ne_bytes(), (1.0_f64 + f64::EPSILON).to_ne_bytes()].concat();
    let bounds = Rect::new(0, 0, 2, 1);
    let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    assert_eq!(source.convert(format).try_read_interleaved(bounds).unwrap(), raw);
    let alpha = source.try_convert(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true)).unwrap();
    let expected = [raw[..8].to_vec(), 1.0_f64.to_ne_bytes().to_vec(), raw[8..].to_vec(), 1.0_f64.to_ne_bytes().to_vec()].concat();
    assert_eq!(alpha.try_read_interleaved(bounds).unwrap(), expected);
    assert!(source.try_convert(PixelFormat::GRAY8).is_err());
    assert_eq!(source.try_convert(PixelFormat::RGBA64F), Err(RasterInputError::UnsupportedConversion));
}

#[test]
fn checked_u32_to_f64_conversion_keeps_adjacent_codes() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let raw = [16_777_216_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat();
    let bounds = Rect::new(0, 0, 2, 1);
    let source = Surface::try_from_interleaved(format, bounds, &raw).unwrap();
    let wide = source.try_convert(format.with_sample(SampleType::F64)).unwrap();
    let values = wide.try_read_region_f64(bounds).unwrap();
    assert_ne!(values[0], values[1]);
    assert_eq!(wide.try_convert(format).unwrap().try_read_interleaved(bounds).unwrap(), raw);
}

#[test]
fn legal_half_widening_retains_exact_storage_and_separate_source_bits() {
    // IEEE754 values with independently specified binary32 encodings. Half is
    // source precision; the canonical Surface stores the explicitly widened F32.
    let cases = [
        (0x0000_u16, 0x0000_0000_u32),
        (0x8000, 0x8000_0000),
        (0x0001, 0x3380_0000),
        (0x03ff, 0x387f_c000),
        (0x0400, 0x3880_0000),
        (0x3555, 0x3eaa_a000),
        (0x3c00, 0x3f80_0000),
        (0xc000, 0xc000_0000),
        (0x7bff, 0x477f_e000),
    ];
    let source: Vec<u8> = cases.iter().flat_map(|(half, _)| half.to_be_bytes()).collect();
    let raw: Vec<u8> = cases
        .iter()
        .flat_map(|(half, expected)| {
            let widened = photocraft_color::widen_f16_to_f32_bits(*half);
            assert_eq!(widened, *expected);
            widened.to_ne_bytes()
        })
        .collect();
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let bounds = Rect::new(-4, -1, 5, 0);
    let mut surface = Surface::try_from_interleaved(fmt, bounds, &raw).unwrap();
    let history = surface.clone();
    surface.try_write_pixel_f64(0, -1, &[0.5]).unwrap();
    assert_eq!(history.try_read_interleaved(bounds).unwrap(), raw);
    assert_eq!(source, cases.iter().flat_map(|(half, _)| half.to_be_bytes()).collect::<Vec<_>>());
    assert_eq!(surface.try_read_interleaved(Rect::new(-4, -1, -2, 0)).unwrap(), raw[..8]);
}

#[test]
fn hdr_nonfinite_and_signed_zero_payloads_survive_raw_copy_at_both_float_depths() {
    for (sample, raw) in [
        (
            SampleType::F32,
            [0xc020_0000_u32, 0x4184_0000, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 0x7fc1_2345].iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<_>>(),
        ),
        (
            SampleType::F64,
            [0xc004_0000_0000_0000_u64, 0x4030_8000_0000_0000, 0x8000_0000_0000_0000, 0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000, 0x7ff8_1234_5678_9abc]
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect::<Vec<_>>(),
        ),
    ] {
        let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let bounds = Rect::new(254, -1, 260, 0);
        let surface = Surface::try_from_interleaved(fmt, bounds, &raw).unwrap();
        assert_eq!(surface.try_read_interleaved(bounds).unwrap(), raw);
        assert_eq!(surface.try_convert(fmt).unwrap().try_read_interleaved(bounds).unwrap(), raw);
    }
}

#[test]
fn u32_default_and_untouched_adjacent_code_survive_local_cow_edit() {
    let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
    let default = 16_777_217_u32.to_ne_bytes();
    let mut source = Surface::try_with_default_bytes(fmt, &default).unwrap();
    let before = source.clone();
    source.try_write_pixel_f64(256, -1, &[0.5]).unwrap();
    assert_eq!(source.default_bytes(), default);
    assert_eq!(source.try_read_interleaved(Rect::new(255, -1, 256, 0)).unwrap(), default);
    assert_eq!(before.try_read_interleaved(Rect::new(256, -1, 257, 0)).unwrap(), default);
}
