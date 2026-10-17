use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, OwnedRasterSnapshot, RasterInputError, SnapshotSurface};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn lazy_backing_and_cow_history_preserve_exact_wide_source() {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let bounds = Rect::new(255, -1, 257, 0);
    let original = [1.0_f64.to_ne_bytes(), (1.0_f64 + f64::EPSILON).to_ne_bytes()].concat();
    let default = 0x8000_0000_0000_0000_u64.to_ne_bytes().to_vec();
    let snapshot = OwnedRasterSnapshot::try_new(format, bounds, original.clone(), default.clone(), 8 << 20).unwrap();
    let mut lazy = SnapshotSurface::try_new(snapshot.clone()).unwrap();
    assert_eq!(lazy.materialized_tile_count(), 0);
    assert!(lazy.snapshot().shares_backing_with(&snapshot));
    let mut raw = [0; 8];
    lazy.try_read_pixel_bytes(256, -1, &mut raw).unwrap();
    assert_eq!(raw, (1.0_f64 + f64::EPSILON).to_ne_bytes());
    assert_eq!(lazy.materialized_tile_count(), 0);
    let history = lazy.clone();
    lazy.try_write_interleaved(Rect::new(255, -1, 256, 0), &4.25_f64.to_ne_bytes(), &Interrupt::NONE).unwrap();
    assert_eq!(lazy.materialized_tile_count(), 1);
    history.try_read_pixel_bytes(255, -1, &mut raw).unwrap();
    assert_eq!(raw, 1.0_f64.to_ne_bytes());
    lazy.try_read_pixel_bytes(256, -1, &mut raw).unwrap();
    assert_eq!(raw, (1.0_f64 + f64::EPSILON).to_ne_bytes());
    assert_eq!(snapshot.bytes(), original);
    let materialized = lazy.try_materialize(&Interrupt::NONE).unwrap();
    assert_eq!(materialized.default_bytes(), default);
    assert_eq!(materialized.try_read_interleaved(bounds).unwrap(), [4.25_f64.to_ne_bytes(), (1.0_f64 + f64::EPSILON).to_ne_bytes()].concat());
}

#[test]
fn limit_cancel_and_hostile_coordinates_fail_without_editing() {
    let format = PixelFormat::GRAY8;
    let snapshot = OwnedRasterSnapshot::try_new(format, Rect::new(0, 0, 1, 1), vec![17], vec![0], 256).unwrap();
    assert_eq!(snapshot.try_read_tile(TileCoord::new(0, 0), &Interrupt::NONE).unwrap_err(), RasterInputError::LimitExceeded);
    assert_eq!(snapshot.try_read_tile(TileCoord::new(i32::MAX, 0), &Interrupt::NONE).unwrap_err(), RasterInputError::InvalidBounds);
    let mut lazy = SnapshotSurface::try_new(snapshot).unwrap();
    assert_eq!(lazy.try_write_interleaved(Rect::new(0, 0, 1, 1), &[99], &Interrupt::NONE), Err(RasterInputError::LimitExceeded));
    let cancel = Interrupt::cancel_only(&|| true);
    assert_eq!(lazy.try_write_interleaved(Rect::new(0, 0, 1, 1), &[99], &cancel), Err(RasterInputError::Cancelled));
    let mut raw = [0];
    lazy.try_read_pixel_bytes(0, 0, &mut raw).unwrap();
    assert_eq!(raw, [17]);
    assert_eq!(lazy.materialized_tile_count(), 0);
}

#[test]
fn adopted_capacity_is_charged_even_when_encoded_length_is_small() {
    let mut bytes = Vec::with_capacity(1024);
    bytes.push(17);
    assert_eq!(OwnedRasterSnapshot::try_new(PixelFormat::GRAY8, Rect::new(0, 0, 1, 1), bytes, vec![0], 100).unwrap_err(), RasterInputError::LimitExceeded);
}

#[test]
fn cancellation_inside_tile_materialization_keeps_snapshot_and_overrides_unchanged() {
    let snapshot = OwnedRasterSnapshot::try_new(PixelFormat::GRAY8, Rect::new(0, 0, 2, 1), vec![17, 18], vec![0], 1 << 20).unwrap();
    let mut lazy = SnapshotSurface::try_new(snapshot).unwrap();
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 8;
    let control = Interrupt::cancel_only(&cancel);
    assert_eq!(lazy.try_write_interleaved(Rect::new(0, 0, 1, 1), &[99], &control), Err(RasterInputError::Cancelled));
    assert!(polls.load(Ordering::Relaxed) > 8);
    assert_eq!(lazy.materialized_tile_count(), 0);
    assert_eq!(lazy.snapshot().bytes(), &[17, 18]);
}
