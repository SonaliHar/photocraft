use photocraft_xcf::{Budget, BudgetKind, ByteSource, ErrorKind, ResourceLimits, ResourcePolicy, Source};

#[test]
fn policy_rejects_raises_unknown_keys_and_unqualified_large() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    assert!(limits.lowered(&[("unknown", 1)]).is_err());
    assert!(limits.lowered(&[("encodedBytes", u64::MAX)]).is_err());
    assert!(ResourceLimits::for_policy(ResourcePolicy::NativeLarge).is_err());
    assert_eq!(limits.lowered(&[("logicalBytes", 0)]).unwrap().get(BudgetKind::Logical), 0);
}

#[test]
fn component_reservations_share_total_and_release_capacity() {
    let limits =
        ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 10), ("decoderBytes", 10), ("cacheBytes", 10)]).unwrap();
    let budget = Budget::new(limits, None);
    assert!(budget.reserve(BudgetKind::Work, 1, "test").is_err());
    let decoder = budget.reserve(BudgetKind::Decoder, 6, "test").unwrap();
    assert_eq!(budget.used(BudgetKind::Resident), 6);
    assert_eq!(budget.reserve(BudgetKind::Cache, 5, "test").unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(budget.used(BudgetKind::Cache), 0);
    drop(decoder);
    let cache = budget.reserve(BudgetKind::Cache, 10, "test").unwrap();
    assert_eq!(budget.used(BudgetKind::Resident), 10);
    drop(cache);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn source_checks_u64_extent_and_cancel_before_copy() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let budget = Budget::new(limits.clone(), None);
    let mut source = ByteSource::new(b"abcdef");
    let mut out = [0; 3];
    source.read_exact_at(2, &mut out, &budget).unwrap();
    assert_eq!(&out, b"cde");
    assert!(source.read_exact_at(u64::MAX, &mut out, &budget).is_err());
    let stop = || true;
    let budget = Budget::new(limits, Some(&stop));
    assert_eq!(source.read_exact_at(0, &mut out, &budget).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(&out, b"cde");
}

#[test]
fn work_is_not_refunded_and_zero_deadline_fails_at_entry() {
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("sampleWorkUnits", 4096)]).unwrap();
    let budget = Budget::new(limits, None);
    budget.consume_work(4096, "test").unwrap();
    assert!(budget.consume_work(1, "test").is_err());
    let zero = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("deadlineMs", 0)]).unwrap();
    assert_eq!(Budget::new(zero, None).poll("test").unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn read_completion_observes_cancellation_and_latches_it() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = AtomicUsize::new(0);
    let stop = || calls.fetch_add(1, Ordering::Relaxed) == 2;
    let budget = Budget::new(ResourceLimits::for_policy(ResourcePolicy::default()).unwrap(), Some(&stop));
    let mut source = ByteSource::new(b"own");
    let mut output = [0; 3];
    assert_eq!(source.read_exact_at(0, &mut output, &budget).unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(budget.poll("after cancellation").unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn authorized_handle_read_rechecks_current_policy_before_seek_and_reports_u64_overflow() {
    use photocraft_xcf::FileSource;
    let path = std::env::temp_dir().join(format!("photocraft-sol1-source-{}.bin", std::process::id()));
    std::fs::write(&path, b"own").unwrap();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
    let initial = Budget::new(limits.clone(), None);
    let mut source = FileSource::new(std::fs::File::open(&path).unwrap(), &initial).unwrap();
    let lower = Budget::new(limits.lowered(&[("encodedBytes", 0)]).unwrap(), None);
    let mut output = [9; 3];
    assert_eq!(source.read_exact_at(0, &mut output, &lower).unwrap_err().kind, ErrorKind::Limit);
    assert_eq!(output, [9; 3]);
    assert_eq!(source.read_exact_at(u64::MAX, &mut output, &initial).unwrap_err().kind, ErrorKind::Structure);
    drop(source);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reservation_resize_is_atomic_and_shrink_survives_cancellation() {
    use std::cell::Cell;
    let cancelled = Cell::new(false);
    let poll = || cancelled.get();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap().lowered(&[("residentBytes", 10), ("decoderBytes", 20)]).unwrap();
    let budget = Budget::new(limits, Some(&poll));
    let mut lease = budget.reserve(BudgetKind::Decoder, 4, "test").unwrap();
    assert!(lease.resize(11, "growth").is_err());
    assert_eq!(budget.used(BudgetKind::Decoder), 4);
    assert_eq!(budget.used(BudgetKind::Resident), 4);
    lease.resize(8, "growth").unwrap();
    cancelled.set(true);
    lease.resize(2, "cleanup").unwrap();
    assert_eq!(budget.used(BudgetKind::Resident), 2);
    assert_eq!(lease.resize(3, "cancelled growth").unwrap_err().kind, ErrorKind::Cancelled);
    drop(lease);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
}

#[test]
fn successful_charge_peaks_survive_cleanup_but_denial_and_cancel_do_not_inflate() {
    use std::cell::Cell;
    let cancelled = Cell::new(false);
    let poll = || cancelled.get();
    let limits = ResourceLimits::for_policy(ResourcePolicy::default())
        .unwrap()
        .lowered(&[("residentBytes", 10), ("decoderBytes", 20), ("sampleWorkUnits", 12)])
        .unwrap();
    let budget = Budget::new(limits, Some(&poll));
    let mut decoder = budget.reserve(BudgetKind::Decoder, 4, "decoder").unwrap();
    let other = budget.reserve(BudgetKind::Resource, 2, "resource").unwrap();
    assert_eq!(budget.peak(BudgetKind::Resident), 6);
    assert!(decoder.resize(9, "denied aggregate").is_err());
    assert_eq!(budget.peak(BudgetKind::Decoder), 4);
    assert_eq!(budget.peak(BudgetKind::Resident), 6);
    decoder.resize(8, "successful growth").unwrap();
    assert_eq!(budget.peak(BudgetKind::Resident), 10);
    budget.consume_work(7, "work").unwrap();
    assert!(budget.consume_work(6, "denied work").is_err());
    assert_eq!(budget.peak(BudgetKind::Work), 7);
    budget.consume_visits(3, "visits").unwrap();
    assert_eq!(budget.peak(BudgetKind::Visits), 3);
    cancelled.set(true);
    assert!(budget.reserve(BudgetKind::Index, 1, "cancelled").is_err());
    assert_eq!(budget.peak(BudgetKind::Index), 0);
    assert!(decoder.resize(9, "cancelled").is_err());
    decoder.resize(1, "cleanup").unwrap();
    drop(decoder);
    drop(other);
    assert_eq!(budget.used(BudgetKind::Resident), 0);
    assert_eq!(budget.peak(BudgetKind::Resident), 10);
    assert_eq!(budget.peak(BudgetKind::Decoder), 8);
    assert_eq!(budget.used(BudgetKind::Work), 7);
}
