//! UNRUN job-publication fence regressions; no runtime acceptance claimed.
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, accounted_document::AccountedDocument};
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture() -> (Session, BackingQuota) {
    let quota = BackingQuota::new(QuotaLimits { resident: 32 << 20, index: 8 << 20, scratch: 8 << 20 });
    let lease = quota.reserve(QuotaKind::Resident, 65536).unwrap();
    let doc = Document::new("owned job journal", Size::new(3, 2), ColorMode::Rgb, SampleType::F64);
    let owner = AccountedDocument::from_owned(doc, Arc::new(lease), quota.clone(), &control()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &control()).unwrap();
    (session, quota)
}
#[test]
fn changed_journal_or_view_refuses_before_document_or_history_publication() {
    let (mut session, quota) = fixture();
    let before = session.active().unwrap().doc.clone();
    let usage = quota.usage().unwrap();
    let prepared = PreparedAccountedCommand::prepare(&session, "select.all", &Value::Null, &control()).unwrap().into_job_journal();
    prepared.validate(&session, &control()).unwrap();
    session.active_mut().unwrap().view_revision += 1;
    assert!(prepared.validate(&session, &control()).is_err());
    session.active_mut().unwrap().view_revision -= 1;
    session.journal.push(("other.command".into(), json!({})));
    assert!(prepared.validate(&session, &control()).is_err());
    assert!(before.shares_state(&session.active().unwrap().doc));
    assert!(!session.active().unwrap().history.can_undo());
    drop(prepared);
    assert_eq!(quota.usage().unwrap(), usage);
}
#[test]
fn final_work_denial_or_cancellation_never_replaces_the_journal() {
    let (session, quota) = fixture();
    let usage = quota.usage().unwrap();
    let prepared = PreparedAccountedCommand::prepare(&session, "select.all", &Value::Null, &control()).unwrap().into_job_journal();
    let used = AtomicU64::new(0);
    let deny = |n| {
        used.fetch_add(n, Ordering::Relaxed);
        Err(BackingError::Work)
    };
    assert!(prepared.validate(&session, &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(used.load(Ordering::Relaxed) > 0);
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(prepared.validate(&session, &ReadControl { interrupt: &cancelled, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    assert!(session.journal.is_empty());
    assert!(!session.active().unwrap().history.can_undo());
    drop(prepared);
    assert_eq!(quota.usage().unwrap(), usage);
}
