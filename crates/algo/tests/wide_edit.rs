use photocraft_algo::wide;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn invert_keeps_alpha_bits_and_undo_input_including_hdr() {
    let fmt = PixelFormat::RGBA64F;
    let input = [1.0_f64 + f64::EPSILON, -2.0, 4.5, 0.75, 0.25, 0.5, 0.75, 0.5];
    let raw: Vec<u8> = input.into_iter().flat_map(f64::to_ne_bytes).collect();
    let rect = Rect::new(0, 0, 2, 1);
    let source = Surface::try_from_interleaved(fmt, rect, &raw).unwrap();
    let history = source.clone();
    let result = wide::try_invert(&source, rect, None, &Interrupt::NONE).unwrap();
    let values = result.try_read_region_f64(rect).unwrap();
    assert_eq!(values[0], -f64::EPSILON);
    assert_eq!(values[1], 3.0);
    assert_eq!(values[2], -3.5);
    assert_eq!(values[3].to_bits(), input[3].to_bits());
    assert_eq!(source, history);
    assert_eq!(history.try_read_interleaved(rect).unwrap(), raw);
}

#[test]
fn raw_copy_crop_translation_preserve_codes_defaults_and_nan_bits() {
    for sample in [SampleType::U32, SampleType::F64] {
        let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let raw = if sample == SampleType::U32 {
            [16_777_216_u32.to_ne_bytes(), 16_777_217_u32.to_ne_bytes()].concat()
        } else {
            [0x8000_0000_0000_0000_u64.to_ne_bytes(), 0x7ff8_1234_5678_9abc_u64.to_ne_bytes()].concat()
        };
        let default = raw[..sample.bytes()].to_vec();
        let rect = Rect::new(255, -1, 257, 0);
        let mut source = Surface::try_with_default_bytes(fmt, &default).unwrap();
        source.try_write_interleaved(rect, &raw).unwrap();
        let moved = wide::try_translate(&source, rect, -17, 9, &Interrupt::NONE).unwrap();
        assert_eq!(moved.try_read_interleaved(Rect::new(238, 8, 240, 9)).unwrap(), raw);
        assert_eq!(moved.default_bytes(), default);
        let cropped = wide::try_crop(&source, rect, &Interrupt::NONE).unwrap();
        assert_eq!(cropped.try_read_interleaved(rect).unwrap(), raw);
        assert_eq!(cropped.default_bytes(), default);
        let legacy_crop = photocraft_algo::resample::crop_surface(&source, rect);
        assert_eq!(legacy_crop.default_bytes(), default);
        assert_eq!(legacy_crop.try_read_interleaved(rect).unwrap(), raw);
        let legacy_move = photocraft_algo::resample::translate_surface(&source, -17, 9);
        assert_eq!(legacy_move.default_bytes(), default);
        assert_eq!(legacy_move.try_read_interleaved(Rect::new(238, 8, 240, 9)).unwrap(), raw);
    }
}

#[test]
fn checked_filter_dispatch_rejects_unimplemented_wide_kernels() {
    let source = Surface::new(PixelFormat::RGBA64F);
    let rect = Rect::new(0, 0, 1, 1);
    let result = photocraft_algo::apply_tiled_with(&source, &photocraft_algo::FilterParams::Invert, rect, rect, None, 256, None, &Interrupt::NONE);
    assert!(result.is_some());
    let params = photocraft_algo::FilterParams::GaussianBlur { radius: 1.0 };
    assert!(photocraft_algo::apply_tiled_with(&source, &params, rect, rect, None, 256, None, &Interrupt::NONE).is_none());
}

#[test]
fn lab_wide_invert_mirrors_stored_chroma_without_narrowing_alpha_or_selection() {
    let rect = Rect::new(255, -1, 257, 0);
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Lab, sample, true);
        let codes = [1_288_490_188u32, 2_358_033_173, 1_852_730_947, 3_000_000_001];
        let floats = [0.3f64, f64::from_bits(0x3fe1_9191_9191_9193), f64::from_bits(0x3fdb_9b9b_9b9b_9b9d), f64::from_bits(0x7ff8_1234_5678_9abc)];
        let pixel: Vec<u8> = if sample == SampleType::U32 {
            codes.into_iter().flat_map(u32::to_ne_bytes).collect()
        } else {
            floats.into_iter().flat_map(f64::to_ne_bytes).collect()
        };
        let raw = pixel.repeat(2);
        let source = Surface::try_from_interleaved(format, rect, &raw).unwrap();
        for weight in [0.0f64, 0.375, 1.0] {
            let selection_bytes = [weight.to_ne_bytes(), 0.0f64.to_ne_bytes()].concat();
            let selection = Surface::try_from_interleaved(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), rect, &selection_bytes).unwrap();
            let result = wide::try_invert(&source, rect, Some(&selection), &Interrupt::NONE).unwrap();
            let mut expected = raw.clone();
            if weight != 0.0 {
                for channel in 0..3 {
                    if sample == SampleType::U32 {
                        // Independent exact integer code reference: MAX is divisible
                        // by255, so Lab neutral mirror has no fractional code here.
                        let center = u64::from(u32::MAX) + if channel == 0 { 0 } else { u64::from(u32::MAX / 255) };
                        let inverted = center - u64::from(codes[channel]);
                        let code = if weight == 1.0 { inverted } else { (u64::from(codes[channel]) * 5 + inverted * 3 + 4) / 8 };
                        expected[channel * 4..channel * 4 + 4].copy_from_slice(&(code as u32).to_ne_bytes());
                    } else {
                        let center = if channel == 0 { 1.0 } else { 256.0 / 255.0 };
                        let inverted = center - floats[channel];
                        let value = if weight == 1.0 { inverted } else { floats[channel] * 0.625 + inverted * 0.375 };
                        expected[channel * 8..channel * 8 + 8].copy_from_slice(&value.to_ne_bytes());
                    }
                }
            }
            assert_eq!(result.try_read_interleaved(rect).unwrap(), expected);
            assert_eq!(source.try_read_interleaved(rect).unwrap(), raw, "input/history immutable");
        }
        assert!(matches!(wide::try_invert(&source, rect, None, &Interrupt::cancel_only(&|| true)), Err(photocraft_raster::RasterInputError::Cancelled)));
        assert_eq!(source.try_read_interleaved(rect).unwrap(), raw);
    }
}
