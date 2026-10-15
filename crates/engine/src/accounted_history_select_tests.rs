//! UNRUN actual registered accounted history selection and carrier drafts.
use super::*;
use crate::accounted_native_save::PreparedAccountedNativeSave;
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use serde_json::json;
use std::sync::Arc;
#[path = "../../xcf/tests/common/mod.rs"]
mod xcf;
fn control() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn initial_session() -> Session {
    let imported = photocraft_io::import_owned_staged_plain_bytes(
        "source.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[31, 61, 91, 255])),
        &Default::default(),
        &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
        &Interrupt::NONE,
    )
    .unwrap();
    let mut session = Session::new();
    session.add_owned_plain_xcf_import(&imported, "source.xcf", Some("authorized/source.xcf"), &control()).unwrap();
    session
}
fn saved_session() -> Session {
    let mut session = initial_session();
    session.execute("layer.setProps", json!({"name":"first"})).unwrap();
    session.execute("layer.setProps", json!({"name":"second"})).unwrap();
    PreparedAccountedNativeSave::prepare(&session, "saved.pcraft", &control()).unwrap().write_and_publish(&mut session, &control(), |_, _| Ok(())).unwrap();
    session
}

const CMD: &str = "history.selectState";
#[test]
fn registered_route_preserves_ignored_fields_and_prepaid_response() {
    let mut session = saved_session();
    let before = session.active().unwrap().doc.clone();
    assert!(supports(CMD));
    let reply = session.execute(CMD, serde_json::json!({"state":0.0,"ignored":123})).unwrap();
    assert_eq!(reply["state"], 0);
    assert_eq!(reply["changed"], true);
    assert!(reply["current"].is_string());
    assert_eq!(session.active().unwrap().doc.name, "saved.pcraft");
    assert_eq!(session.active().unwrap().doc.source_color_context, before.source_color_context);
    assert_eq!(session.journal.last().unwrap().1["ignored"], 123);
    let revision = session.active().unwrap().revision;
    let reply = session.execute(CMD, serde_json::json!({"state":0})).unwrap();
    assert_eq!(reply, serde_json::json!({"state":0,"changed":false}));
    assert_eq!(session.active().unwrap().revision, revision);
}
#[test]
fn invalid_state_fails_before_journal_copy_or_model_publication() {
    for params in [
        serde_json::json!({}),
        serde_json::json!({"state":null}),
        serde_json::json!({"state":"0"}),
        serde_json::json!({"state":-1}),
        serde_json::json!({"state":1000}),
    ] {
        let mut session = saved_session();
        let before = session.active().unwrap().doc.clone();
        let journal = session.journal.clone();
        let quota = before.quota().unwrap();
        let used = quota.usage().unwrap();
        assert!(session.execute(CMD, params).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.journal, journal);
        assert_eq!(quota.usage().unwrap(), used);
    }
}
#[test]
fn prepared_view_and_journal_fences_preserve_actual_model() {
    for alter_view in [true, false] {
        let mut session = saved_session();
        let params = serde_json::json!({"state":0});
        let prepared = PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.clone();
        if alter_view {
            session.active_mut().unwrap().view_revision += 1;
        } else {
            session.journal.push(("external".into(), Value::Null));
        }
        assert!(prepared.execute(&mut session, &control()).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
    }
}
#[test]
fn real_response_allowance_is_retained_and_quota_failure_has_no_side_effects() {
    let mut session = saved_session();
    let params = serde_json::json!({"state":0});
    let before = session.active().unwrap().doc.clone();
    let quota = before.quota().unwrap();
    let used = quota.usage().unwrap();
    let mut low = 0u64;
    let mut high = u64::MAX;
    while low < high {
        let mid = low + (high - low) / 2 + 1;
        match quota.reserve(QuotaKind::Index, mid) {
            Ok(lease) => {
                drop(lease);
                low = mid
            }
            Err(_) => high = mid - 1,
        }
    }
    let blocker = quota.reserve(QuotaKind::Index, low).unwrap();
    assert!(PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    drop(blocker);
    assert_eq!(quota.usage().unwrap(), used);
    let prepared = PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).unwrap();
    let CommandLease::History(response) = &prepared.lease else { panic!("actual history response allowance missing") };
    assert!(response.lease.bytes() >= 4096);
    assert!(response.quota.shares_ledger(quota));
    prepared.execute(&mut session, &control()).unwrap();
    assert!(session.accounted_journal.as_ref().unwrap().bytes() >= 4096);
}

#[test]
fn changed_history_owner_without_revision_is_stale() {
    let mut session = saved_session();
    let params = serde_json::json!({"state":0});
    let prepared = PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).unwrap();
    let before = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    session.active_mut().unwrap().history.clear(&control()).unwrap();
    assert!(prepared.execute(&mut session, &control()).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().revision, revision);
}

#[test]
fn ignored_large_payload_requires_copy_work_before_destination_allocation() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let session = saved_session();
    let params = json!({"state":0,"ignored":"x".repeat(4096)});
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.clone();
    let journal = session.journal.clone();
    let quota = before.quota().unwrap();
    let usage = quota.usage().unwrap();
    let largest = AtomicU64::new(0);
    let work = |n| {
        largest.fetch_max(n, Ordering::Relaxed);
        if n >= 4096 { Err(BackingError::Work) } else { Ok(()) }
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &work };
    assert!(PreparedAccountedCommand::prepare(&session, CMD, &params, &c).is_err());
    assert!(largest.load(Ordering::Relaxed) >= 4096);
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(session.journal, journal);
    assert_eq!(quota.usage().unwrap(), usage);
}

#[test]
fn original_initial_only_history_is_disabled_before_any_paid_copy() {
    let mut session = initial_session();
    let params = json!({"state":0});
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.clone();
    let revision = session.active().unwrap().revision;
    let journal = session.journal.clone();
    let quota = before.quota().unwrap();
    let usage = quota.usage().unwrap();
    assert_eq!(disabled_reason(&session, CMD, &Value::Null).unwrap(), Some("the history holds only one state"));
    assert_eq!(disabled_reason(&session, CMD, &params).unwrap(), Some("the history holds only one state"));
    assert!(matches!(PreparedAccountedCommand::prepare(&session,CMD,&params,&control()),Err(EngineError::Disabled(id,_)) if id==CMD));
    assert!(matches!(session.execute(CMD,params),Err(EngineError::Disabled(id,_)) if id==CMD));
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.journal, journal);
    assert_eq!(quota.usage().unwrap(), usage);
}
#[test]
fn direct_prepared_execution_rechecks_original_enabled_policy() {
    let mut session = saved_session();
    let params = json!({"state":0});
    let prepared = PreparedAccountedCommand::prepare(&session, CMD, &params, &control()).unwrap();
    session.active_mut().unwrap().history.clear(&control()).unwrap();
    let before = session.active().unwrap().doc.clone();
    assert!(matches!(prepared.execute(&mut session,&control()),Err(EngineError::Disabled(id,_)) if id==CMD));
    assert!(session.active().unwrap().doc.shares_state(&before));
}
