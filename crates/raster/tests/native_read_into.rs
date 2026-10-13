use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn caller_owned_native_read_matches_all_runtime_formats_and_sparse_defaults() {
    for mode in [
        ColorMode::Bitmap,
        ColorMode::Grayscale,
        ColorMode::Indexed,
        ColorMode::Rgb,
        ColorMode::Cmyk,
        ColorMode::Lab,
        ColorMode::Multichannel,
        ColorMode::Duotone,
    ] {
        for sample in SampleType::ALL {
            for alpha in [false, true] {
                let format = PixelFormat::new(mode, sample, alpha);
                let default: Vec<_> = (0..format.bytes_per_pixel()).map(|i| (i * 37 + 129) as u8).collect();
                let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
                let written = Rect::new(-260, -3, 270, 3);
                let raw: Vec<_> = (0..written.width() as usize * written.height() as usize * format.bytes_per_pixel()).map(|i| (i * 97 + 11) as u8).collect();
                surface.try_write_interleaved(written, &raw).unwrap();
                let before = surface.clone();
                let area = Rect::new(-270, -8, 280, 8);
                let mut output = vec![0xa5; area.width() as usize * area.height() as usize * format.bytes_per_pixel()];
                surface.try_read_interleaved_into_with(area, &mut output, &Interrupt::NONE).unwrap();
                let mut expected = Vec::new();
                for y in area.y0..area.y1 {
                    for x in area.x0..area.x1 {
                        if x >= written.x0 && x < written.x1 && y >= written.y0 && y < written.y1 {
                            let start = ((y - written.y0) as usize * written.width() as usize + (x - written.x0) as usize) * format.bytes_per_pixel();
                            expected.extend_from_slice(&raw[start..start + format.bytes_per_pixel()]);
                        } else {
                            expected.extend_from_slice(&default);
                        }
                    }
                }
                assert_eq!(output, expected, "{format:?}");
                assert_eq!(surface, before);
            }
        }
    }
}

#[test]
fn raw_float_payloads_signed_zero_and_adjacent_codes_are_preserved() {
    for (format, pixel) in [
        (PixelFormat::RGBA32F, [0x80000000u32, 0x3f000001, 0x7fc01234, 0x7f800000].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()),
        (
            PixelFormat::RGBA64F,
            [0x8000000000000000u64, 0x3fe0000000000001, 0x7ff8123412341234, 0x7ff0000000000000].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>(),
        ),
    ] {
        let surface = Surface::try_with_default_bytes(format, &pixel).unwrap();
        let mut output = vec![0; pixel.len() * 513];
        surface.try_read_interleaved_into_with(Rect::new(-257, 0, 256, 1), &mut output, &Interrupt::NONE).unwrap();
        assert_eq!(output, pixel.repeat(513));
    }
}

#[test]
fn invalid_output_bounds_and_cancellation_are_graceful() {
    let surface = Surface::new(PixelFormat::RGBA8);
    let mut output = [0xa5; 4];
    assert_eq!(surface.try_read_interleaved_into_with(Rect::new(0, 0, 2, 1), &mut output, &Interrupt::NONE), Err(RasterInputError::LengthMismatch));
    assert_eq!(output, [0xa5; 4]);
    assert_eq!(surface.try_read_interleaved_into_with(Rect::new(2, 0, 1, 1), &mut output, &Interrupt::NONE), Err(RasterInputError::InvalidBounds));
    assert!(surface.try_read_interleaved_into_with(Rect::new(i32::MIN, 0, i32::MAX, 1), &mut output, &Interrupt::NONE).is_err());
    let cancel = || true;
    assert_eq!(surface.try_read_interleaved_into_with(Rect::new(0, 0, 1, 1), &mut output, &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled));
    assert_eq!(output, [0xa5; 4]);
    surface.try_read_interleaved_into_with(Rect::new(-1, 0, -1, 3), &mut [], &Interrupt::NONE).unwrap();
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 3;
    let mut output = vec![0xa5; 1024 * 2 * 4];
    assert_eq!(
        surface.try_read_interleaved_into_with(Rect::new(0, 0, 1024, 2), &mut output, &Interrupt::cancel_only(&cancel)),
        Err(RasterInputError::Cancelled)
    );
    assert!(output.contains(&0xa5));
    assert!(output.contains(&0));
}

#[test]
#[ignore = "manual24/36MP release native read comparison; no aggregate/RSS/admission proof"]
fn release_native_read_comparison() {
    for (width, height) in [(6000, 4000), (7200, 5000)] {
        let bounds = Rect::new(0, 0, width, height);
        let raw = [19, 89, 173, 211].repeat(width as usize * height as usize);
        let surface = Surface::try_from_interleaved(PixelFormat::RGBA8, bounds, &raw).unwrap();
        let mut bulk = vec![0; raw.len()];
        let start = std::time::Instant::now();
        let old = surface.try_read_interleaved(bounds).unwrap();
        let old_seconds = start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        surface.try_read_interleaved_into_with(bounds, &mut bulk, &Interrupt::NONE).unwrap();
        let bulk_seconds = start.elapsed().as_secs_f64();
        assert_eq!(old, raw);
        assert_eq!(bulk, raw);
        println!(
            "NATIVE_READ width={width} height={height} old_seconds={old_seconds:.6} bulk_seconds={bulk_seconds:.6} output_bytes={} caller_buffer_preallocated=true",
            raw.len()
        );
    }
}
