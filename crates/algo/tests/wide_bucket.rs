use photocraft_algo::wide_paint::{BucketOptions, PaintLimits, try_paint_bucket_exact};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};
fn options(contiguous: bool) -> BucketOptions {
    BucketOptions { seed: (0, 0), contiguous, opacity: 1.0, lock_transparency: false }
}
#[test]
fn malformed_retained_tile_origin_is_rejected_without_mutation() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut materialized = Surface::new(format);
    materialized.try_write_pixel_f64(0, 0, &[1.0]).unwrap();
    let tile = materialized.tiles().next().unwrap().1.clone();
    let mut target = Surface::new(format);
    target.put_tiles([(photocraft_geom::TileCoord::new(i32::MAX, i32::MIN), tile)]);
    let before = target.clone();
    assert_eq!(
        try_paint_bucket_exact(&mut target, Rect::new(0, 0, 4, 4), options(true), &[1.0], None, Default::default(), &Interrupt::NONE),
        Err(RasterInputError::InvalidBounds)
    );
    assert_eq!(target, before);
}
#[test]
fn exact_native_flood_distinguishes_adjacent_codes_and_global_matches() {
    for sample in [SampleType::U32, SampleType::F64] {
        let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let bytes = if sample == SampleType::U32 {
            [0x80000001_u32, 0x80000002, 0x80000001, 0x80000001].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        } else {
            [0.5 + 2.0_f64.powi(-40), 0.5 + 2.0_f64.powi(-39), 0.5 + 2.0_f64.powi(-40), 0.5 + 2.0_f64.powi(-40)]
                .into_iter()
                .flat_map(f64::to_ne_bytes)
                .collect()
        };
        let original = Surface::try_from_interleaved(format, Rect::new(0, 0, 4, 1), &bytes).unwrap();
        let mut legacy = [[0_u8; 4]; 2];
        original.read_rgba8_into(Rect::new(0, 0, 2, 1), &mut legacy);
        assert_eq!(legacy[0], legacy[1]);
        for contiguous in [true, false] {
            let mut target = original.clone();
            assert!(
                try_paint_bucket_exact(&mut target, Rect::new(0, 0, 4, 1), options(contiguous), &[0.0], None, PaintLimits::default(), &Interrupt::NONE)
                    .unwrap()
            );
            assert_eq!(target.try_read_region_f64(Rect::new(0, 0, 1, 1)).unwrap(), vec![0.0]);
            assert_eq!(target.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), original.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap());
            if contiguous {
                assert_eq!(target.try_read_interleaved(Rect::new(2, 0, 4, 1)).unwrap(), original.try_read_interleaved(Rect::new(2, 0, 4, 1)).unwrap());
            } else {
                assert_eq!(target.try_read_region_f64(Rect::new(2, 0, 4, 1)).unwrap(), vec![0.0; 2]);
            }
            assert_eq!(target.default_bytes(), original.default_bytes());
            assert_eq!(original.try_read_interleaved(Rect::new(0, 0, 4, 1)).unwrap(), bytes);
        }
    }
}
#[test]
fn bounds_limits_and_nonfinite_rejection_are_atomic() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut target = Surface::new(format);
    let before = target.clone();
    let mut opts = options(true);
    opts.seed = (-1, 0);
    assert!(!try_paint_bucket_exact(&mut target, Rect::new(0, 0, 4, 4), opts, &[1.0], None, PaintLimits::default(), &Interrupt::NONE).unwrap());
    assert_eq!(target, before);
    assert_eq!(
        try_paint_bucket_exact(
            &mut target,
            Rect::new(0, 0, 4, 4),
            options(true),
            &[1.0],
            None,
            PaintLimits { resident_bytes: 1, work_units: 1 },
            &Interrupt::NONE
        ),
        Err(RasterInputError::LimitExceeded)
    );
    assert_eq!(target, before);
    target.try_write_pixel_f64(0, 0, &[f64::NAN]).unwrap();
    let raw = target.try_read_interleaved(Rect::new(0, 0, 4, 4)).unwrap();
    assert_eq!(
        try_paint_bucket_exact(&mut target, Rect::new(0, 0, 4, 4), options(true), &[1.0], None, PaintLimits::default(), &Interrupt::NONE),
        Err(RasterInputError::InvalidSample)
    );
    assert_eq!(target.try_read_interleaved(Rect::new(0, 0, 4, 4)).unwrap(), raw);
}

#[test]
fn selection_and_mid_sampling_cancel_preserve_exact_source_snapshot() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let mut target = Surface::try_with_default_bytes(format, &(-0.0_f64).to_ne_bytes()).unwrap();
    let original = target.clone();
    let mut selection = Surface::new(format);
    let coverage = 0.25 + 2.0_f64.powi(-40);
    selection.try_write_pixel_f64(1, 1, &[coverage]).unwrap();
    try_paint_bucket_exact(&mut target, Rect::new(0, 0, 4, 4), options(true), &[1.0], Some(&selection), Default::default(), &Interrupt::NONE).unwrap();
    assert_eq!(target.try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap(), vec![coverage]);
    assert_eq!(target.try_read_interleaved(Rect::new(2, 0, 4, 4)).unwrap(), original.try_read_interleaved(Rect::new(2, 0, 4, 4)).unwrap());
    let before = target.clone();
    let count = AtomicUsize::new(0);
    let cancelled = || count.fetch_add(1, Ordering::Relaxed) >= 12;
    let control = Interrupt::cancel_only(&cancelled);
    assert_eq!(
        try_paint_bucket_exact(&mut target, Rect::new(0, 0, 64, 64), options(true), &[1.0], None, Default::default(), &control),
        Err(RasterInputError::Cancelled)
    );
    assert_eq!(target, before);
}
