//! Authored regression groups only; no execution or performance qualification.
use super::*;

fn standard() -> ResourceLimits {
    ResourceLimits::for_policy(ResourcePolicy::default()).unwrap()
}

#[test]
#[ignore = "UNRUN: finite legacy and selected-policy counter boundaries"]
fn selection_preserves_capture_debits_and_allows_only_one_policy() {
    let meter = OperationWork::legacy(8);
    meter.check(7).unwrap();
    let started = meter.state().started;
    let limits = standard().lowered(&[("sampleWorkUnits", 11)]).unwrap();
    meter.select_standard_box(&limits).unwrap();
    assert_eq!(meter.state().started, started);
    assert_eq!(meter.diagnostics().used, 7);
    assert_eq!(meter.diagnostics().limit, 11);
    meter.check(4).unwrap();
    assert_eq!(meter.check(1), Err(BackingError::Work));
    assert_eq!(meter.diagnostics().used, 11);
    assert_eq!(meter.select_standard_box(&standard()), Err(BackingError::Integrity));
    assert_eq!(meter.diagnostics().limit, 11);

    // A lower selected cap cannot erase already-performed capture work.
    let lower = OperationWork::legacy(100);
    lower.check(20).unwrap();
    let limits = standard().lowered(&[("sampleWorkUnits", 19)]).unwrap();
    assert_eq!(lower.select_standard_box(&limits), Err(BackingError::Work));
    assert_eq!(lower.diagnostics().used, 20);
    assert_eq!(lower.select_standard_box(&standard()), Err(BackingError::Integrity));
    assert_eq!(lower.check(0), Err(BackingError::Work));
}

#[test]
#[ignore = "UNRUN: interactive wait exclusion and zero deadline without sleeping"]
fn clock_starts_at_active_work_and_never_restarts_on_selection() {
    let meter = OperationWork::legacy(64);
    let zero_deadline = standard().lowered(&[("deadlineMs", 0)]).unwrap();
    meter.select_standard_box(&zero_deadline).unwrap();
    assert!(meter.state().started.is_none());
    assert!(!meter.deadline_expired());
    // No clock was started just by creating/selecting a picker request.
    meter.begin_active();
    let first = meter.state().started;
    meter.begin_active();
    assert_eq!(meter.state().started, first);
    assert!(meter.deadline_expired());
    assert_eq!(meter.check(0), Err(BackingError::Cancelled));
    assert_eq!(meter.diagnostics().used, 0);

    let capture = OperationWork::legacy(64);
    capture.check(9).unwrap();
    let first = capture.state().started;
    assert_eq!(capture.select_standard_box(&zero_deadline), Err(BackingError::Cancelled));
    assert_eq!(capture.state().started, first);
    assert_eq!(capture.diagnostics().used, 9);
    assert_eq!(capture.select_standard_box(&standard()), Err(BackingError::Integrity));
}

#[test]
#[ignore = "UNRUN: standard hard ceilings, lower-only validation and legacy invariance"]
fn policy_ceilings_and_legacy_zero_overflow_are_finite() {
    let limits = standard();
    let ceiling = if cfg!(target_arch = "wasm32") { 1u64 << 36 } else { 1u64 << 40 };
    assert_eq!(limits.get(BudgetKind::Work), ceiling);
    assert_eq!(limits.get(BudgetKind::Deadline), 120_000);
    assert!(limits.lowered(&[("sampleWorkUnits", ceiling + 1)]).is_err());
    assert!(limits.lowered(&[("deadlineMs", 120_001)]).is_err());
    assert!(limits.lowered(&[("unknown", 0)]).is_err());
    assert!(ResourceLimits::for_policy(ResourcePolicy::NativeLarge).is_err());
    let meter = OperationWork::legacy(0);
    meter.check(0).unwrap();
    assert_eq!(meter.check(1), Err(BackingError::Work));
    assert_eq!(meter.diagnostics().policy, None);
    assert_eq!(meter.diagnostics().deadline_ms, None);
    assert!(!meter.deadline_expired());
    let legacy = OperationWork::legacy(10);
    legacy.check(10).unwrap();
    assert_eq!(legacy.check(u64::MAX), Err(BackingError::Work));
    assert_eq!(legacy.diagnostics().used, 10);
    let zero = standard().lowered(&[("sampleWorkUnits", 0)]).unwrap();
    let selected = OperationWork::legacy(100);
    selected.select_standard_box(&zero).unwrap();
    selected.check(0).unwrap();
    assert_eq!(selected.check(1), Err(BackingError::Work));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "UNRUN: concurrent provider/capture callers share one finite counter"]
