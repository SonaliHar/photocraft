//! UNRUN prospective/actual existing constructor budget boundary drafts.
use super::*;
use quota::QuotaLimits;
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Debug)]
struct Store {
    descriptor: NativePlaneDescriptor,
    reads: AtomicUsize,
}
impl NativePlaneStore for Store {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Err(BackingError::Io)
    }
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
#[test]
fn actual_constructor_peak_uses_the_same_prospective_reservations_without_reads() {
    for count in [1, 64, MAX_ENTRIES] {
        let limits = BackingLimits { cache_tiles: count, override_tiles: MAX_ENTRIES };
        let requirement = BackedRaster::metadata_reservation_bytes(limits).unwrap();
        assert_eq!(requirement, entry_reservation_bytes(0).unwrap() + entry_reservation_bytes(count).unwrap());
        for enough in [false, true] {
            let available = requirement - u64::from(!enough);
            let q = BackingQuota::new(QuotaLimits { resident: available, index: available, scratch: 0 });
            let store = Arc::new(Store {
                descriptor: NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]).unwrap(),
                reads: AtomicUsize::new(0),
            });
            let result = BackedRaster::new_with_limits(store.clone(), q.clone(), limits, &control());
            if enough {
                assert!(result.is_ok());
                assert!(q.usage().unwrap().index <= requirement);
            } else {
                assert!(matches!(result, Err(BackingError::Quota)));
            }
            assert_eq!(store.reads.load(Ordering::Relaxed), 0);
            drop(result);
            assert_eq!(q.usage().unwrap(), quota::QuotaUsage::default());
        }
    }
}
#[test]
fn prospective_limits_match_constructor_refusals_and_checked_counts() {
    for limits in [
        BackingLimits { cache_tiles: 0, override_tiles: 1 },
        BackingLimits { cache_tiles: MAX_ENTRIES + 1, override_tiles: 1 },
        BackingLimits { cache_tiles: 1, override_tiles: 0 },
        BackingLimits { cache_tiles: 1, override_tiles: MAX_ENTRIES + 1 },
    ] {
        assert_eq!(BackedRaster::metadata_reservation_bytes(limits), Err(BackingError::Bounds));
        let q = BackingQuota::new(QuotaLimits { resident: u64::MAX, index: u64::MAX, scratch: 0 });
        let store =
            Arc::new(Store { descriptor: NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]).unwrap(), reads: AtomicUsize::new(0) });
        assert!(matches!(BackedRaster::new_with_limits(store, q.clone(), limits, &control()), Err(BackingError::Bounds)));
        assert_eq!(q.usage().unwrap(), quota::QuotaUsage::default());
    }
    if usize::BITS >= 64 {
        assert_eq!(entry_reservation_bytes(usize::MAX), Err(BackingError::Quota));
    }
}
