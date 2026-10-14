//! UNRUN owning-batch drafts; receipts do not assert native/source semantics.
use super::*;
use crate::{ResourceLimits, ResourcePolicy};
use std::sync::atomic::{AtomicUsize, Ordering};
fn limits() -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::default()).unwrap()
}
#[test]
fn actual_physical_contributions_exclude_encoded_and_logical_counters() {
    let budget = Budget::new_persistent(limits(), None).unwrap();
    let header = budget.used(BudgetKind::Resident);
    let mut encoded = budget.reserve(BudgetKind::Encoded, 100, "encoded").unwrap();
    let mut logical = budget.reserve(BudgetKind::Logical, 200, "logical").unwrap();
    let mut native = budget.reserve(BudgetKind::Resident, 12, "native").unwrap();
    let mut index = budget.reserve(BudgetKind::Index, 5, "index").unwrap();
    let scratch_cap = budget.limits().get(BudgetKind::Scratch);
    let scratch = if scratch_cap == 0 { 0 } else { 7 };
    let mut temporary = budget.reserve(BudgetKind::Scratch, scratch, "scratch").unwrap();
    let batch = Reservation::detach_owned_batch(&mut [&mut encoded, &mut logical, &mut native, &mut index, &mut temporary]).unwrap();
    assert_eq!(batch.reservations().len(), 5);
    assert_eq!(batch.reservations()[0].physical_usage(), PhysicalUsage::default());
    assert_eq!(batch.reservations()[1].physical_usage(), PhysicalUsage::default());
    assert_eq!(batch.reservations()[2].physical_usage(), PhysicalUsage { resident_bytes: 12, index_bytes: 0, scratch_bytes: 0 });
    let fee = batch._fee.bytes();
    assert_eq!(batch.physical_usage(), PhysicalUsage { resident_bytes: 17 + fee, index_bytes: 5 + fee, scratch_bytes: scratch });
    assert_eq!(batch.ledger_header().physical_usage(), PhysicalUsage { resident_bytes: header, index_bytes: header, scratch_bytes: 0 });
    assert_eq!(budget.used(BudgetKind::Resident), header + batch.physical_usage().resident_bytes);
    assert_eq!([encoded.bytes(), logical.bytes(), native.bytes(), index.bytes(), temporary.bytes()], [0; 5]);
    drop(batch);
    assert_eq!(budget.used(BudgetKind::Resident), header);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Logical), 0);
}
#[test]
fn local_and_mixed_ledgers_fail_without_transferring_any_receipt() {
    let local = Budget::new(limits(), None);
    let mut ordinary = local.reserve(BudgetKind::Resident, 5, "local").unwrap();
    assert!(Reservation::detach_owned_batch(&mut [&mut ordinary]).is_err());
    assert_eq!(ordinary.bytes(), 5);
    let first = Budget::new_persistent(limits(), None).unwrap();
    let second = Budget::new_persistent(limits(), None).unwrap();
    let mut a = first.reserve(BudgetKind::Resident, 11, "first").unwrap();
    let mut b = second.reserve(BudgetKind::Resident, 12, "second").unwrap();
    let a_used = first.used(BudgetKind::Resident);
    let b_used = second.used(BudgetKind::Resident);
    assert!(Reservation::detach_owned_batch(&mut [&mut a, &mut b]).is_err());
    assert_eq!(a.bytes(), 11);
    assert_eq!(b.bytes(), 12);
    assert_eq!(first.used(BudgetKind::Resident), a_used);
    assert_eq!(second.used(BudgetKind::Resident), b_used);
}
#[test]
fn metadata_fee_denial_and_late_cancellation_leave_all_original_charges_live() {
    let probe = Budget::new_persistent(limits(), None).unwrap();
    let header = probe.used(BudgetKind::Index);
    drop(probe);
    let lower = limits().lowered(&[("indexBytes", header)]).unwrap();
    let budget = Budget::new_persistent(lower, None).unwrap();
    let mut r = budget.reserve(BudgetKind::Resident, 19, "payload").unwrap();
    let used = budget.used(BudgetKind::Resident);
    assert!(Reservation::detach_owned_batch(&mut [&mut r]).is_err());
    assert_eq!(r.bytes(), 19);
    assert_eq!(budget.used(BudgetKind::Resident), used);
    let polls = AtomicUsize::new(0);
    let cancelled = || polls.fetch_add(1, Ordering::Relaxed) + 1 >= 8;
    let budget = Budget::new_persistent(limits(), Some(&cancelled)).unwrap();
    let mut a = budget.reserve(BudgetKind::Resident, 3, "a").unwrap();
    let mut b = budget.reserve(BudgetKind::Index, 4, "b").unwrap();
    polls.store(0, Ordering::Relaxed);
    let used = budget.used(BudgetKind::Resident);
    assert!(Reservation::detach_owned_batch(&mut [&mut a, &mut b]).is_err());
    assert_eq!(a.bytes(), 3);
    assert_eq!(b.bytes(), 4);
    assert_eq!(budget.used(BudgetKind::Resident), used);
}
#[test]
fn retained_header_tokens_deduplicate_one_actual_ledger_after_budgets_drop() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OwnedLedgerHeader>();
    send_sync::<OwnedReservationBatch>();
    let (one, two, baseline) = {
        let budget = Budget::new_persistent(limits(), None).unwrap();
        let baseline = budget.used(BudgetKind::Resident);
        let mut a = budget.reserve(BudgetKind::Resident, 1, "one").unwrap();
        let mut b = budget.reserve(BudgetKind::Resident, 2, "two").unwrap();
        let one = Reservation::detach_owned_batch(&mut [&mut a]).unwrap();
        let two = Reservation::detach_owned_batch(&mut [&mut b]).unwrap();
        (one, two, baseline)
    };
    assert!(one.ledger_header().shares_ledger(two.ledger_header()));
    let extra = one.reservations()[0].ledger_header();
    assert!(extra.shares_ledger(one.ledger_header()));
    let ledger = extra.ledger.clone();
    assert_eq!(extra.physical_usage().resident_bytes, baseline);
    drop(one);
    drop(two);
    assert_eq!(ledger.used(BudgetKind::Resident), baseline);
}
#[test]
fn empty_oversized_and_denied_work_batches_cannot_empty_original_receipts() {
    assert!(Reservation::detach_owned_batch(&mut []).is_err());
    let budget = Budget::new_persistent(limits(), None).unwrap();
    let mut entries: Vec<_> = (0..=MAX_RECEIPTS).map(|_| budget.reserve(BudgetKind::Resident, 0, "count").unwrap()).collect();
    let mut references: Vec<_> = entries.iter_mut().collect();
    assert!(Reservation::detach_owned_batch(&mut references).is_err());
    drop(references);
    let lower = limits().lowered(&[("sampleWorkUnits", 0)]).unwrap();
    let budget = Budget::new_persistent(lower, None).unwrap();
    let mut r = budget.reserve(BudgetKind::Resident, 1, "kept").unwrap();
    assert!(Reservation::detach_owned_batch(&mut [&mut r]).is_err());
    assert_eq!(r.bytes(), 1);
}
