//! UNRUN coordinate-view regressions. Generated immutable bytes are a trusted
//! allocation-free test provider; expected/output buffers belong to the harness.
use super::*;
use crate::backing::quota::QuotaLimits;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
struct Generated {
    descriptor: NativePlaneDescriptor,
    reads: AtomicU64,
}
fn byte(index: u64) -> u8 {
    ((index % 251) * 37 % 251) as u8
}
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(output.len() as u64)?;
        if offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)? > self.descriptor.logical_bytes() {
            return Err(BackingError::Length);
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        for (i, target) in output.iter_mut().enumerate() {
            *target = byte(offset + i as u64);
        }
        Ok(())
    }
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(format: PixelFormat, bounds: Rect) -> (BackedRaster, Arc<Generated>) {
    let quota = BackingQuota::new(QuotaLimits { resident: 64 << 20, index: 4 << 20, scratch: 0 });
    let default = [0x80; 40]; // Exact native default bits; no numeric interpretation.
    let store =
        Arc::new(Generated { descriptor: NativePlaneDescriptor::new(format, bounds, &default[..format.bytes_per_pixel()]).unwrap(), reads: AtomicU64::new(0) });
    let raster = BackedRaster::new_with_limits(store.clone(), quota, BackingLimits { cache_tiles: 2, override_tiles: 8 }, &ctl()).unwrap();
    (raster, store)
}

#[test]
fn unaligned_negative_target_tiles_preserve_every_native_word_and_default_exterior() {
    for format in [PixelFormat::RGBA16, PixelFormat::RGBA32, PixelFormat::RGBA32F, PixelFormat::RGBA64F] {
        let bounds = Rect::new(-3, -2, 260, 259);
        let (root, _) = fixture(format, bounds);
        let view = root.try_translated(-257, 255, &ctl()).unwrap();
        let bpp = format.bytes_per_pixel();
        for coord in view.descriptor().bounds().tiles() {
            let tile = tile_rect(coord).unwrap();
            let pin = view.pin_tile(coord, &ctl()).unwrap();
            assert_eq!(pin.bytes().len(), tile_length(format).unwrap());
            for y in tile.y0..tile.y1 {
                for x in tile.x0..tile.x1 {
                    let at = pixel_offset(tile, x, y, bpp).unwrap();
                    let actual = &pin.bytes()[at..at + bpp];
                    if view.descriptor().bounds().contains(x, y) {
                        let sx = x + 257;
                        let sy = y - 255;
                        let offset = ((sy - bounds.y0) as u64 * u64::from(bounds.width()) + (sx - bounds.x0) as u64) * bpp as u64;
                        let expected: Vec<_> = (0..bpp).map(|i| byte(offset + i as u64)).collect();
                        assert_eq!(actual, expected);
                    } else {
                        assert_eq!(actual, view.descriptor().default_bytes());
                    }
                }
            }
        }
    }
}

#[test]
fn fixed_view_composition_and_inverse_cow_preserve_history_and_raw_f64_bits() {
    let (root, _) = fixture(PixelFormat::RGBA64F, Rect::new(-2, -2, 260, 260));
    let first = root.try_translated(257, -259, &ctl()).unwrap();
    let mut view = first.try_translated(-256, 258, &ctl()).unwrap();
    assert!(view.shares_exact_state(&root.try_translated(1, -1, &ctl()).unwrap()));
    assert!(view.try_translated(-1, 1, &ctl()).unwrap().shares_exact_state(&root));
    assert!(Arc::ptr_eq(&root.store, &view.store));
    assert!(Arc::ptr_eq(&root.cache, &view.cache));
    let before = view.clone();
    let words = [(-0.0f64).to_ne_bytes(), 0.10000000000000002f64.to_ne_bytes(), 1.125f64.to_ne_bytes(), (-0.0f64).to_ne_bytes()].concat();
    view.write_region(Rect::new(0, 0, 1, 1), &words, &ctl()).unwrap();
    let mut actual = [0; 32];
    view.read_pixel_into(0, 0, &mut actual, &ctl()).unwrap();
    assert_eq!(&actual[..], words);
    let returned = view.try_translated(-1, 1, &ctl()).unwrap();
    returned.read_pixel_into(-1, 1, &mut actual, &ctl()).unwrap();
    assert_eq!(&actual[..], words);
    assert_eq!(view.override_count(), 1);
    assert_eq!(root.override_count(), 0);
    assert!(!view.shares_exact_state(&before));
    before.read_pixel_into(0, 0, &mut actual, &ctl()).unwrap();
    assert_ne!(&actual[..], words);
}

