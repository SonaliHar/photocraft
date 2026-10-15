//! UNRUN genuine selected assigned-ICC geometry CURRENT metadata queries.
//! Source buffers and expected JSON are external harness oracles, not renderer
//! proof or accounting evidence for production allocations.
use super::*;
use photocraft_doc::item_identity::ItemRef;
use photocraft_raster::{Interrupt, backing::quota::QuotaLimits};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf};

const CASES: [(&str, &str); 2] = [
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788"),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.xcf", "0649a852c01bf152bf5e7f384fd64896cec4394eaca794c977d8142d168d80ab"),
];
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn fixture(case: usize) -> (Session, BackingQuota) {
    let (relative, expected) = CASES[case];
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap());
    let bytes = std::fs::read(root.join(relative)).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, expected);
    let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_xcf_from_reader_in_quota(
        "geometry.xcf",
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::AnalyticGeometry(imported) = imported else { panic!("actual original geometry family") };
    let mut session = Session::new();
    session.add_owned_analytic_geometry_xcf_import(&imported, "geometry.xcf", None, &ctl()).unwrap();
    (session, q)
}

#[test]
#[ignore = "UNRUN: pinned RGB F64 and Gray U32 masked geometry paid query matches exact existing metadata schema"]
fn genuine_mask_items_are_paginated_and_paid_without_mutating_current_state() {
    for case in 0..CASES.len() {
        let (session, q) = fixture(case);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.clone();
        let revision = session.active().unwrap().revision;
        let journal = session.journal.len();
        let state = before.xcf_state.as_ref().unwrap();
        assert_eq!(state.items.len(), 3);
        assert_eq!(state.items.iter().filter(|item| matches!(item.item, ItemRef::Mask(_))).count(), 1);
        // Charge the external expected JSON before its legacy test-only builder.
        let oracle = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
        for params in
            [Value::Null, json!({"offset":0,"limit":1}), json!({"offset":1,"limit":1}), json!({"offset":2,"limit":1}), json!({"offset":3,"limit":128})]
        {
            let (offset, limit) = pagination(&params, &ctl()).unwrap();
            let resolution = before.effective_resolution();
            let expected = crate::xcf_inspect_cmds::build_inspect_reply(&before, state, &resolution, offset, limit, None).unwrap();
            let baseline = q.usage().unwrap();
            let reply = prepare(&session, &params, &ctl()).unwrap();
            assert!(reply.quota().shares_ledger(&q));
            assert_eq!(reply.command(), "xcf.inspect");
            assert_eq!(reply.value(), &expected);
            assert_eq!(reply.value()["items"]["total"], 3);
            assert_eq!(reply.value()["filters"]["total"], 0);
            assert_eq!(reply.value()["filterStacks"]["total"], 0);
            assert_eq!(reply.value()["opaqueRecords"], state.opaque.len());
            assert!(q.usage().unwrap().index > baseline.index);
            drop(reply);
            assert_eq!(q.usage().unwrap(), baseline);
        }
        drop(oracle);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.journal.len(), journal);
    }
}

#[test]
#[ignore = "UNRUN: masked metadata query work/cancel/quota/page refusals preserve paid source and grant no filter target"]
fn query_failures_refund_and_do_not_open_filter_or_edit_routes() {
    let (mut session, q) = fixture(0);
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.clone();
    let revision = session.active().unwrap().revision;
    let journal = session.journal.len();
    let baseline = q.usage().unwrap();
    for params in [json!({"limit":0}), json!({"offset":100001}), json!({"extra":true})] {
        assert!(prepare(&session, &params, &ctl()).is_err());
    }
    let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
    assert!(prepare(&session, &Value::Null, &denied).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(prepare(&session, &Value::Null, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    assert_eq!(q.usage().unwrap(), baseline);
    let blocker = q.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    assert!(prepare(&session, &Value::Null, &ctl()).is_err());
    drop(blocker);
    assert!(crate::accounted_filter_inspect::prepare(&session, &json!({"filterId":"1"}), &ctl()).is_err());
    assert!(session.execute("layer.setProps", json!({"blend":"Multiply"})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert!(session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.journal.len(), journal);
    assert_eq!(q.usage().unwrap(), baseline);
}
