use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_io::native_plane_store::{NATIVE_STORE_MAX_READ_BYTES, NativeStoreLimits, PersistentNativeStorage, VerifiedNativePlaneStore};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Debug, Default)]
struct Fault {
    corrupt: AtomicBool,
    io_after: AtomicU64,
    reads: AtomicU64,
    fail_seal: AtomicBool,
}
#[derive(Debug)]
struct MemoryStorage {
    bytes: Vec<u8>,
    sealed: bool,
    quota: BackingQuota,
    fault: Arc<Fault>,
    drops: Arc<Mutex<Vec<QuotaUsage>>>,
}
impl PersistentNativeStorage for MemoryStorage {
    fn resident_envelope(&self) -> u64 {
        self.bytes.capacity() as u64 + 256
    }
    fn resident_bytes(&self) -> u64 {
        self.bytes.capacity() as u64 + 256
    }
    fn len(&mut self) -> Result<u64, BackingError> {
        Ok(self.bytes.len() as u64)
    }
    fn read_exact_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), BackingError> {
        let number = self.fault.reads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fault.io_after.load(Ordering::SeqCst) != 0 && number >= self.fault.io_after.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        let start = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = start.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        output.copy_from_slice(self.bytes.get(start..end).ok_or(BackingError::Io)?);
        if self.fault.corrupt.load(Ordering::SeqCst)
            && let Some(first) = output.first_mut()
        {
            *first ^= 1;
        }
        Ok(())
    }
    fn write_all_at(&mut self, offset: u64, input: &[u8]) -> Result<(), BackingError> {
        if self.sealed || offset != self.bytes.len() as u64 || self.bytes.len() + input.len() > self.bytes.capacity() {
            return Err(BackingError::Integrity);
        }
        self.bytes.extend_from_slice(input);
        Ok(())
    }
    fn seal(&mut self) -> Result<(), BackingError> {
        if self.fault.fail_seal.load(Ordering::SeqCst) {
            return Err(BackingError::Io);
        }
        self.sealed = true;
        Ok(())
    }
}
impl Drop for MemoryStorage {
    fn drop(&mut self) {
        self.drops.lock().unwrap().push(self.quota.usage().unwrap());
    }
}
fn quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 4 << 20, scratch: 1 << 20, index: 1 << 20 })
}
fn memory(length: usize, quota: &BackingQuota, fault: Arc<Fault>, drops: Arc<Mutex<Vec<QuotaUsage>>>) -> MemoryStorage {
    MemoryStorage { bytes: Vec::with_capacity(length), sealed: false, quota: quota.clone(), fault, drops }
}
fn descriptor(format: PixelFormat) -> NativePlaneDescriptor {
    NativePlaneDescriptor::new(format, Rect::new(-3, -2, 130, 19), &vec![0; format.bytes_per_pixel()]).unwrap()
}
fn capture(
    format: PixelFormat,
    quota: &BackingQuota,
    fault: Arc<Fault>,
    drops: Arc<Mutex<Vec<QuotaUsage>>>,
    control: &ReadControl<'_>,
) -> (VerifiedNativePlaneStore<MemoryStorage>, Vec<u8>) {
    let descriptor = descriptor(format);
    let mut bytes: Vec<_> = (0..descriptor.logical_bytes() as usize).map(|i| ((i * 97 + 31) % 256) as u8).collect();
    bytes[..16].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0, 0, 240, 63]);
    let store = VerifiedNativePlaneStore::capture(
        memory(bytes.len(), quota, fault, drops),
        descriptor,
        quota.clone(),
        NativeStoreLimits::default(),
        control,
        |offset, out, _| {
            let start = offset as usize;
            out.copy_from_slice(&bytes[start..start + out.len()]);
            Ok(())
        },
    )
    .unwrap();
    (store, bytes)
}
#[test]
fn exact_wide_native_bytes_lifetime_and_bounded_reads() {
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let quota = quota();
            let drops = Arc::new(Mutex::new(Vec::new()));
            let (store, bytes) = capture(PixelFormat::new(mode, sample, true), &quota, Arc::new(Fault::default()), drops.clone(), &control);
            let store = Arc::new(store);
            assert_eq!(store.descriptor().logical_bytes(), bytes.len() as u64);
            assert_eq!(quota.usage().unwrap().scratch, bytes.len() as u64);
            assert!(quota.usage().unwrap().index > 0);
            let other = store.clone();
            drop(store);
            for offset in [0, 1, 4093, bytes.len() - 7] {
                let length = (bytes.len() - offset).min(8193);
                let mut out = vec![0; length];
                other.read_exact_at(offset as u64, &mut out, &control).unwrap();
                assert_eq!(out, &bytes[offset..offset + length]);
            }
            let mut too_big = vec![91; NATIVE_STORE_MAX_READ_BYTES + 1];
            assert_eq!(other.read_exact_at(0, &mut too_big, &control), Err(BackingError::Bounds));
            assert!(too_big.iter().all(|v| *v == 91));
            drop(other);
            assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
            let usage = drops.lock().unwrap()[0];
            assert!(usage.resident > 0);
            assert_eq!(usage.scratch, bytes.len() as u64);
        }
    }
}
#[test]
fn late_io_integrity_work_cancel_and_quota_do_not_modify_output_or_leak() {
    let quota = quota();
    let fault = Arc::new(Fault::default());
    let drops = Arc::new(Mutex::new(Vec::new()));
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let (store, _) = capture(PixelFormat::RGBA8, &quota, fault.clone(), drops.clone(), &control);
    let baseline = quota.usage().unwrap();
    fault.io_after.store(fault.reads.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    let mut output = vec![77; 6000];
    assert_eq!(store.read_exact_at(0, &mut output, &control), Err(BackingError::Io));
    assert_eq!(output, vec![77; 6000]);
    assert_eq!(quota.usage().unwrap(), baseline);
    fault.io_after.store(0, Ordering::SeqCst);
    fault.corrupt.store(true, Ordering::SeqCst);
    assert_eq!(store.read_exact_at(0, &mut output, &control), Err(BackingError::Integrity));
    assert_eq!(output, vec![77; 6000]);
    fault.corrupt.store(false, Ordering::SeqCst);
    let units = AtomicU64::new(0);
    let denied = |n| {
        if units.fetch_add(n, Ordering::SeqCst) > 30_000 { Err(BackingError::Work) } else { Ok(()) }
    };
    assert_eq!(store.read_exact_at(0, &mut output, &ReadControl { interrupt: &Interrupt::NONE, work: &denied }), Err(BackingError::Work));
    let calls = AtomicU64::new(0);
    let cancelled = || calls.fetch_add(1, Ordering::SeqCst) > 7;
    let interrupt = Interrupt::cancel_only(&cancelled);
    assert_eq!(store.read_exact_at(0, &mut output, &ReadControl { interrupt: &interrupt, work: &work }), Err(BackingError::Cancelled));
    assert_eq!(output, vec![77; 6000]);
    assert_eq!(quota.usage().unwrap(), baseline);
    let blocker = quota.reserve(photocraft_raster::backing::quota::QuotaKind::Resident, (4 << 20) - baseline.resident).unwrap();
    assert_eq!(store.read_exact_at(0, &mut output, &control), Err(BackingError::Quota));
    assert_eq!(output, vec![77; 6000]);
    drop(blocker);
    drop(store);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn capture_failure_destroys_storage_before_refunds_and_never_publishes() {
    let format = PixelFormat::RGBA8;
    let descriptor = descriptor(format);
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    for fault_kind in 0..4 {
        let quota = quota();
        let fault = Arc::new(Fault::default());
        let drops = Arc::new(Mutex::new(Vec::new()));
        if fault_kind == 0 {
            fault.fail_seal.store(true, Ordering::SeqCst);
        }
        if fault_kind == 1 {
            fault.corrupt.store(true, Ordering::SeqCst);
        }
        let result = VerifiedNativePlaneStore::capture(
            memory(descriptor.logical_bytes() as usize, &quota, fault, drops.clone()),
            descriptor,
            quota.clone(),
            NativeStoreLimits::default(),
            &control,
            |offset, out, _| {
                if fault_kind == 2 && offset > 0 {
                    return Err(BackingError::Io);
                }
                if fault_kind == 3 && offset > 0 {
                    return Err(BackingError::Cancelled);
                }
                out.fill(17);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
        assert_eq!(drops.lock().unwrap()[0].scratch, descriptor.logical_bytes());
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 1 << 20, scratch: 0, index: 1 << 20 });
    let drops = Arc::new(Mutex::new(Vec::new()));
    let result = VerifiedNativePlaneStore::capture(
        memory(descriptor.logical_bytes() as usize, &quota, Arc::new(Fault::default()), drops.clone()),
        descriptor,
        quota.clone(),
        NativeStoreLimits::default(),
        &control,
        |_, _, _| Ok(()),
    );
    assert!(matches!(result, Err(BackingError::Quota)));
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    assert!(drops.lock().unwrap()[0].resident > 0);
}
#[test]
fn existing_backed_raster_uses_store_and_outlives_capture_source() {
    let quota = quota();
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let (store, source) = capture(PixelFormat::RGBA8, &quota, Arc::new(Fault::default()), Arc::new(Mutex::new(Vec::new())), &control);
    let expected = source[..4].to_vec();
    drop(source);
    let raster = BackedRaster::new(Arc::new(store), quota.clone(), 1, &control).unwrap();
    let mut out = [0; 4];
    raster.read_pixel_into(-3, -2, &mut out, &control).unwrap();
    assert_eq!(out, expected.as_slice());
    let pin = raster.pin_tile(TileCoord::containing(-3, -2), &control).unwrap();
    raster.clear_cache().unwrap();
    drop(raster);
    assert!(quota.usage().unwrap().resident > 0);
    assert_eq!(quota.usage().unwrap().scratch, 0);
    drop(pin);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn bounded_index_and_concurrent_reads_keep_one_storage_lease() {
    let quota = quota();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let descriptor = descriptor(PixelFormat::RGBA8);
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let result = VerifiedNativePlaneStore::capture(
        memory(descriptor.logical_bytes() as usize, &quota, Arc::new(Fault::default()), drops.clone()),
        descriptor,
        quota.clone(),
        NativeStoreLimits { max_blocks: 1 },
        &control,
        |_, _, _| Ok(()),
    );
    assert!(matches!(result, Err(BackingError::Bounds)));
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    let (store, bytes) = capture(PixelFormat::RGBA8, &quota, Arc::new(Fault::default()), drops, &control);
    let store = Arc::new(store);
    let baseline = quota.usage().unwrap();
    let mut jobs = Vec::new();
    for offset in [1usize, 4093, 8193, 0] {
        let store = store.clone();
        let expected = bytes[offset..offset + 1500].to_vec();
        jobs.push(std::thread::spawn(move || {
            let work = |_| Ok(());
            let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
            let mut output = vec![0; 1500];
            store.read_exact_at(offset as u64, &mut output, &control).unwrap();
            assert_eq!(output, expected);
        }));
    }
    for job in jobs {
        job.join().unwrap();
    }
    assert_eq!(quota.usage().unwrap(), baseline);
    let mut output = [123; 3];
    for offset in [u64::MAX, descriptor.logical_bytes() - 1] {
        assert_eq!(store.read_exact_at(offset, &mut output, &control), Err(BackingError::Bounds));
        assert_eq!(output, [123; 3]);
    }
    drop(store);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
