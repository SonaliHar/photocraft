//! Regression drafts. Authored without execution.
use super::*;

fn limits(header: u64, extra: u64) -> QuotaLimits {
    QuotaLimits { resident: header + extra, scratch: extra, index: header + extra }
}

#[test]
fn initial_index_is_part_of_resident_and_baseline_refunds_only_initial() {
    let h = BackingQuota::aggregate_header_bytes();
    let initial = QuotaUsage { resident: 10, index: 7, scratch: 3 };
    let (quota, baseline) = BackingQuota::new_with_aggregate_initial(limits(h, 13), h + 13, initial).unwrap();
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: h + 10, index: h + 7, scratch: 3 });
    assert!(quota.reserve(QuotaKind::Scratch, 1).is_err());
    drop(baseline);
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: h, index: h, scratch: 0 });
    let lease = quota.reserve(QuotaKind::Resident, 13).unwrap();
    assert!(quota.reserve(QuotaKind::Scratch, 1).is_err());
    drop(lease);
    assert_eq!(quota.usage().unwrap().resident, h);
}

#[test]
fn header_initial_categories_and_aggregate_are_admitted_before_arc_construction() {
    let h = BackingQuota::aggregate_header_bytes();
    assert!(h >= std::mem::size_of::<Mutex<State>>() as u64 + 128);
    let zero = QuotaUsage::default();
    assert!(BackingQuota::new_with_aggregate_initial(limits(h, 0), h - 1, zero).is_err());
    assert!(BackingQuota::new_with_aggregate_initial(QuotaLimits { index: h - 1, ..limits(h, 0) }, h, zero).is_err());
    assert!(BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: h - 1, ..limits(h, 0) }, h, zero).is_err());
    assert!(matches!(
        BackingQuota::new_with_aggregate_initial(limits(h, 10), h + 10, QuotaUsage { resident: 1, index: 2, scratch: 0 }),
        Err(QuotaError::Invariant)
    ));
    assert!(BackingQuota::new_with_aggregate_initial(limits(h, 10), h + 10, QuotaUsage { resident: 8, index: 2, scratch: 3 }).is_err());
    assert!(matches!(
        BackingQuota::new_with_aggregate_initial(
            QuotaLimits { resident: u64::MAX, scratch: u64::MAX, index: u64::MAX },
            u64::MAX,
            QuotaUsage { resident: u64::MAX, scratch: 0, index: 0 }
        ),
        Err(QuotaError::Overflow)
    ));
}

#[test]
fn rejected_reserve_and_growth_publish_neither_lease_nor_ledger_changes() {
    let h = BackingQuota::aggregate_header_bytes();
    let (quota, baseline) = BackingQuota::new_with_aggregate_initial(limits(h, 30), h + 12, QuotaUsage::default()).unwrap();
    let mut index = quota.reserve(QuotaKind::Index, 4).unwrap();
    let scratch = quota.reserve(QuotaKind::Scratch, 8).unwrap();
    let before = quota.usage().unwrap();
    assert!(quota.reserve(QuotaKind::Resident, 1).is_err());
    assert!(index.try_resize(5).is_err());
    assert_eq!(index.bytes(), 4);
    assert_eq!(quota.usage().unwrap(), before);
    index.try_resize(2).unwrap();
    let resident = quota.reserve(QuotaKind::Resident, 2).unwrap();
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: h + 4, scratch: 8, index: h + 2 });
    drop((resident, scratch, index, baseline));
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: h, scratch: 0, index: h });
}

#[test]
fn baseline_alone_keeps_actual_ledger_alive() {
    let h = BackingQuota::aggregate_header_bytes();
    let (quota, baseline) = BackingQuota::new_with_aggregate_initial(limits(h, 4), h + 4, QuotaUsage { resident: 3, scratch: 1, index: 2 }).unwrap();
    let weak = Arc::downgrade(&quota.state);
    drop(quota);
    assert!(weak.upgrade().is_some());
    drop(baseline);
    assert!(weak.upgrade().is_none());
}

#[test]
fn aggregate_reservations_compete_on_the_same_atomic_ledger() {
    let h = BackingQuota::aggregate_header_bytes();
    let (quota, _baseline) = BackingQuota::new_with_aggregate_initial(limits(h, 4), h + 3, QuotaUsage::default()).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let q = quota.clone();
        let b = barrier.clone();
        threads.push(std::thread::spawn(move || {
            b.wait();
            q.reserve(QuotaKind::Scratch, 2)
        }));
    }
    barrier.wait();
    let outcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(quota.usage().unwrap().scratch, 2);
    drop(outcomes);
    assert_eq!(quota.usage().unwrap().scratch, 0);
}

#[test]
fn ordinary_new_retains_independent_component_semantics() {
    let quota = BackingQuota::new(QuotaLimits { resident: 4, index: 4, scratch: 4 });
    assert_eq!(quota.usage().unwrap(), QuotaUsage::default());
    let _index = quota.reserve(QuotaKind::Index, 4).unwrap();
    let _scratch = quota.reserve(QuotaKind::Scratch, 4).unwrap();
    assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: 4, index: 4, scratch: 4 });
}
