#![cfg(windows)]
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, PersistentNativeStorage, VerifiedNativePlaneStore},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackedRaster, BackingError, NativePlaneDescriptor, NativePlaneStore, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, BudgetKind, ResourceLimits, ResourcePolicy};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-sol4-owned-native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir(&self.0) {
            eprintln!("private owned-native test directory cleanup: {e}");
        }
    }
}
fn new_quota() -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 4 << 20, scratch: 2 << 20, index: 1 << 20 })
}
fn budget() -> Budget<'static> {
    Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None)
}
fn pixel_byte(i: u64) -> u8 {
    i.wrapping_mul(73).wrapping_add(31) as u8
}
#[test]
fn actual_windows_store_outlives_source_provider_and_job_then_last_owner_removes_file() {
    let root = Root::new();
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(-2, -1, 508, 512), &[0; 4]).unwrap();
    let quota = BackingQuota::new(QuotaLimits { resident: 4 << 20, scratch: descriptor.logical_bytes(), index: 1 << 20 });
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let store = {
        let budget = budget();
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
        let source_path = root.0.join("original-native");
        let bytes: Vec<_> = (0..descriptor.logical_bytes()).map(pixel_byte).collect();
        std::fs::write(&source_path, &bytes).unwrap();
        drop(bytes);
        let mut source = File::open(&source_path).unwrap();
        let handle = directory.create_owned_native_storage(descriptor, &quota, &control).unwrap();
        assert_eq!(root.entries(), 2);
        let store = VerifiedNativePlaneStore::capture(handle, descriptor, quota.clone(), NativeStoreLimits::default(), &control, |offset, out, _| {
            source.seek(SeekFrom::Start(offset)).map_err(|_| BackingError::Io)?;
            source.read_exact(out).map_err(|_| BackingError::Io)
        })
        .unwrap();
        drop(source);
        std::fs::remove_file(&source_path).unwrap();
        drop(directory);
        assert_eq!(budget.used(BudgetKind::Resident), 0);
        store
    };
    assert_eq!(root.entries(), 1);
    let store = Arc::new(store);
    let retained = store.clone();
    drop(store);
    let baseline = quota.usage().unwrap();
    assert_eq!(baseline.scratch, descriptor.logical_bytes());
    let task_store = retained.clone();
    let job = std::thread::spawn(move || {
        let work = |_| Ok(());
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
        let mut out = [0; 8193];
        task_store.read_exact_at(4093, &mut out, &control).unwrap();
        for (i, v) in out.iter().enumerate() {
            assert_eq!(*v, pixel_byte(4093 + i as u64));
        }
    });
    job.join().unwrap();
    assert_eq!(quota.usage().unwrap(), baseline);
    let other = new_quota();
    assert_eq!(BackedRaster::new(retained.clone(), other.clone(), 1, &control).unwrap_err(), BackingError::Quota);
    assert_eq!(other.usage().unwrap(), QuotaUsage::default());
    assert_eq!(quota.usage().unwrap(), baseline);
    drop(retained);
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn actual_creation_and_late_capture_denial_cancel_remove_storage_and_refund() {
    let root = Root::new();
    let budget = budget();
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let job_base = budget.used(BudgetKind::Resident);
    let descriptor = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 129, 129), &[0; 4]).unwrap();
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let denied = BackingQuota::new(QuotaLimits::default());
    assert_eq!(directory.create_owned_native_storage(descriptor, &denied, &control).unwrap_err(), BackingError::Quota);
    assert_eq!(root.entries(), 0);
    let denied_scratch = BackingQuota::new(QuotaLimits { resident: 4 << 20, scratch: descriptor.logical_bytes() - 1, index: 1 << 20 });
    assert_eq!(directory.create_owned_native_storage(descriptor, &denied_scratch, &control).unwrap_err(), BackingError::Quota);
    assert_eq!(root.entries(), 0);
    assert_eq!(denied_scratch.usage().unwrap(), QuotaUsage::default());
    let quota = new_quota();
    let stopped = AtomicBool::new(false);
    let count = AtomicU64::new(0);
    let poll = || stopped.load(Ordering::SeqCst);
    let interrupt = Interrupt::cancel_only(&poll);
    let after_open = |_| {
        if count.fetch_add(1, Ordering::SeqCst) > 0 {
            assert_eq!(root.entries(), 1);
            stopped.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    assert_eq!(
        directory.create_owned_native_storage(descriptor, &quota, &ReadControl { interrupt: &interrupt, work: &after_open }).unwrap_err(),
        BackingError::Cancelled
    );
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    stopped.store(false, Ordering::SeqCst);
    let handle = directory.create_owned_native_storage(descriptor, &quota, &control).unwrap();
    let result = VerifiedNativePlaneStore::capture(
        handle,
        descriptor,
        quota.clone(),
        NativeStoreLimits::default(),
        &ReadControl { interrupt: &interrupt, work: &work },
        |offset, out, _| {
            out.fill(7);
            if offset >= 4096 {
                stopped.store(true, Ordering::SeqCst);
            }
            Ok(())
        },
    );
    assert!(matches!(result, Err(BackingError::Cancelled)));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    stopped.store(false, Ordering::SeqCst);
    let handle = directory.create_owned_native_storage(descriptor, &quota, &control).unwrap();
    let distinct = new_quota();
    assert!(matches!(
        VerifiedNativePlaneStore::capture(handle, descriptor, distinct.clone(), NativeStoreLimits::default(), &control, |_, _, _| Ok(())),
        Err(BackingError::Quota)
    ));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    assert_eq!(distinct.usage().unwrap(), QuotaUsage::default());
    let handle = directory.create_owned_native_storage(descriptor, &quota, &control).unwrap();
    let different = NativePlaneDescriptor::new(PixelFormat::RGBA8, Rect::new(0, 0, 128, 129), &[0; 4]).unwrap();
    let called = AtomicBool::new(false);
    assert!(matches!(
        VerifiedNativePlaneStore::capture(handle, different, quota.clone(), NativeStoreLimits::default(), &control, |_, _, _| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        }),
        Err(BackingError::Quota)
    ));
    assert!(!called.load(Ordering::SeqCst));
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    assert_eq!(budget.used(BudgetKind::Resident), job_base);
    drop(directory);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}
