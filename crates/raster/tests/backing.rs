use backing::quota::{BackingQuota, QuotaKind, QuotaLimits};
use backing::{BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, backing};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
fn unlimited(_: u64) -> Result<(), BackingError> {
    Ok(())
}
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &unlimited }
}
fn quota(resident: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident, scratch: 1 << 20, index: 1 << 20 })
}
#[derive(Debug)]
struct Generated {
    descriptor: NativePlaneDescriptor,
    failure: AtomicBool,
    last: AtomicU64,
    dropped: Arc<AtomicBool>,
}
impl Drop for Generated {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl NativePlaneStore for Generated {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(0)?;
        if self.failure.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        if offset.checked_add(output.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Length);
        }
        self.last.store(offset, Ordering::SeqCst);
        for (i, b) in output.iter_mut().enumerate() {
            *b = offset.wrapping_add(i as u64).wrapping_mul(37) as u8;
        }
        Ok(())
    }
}
fn generated(format: PixelFormat, bounds: Rect, default: &[u8]) -> Arc<Generated> {
    Arc::new(Generated {
        descriptor: NativePlaneDescriptor::new(format, bounds, default).unwrap(),
        failure: AtomicBool::new(false),
        last: AtomicU64::new(0),
        dropped: Arc::new(AtomicBool::new(false)),
    })
}
#[test]
fn logical_plane_need_not_fit_resident() {
    // Sparse mathematical store tests U64 addressing, not a real >4GiB migration.
    let store = generated(PixelFormat::RGBA64F, Rect::new(0, 0, 32768, 32768), &[0; 32]);
    assert!(store.descriptor.logical_bytes() > 2_u64 << 30);
    let q = quota(9 << 20);
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &control()).unwrap();
    let a = raster.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    let b = raster.pin_tile(TileCoord::new(127, 127), &control()).unwrap();
    assert_eq!(a.bytes().len(), 256 * 256 * 32);
    assert_eq!(b.bytes().len(), a.bytes().len());
    assert!(store.last.load(Ordering::SeqCst) > u64::from(u32::MAX));
    assert!(q.usage().unwrap().resident < 5 << 20);
    drop((a, b, raster, store));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn pin_lease_survives_eviction_and_last_owner() {
    let q = quota(2 << 20);
    let raster = BackedRaster::new(generated(PixelFormat::RGBA8, Rect::new(0, 0, 768, 256), &[0; 4]), q.clone(), 1, &control()).unwrap();
    let pin = raster.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    let shared = pin.clone();
    raster.clear_cache().unwrap();
    let base = q.usage().unwrap().resident;
    assert!(base >= 256 * 256 * 4);
    let other = raster.pin_tile(TileCoord::new(1, 0), &control()).unwrap();
    drop(raster);
    assert!(q.usage().unwrap().resident >= 2 * 256 * 256 * 4);
    drop(pin);
    assert!(q.usage().unwrap().resident >= 2 * 256 * 256 * 4);
    drop(shared);
    assert_eq!(q.usage().unwrap().resident, (256 * 256 * 4 + 128) as u64);
    drop(other);
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn all_formats_defaults_negative_offsets_and_exact_cow_history() {
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let format = PixelFormat::new(mode, sample, true);
            let bpp = format.bytes_per_pixel();
            let default = vec![0x80; bpp];
            let store = generated(format, Rect::new(-2, -1, 260, 258), &default);
            let q = quota(40 << 20);
            let mut raster = BackedRaster::new(store, q.clone(), 2, &control()).unwrap();
            assert_eq!(raster.descriptor().format(), format);
            let history = raster.clone();
            assert_eq!(raster.override_count(), 0);
            let mut pixel = vec![0; bpp];
            raster.read_pixel_into(-3, -1, &mut pixel, &control()).unwrap();
            assert_eq!(pixel, default);
            raster.clear_cache().unwrap();
            let exact: Vec<u8> = (0..bpp).map(|i| if sample == SampleType::F64 && i % 8 == 7 { 0x80 } else { i as u8 }).collect();
            raster.write_region(Rect::new(-2, -1, -1, 0), &exact, &control()).unwrap();
            raster.read_pixel_into(-2, -1, &mut pixel, &control()).unwrap();
            assert_eq!(pixel, exact);
            assert_eq!(raster.override_count(), 1);
            assert!(!raster.shares_exact_state(&history));
            history.read_pixel_into(-2, -1, &mut pixel, &control()).unwrap();
            assert_eq!(pixel, (0..bpp).map(|i| (i as u64 * 37) as u8).collect::<Vec<_>>());
            let after = raster.clone();
            raster.clear_cache().unwrap();
            assert!(raster.shares_exact_state(&after));
            drop((after, raster, history));
            assert_eq!(q.usage().unwrap().resident, 0);
        }
    }
}
#[test]
fn io_failure_is_not_default_and_retry_is_explicit() {
    let store = generated(PixelFormat::RGBA8, Rect::new(0, 0, 512, 256), &[9; 4]);
    let q = quota(4 << 20);
    let raster = BackedRaster::new(store.clone(), q.clone(), 2, &control()).unwrap();
    let base = q.usage().unwrap().resident;
    store.failure.store(true, Ordering::SeqCst);
    let mut output = [77; 4];
    assert_eq!(raster.read_pixel_into(0, 0, &mut output, &control()), Err(BackingError::Io));
    assert_eq!(output, [77; 4]);
    assert_eq!(q.usage().unwrap().resident, base);
    store.failure.store(false, Ordering::SeqCst);
    raster.read_pixel_into(0, 0, &mut output, &control()).unwrap();
    assert_ne!(output, [9; 4]);
}
#[test]
fn cancel_and_work_failure_leave_edit_state_exact() {
    let q = quota(8 << 20);
    let mut raster = BackedRaster::new(generated(PixelFormat::RGBA8, Rect::new(0, 0, 512, 256), &[0; 4]), q.clone(), 2, &control()).unwrap();
    let before = raster.clone();
    let work = AtomicU64::new(0);
    let fail = |n: u64| {
        if work.fetch_add(n, Ordering::SeqCst) > 280000 { Err(BackingError::Work) } else { Ok(()) }
    };
    let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &fail };
    let bytes = vec![6; 512 * 4];
    assert_eq!(raster.write_region(Rect::new(0, 0, 512, 1), &bytes, &ctl), Err(BackingError::Work));
    assert!(raster.shares_exact_state(&before));
    raster.clear_cache().unwrap();
    let cancel = || true;
    let interrupt = Interrupt::cancel_only(&cancel);
    assert_eq!(raster.write_region(Rect::new(0, 0, 1, 1), &[2; 4], &ReadControl { interrupt: &interrupt, work: &unlimited }), Err(BackingError::Cancelled));
    assert!(raster.shares_exact_state(&before));
    drop((raster, before));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn quota_denial_is_atomic_and_no_hidden_whole_materialization() {
    let q = quota(600000);
    let mut raster = BackedRaster::new(generated(PixelFormat::RGBA8, Rect::new(0, 0, 512, 256), &[0; 4]), q.clone(), 1, &control()).unwrap();
    let before = raster.clone();
    let pin = raster.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    assert_eq!(raster.write_region(Rect::new(0, 0, 1, 1), &[5; 4], &control()), Err(BackingError::Quota));
    assert!(raster.shares_exact_state(&before));
    assert_eq!(pin.bytes().get(..4).unwrap(), &[0, 37, 74, 111]);
    drop((raster, before, pin));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn distinct_store_comparison_fallible_and_cache_inert() {
    let q = quota(4 << 20);
    let mut a = BackedRaster::new(generated(PixelFormat::GRAYA8, Rect::new(-1, -1, 2, 2), &[7; 2]), q.clone(), 1, &control()).unwrap();
    let b = BackedRaster::new(generated(PixelFormat::GRAYA8, Rect::new(-1, -1, 2, 2), &[7; 2]), q.clone(), 1, &control()).unwrap();
    assert!(!a.shares_exact_state(&b));
    assert!(a.try_content_equal(&b, &control()).unwrap());
    a.write_region(Rect::new(0, 0, 1, 1), &[1, 2], &control()).unwrap();
    assert!(!a.try_content_equal(&b, &control()).unwrap());
}
#[test]
fn store_outlives_capture_scope_and_reads_across_threads() {
    let q = quota(4 << 20);
    let store = generated(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]);
    let dropped = store.dropped.clone();
    let raster = BackedRaster::new(store, q.clone(), 1, &control()).unwrap();
    let shared = raster.clone();
    let thread = std::thread::spawn(move || {
        let mut bytes = [0; 4];
        shared.read_pixel_into(0, 0, &mut bytes, &control()).unwrap();
        bytes
    });
    assert_eq!(thread.join().unwrap(), [0, 37, 74, 111]);
    assert!(!dropped.load(Ordering::SeqCst));
    drop(raster);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[derive(Debug)]
struct Short {
    descriptor: NativePlaneDescriptor,
}
impl NativePlaneStore for Short {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, _: u64, _: &mut [u8], _: &ReadControl<'_>) -> Result<(), BackingError> {
        Err(BackingError::Integrity)
    }
}
#[test]
fn bounds_short_integrity_and_descriptor_errors() {
    assert_eq!(NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::EMPTY, &[0; 4]), Err(BackingError::Bounds));
    assert_eq!(NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 3]), Err(BackingError::Bounds));
    let raster = BackedRaster::new(
        Arc::new(Short { descriptor: NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]).unwrap() }),
        quota(1 << 20),
        1,
        &control(),
    )
    .unwrap();
    assert_eq!(raster.read_pixel_into(0, 0, &mut [0; 4], &control()), Err(BackingError::Integrity));
    assert_eq!(raster.pin_tile(TileCoord::new(i32::MAX, 0), &control()).unwrap_err(), BackingError::Bounds);
    assert_eq!(raster.read_pixel_into(0, 0, &mut [0; 3], &control()), Err(BackingError::Length));
    assert_ne!(BackingError::Unsupported, BackingError::Io);
    let q = quota(1000);
    let lease = q.reserve(QuotaKind::Scratch, 5).unwrap();
    drop(lease);
    assert_eq!(q.usage().unwrap().scratch, 0);
}
#[test]
fn small_generated_plane_admission_and_exact_capacity_cache_budget() {
    let q = quota(2 << 20);
    let store = generated(PixelFormat::RGBA8, Rect::new(0, 0, 4096, 4096), &[0; 4]);
    assert!(q.reserve(QuotaKind::Resident, store.descriptor.logical_bytes()).is_err());
    let raster = BackedRaster::new(store, q.clone(), 2, &control()).unwrap();
    let left = raster.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    let right = raster.pin_tile(TileCoord::new(15, 15), &control()).unwrap();
    assert_eq!(left.bytes().len(), right.bytes().len());
    assert!(q.usage().unwrap().resident < 1 << 20);
    drop((left, right, raster));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[derive(Debug)]
struct OwnedBytes {
    descriptor: NativePlaneDescriptor,
    bytes: Vec<u8>,
    _lease: backing::quota::QuotaLease,
}
impl NativePlaneStore for OwnedBytes {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], ctl: &ReadControl<'_>) -> Result<(), BackingError> {
        ctl.check(output.len() as u64)?;
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        output.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        Ok(())
    }
}
#[test]
fn original_mutation_independence_f64_payload_and_low_u32_bits() {
    for (format, original) in [
        (PixelFormat::RGBA64F, [(-0.0f64).to_ne_bytes(), 0.10000000000000002f64.to_ne_bytes(), 1.125f64.to_ne_bytes(), 0.0f64.to_ne_bytes()].concat()),
        (PixelFormat::RGBA32, [1u32.to_ne_bytes(), 2u32.to_ne_bytes(), u32::MAX.to_ne_bytes(), 0u32.to_ne_bytes()].concat()),
    ] {
        let q = quota(8 << 20);
        let mut external = original.clone();
        let bytes = external.clone();
        let lease = q.reserve(QuotaKind::Resident, bytes.capacity() as u64).unwrap();
        assert_eq!(lease.bytes(), bytes.capacity() as u64);
        assert_eq!(lease.kind(), QuotaKind::Resident);
        let descriptor = NativePlaneDescriptor::new(format, Rect::new(-1, -1, 0, 0), &vec![0; format.bytes_per_pixel()]).unwrap();
        let raster = BackedRaster::new(Arc::new(OwnedBytes { descriptor, bytes, _lease: lease }), q.clone(), 1, &control()).unwrap();
        external.fill(255);
        drop(external);
        let mut output = vec![0; original.len()];
        raster.read_pixel_into(-1, -1, &mut output, &control()).unwrap();
        assert_eq!(output, original);
        drop(raster);
        assert_eq!(q.usage().unwrap().resident, 0);
    }
}
#[test]
fn mid_copy_cancel_keeps_history_and_releases_staged_payload() {
    let q = quota(8 << 20);
    let mut raster = BackedRaster::new(generated(PixelFormat::RGBA8, Rect::new(0, 0, 512, 256), &[0; 4]), q.clone(), 2, &control()).unwrap();
    let before = raster.clone();
    // Seed base cache: cancellation now occurs during actual private COW copying.
    raster.pin_tile(TileCoord::new(0, 0), &control()).unwrap();
    let count = AtomicU64::new(0);
    let cancel = || count.fetch_add(1, Ordering::SeqCst) > 20;
    let interrupt = Interrupt::cancel_only(&cancel);
    let ctl = ReadControl { interrupt: &interrupt, work: &unlimited };
    assert_eq!(raster.write_region(Rect::new(0, 0, 1, 1), &[8; 4], &ctl), Err(BackingError::Cancelled));
    assert!(raster.shares_exact_state(&before));
    raster.clear_cache().unwrap();
    drop((raster, before));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn missing_store_data_never_inserts_cached_placeholder() {
    let q = quota(1 << 20);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 2, 1), &[19; 4]).unwrap();
    let source = vec![1, 2, 3, 4]; // claimed plane deliberately truncated: trusted-contract violation must still error.
    let lease = q.reserve(QuotaKind::Resident, source.capacity() as u64).unwrap();
    let raster = BackedRaster::new(Arc::new(OwnedBytes { descriptor, bytes: source, _lease: lease }), q.clone(), 1, &control()).unwrap();
    let baseline = q.usage().unwrap().resident;
    for _ in 0..2 {
        let mut output = [33; 4];
        assert_eq!(raster.read_pixel_into(1, 0, &mut output, &control()), Err(BackingError::Io));
        assert_eq!(output, [33; 4]);
        assert_eq!(q.usage().unwrap().resident, baseline);
    }
}
#[test]
fn full_override_limit_allows_replacement_but_denies_new_key() {
    let q = quota(8 << 20);
    let store = generated(PixelFormat::RGBA8, Rect::new(0, 0, 768, 256), &[0; 4]);
    let mut raster = BackedRaster::new_with_limits(store, q.clone(), backing::BackingLimits { cache_tiles: 2, override_tiles: 2 }, &control()).unwrap();
    raster.write_region(Rect::new(0, 0, 1, 1), &[1; 4], &control()).unwrap();
    raster.write_region(Rect::new(256, 0, 257, 1), &[2; 4], &control()).unwrap();
    let history = raster.clone();
    assert_eq!(raster.override_count(), 2);
    raster.write_region(Rect::new(0, 0, 1, 1), &[3; 4], &control()).unwrap();
    let after = raster.clone();
    assert_eq!(raster.write_region(Rect::new(512, 0, 513, 1), &[4; 4], &control()), Err(BackingError::Quota));
    assert!(raster.shares_exact_state(&after));
    let mut bytes = [0; 4];
    history.read_pixel_into(0, 0, &mut bytes, &control()).unwrap();
    assert_eq!(bytes, [1; 4]);
    raster.read_pixel_into(0, 0, &mut bytes, &control()).unwrap();
    assert_eq!(bytes, [3; 4]);
    drop((raster, history, after));
    assert_eq!(q.usage().unwrap().resident, 0);
}
#[test]
fn descriptor_extreme_u64_arithmetic_is_checked() {
    assert_eq!(NativePlaneDescriptor::new(PixelFormat::RGBA64F, Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), &[0; 32]), Err(BackingError::Bounds));
    let safe = NativePlaneDescriptor::new(PixelFormat::GRAY8, Rect::new(i32::MIN, 0, i32::MAX, 1), &[0]).unwrap();
    assert_eq!(safe.logical_bytes(), u64::from(u32::MAX));
}

