//! UNRUN actual private decoder-receipt handoff drafts; no source qualification.
use super::*;
fn limits() -> crate::ResourceLimits {
    crate::ResourceLimits::for_policy(crate::ResourcePolicy::default()).unwrap()
}
#[test]
fn decoder_optional_source_receipts_and_retained_model_move_in_one_batch() {
    for (encoded, resident) in [(false, false), (true, false), (false, true), (true, true)] {
        let budget = Budget::new_persistent(limits(), None).unwrap();
        let baseline = budget.used(BudgetKind::Resident);
        let mut leases = DecodeReservations {
            source_encoded: encoded.then(|| budget.reserve(BudgetKind::Encoded, 31, "encoded").unwrap()),
            source_resident: resident.then(|| budget.reserve(BudgetKind::Resident, 47, "source").unwrap()),
            retained: budget.reserve(BudgetKind::Resident, 59, "model").unwrap(),
        };
        let batch = leases.detach_owned().unwrap();
        assert_eq!(leases.encoded_bytes(), 0);
        assert_eq!(leases.source_bytes(), 0);
        assert_eq!(leases.retained_bytes(), 0);
        assert_eq!(batch.reservations().len(), 1 + usize::from(encoded) + usize::from(resident));
        assert_eq!(batch.physical_usage().resident_bytes, budget.used(BudgetKind::Resident) - baseline);
        drop(leases);
        drop(batch);
        assert_eq!(budget.used(BudgetKind::Resident), baseline);
        assert_eq!(budget.used(BudgetKind::Encoded), 0);
    }
}
#[test]
fn decode_local_failure_preserves_all_private_source_and_model_receipts() {
    let budget = Budget::new(limits(), None);
    let mut leases = DecodeReservations {
        source_encoded: Some(budget.reserve(BudgetKind::Encoded, 2, "encoded").unwrap()),
        source_resident: Some(budget.reserve(BudgetKind::Resident, 3, "source").unwrap()),
        retained: budget.reserve(BudgetKind::Resident, 5, "model").unwrap(),
    };
    assert!(leases.detach_owned().is_err());
    assert_eq!(leases.encoded_bytes(), 2);
    assert_eq!(leases.source_bytes(), 3);
    assert_eq!(leases.retained_bytes(), 5);
    assert_eq!(budget.used(BudgetKind::Resident), 8);
}