#[test]
fn concrete_handle_seal_has_no_public_write_alias_and_bounds_reads() {
    let root = Root::new();
    let budget = budget();
    let quota = new_quota();
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &budget).unwrap();
    let descriptor = NativePlaneDescriptor::new(
        photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U8, false),
        Rect::new(0, 0, 6, 1),
        &[0],
    )
    .unwrap();
    let mut handle = directory.create_owned_native_storage(descriptor, &quota, &control).unwrap();
    assert!(handle.validate_quota(&quota).is_ok());
    assert_eq!(handle.resident_envelope(), 0);
    assert_eq!(handle.write_all_at(1, b"gap"), Err(BackingError::Integrity));
    assert_eq!(handle.seal(), Err(BackingError::Integrity));
    assert_eq!(quota.usage().unwrap().scratch, 6);
    handle.write_all_at(0, b"native").unwrap();
    assert_eq!(handle.write_all_at(6, b"growth"), Err(BackingError::Bounds));
    assert_eq!(quota.usage().unwrap().scratch, 6);
    handle.seal().unwrap();
    drop(directory);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(handle.write_all_at(6, b"mutate"), Err(BackingError::Integrity));
    let mut out = [0; 6];
    handle.read_exact_at(0, &mut out).unwrap();
    assert_eq!(&out, b"native");
    assert_eq!(handle.read_exact_at(0, &mut [0; 4097]), Err(BackingError::Bounds));
    drop(handle);
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
