//! UNRUN persistent-ledger drafts; no decoder or source-publication admission.
use super::*;
use crate::ResourcePolicy;
use std::cell::Cell;
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::default()).unwrap()
}
#[test]
fn default_local_receipt_and_lower_only_policy_remain_unchanged() {
    let limits = limits();
    assert!(limits.lowered(&[("residentBytes", limits.get(BudgetKind::Resident) + 1)]).is_err());
    let budget = Budget::new(limits, None);
    assert!(budget.shared.is_none());
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    let mut r = budget.reserve(BudgetKind::Index, 32, "local").unwrap();
    assert_eq!(budget.used(BudgetKind::Index), 32);
    assert_eq!(budget.used(BudgetKind::Resident), 32);
    assert!(r.detach_owned().is_err());
    assert_eq!(r.bytes(), 32);
    assert_eq!(budget.used(BudgetKind::Resident), 32);
    r.resize(16, "local shrink").unwrap();
    budget.consume_work(9, "local work").unwrap();
    drop(r);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.peak(BudgetKind::Resident), 32);
    assert_eq!(budget.used(BudgetKind::Work), 9);
}
#[test]
fn detached_charge_outlives_budget_and_callback_without_refund_or_duplicate() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnedReservation>();
    let (owned, ledger, baseline) = {
        let cancelled = Cell::new(false);
        let stop = || cancelled.get();
        let budget = Budget::new_persistent(limits(), Some(&stop)).unwrap();
        let baseline = budget.used(BudgetKind::Resident);
        assert!(baseline > 0);
        let mut reservation = budget.reserve(BudgetKind::Index, 123, "actual charge").unwrap();
        let owned = reservation.detach_owned().unwrap();
        assert_eq!(reservation.bytes(), 0);
        assert_eq!(owned.bytes(), 123);
        assert!(owned.belongs_to(&budget));
        assert_eq!(budget.used(BudgetKind::Resident), baseline + 123);
        drop(reservation);
        assert_eq!(budget.used(BudgetKind::Resident), baseline + 123);
        let ledger = owned.ledger.clone();
        cancelled.set(true);
        assert_eq!(budget.poll("late cancellation").unwrap_err().kind, ErrorKind::Cancelled);
        (owned, ledger, baseline)
    };
    assert_eq!(ledger.used(BudgetKind::Resident), baseline + 123);
    drop(owned);
    assert_eq!(ledger.used(BudgetKind::Resident), baseline);
}
#[test]
fn every_component_and_aggregate_growth_is_atomic_and_work_never_refunds() {
    let probe = Budget::new_persistent(limits(), None).unwrap();
    let header = probe.used(BudgetKind::Resident);
    drop(probe);
    let lower = limits()
        .lowered(&[
            ("residentBytes", header + 48),
            ("indexBytes", header + 32),
            ("decoderBytes", 16),
            ("resourceBytes", 64),
            ("sampleWorkUnits", 10),
            ("visits", 3),
        ])
        .unwrap();
    let budget = Budget::new_persistent(lower, None).unwrap();
    let mut index = budget.reserve(BudgetKind::Index, 32, "index").unwrap();
    let before = budget.used(BudgetKind::Resident);
    let peak = budget.peak(BudgetKind::Resident);
    assert!(budget.reserve(BudgetKind::Decoder, 17, "component denial").is_err());
    assert!(budget.reserve(BudgetKind::Resource, 17, "aggregate denial").is_err());
    assert_eq!(budget.used(BudgetKind::Resident), before);
    assert_eq!(budget.peak(BudgetKind::Resident), peak);
    assert_eq!(budget.used(BudgetKind::Resource), 0);
    let resource = budget.reserve(BudgetKind::Resource, 16, "allowed exact aggregate").unwrap();
    assert!(index.resize(33, "denied growth").is_err());
    assert_eq!(index.bytes(), 32);
    assert_eq!(budget.used(BudgetKind::Resident), header + 48);
    budget.consume_work(10, "work").unwrap();
    assert!(budget.consume_work(1, "work denial").is_err());
    budget.consume_visits(3, "visits").unwrap();
    assert!(budget.consume_visits(1, "visit denial").is_err());
    assert!(budget.reserve(BudgetKind::Work, 1, "not refundable").is_err());
    drop(index);
    drop(resource);
    assert_eq!(budget.used(BudgetKind::Resident), header);
    assert_eq!(budget.peak(BudgetKind::Resident), header + 48);
    assert_eq!(budget.used(BudgetKind::Work), 10);
    assert_eq!(budget.used(BudgetKind::Visits), 3);
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn owned_thread_refunds_share_actual_ledger_with_live_borrowed_growth() {
    let budget = Budget::new_persistent(limits(), None).unwrap();
    let baseline = budget.used(BudgetKind::Resident);
    let mut owned = Vec::new();
    for _ in 0..128 {
        let mut r = budget.reserve(BudgetKind::Decoder, 8, "thread owner").unwrap();
        owned.push(r.detach_owned().unwrap());
    }
    assert!(owned[0].shares_ledger(&owned[127]));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let handle = std::thread::spawn(move || {
        other.wait();
        for item in owned {
            drop(item);
            std::thread::yield_now();
        }
    });
    barrier.wait();
    for _ in 0..128 {
        let mut r = budget.reserve(BudgetKind::Cache, 8, "concurrent").unwrap();
        r.resize(16, "grow").unwrap();
        r.resize(4, "shrink").unwrap();
        drop(r);
    }
    handle.join().unwrap();
    assert_eq!(budget.used(BudgetKind::Resident), baseline);
    assert_eq!(budget.used(BudgetKind::Decoder), 0);
    assert_eq!(budget.used(BudgetKind::Cache), 0);
}
#[test]
fn cancellation_rejects_transfer_and_growth_but_cleanup_remains_exact() {
    let stop = Cell::new(false);
    let cancelled = || stop.get();
    let budget = Budget::new_persistent(limits(), Some(&cancelled)).unwrap();
    let baseline = budget.used(BudgetKind::Resident);
    let mut r = budget.reserve(BudgetKind::Index, 32, "before cancellation").unwrap();
    let mut second = budget.reserve(BudgetKind::Decoder, 8, "owned before cancellation").unwrap();
    let owned = second.detach_owned().unwrap();
    budget.consume_work(17, "nonrefundable work").unwrap();
    stop.set(true);
    assert!(r.resize(33, "cancelled growth").is_err());
    assert_eq!(r.bytes(), 32);
    assert!(r.detach_owned().is_err());
    assert_eq!(r.bytes(), 32);
    r.resize(8, "cleanup after cancellation").unwrap();
    drop(r);
    drop(second);
    drop(owned);
    assert_eq!(budget.used(BudgetKind::Resident), baseline);
    assert_eq!(budget.used(BudgetKind::Work), 17);
    let zero = limits().lowered(&[("indexBytes", 0)]).unwrap();
    assert!(Budget::new_persistent(zero, None).is_err());
    assert!(Budget::new_persistent(limits(), Some(&|| true)).is_err());
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn poisoned_ledger_fails_closed_but_owned_cleanup_refunds_checked_bytes() {
    let budget = Budget::new_persistent(limits(), None).unwrap();
    let baseline = budget.used(BudgetKind::Resident);
    let mut r = budget.reserve(BudgetKind::Index, 41, "owner").unwrap();
    let owned = r.detach_owned().unwrap();
    let state = owned.ledger.state.clone();
    let thread_state = state.clone();
    assert!(
        std::thread::spawn(move || {
            let _held = thread_state.lock().unwrap();
            panic!("synthetic poison");
        })
        .join()
        .is_err()
    );
    assert!(budget.poll("poison").is_err());
    assert_eq!(budget.used(BudgetKind::Resident), u64::MAX);
    assert_eq!(budget.peak(BudgetKind::Index), u64::MAX);
    assert!(budget.reserve(BudgetKind::Resident, 1, "denied poison").is_err());
    drop(owned);
    drop(r);
    let state = state.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(state.used[BudgetKind::Resident.index()], baseline);
}
#[test]
fn impossible_refund_latches_invariant_without_partial_subtraction() {
    let budget = Budget::new_persistent(limits(), None).unwrap();
    let ledger = budget.shared.as_ref().unwrap();
    let before = ledger.used(BudgetKind::Resident);
    ledger.refund(BudgetKind::Decoder, 1, true);
    assert!(budget.poll("invariant").is_err());
    assert_eq!(budget.used(BudgetKind::Resident), u64::MAX);
    let state = ledger.state.lock().unwrap();
    assert!(state.failed);
    assert_eq!(state.used[BudgetKind::Resident.index()], before);
}
