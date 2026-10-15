//! UNRUN checked transient capacity boundaries and raw storage preservation.
use super::*;

#[test]
fn reserved_capacity_rejects_unpriced_growth_and_arithmetic_overflow_without_allocating() {
    for (actual, requested) in [(0, 0), (1, 1), (2, 1), (1024, 1024), (2048, 1024)] {
        assert!(checked_reserved_capacity(actual, requested).is_ok());
    }
    for (actual, requested) in [(1, 0), (0, 1), (3, 1), (2049, 1024)] {
        assert_eq!(checked_reserved_capacity(actual, requested), Err(AllocationError::NotEnoughMemory { bytes: requested }));
    }
    assert_eq!(checked_reserved_capacity(usize::MAX, usize::MAX), Err(AllocationError::SizeOverflow));
}

#[test]
fn checked_default_tile_clone_and_encoded_cow_writes_preserve_raw_words_at_every_depth() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(mode, sample, true);
            // Encoded words intentionally are not numerically normalized.
            let default = vec![0xa5; format.bytes_per_pixel()];
            let tile = Tile::try_filled(&format, &default).unwrap();
            let copy = tile.try_clone().unwrap();
            assert_eq!(tile.bytes(), copy.bytes());
            assert_eq!(tile.bytes().len(), (TILE_SIZE * TILE_SIZE) as usize * format.bytes_per_pixel());
            let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
            let original = surface.clone();
            let rect = Rect::new(-1, -1, 1, 1);
            let encoded = vec![0x5a; 4 * format.bytes_per_pixel()];
            surface.try_write_interleaved_with(rect, &encoded, &Interrupt::NONE).unwrap();
            let mut read = vec![0; format.bytes_per_pixel()];
            surface.try_read_pixel_bytes(-1, -1, &mut read).unwrap();
            assert_eq!(read, vec![0x5a; format.bytes_per_pixel()]);
            original.try_read_pixel_bytes(-1, -1, &mut read).unwrap();
            assert_eq!(read, default);
            assert_eq!(surface.default_bytes(), default);
            let wide_rect = Rect::new(2, 2, 4, 4);
            let values = vec![1.0; 4 * format.channels()];
            surface.try_write_region_f64_with(wide_rect, &values, &Interrupt::NONE).unwrap();
            let mut pixel = vec![0.0; format.channels()];
            surface.try_read_pixel_f64(2, 2, &mut pixel).unwrap();
            assert_eq!(pixel, vec![1.0; format.channels()]);
            surface.try_read_pixel_bytes(-1, -1, &mut read).unwrap();
            assert_eq!(read, vec![0x5a; format.bytes_per_pixel()]);
            assert_eq!(surface.default_bytes(), default);
        }
    }
}
