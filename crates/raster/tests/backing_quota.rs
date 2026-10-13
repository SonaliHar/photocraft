use photocraft_raster::backing::quota::{BackingQuota, QuotaError, QuotaKind, QuotaLease, QuotaLimits, QuotaUsage};
use std::sync::{Arc, Barrier};

#[test]
fn independent_exact_limits_and_index_resident_subquota() {
    let quota = BackingQuota::new(QuotaLimits { resident: 10, scratch: 5, index: 3 });
    let resident = quota.reserve(QuotaKind::Resident, 7).unwrap();
    let index = quota.reserve(QuotaKind::Index, 3).unwrap();
    let scratch = quota.reserve(QuotaKind::Scratch, 5).unwrap();
    assert_eq!(index.kind(), QuotaKind::Index);
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: 10, scratch: 5, index: 3 });
    for kind in [QuotaKind::Resident, QuotaKind::Scratch, QuotaKind::Index] {
        assert!(matches!(quota.reserve(kind, 1), Err(QuotaError::LimitExceeded(_))));
    }
    drop(resident);
    assert!(matches!(quota.reserve(QuotaKind::Index, 1), Err(QuotaError::LimitExceeded(QuotaKind::Index))));
    drop((index, scratch));
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    let zero = BackingQuota::new(QuotaLimits::default());
    drop(zero.reserve(QuotaKind::Resident, 0).unwrap());
    assert!(zero.reserve(QuotaKind::Resident, 1).is_err());
}
#[test]
fn overflow_and_capacity_growth_denial_are_atomic() {
    let quota = BackingQuota::new(QuotaLimits { resident: u64::MAX, scratch: u64::MAX, index: u64::MAX });
    let mut lease = quota.reserve(QuotaKind::Index, u64::MAX).unwrap();
    assert_eq!(quota.reserve(QuotaKind::Resident, 1).unwrap_err(), QuotaError::Overflow);
    assert_eq!(quota.usage().unwrap().resident, u64::MAX);
    lease.reconcile_capacity(8).unwrap();
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: 8, scratch: 0, index: 8 });
    assert_eq!(lease.reconcile_capacity(9), Err(QuotaError::InvalidCapacity));
    drop(lease);
    let small = BackingQuota::new(QuotaLimits { resident: 10, scratch: 0, index: 5 });
    let mut lease = small.reserve(QuotaKind::Index, 4).unwrap();
    assert_eq!(lease.try_resize(6), Err(QuotaError::LimitExceeded(QuotaKind::Index)));
    assert_eq!(lease.bytes(), 4);
    assert_eq!(small.usage().unwrap(), QuotaUsage { resident: 4, scratch: 0, index: 4 });
    lease.try_resize(5).unwrap();
    lease.reconcile_capacity(2).unwrap();
    drop(lease);
    assert_eq!(small.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn shared_physical_owner_has_one_last_drop_refund_and_cow_is_separate() {
    struct Payload {
        _bytes: Vec<u8>,
        _lease: QuotaLease,
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 16, scratch: 0, index: 0 });
    let lease = quota.reserve(QuotaKind::Resident, 8).unwrap();
    let payload = Arc::new(Payload { _bytes: vec![7; 8], _lease: lease });
    let copy = payload.clone();
    drop(payload);
    assert_eq!(quota.usage().unwrap().resident, 8);
    let cow = quota.reserve(QuotaKind::Resident, 8).unwrap();
    assert_eq!(quota.usage().unwrap().resident, 16);
    drop(copy);
    assert_eq!(quota.usage().unwrap().resident, 8);
    drop(cow);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn concurrent_reservations_observe_one_global_limit_and_refund() {
    let quota = BackingQuota::new(QuotaLimits { resident: 4, scratch: 0, index: 0 });
    let start = Arc::new(Barrier::new(9));
    let held = Arc::new(Barrier::new(9));
    let finish = Arc::new(Barrier::new(9));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let quota = quota.clone();
        let start = start.clone();
        let held = held.clone();
        let finish = finish.clone();
        handles.push(std::thread::spawn(move || {
            start.wait();
            let lease = quota.reserve(QuotaKind::Resident, 1);
            held.wait();
            finish.wait();
            lease.is_ok()
        }));
    }
    start.wait();
    held.wait();
    assert_eq!(quota.usage().unwrap().resident, 4);
    finish.wait();
    let successes = handles.into_iter().map(|h| usize::from(h.join().unwrap())).sum::<usize>();
    assert_eq!(successes, 4);
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn payload_storage_drops_before_lease_and_early_errors_refund() {
    struct ObservedStorage {
        bytes: Vec<u8>,
        quota: BackingQuota,
    }
    impl Drop for ObservedStorage {
        fn drop(&mut self) {
            assert_eq!(self.quota.usage().unwrap().resident, self.bytes.capacity() as u64);
        }
    }
    struct Owner {
        _storage: ObservedStorage,
        _lease: QuotaLease,
    }
    let quota = BackingQuota::new(QuotaLimits { resident: 16, scratch: 0, index: 0 });
    {
        let mut lease = quota.reserve(QuotaKind::Resident, 16).unwrap();
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(16).unwrap();
        lease.reconcile_capacity(bytes.capacity() as u64).unwrap();
        let _owner = Owner { _storage: ObservedStorage { bytes, quota: quota.clone() }, _lease: lease };
        assert!(quota.reserve(QuotaKind::Resident, 1).is_err());
    }
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    let fail_after_reservation = || -> Result<(), QuotaError> {
        let _lease = quota.reserve(QuotaKind::Resident, 16)?;
        quota.reserve(QuotaKind::Resident, 1)?;
        Ok(())
    };
    assert!(fail_after_reservation().is_err());
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
}

#[test]
fn equal_limits_are_not_shared_ownership_but_clones_are() {
    let limits = QuotaLimits { resident: 8, scratch: 4, index: 2 };
    let a = BackingQuota::new(limits);
    let b = BackingQuota::new(limits);
    assert!(a.shares_ledger(&a.clone()));
    assert!(!a.shares_ledger(&b));
    let lease = a.reserve(QuotaKind::Resident, 8).unwrap();
    assert_eq!(b.usage().unwrap(), QuotaUsage::default());
    drop(lease);
    assert_eq!(a.usage().unwrap(), QuotaUsage::default());
}