#[test]
fn large_logical_planes_and_checked_endpoints_have_no_override_count_admission() {
    let (root, store) = fixture(PixelFormat::RGBA8, Rect::new(0, 0, 32768, 32768));
    let view = root.try_translated(-13, 17, &ctl()).unwrap();
    assert_eq!(view.override_count(), 0);
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert_eq!(view.descriptor().logical_bytes(), root.descriptor().logical_bytes());
    assert!(root.try_translated(i32::MAX, 0, &ctl()).is_err());
    let (edge, _) = fixture(PixelFormat::RGBA8, Rect::new(i32::MIN, 0, i32::MIN + 1, 1));
    assert!(edge.try_translated(-1, 0, &ctl()).is_err());
    let far = root.try_translated(i32::MIN, 0, &ctl()).unwrap();
    // An exterior tile must not inverse-map its huge coordinates unnecessarily.
    let pin = far.pin_tile(TileCoord { tx: 0, ty: 0 }, &ctl()).unwrap();
    assert!(pin.bytes().as_chunks::<4>().0.iter().all(|pixel| pixel == far.descriptor().default_bytes()));
}

#[test]
fn denied_work_quota_and_late_cancellation_never_publish_cow_or_leak_payload_fees() {
    let (root, store) = fixture(PixelFormat::RGBA8, Rect::new(0, 0, 260, 260));
    let mut view = root.try_translated(-1, 1, &ctl()).unwrap();
    let before = view.clone();
    let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(matches!(view.pin_tile(TileCoord { tx: 0, ty: 0 }, &deny), Err(BackingError::Work)));
    assert_eq!(store.reads.load(Ordering::Relaxed), 0);
    assert!(view.write_region(Rect::new(0, 1, 1, 2), &[1; 4], &deny).is_err());
    assert!(view.shares_exact_state(&before));
    let q = view.quota().clone();
    let available = (64 << 20) - q.usage().unwrap().resident;
    let full = q.reserve(QuotaKind::Resident, available).unwrap();
    assert!(matches!(view.pin_tile(TileCoord { tx: 0, ty: 0 }, &ctl()), Err(BackingError::Quota)));
    drop(full);
    let calls = AtomicU64::new(0);
    let late = |_: u64| if calls.fetch_add(1, Ordering::Relaxed) >= 20 { Err(BackingError::Cancelled) } else { Ok(()) };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &late };
    assert!(matches!(view.pin_tile(TileCoord { tx: 0, ty: 0 }, &control), Err(BackingError::Cancelled)));
    view.clear_cache().unwrap();
    let usage = q.usage().unwrap();
    assert_eq!(usage.resident, usage.index); // Only original cache/COW index headers remain.
    assert!(view.shares_exact_state(&before));
}

#[test]
fn zero_view_preserves_the_existing_read_and_write_control_schedule() {
    let (root, _) = fixture(PixelFormat::RGBA8, Rect::new(0, 0, 4, 4));
    let zero = root.try_translated(0, 0, &ctl()).unwrap();
    assert!(root.shares_exact_state(&zero));
    // Both entry points reach their first legacy check before any provider IO.
    let work = AtomicU64::new(0);
    let deny = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(matches!(root.pin_tile(TileCoord { tx: 0, ty: 0 }, &c), Err(BackingError::Work)));
    assert_eq!(work.swap(0, Ordering::Relaxed), 1);
    assert!(matches!(zero.pin_tile(TileCoord { tx: 0, ty: 0 }, &c), Err(BackingError::Work)));
    assert_eq!(work.load(Ordering::Relaxed), 1);
}

#[test]
fn aligned_partial_tiles_share_actual_payload_and_exact_default_padding() {
    let (root, _) = fixture(PixelFormat::RGBA64F, Rect::new(-3, -2, 260, 259));
    let source = root.pin_tile(TileCoord { tx: -1, ty: -1 }, &ctl()).unwrap();
    let view = root.try_translated(256, -512, &ctl()).unwrap();
    let before = root.quota().usage().unwrap();
    let target = view.pin_tile(TileCoord { tx: 0, ty: -3 }, &ctl()).unwrap();
    assert!(Arc::ptr_eq(&source.payload, &target.payload));
    assert_eq!(root.quota().usage().unwrap(), before);
    assert_eq!(target.bytes(), source.bytes());
    let exterior = view.pin_tile(TileCoord { tx: 3, ty: 3 }, &ctl()).unwrap();
    assert!(exterior.bytes().as_chunks::<32>().0.iter().all(|p| p == view.descriptor().default_bytes()));
}
