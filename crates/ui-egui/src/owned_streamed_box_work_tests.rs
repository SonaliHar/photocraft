//! UNRUN host policy/capture controls; no large-image completion claim.
use super::*;
#[path = "canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
use photocraft_doc::SampleType;

fn capture(request: Request, bytes: &[u8]) -> Capture {
    let mut builder = request.begin_capture("CURRENT.pcraft", bytes.len()).unwrap();
    for chunk in bytes.chunks(65_536) {
        builder.append(chunk).unwrap();
    }
    builder.finish().unwrap()
}

#[test]
#[ignore = "UNRUN: explicit native browser capture selects standard before input and retains one used meter/cancel owner through load/adoption; future provider reads use caller control, failed strict recipe cannot publish or reset work"]
fn box_capture_retains_upfront_standard_meter() {
    let (owner, literal) = fixture::fixture(SampleType::F32, false, 0);
    let request = Request::with_family(owner.quota().clone(), Family::Native).unwrap();
    let original_work = request.work.clone();
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
    let waiting = request.work.meter.diagnostics();
    assert_eq!(waiting.used, 0);
    assert_eq!(waiting.limit, limits.get(photocraft_xcf::BudgetKind::Work));
    assert_eq!(waiting.policy, Some(limits.policy()));
    assert_eq!(waiting.deadline_ms, Some(limits.get(photocraft_xcf::BudgetKind::Deadline)));
    assert!(!waiting.deadline_expired);
    // This file Vec is an external harness writer capture. Capture/provider
    // fees, immutable native archive and model remain actual paid owners.
    let captured = capture(request.clone(), &fixture::encoded(&owner));
    let before = request.work.meter.diagnostics();
    assert!(before.used > 0);
    assert_eq!(before.policy, waiting.policy);
    assert!(Arc::ptr_eq(&original_work, &captured.request.work));
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    apply(&mut app, captured).unwrap();
    let selected = request.work.meter.diagnostics();
    assert!(selected.used > before.used);
    assert_eq!(selected.limit, limits.get(photocraft_xcf::BudgetKind::Work));
    assert_eq!(selected.policy, Some(limits.policy()));
    assert_eq!(selected.deadline_ms, Some(limits.get(photocraft_xcf::BudgetKind::Deadline)));
    assert!(!selected.deadline_expired);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.session.active().unwrap().path.is_none());
    request.work.cancelled.store(true, Ordering::Relaxed);
    assert!(request.with_control(|c| c.check(0).map_err(|e| e.to_string())).is_err());
    // The retained provider never uses that now-cancelled capture Request.
    assert_eq!(fixture::native(&app.session.active().unwrap().doc), literal.raw);
    let current = app.session.active().unwrap().doc.clone();
    let bad = fixture::malformed_fixture(SampleType::F32, false);
    let rejected = Request::with_family(owner.quota().clone(), Family::Native).unwrap();
    let captured = capture(rejected.clone(), &fixture::encoded(&bad));
    let used = rejected.work.meter.diagnostics().used;
    assert!(apply(&mut app, captured).is_err());
    assert!(rejected.work.meter.diagnostics().used > used);
    assert_eq!(rejected.work.meter.diagnostics().policy, Some(limits.policy()));
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.session.active().unwrap().doc.shares_state(&current));
    // Selection is once-only even following a failed strict publication.
    assert!(rejected.work.meter.select_standard(&limits).is_err());
}
