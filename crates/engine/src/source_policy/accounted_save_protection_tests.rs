//! UNRUN extraction regressions: real owned catalog, no filesystem output.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
};
use std::sync::Arc;
#[path = "../../../xcf/tests/common/mod.rs"]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn session() -> (crate::Session, BackingQuota) {
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let mut session = crate::Session::new();
    for (name, color) in [("first.xcf", 31), ("second.xcf", 61)] {
        let capture = q.reserve(QuotaKind::Resident, 65536).unwrap();
        let bytes = Arc::new(xcf::image(13, 150, 0, 1, 1, &[color, 91, 127, 255]));
        let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota(name, bytes, &Default::default(), &limits, q.clone(), &control()).unwrap();
        session.add_owned_plain_xcf_import(&imported, name, Some(&format!("authorized/{name}")), &control()).unwrap();
        drop(imported);
        drop(capture);
    }
    (session, q)
}
#[test]
fn every_open_protected_source_is_included_and_guard_retains_real_catalog_fee() {
    let (session, q) = session();
    let baseline = q.usage().unwrap();
    let guard = AccountedSaveProtection::prepare(&session, &control()).unwrap();
    assert_eq!(guard.protection.sources, ["authorized/first.xcf", "authorized/second.xcf"]);
    for state in session.documents() {
        let source = state.doc.xcf_state.as_ref().unwrap().provenance.source_hash.as_ref().unwrap();
        assert!(guard.protection.fingerprints.contains(source));
        assert!(state.source.as_ref().unwrap().check_destination(state.source.as_ref().unwrap().path.as_deref().unwrap()).is_err());
    }
    assert!(q.usage().unwrap().index > baseline.index);
    // The result has no borrowed control lifetime and confers no read handle.
    drop(guard);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn cancellation_work_and_real_quota_refusal_return_all_temporary_protection_fees() {
    let (session, q) = session();
    let baseline = q.usage().unwrap();
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(AccountedSaveProtection::prepare(&session, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let deny = |_| Err(photocraft_raster::backing::BackingError::Work);
    assert!(AccountedSaveProtection::prepare(&session, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let hold = q.reserve(QuotaKind::Resident, (256 << 20) - baseline.resident).unwrap();
    assert!(AccountedSaveProtection::prepare(&session, &control()).is_err());
    drop(hold);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
fn missing_accounted_catalog_and_hostile_path_keep_original_errors_and_never_publish() {
    let empty = crate::Session::new();
    assert!(matches!(AccountedSaveProtection::prepare(&empty,&control()),Err(e) if e=="accounted-save.source-protection-limit"));
    let (mut session, q) = session();
    let before = session.active().unwrap().doc.clone();
    session.active_mut().unwrap().source.as_mut().unwrap().path = Some("x".repeat(4097));
    let baseline = q.usage().unwrap();
    assert!(matches!(AccountedSaveProtection::prepare(&session,&control()),Err(e) if e=="accounted-save.source-protection-limit"));
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(q.usage().unwrap(), baseline);
}