fn shared_callers_cannot_each_spend_the_same_allowance() {
    let meter = std::sync::Arc::new(OperationWork::legacy(1));
    meter.select_standard_box(&standard().lowered(&[("sampleWorkUnits", 4000)]).unwrap()).unwrap();
    let mut workers = Vec::new();
    for _ in 0..4 {
        let meter = std::sync::Arc::clone(&meter);
        workers.push(std::thread::spawn(move || {
            for _ in 0..1000 {
                meter.check(1).unwrap();
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(meter.diagnostics().used, 4000);
    assert_eq!(meter.check(1), Err(BackingError::Work));
}

#[test]
#[ignore = "UNRUN: generic owned intake selects before active capture and respects early deadline"]
fn generic_standard_constructor_is_lazy_but_zero_deadline_refuses_first_work() {
    let zero_deadline = standard().lowered(&[("deadlineMs", 0)]).unwrap();
    let meter = OperationWork::standard(&zero_deadline).unwrap();
    assert!(meter.state().started.is_none());
    assert!(!meter.deadline_expired());
    assert_eq!(meter.diagnostics().policy, Some(ResourcePolicy::default()));
    assert_eq!(meter.diagnostics().used, 0);
    assert_eq!(meter.check(1), Err(BackingError::Cancelled));
    assert_eq!(meter.diagnostics().used, 0);
    assert!(meter.deadline_expired());
    assert_eq!(meter.select_standard_box(&standard()), Err(BackingError::Integrity));

    let zero_work = standard().lowered(&[("sampleWorkUnits", 0)]).unwrap();
    let meter = OperationWork::standard(&zero_work).unwrap();
    meter.check(0).unwrap();
    assert_eq!(meter.check(1), Err(BackingError::Work));
}

#[test]
#[ignore = "UNRUN: generic selection and Box compatibility share one nonrefundable capture ledger"]
fn generic_and_box_names_cannot_reselect_or_reset_capture() {
    let limits = standard().lowered(&[("sampleWorkUnits", 9)]).unwrap();
    let generic = OperationWork::legacy(4);
    generic.check(3).unwrap();
    let started = generic.state().started;
    generic.select_standard(&limits).unwrap();
    assert_eq!(generic.state().started, started);
    assert_eq!(generic.diagnostics().used, 3);
    assert_eq!(generic.select_standard_box(&standard()), Err(BackingError::Integrity));
    generic.check(6).unwrap();
    assert_eq!(generic.check(1), Err(BackingError::Work));

    let compatibility = OperationWork::legacy(4);
    compatibility.check(3).unwrap();
    compatibility.select_standard_box(&limits).unwrap();
    assert_eq!(compatibility.select_standard(&standard()), Err(BackingError::Integrity));
    assert_eq!(compatibility.diagnostics().used, 3);
    assert_eq!(compatibility.diagnostics().limit, generic.diagnostics().limit);
}

#[test]
#[ignore = "UNRUN: generic standard meter does not replace caller-supplied lower ReadControl"]
fn external_lower_control_remains_authoritative() {
    use photocraft_raster::{Interrupt, backing::ReadControl};
    let meter = OperationWork::standard(&standard()).unwrap();
    let external_used = std::sync::atomic::AtomicU64::new(0);
    let external = |n| {
        external_used
            .try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |used| used.checked_add(n).filter(|next| *next <= 2))
            .map(|_| ())
            .map_err(|_| BackingError::Work)
    };
    // A service that deliberately composes both controls must still propagate
    // its caller's lower refusal instead of substituting the selected ceiling.
    let charge = |n| {
        external(n)?;
        meter.check(n)
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
    control.check(2).unwrap();
    assert_eq!(control.check(1), Err(BackingError::Work));
    // The failed extra debit is not retried against the larger selected limit.
    assert_eq!(external_used.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(meter.diagnostics().used, 2);
}
