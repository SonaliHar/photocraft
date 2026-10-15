//! UNRUN genuine empty CURRENT discovery after two registered removals.
//! The retained empty stack is not an operation or original-source witness.
use super::*;
use photocraft_raster::{
    Interrupt,
    backing::quota::{BackingQuota, QuotaKind, QuotaLimits},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
fn empty_session() -> (Session, BackingQuota, Vec<String>) {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
    assert_eq!(hash(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()), "99038395a4cdf4c3b3ea420ffa5aee46f6bf47c679870d76be933d8e5405170a");
    let bytes = std::fs::read(root.join("stack.xcf")).unwrap();
    assert_eq!(hash(&bytes), "f8f1c11303df47a49b817c6cd745c08edd18fbb1045daa8fdf77111c89f4cbdd");
    let quota = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "stack.xcf",
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        quota.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::Point(imported) = imported else { panic!("genuine witnessed original") };
    let mut session = Session::new();
    session.add_owned_point_xcf_import(&imported, "stack.xcf", None, &ctl()).unwrap();
    let ids: Vec<_> = session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.iter().map(|n| n.id.0.to_string()).collect();
    assert_eq!(ids.len(), 2);
    for id in &ids {
        session.execute("xcf.filter.remove", json!({"filterId":id})).unwrap();
    }
    (session, quota, ids)
}

#[test]
#[ignore = "UNRUN: paid zero-filter pages preserve actual empty stack/revision/current owner/history"]
fn empty_current_project_reports_zero_filters_but_keeps_owner_and_revision() {
    let (s, quota, ids) = empty_session();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let revision = s.active().unwrap().revision;
    let journal = s.journal.len();
    let state = before.xcf_state.as_ref().unwrap();
    assert_eq!(state.items.len(), 1);
    assert!(state.filter_masks.is_empty());
    assert_eq!(state.filter_stacks.len(), 1);
    assert!(state.filter_stacks[0].nodes.is_empty());
    assert_eq!(state.current_filter_output_precision, photocraft_doc::FilterOutputPrecision::Canonical);
    for offset in [0, 1, 100000] {
        let baseline = quota.usage().unwrap();
        let reply = prepare(&s, &json!({"offset":offset,"limit":1}), &ctl()).unwrap();
        assert!(reply.quota().shares_ledger(&quota));
        assert_eq!(reply.value()["filters"]["total"], 0);
        assert!(reply.value()["filters"]["entries"].as_array().unwrap().is_empty());
        assert_eq!(reply.value()["filterStacks"]["total"], 1);
        assert_eq!(reply.value()["items"]["total"], 1);
        if offset == 0 {
            assert_eq!(reply.value()["filterStacks"]["entries"][0]["nodes"], 0);
            assert_eq!(reply.value()["filterStacks"]["entries"][0]["revision"], state.filter_stacks[0].revision.to_string());
            assert_eq!(reply.value()["items"]["entries"][0]["item"]["kind"], "layer");
        } else {
            assert!(reply.value()["filterStacks"]["entries"].as_array().unwrap().is_empty());
            assert!(reply.value()["items"]["entries"].as_array().unwrap().is_empty());
        }
        assert_eq!(reply.value()["opaqueRecords"], state.opaque.len());
        assert!(quota.usage().unwrap().index > baseline.index);
        drop(reply);
        assert_eq!(quota.usage().unwrap(), baseline);
    }
    for id in ids {
        assert!(crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":id}), &ctl()).is_err());
    }
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.journal.len(), journal);
}

#[test]
#[ignore = "UNRUN: zero-filter query fees/cancellation/budget and malformed pages remain transactional"]
fn empty_current_query_failures_do_not_change_owner_or_leak_reply_fees() {
    let (s, quota, _) = empty_session();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.len();
    let baseline = quota.usage().unwrap();
    for params in [json!({"limit":0}), json!({"offset":100001}), json!({"extra":true})] {
        assert!(prepare(&s, &params, &ctl()).is_err());
    }
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(prepare(&s, &Value::Null, &denied).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(prepare(&s, &Value::Null, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    assert_eq!(quota.usage().unwrap(), baseline);
    let blocker = quota.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &Value::Null, &ctl()).is_err());
    drop(blocker);
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal.len(), journal);
}