#[derive(Debug)]
struct LedgerStore {
    base: Arc<Generated>,
    quota: BackingQuota,
    validations: AtomicU64,
}
impl NativePlaneStore for LedgerStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.base.descriptor()
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        // This constructor test requires validation before metadata reservation.
        assert_eq!(quota.usage().unwrap().resident, 0);
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        self.base.read_exact_at(offset, output, control)
    }
}
#[test]
fn persistent_store_denies_mixed_ledger_before_cache_reservation() {
    let retained = quota(4 << 20);
    let unrelated = quota(4 << 20); // identical limits are not ledger identity
    let store =
        Arc::new(LedgerStore { base: generated(PixelFormat::RGBA8, Rect::new(0, 0, 1, 1), &[0; 4]), quota: retained.clone(), validations: AtomicU64::new(0) });
    assert!(matches!(BackedRaster::new(store.clone(), unrelated.clone(), 1, &control()), Err(BackingError::Quota)));
    assert_eq!(unrelated.usage().unwrap().resident, 0);
    assert_eq!(retained.usage().unwrap().resident, 0);
    assert_eq!(store.validations.load(Ordering::SeqCst), 1);
    assert_eq!(store.base.last.load(Ordering::SeqCst), 0);
    let current = BackedRaster::new(store.clone(), retained.clone(), 1, &control()).unwrap();
    assert!(retained.usage().unwrap().resident > 0);
    assert_eq!(store.validations.load(Ordering::SeqCst), 2);
    drop(current);
    assert_eq!(retained.usage().unwrap().resident, 0);
}
