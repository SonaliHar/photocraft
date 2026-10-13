use photocraft_color::PixelFormat;
use photocraft_geom::TileCoord;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

#[test]
fn validates_retained_keys_without_reading_samples_and_polls_during_scan() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut source = Surface::new(PixelFormat::RGBA64F);
    source.try_write_pixel_f64(0, 0, &[f64::NAN, -0.0, 1.0, 1.0]).unwrap();
    let tile = source.tiles().next().unwrap().1.clone();
    source.put_tiles((1..300).map(|tx| (TileCoord::new(tx, 0), tile.clone())));
    let before = source.clone();
    source.try_validate_tile_origins().unwrap();
    assert_eq!(source, before);
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 1;
    assert_eq!(source.try_validate_tile_origins_with(&Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(source, before);
    source.put_tiles([(TileCoord::new(i32::MIN, i32::MAX), tile)]);
    assert_eq!(source.try_validate_tile_origins(), Err(RasterInputError::InvalidBounds));
}
