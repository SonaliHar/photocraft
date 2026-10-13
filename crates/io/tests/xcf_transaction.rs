//! Source ownership and cancellation are checked before any document can be installed.
use photocraft_io::xcf_source::OwnedXcfSnapshot;
use photocraft_xcf::{Budget, BudgetKind, ByteSource, ErrorKind, ResourceLimits, ResourcePolicy, Source};

fn budget() -> Budget<'static> {
    Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), None)
}

#[test]
fn source_snapshot_survives_original_mutation_and_has_checked_u64_reads() {
    let budget = budget();
    let mut original = b"gimp xcf own immutable source".to_vec();
    let mut source = ByteSource::new(&original);
    let mut snapshot = OwnedXcfSnapshot::from_source(&mut source, &budget).unwrap();
    original.fill(0);
    assert_eq!(snapshot.bytes(), b"gimp xcf own immutable source");
    let mut out = [0; 4];
    snapshot.read_exact_at(5, &mut out, &budget).unwrap();
    assert_eq!(&out, b"xcf ");
    assert!(snapshot.read_exact_at(u64::MAX, &mut out, &budget).is_err());
    assert_eq!(budget.used(BudgetKind::Encoded), snapshot.len());
    assert_eq!(budget.used(BudgetKind::Resident), snapshot.len());
    drop(snapshot);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn denied_or_cancelled_snapshot_releases_all_reservations() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 2)]).unwrap();
    let budget = Budget::new(limits, None);
    let mut source = ByteSource::new(b"four");
    assert!(OwnedXcfSnapshot::from_source(&mut source, &budget).is_err());
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);

    let cancelled = || true;
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), Some(&cancelled));
    let error = OwnedXcfSnapshot::from_source(&mut source, &budget).err().unwrap();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn owned_snapshot_checks_cancellation_after_reservations_before_publication() {
    use std::cell::Cell;
    let polls = Cell::new(0);
    // Each reservation polls before admission and again during its limit check.
    // The fifth poll must occur after both reservations, before publication.
    let check = || {
        polls.set(polls.get() + 1);
        polls.get() >= 5
    };
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), Some(&check));
    let error = OwnedXcfSnapshot::from_owned(vec![1; 4096], &budget).err().unwrap();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}

#[test]
fn source_failure_does_not_publish_partial_snapshot() {
    struct FailingSource;
    impl Source for FailingSource {
        fn len(&self) -> u64 {
            16
        }
        fn read_exact_at(&mut self, _: u64, _: &mut [u8], _: &Budget<'_>) -> Result<(), photocraft_xcf::XcfError> {
            Err(photocraft_xcf::XcfError { kind: ErrorKind::Truncated, offset: 0, item: String::new(), message: "source changed during capture".into() })
        }
    }
    let budget = budget();
    assert!(OwnedXcfSnapshot::from_source(&mut FailingSource, &budget).is_err());
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn cancellation_during_capture_stops_before_initializing_another_slice() {
    use std::cell::Cell;
    struct CancellingSource<'a> {
        calls: &'a Cell<usize>,
        cancelled: &'a Cell<bool>,
    }
    impl Source for CancellingSource<'_> {
        fn len(&self) -> u64 {
            16 << 20
        }
        fn read_exact_at(&mut self, _: u64, output: &mut [u8], _: &Budget<'_>) -> Result<(), photocraft_xcf::XcfError> {
            self.calls.set(self.calls.get() + 1);
            assert!(output.len() <= 4096);
            output.fill(1);
            self.cancelled.set(true);
            Ok(())
        }
    }
    let calls = Cell::new(0);
    let cancelled = Cell::new(false);
    let check = || cancelled.get();
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), Some(&check));
    let error = OwnedXcfSnapshot::from_source(&mut CancellingSource { calls: &calls, cancelled: &cancelled }, &budget).err().unwrap();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(calls.get(), 1);
    assert_eq!(budget.used(BudgetKind::Work), 4096);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}
#[path = "common/xcf.rs"]
mod common;

#[test]
fn snapshot_decoder_and_mapper_share_one_resident_ceiling() {
    use photocraft_xcf::{DecodeOptions, SourceAccounting};
    let bytes = common::image(12, 150, 0, 1, 1, &[20, 70, 120, 255]);
    let limits = ResourceLimits::for_policy(ResourcePolicy::NativeStandard).unwrap().lowered(&[("residentBytes", 2 << 20)]).unwrap();
    let budget = Budget::new(limits, None);
    let snapshot = OwnedXcfSnapshot::from_owned(bytes, &budget).unwrap();
    let source_bytes = budget.used(BudgetKind::Resident);
    let imported = photocraft_io::xcf_import::import_candidate_with_budget(
        "owned.xcf",
        snapshot.bytes(),
        &photocraft_raster::Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        SourceAccounting::CallerReserved,
    )
    .unwrap()
    .unwrap();
    assert_eq!(imported.document.size.width, 1);
    assert_eq!(budget.used(BudgetKind::Resident), source_bytes);
    let other = budget.reserve(BudgetKind::Resident, 1 << 20, "another live transaction owner").unwrap();
    let before = budget.used(BudgetKind::Resident);
    let error = photocraft_io::xcf_import::import_candidate_with_budget(
        "denied.xcf",
        snapshot.bytes(),
        &photocraft_raster::Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        SourceAccounting::CallerReserved,
    )
    .unwrap_err();
    assert!(matches!(error, photocraft_io::IoError::Xcf(ref error) if error.kind == ErrorKind::Limit));
    assert_eq!(budget.used(BudgetKind::Resident), before);
    assert_eq!(budget.used(BudgetKind::Encoded), snapshot.bytes().len() as u64);
    drop(other);
    drop(snapshot);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}

#[test]
fn mapping_materialization_cannot_reset_the_callers_work_budget() {
    use photocraft_xcf::{DecodeOptions, SourceAccounting};
    let bytes = common::image(12, 150, 0, 1, 1, &[20, 70, 120, 255]);
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 64)]).unwrap();
    let budget = Budget::new(limits, None);
    let error = photocraft_io::xcf_import::import_candidate_with_budget(
        "work denied.xcf",
        &bytes,
        &photocraft_raster::Interrupt::NONE,
        &DecodeOptions::default(),
        &budget,
        SourceAccounting::Borrowed,
    )
    .unwrap_err();
    assert!(
        matches!(error, photocraft_io::IoError::Xcf(ref error) if error.kind == ErrorKind::Limit && error.message.contains("mapping materialization")),
        "{error}"
    );
    assert!(budget.used(BudgetKind::Work) > 0);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.used(BudgetKind::Encoded), 0);
}
