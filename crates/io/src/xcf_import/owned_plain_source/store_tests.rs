//! UNRUN generic retention-owner / exact native-store regression drafts.
use super::*;
use photocraft_raster::backing::quota::{QuotaKind, QuotaLease, QuotaLimits};
use photocraft_raster::{ColorMode, PixelFormat, SampleType};
use std::sync::atomic::{AtomicUsize, Ordering};

struct FirstOwner {
    quota: BackingQuota,
    drops: Arc<AtomicUsize>,
    _fee: QuotaLease,
}
impl Drop for FirstOwner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
impl RetainedSource for FirstOwner {
    fn actual_quota(&self) -> &BackingQuota {
        &self.quota
    }
}
struct SecondOwner(FirstOwner);
impl RetainedSource for SecondOwner {
    fn actual_quota(&self) -> &BackingQuota {
        &self.0.quota
    }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 4 << 20, index: 1 << 20, scratch: 0 })
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn owner(q: &BackingQuota, drops: &Arc<AtomicUsize>) -> FirstOwner {
    FirstOwner { quota: q.clone(), drops: drops.clone(), _fee: q.reserve(QuotaKind::Resident, 256).unwrap() }
}
fn fixture() -> (Surface, NativePlaneDescriptor) {
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
    let bounds = Rect::new(0, 0, 2, 1);
    let bytes = [0x7ff8000000001234u64, (-0.0f64).to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
    let surface = Surface::try_from_interleaved(format, bounds, &bytes).unwrap();
    let descriptor = validate(&surface, bounds, &control(), &|| Ok(())).unwrap();
    (surface, descriptor)
}
#[test]
fn concrete_owner_tags_keep_the_actual_owner_and_real_fee_alive() {
    let q = quota();
    let drops = Arc::new(AtomicUsize::new(0));
    let (surface, descriptor) = fixture();
    let retained = Arc::new(owner(&q, &drops));
    let weak = Arc::downgrade(&retained);
    let first = SurfaceStore::new(surface, descriptor, retained.clone(), q.clone());
    drop(retained);
    assert!(weak.upgrade().is_some());
    assert_eq!(q.usage().unwrap().resident, 256);
    first.validate_quota(&q).unwrap();
    let (surface, descriptor) = fixture();
    let second = SurfaceStore::new(surface, descriptor, Arc::new(SecondOwner(owner(&q, &drops))), q.clone());
    second.validate_quota(&q).unwrap();
    drop(first);
    assert!(weak.upgrade().is_none());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(q.usage().unwrap().resident, 256);
    drop(second);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn both_store_and_retention_ledgers_must_match() {
    let q = quota();
    let foreign = quota();
    let drops = Arc::new(AtomicUsize::new(0));
    let (surface, descriptor) = fixture();
    let store = SurfaceStore::new(surface, descriptor, Arc::new(owner(&foreign, &drops)), q.clone());
    assert!(matches!(store.validate_quota(&q), Err(BackingError::Quota)));
    assert!(matches!(store.validate_quota(&foreign), Err(BackingError::Quota)));
}
#[test]
fn generic_owner_preserves_full_native_nan_signed_zero_padding_and_bounded_visits() {
    let q = quota();
    let drops = Arc::new(AtomicUsize::new(0));
    let (surface, descriptor) = fixture();
    let coord = TileCoord::containing(0, 0);
    let expected = surface.tile(coord).unwrap().bytes().to_vec();
    let default = surface.default_bytes().to_vec();
    let store = SurfaceStore::new(surface, descriptor, Arc::new(SecondOwner(owner(&q, &drops))), q);
    assert_eq!(store.descriptor().default_bytes(), default);
    let mut copied = Vec::new();
    store
        .visit_native_tile(coord, &control(), &mut |offset, bytes| {
            assert!(bytes.len() <= 65536);
            assert_eq!(offset as usize, copied.len());
            copied.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
    assert_eq!(copied, expected);
    let mut raw = [0u8; 16];
    store.read_exact_at(0, &mut raw, &control()).unwrap();
    assert_eq!(raw[..8], 0x7ff8000000001234u64.to_ne_bytes());
    assert_eq!(raw[8..], (-0.0f64).to_bits().to_ne_bytes());
    let stopped = AtomicUsize::new(0);
    assert!(matches!(
        store.visit_native_tile(coord, &control(), &mut |_, _| {
            stopped.fetch_add(1, Ordering::Relaxed);
            Err(BackingError::Cancelled)
        }),
        Err(BackingError::Cancelled)
    ));
    assert_eq!(stopped.load(Ordering::Relaxed), 1);
}
#[test]
fn default_concrete_retention_keeps_the_previous_sized_arc_layout() {
    #[allow(dead_code)]
    struct PreviousLayout {
        surface: Surface,
        descriptor: NativePlaneDescriptor,
        retention: Arc<Retention>,
        quota: BackingQuota,
    }
    assert_eq!(std::mem::size_of::<SurfaceStore>(), std::mem::size_of::<PreviousLayout>());
    assert_eq!(std::mem::size_of::<SurfaceStore<FirstOwner>>(), std::mem::size_of::<SurfaceStore>());
    assert_eq!(std::mem::size_of::<SurfaceStore<SecondOwner>>(), std::mem::size_of::<SurfaceStore>());
}
