//! UNRUN authentic two-node discovery; metadata grants no additional source or
//! render eligibility. The production selected reader owns encoded capture.
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
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
fn session() -> (Session, BackingQuota) {
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
    let photocraft_io::OwnedXcfImport::Point(imported) = imported else { panic!("exact witnessed PointStackEdits family") };
    let mut session = Session::new();
    session.add_owned_point_xcf_import(&imported, "stack.xcf", None, &ctl()).unwrap();
    assert_eq!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 2);
    (session, quota)
}
#[test]
#[ignore = "UNRUN: SHA-pinned two-node original and bounded paid discovery pages"]
fn actual_two_node_pages_keep_order_ids_totals_and_all_item_text_facts_paid() {
    let (mut s, quota) = session();
    // Canonical current text only; the source/native/effect arrays stay held.
    s.execute("layer.setProps", json!({"name":"λ".repeat(260)})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.len();
    let state = before.xcf_state.as_ref().unwrap();
    let nodes = &state.filter_stacks[0].nodes;
    let ids: Vec<_> = nodes.iter().map(|n| n.id.0.to_string()).collect();
    assert_ne!(ids[0], ids[1]);
    assert_eq!(state.filter_masks.len(), 2);
    assert_eq!(state.items.len(), 1 + state.filter_masks.len());
    assert!(state.items.len() <= 3);
    for (offset, limit) in [(0, 128), (0, 1), (1, 1), (2, 128), (100000, 128)] {
        let baseline = quota.usage().unwrap();
        let reply = prepare(&s, &json!({"offset":offset,"limit":limit}), &ctl()).unwrap();
        assert!(reply.quota().shares_ledger(&quota));
        assert_eq!(reply.value()["filters"]["total"], 2);
        let rows = reply.value()["filters"]["entries"].as_array().unwrap();
        let expected: Vec<_> = ids.iter().skip(offset).take(limit).collect();
        assert_eq!(rows.len(), expected.len());
        for (row, id) in rows.iter().zip(expected) {
            assert_eq!(row["id"], *id);
            assert_eq!(row["inspectCommand"], "xcf.filter.inspect");
        }
        assert_eq!(reply.value()["items"]["total"], state.items.len());
        let item_rows = reply.value()["items"]["entries"].as_array().unwrap();
        assert_eq!(item_rows.len(), state.items.len().saturating_sub(offset).min(limit));
        for (row, item) in item_rows.iter().zip(state.items.iter().skip(offset).take(limit)) {
            let (kind, id) = match item.item {
                photocraft_doc::item_identity::ItemRef::Layer(id) => ("layer", id.0),
                photocraft_doc::item_identity::ItemRef::Mask(id) => ("mask", id.0),
                _ => panic!("closed layer/mask item shape"),
            };
            assert_eq!(row["item"]["kind"], kind);
            assert_eq!(row["item"]["id"], id.to_string());
        }
        if let Some(layer) = item_rows.iter().find(|row| row["item"]["kind"] == "layer") {
            assert_eq!(layer["nameTruncated"], true);
        }
        assert_eq!(reply.value()["filterStacks"]["total"], 1);
        assert!(quota.usage().unwrap().index > baseline.index);
        drop(reply);
        assert_eq!(quota.usage().unwrap(), baseline);
    }
    for node in nodes {
        let reply = crate::accounted_filter_inspect::prepare(&s, &json!({"filterId":node.id.0.to_string()}), &ctl()).unwrap();
        assert_eq!(reply.value()["filterId"], node.id.0.to_string());
        assert_eq!(reply.value()["operation"]["name"], node.operation.name);
    }
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal.len(), journal);
}
#[test]
#[ignore = "UNRUN: actual two-node owner quota/control failures and pagination guards"]
fn two_node_project_failures_refund_reply_without_snapshot_or_journal_changes() {
    let (s, quota) = session();
    let before = s.active().unwrap().doc.clone();
    let baseline = quota.usage().unwrap();
    let journal = s.journal.len();
    for p in [json!({"limit":0}), json!({"offset":100001}), json!({"extra":true})] {
        assert!(prepare(&s, &p, &ctl()).is_err());
        assert_eq!(quota.usage().unwrap(), baseline);
    }
    assert!(prepare(&s, &Value::Null, &ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) }).is_err());
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    assert!(matches!(prepare(&s, &Value::Null, &ReadControl { interrupt: &interrupt, work: &|_| Ok(()) }), Err(EngineError::Cancelled)));
    assert_eq!(quota.usage().unwrap(), baseline);
    let blocker = quota.reserve(QuotaKind::Index, (32 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &Value::Null, &ctl()).is_err());
    drop(blocker);
    assert_eq!(quota.usage().unwrap(), baseline);
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(s.journal.len(), journal);
}
