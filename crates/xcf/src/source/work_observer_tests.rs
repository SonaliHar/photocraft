//! UNRUN caller-work bridge regression drafts; no parser/source qualification.
use super::*;
use std::cell::{Cell, RefCell};
fn limits(work: u64) -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", work)]).unwrap()
}
#[test]
fn actual_work_is_forwarded_in_bounded_chunks_with_one_cumulative_cap() {
    let chunks = RefCell::new(Vec::new());
    let parent = Cell::new(0u64);
    let observer = |n| {
        assert!(n <= 4096);
        chunks.borrow_mut().push(n);
        parent.set(parent.get() + n);
        Ok(())
    };
    let budget = Budget::new_persistent_with_work(limits(10_000), None, Some(&observer)).unwrap();
    budget.consume_work(9_000, "first").unwrap();
    budget.consume_work(1_000, "second").unwrap();
    assert_eq!(*chunks.borrow(), vec![4096, 4096, 808, 1000]);
    assert_eq!(parent.get(), 10_000);
    assert_eq!(budget.used(BudgetKind::Work), 10_000);
    assert!(budget.consume_work(1, "exhausted").is_err());
    assert_eq!(parent.get(), 10_000);
    budget.consume_work(0, "zero").unwrap();
    assert_eq!(chunks.borrow().len(), 4);
}
#[test]
fn impossible_or_overflow_local_admission_never_spends_parent_work() {
    let calls = Cell::new(0u64);
    let observer = |n| {
        calls.set(calls.get() + n);
        Ok(())
    };
    let budget = Budget::new_persistent_with_work(limits(9), None, Some(&observer)).unwrap();
    assert!(budget.consume_work(10, "too much").is_err());
    assert_eq!(calls.get(), 0);
    budget.consume_work(9, "fit").unwrap();
    assert!(budget.consume_work(u64::MAX, "overflow").is_err());
    assert_eq!(calls.get(), 9);
    assert_eq!(budget.used(BudgetKind::Work), 9);
}
#[test]
fn parent_limit_preserves_successful_prefix_and_original_error_fields() {
    let calls = Cell::new(0usize);
    let parent = Cell::new(0u64);
    let observer = |n| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            return Err(XcfError { kind: ErrorKind::Limit, offset: 77, item: "parent".into(), message: "parent refusal".into() });
        }
        parent.set(parent.get() + n);
        Ok(())
    };
    let budget = Budget::new_persistent_with_work(limits(10_000), None, Some(&observer)).unwrap();
    let error = budget.consume_work(9_000, "attempt").unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert_eq!(error.offset, 77);
    assert_eq!(error.item, "parent");
    assert_eq!(error.message, "parent refusal");
    assert_eq!(parent.get(), 4096);
    assert_eq!(budget.used(BudgetKind::Work), 4096);
    budget.consume_work(4096, "retry consumes remaining").unwrap();
    assert_eq!(parent.get(), 8192);
    assert_eq!(budget.used(BudgetKind::Work), 8192);
    assert!(budget.consume_work(1809, "not reset").is_err());
    assert_eq!(parent.get(), 8192);
}
#[test]
fn parent_cancellation_stays_observed_and_successful_prefix_is_not_refunded() {
    let calls = Cell::new(0usize);
    let observer = |_| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 { Err(XcfError::new(ErrorKind::Cancelled, 0, "parent cancel")) } else { Ok(()) }
    };
    let budget = Budget::new_persistent_with_work(limits(20_000), None, Some(&observer)).unwrap();
    assert_eq!(budget.consume_work(9000, "work").unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Work), 4096);
    assert_eq!(budget.poll("sticky").unwrap_err().kind, ErrorKind::Cancelled);
    assert!(budget.consume_work(1, "retry").is_err());
    assert_eq!(calls.get(), 2);
}
#[test]
fn cancellation_during_last_successful_callback_prevents_associated_work() {
    let stop = Cell::new(false);
    let cancelled = || stop.get();
    let observer = |_| {
        stop.set(true);
        Ok(())
    };
    let budget = Budget::new_persistent_with_work(limits(10), Some(&cancelled), Some(&observer)).unwrap();
    assert_eq!(budget.consume_work(10, "last").unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Work), 10);
}
#[test]
fn receipts_outlive_job_callback_and_old_constructors_have_no_observer() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnedReservation>();
    send_sync::<OwnedReservationBatch>();
    let receipt = {
        let calls = Cell::new(0u64);
        let observer = |n| {
            calls.set(calls.get() + n);
            Ok(())
        };
        let budget = Budget::new_persistent_with_work(limits(50), None, Some(&observer)).unwrap();
        budget.consume_work(11, "actual").unwrap();
        assert_eq!(calls.get(), 11);
        let mut reservation = budget.reserve(BudgetKind::Index, 32, "retained").unwrap();
        reservation.detach_owned().unwrap()
    };
    assert_eq!(receipt.bytes(), 32);
    drop(receipt);
    let local = Budget::new(limits(50), None);
    let persistent = Budget::new_persistent(limits(50), None).unwrap();
    assert!(local.work_observer.is_none());
    assert!(persistent.work_observer.is_none());
    local.consume_work(41, "old local").unwrap();
    persistent.consume_work(41, "old persistent").unwrap();
    assert_eq!(local.used(BudgetKind::Work), 41);
    assert_eq!(persistent.used(BudgetKind::Work), 41);
}
