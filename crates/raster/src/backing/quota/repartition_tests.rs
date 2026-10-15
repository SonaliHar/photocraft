//! UNRUN genuine lease repartition/rollback regression drafts.
use super::*;
#[test]
fn tight_resident_total_moves_index_headroom_without_transient_growth() {
    let q = BackingQuota::new(QuotaLimits { resident: 100, index: 80, scratch: 0 });
    let mut index = q.reserve(QuotaKind::Index, 80).unwrap();
    let mut resident = q.reserve(QuotaKind::Resident, 20).unwrap();
    QuotaLease::repartition_resident_index(&mut index, &mut resident, 10, 90).unwrap();
    assert_eq!(q.usage().unwrap(), QuotaUsage { resident: 100, index: 10, scratch: 0 });
    assert_eq!((index.bytes(), resident.bytes()), (10, 90));
    QuotaLease::repartition_resident_index(&mut index, &mut resident, 5, 15).unwrap();
    assert_eq!(q.usage().unwrap(), QuotaUsage { resident: 20, index: 5, scratch: 0 });
    drop(index);
    drop(resident);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
#[test]
fn invalid_kinds_foreign_ledgers_and_caps_leave_every_charge_unchanged() {
    let limits = QuotaLimits { resident: 100, index: 80, scratch: 20 };
    let q = BackingQuota::new(limits);
    let other = BackingQuota::new(limits);
    let mut index = q.reserve(QuotaKind::Index, 80).unwrap();
    let mut resident = q.reserve(QuotaKind::Resident, 20).unwrap();
    let mut foreign = other.reserve(QuotaKind::Resident, 10).unwrap();
    let mut scratch = q.reserve(QuotaKind::Scratch, 10).unwrap();
    let before = q.usage().unwrap();
    let other_before = other.usage().unwrap();
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut foreign, 1, 1).is_err());
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut scratch, 1, 1).is_err());
    assert!(QuotaLease::repartition_resident_index(&mut resident, &mut index, 1, 1).is_err());
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut resident, 81, 19).is_err());
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut resident, 1, 100).is_err());
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut resident, u64::MAX, u64::MAX).is_err());
    assert_eq!(q.usage().unwrap(), before);
    assert_eq!(other.usage().unwrap(), other_before);
    assert_eq!((index.bytes(), resident.bytes()), (80, 20));
}
#[test]
fn permanent_header_scratch_and_other_owners_survive_atomic_repartition() {
    let header = BackingQuota::aggregate_header_bytes();
    let (q, baseline) =
        BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: header + 200, index: header + 100, scratch: 10 }, header + 110, QuotaUsage::default())
            .unwrap();
    let mut index = q.reserve(QuotaKind::Index, 80).unwrap();
    let mut resident = q.reserve(QuotaKind::Resident, 20).unwrap();
    let scratch = q.reserve(QuotaKind::Scratch, 10).unwrap();
    QuotaLease::repartition_resident_index(&mut index, &mut resident, 10, 90).unwrap();
    let before = q.usage().unwrap();
    assert_eq!(before, QuotaUsage { resident: header + 100, index: header + 10, scratch: 10 });
    assert!(QuotaLease::repartition_resident_index(&mut index, &mut resident, 10, 91).is_err());
    assert_eq!(q.usage().unwrap(), before);
    drop(index);
    drop(resident);
    drop(scratch);
    drop(baseline);
    assert_eq!(q.usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
}

#[test]
fn baseline_moves_actual_fees_and_returns_every_rejected_lease() {
    let q = BackingQuota::new(QuotaLimits { resident: 100, index: 50, scratch: 20 });
    let index = q.reserve(QuotaKind::Index, 30).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 60).unwrap();
    let scratch = q.reserve(QuotaKind::Scratch, 10).unwrap();
    let before = q.usage().unwrap();
    let baseline = QuotaBaseline::from_component_leases(index, resident, scratch).unwrap();
    assert_eq!(q.usage().unwrap(), before);
    drop(baseline);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
    let index = q.reserve(QuotaKind::Index, 30).unwrap();
    let resident = q.reserve(QuotaKind::Resident, 60).unwrap();
    let scratch = q.reserve(QuotaKind::Scratch, 10).unwrap();
    let before = q.usage().unwrap();
    let (error, returned_resident, returned_index, returned_scratch) = QuotaBaseline::from_component_leases(resident, index, scratch).unwrap_err();
    assert_eq!(error, QuotaError::Invariant);
    assert_eq!(q.usage().unwrap(), before);
    let restored = QuotaBaseline::from_component_leases(returned_index, returned_resident, returned_scratch).unwrap();
    assert_eq!(q.usage().unwrap(), before);
    drop(restored);
    assert_eq!(q.usage().unwrap(), QuotaUsage::default());
}
