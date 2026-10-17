#![cfg(windows)]
use photocraft_io::{native_plane_store::PersistentNativeStorage, xcf_source::ServiceAuthorizedScratchDirectory};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaLimits, QuotaUsage},
    },
};
use photocraft_xcf::{Budget, BudgetKind, ResourceLimits, ResourcePolicy};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("photocraft-owned-byte-service-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
            eprintln!("owned byte service test cleanup: {e}");
        }
    }
}
fn quota(scratch: u64) -> BackingQuota {
    BackingQuota::new(QuotaLimits { resident: 8192, scratch, index: 4096 })
}
fn budget() -> Budget<'static> {
    Budget::new(ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap(), None)
}

#[test]
fn arbitrary_exact_byte_length_owns_storage_after_provider_and_job_drop() {
    let root = Root::new();
    let quota = quota(4097);
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let mut handle = {
        let job = budget();
        let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
        let mut handle = directory.create_owned_byte_storage(4097, &quota, &control).unwrap();
        assert_eq!(root.entries(), 1);
        assert_eq!(quota.usage().unwrap().scratch, 4097);
        handle.write_all_at(0, &[0x57; 4096]).unwrap();
        handle.write_all_at(4096, &[0xa3]).unwrap();
        assert_eq!(handle.write_all_at(4097, &[1]), Err(BackingError::Bounds));
        handle.seal().unwrap();
        drop(directory);
        assert_eq!(job.used(BudgetKind::Resident), 0);
        handle
    };
    assert_eq!(handle.len().unwrap(), 4097);
    let mut tail = [0; 2];
    handle.read_exact_at(4095, &mut tail).unwrap();
    assert_eq!(tail, [0x57, 0xa3]);
    assert_eq!(handle.write_all_at(0, &[1]), Err(BackingError::Integrity));
    assert_eq!(root.entries(), 1);
    drop(handle);
    assert_eq!(root.entries(), 0);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn byte_service_refuses_bad_lengths_and_quota_before_file_creation() {
    let root = Root::new();
    let job = budget();
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let work = |_| Ok(());
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    let q = quota(4097);
    for length in [
        0,
        u64::from(photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS) * photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64 + 1,
        u64::MAX,
    ] {
        assert_eq!(directory.create_owned_byte_storage(length, &q, &control).unwrap_err(), BackingError::Bounds);
        assert_eq!(root.entries(), 0);
        assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    }
    let denied = quota(4096);
    assert_eq!(directory.create_owned_byte_storage(4097, &denied, &control).unwrap_err(), BackingError::Quota);
    assert_eq!(root.entries(), 0);
    assert_eq!(denied.usage().unwrap(), QuotaUsage::default());
    let denied_resident = BackingQuota::new(QuotaLimits { resident: 0, scratch: 4097, index: 4096 });
    assert_eq!(directory.create_owned_byte_storage(4097, &denied_resident, &control).unwrap_err(), BackingError::Quota);
    assert_eq!(root.entries(), 0);
    assert_eq!(denied_resident.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn byte_service_mid_open_cancellation_destroys_handle_before_refund() {
    let root = Root::new();
    let q = quota(4097);
    let job = budget();
    let directory = ServiceAuthorizedScratchDirectory::from_platform_authorized(root.0.clone(), &job).unwrap();
    let work = |_| if root.entries() == 1 { Err(BackingError::Cancelled) } else { Ok(()) };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert_eq!(directory.create_owned_byte_storage(4097, &q, &control).unwrap_err(), BackingError::Cancelled);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let cancelled = || true;
    let interrupt = Interrupt::cancel_only(&cancelled);
    let control = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert_eq!(directory.create_owned_byte_storage(4097, &q, &control).unwrap_err(), BackingError::Cancelled);
    assert_eq!(root.entries(), 0);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
