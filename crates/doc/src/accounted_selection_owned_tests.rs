//! UNRUN native archive ownership-transfer regression drafts.
use super::*;
use photocraft_raster::backing::quota::QuotaLimits;

fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 8 << 20, scratch: 8 << 20, index: 8 << 20 })
}

#[test]
fn adopted_f64_default_padding_words_and_original_guard_survive_view() {
    let q = quota();
    // Genuine codec fixture pays existing storage before constructing it.
    let retained = Arc::new(q.reserve(QuotaKind::Resident, (65536 * 8 + 4096) as u64).unwrap());
    let weak = Arc::downgrade(&retained);
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let default = 0.25f64.to_ne_bytes();
    let mut surface = Surface::try_with_default_bytes(format, &default).unwrap();
    let coordinate = TileCoord::new(0, 0);
    for pixel in surface.try_tile_mut(coordinate).unwrap().bytes_mut().as_chunks_mut::<8>().0 {
        pixel.copy_from_slice(&0.5000000000000001f64.to_ne_bytes());
    }
    let tile = surface.tile(coordinate).unwrap().clone();
    let native_before = q.usage().unwrap();
    let selected = AccountedSelection::adopt_codec_surface(surface, Rect::new(1, 1, 3, 2), retained, &q, &ctl()).unwrap();
    let after = q.usage().unwrap();
    assert_eq!(after.resident - after.index, native_before.resident - native_before.index); // Existing payload not charged again.
    assert_eq!(after.index - native_before.index, selected._allocation.bytes());
    let view = selected.clone_view(&ctl()).unwrap();
    assert_eq!(view.surface.default_bytes(), default);
    assert!(Arc::ptr_eq(view.surface.tile(coordinate).unwrap(), &tile));
    assert_eq!(&view.surface.tile(coordinate).unwrap().bytes()[..8], &0.5000000000000001f64.to_ne_bytes());
    drop(view);
    assert!(weak.upgrade().is_some());
    drop(tile);
    drop(selected);
    assert!(weak.upgrade().is_none());
}

#[test]
fn cancel_and_nonunit_native_words_refuse_without_selection_publication() {
    let q = quota();
    let retained = Arc::new(q.reserve(QuotaKind::Resident, 1 << 20).unwrap());
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let surface = Surface::try_with_default_bytes(format, &f64::NAN.to_ne_bytes()).unwrap();
    assert!(matches!(AccountedSelection::adopt_codec_surface(surface, Rect::new(0, 0, 2, 2), retained.clone(), &q, &ctl()), Err(BackingError::Unsupported)));
    let cancelled = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Err(BackingError::Cancelled) };
    let surface = Surface::try_with_default_bytes(format, &0.0f64.to_ne_bytes()).unwrap();
    assert!(matches!(AccountedSelection::adopt_codec_surface(surface, Rect::new(0, 0, 2, 2), retained, &q, &cancelled), Err(BackingError::Cancelled)));
}
