//! UNRUN: paid CURRENT metadata fixtures are not original-XCF lineage proofs.
use super::*;
use photocraft_doc::{
    accounted_document::AccountedDocument,
    item_identity::{ItemRef, ItemSetId, ResourceId},
    xcf_editing::{ItemFamily, ItemSet, ItemSetRule},
    xcf_resources::{ResourceKind, ResourceReference},
};
use photocraft_raster::{
    Interrupt,
    backing::quota::{QuotaLimits, QuotaUsage},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[path = "../../xcf/tests/common/mod.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod xcf;
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
// Own every shared original payload before the separate prepaid current-model
// envelope can be refunded. This trusted adoption fixture makes no source claim.
struct RetainedCurrent {
    _original: AccountedDocument,
    _metadata: QuotaLease,
}
fn fixture(long: bool) -> Session {
    let q = BackingQuota::new_with_aggregate_initial(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 16 << 20 }, 256 << 20, QuotaUsage::default())
        .unwrap()
        .0;
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 128 << 20), ("indexBytes", 16 << 20), ("scratchBytes", 0)])
        .unwrap();
    let imported = photocraft_io::import_owned_staged_plain_bytes_in_quota(
        "original.xcf",
        Arc::new(xcf::image(13, 150, 0, 1, 1, &[80, 120, 160, 255])),
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let original = imported.document().clone();
    // Reserve a real conservative standard-allocator envelope BEFORE cloning the
    // model and adding bounded Strings/Vecs/Arcs, not a numeric ownership assertion.
    let metadata = q.reserve(QuotaKind::Index, 8 << 20).unwrap();
    let mut doc = original.document().clone();
    let layer = doc.layers[0].id;
    let state = doc.xcf_state.as_mut().unwrap();
    for index in 0..3u64 {
        state.resources.push(ResourceReference {
            id: ResourceId(100 + index),
            kind: ResourceKind::Pattern,
            name: format!("resource-{index}"),
            collection: None,
            internal: false,
            locator: Some("inert/never-opened".into()),
            content_hash: Some(if long && index == 1 { "\u{03bb}".repeat(40000) } else { format!("{index:064x}") }),
            owned_bytes: Some(Arc::new(vec![index as u8; 5])),
            resolution: None,
        });
        state.item_sets.push(ItemSet {
            id: ItemSetId(200 + index),
            name: format!("set-{index}"),
            family: ItemFamily::Layer,
            rule: ItemSetRule::Regex(if long && index == 1 { "\u{03bb}".repeat(40000) } else { ".*".into() }),
            members: vec![ItemRef::Layer(layer); 3],
        });
    }
    let owner = AccountedDocument::from_owned(doc, Arc::new(RetainedCurrent { _original: original, _metadata: metadata }), q, &ctl()).unwrap();
    owner.validate_plain_xcf_inspection(&ctl()).unwrap();
    let mut session = Session::new();
    session.add_accounted_document(owner, None, &ctl()).unwrap();
    session
}
#[test]
#[ignore = "UNRUN: genuine paid plain current-model resource/set fixture"]
fn full_schema_and_independent_nested_pagination_match_existing_builder() {
    let s = fixture(false);
    let doc = &s.active().unwrap().doc;
    let state = doc.xcf_state.as_ref().unwrap();
    let resolution = doc.effective_resolution();
    // The comparison tree also owns allocations, covered by a real test lease.
    let _oracle = doc.accounted().unwrap().quota().reserve(QuotaKind::Index, 4 << 20).unwrap();
    for params in [Value::Null, json!({}), json!({"offset":1,"limit":1}), json!({"offset":100000,"limit":128})] {
        let (offset, limit) = pagination(&params, &ctl()).unwrap();
        let expected = crate::xcf_inspect_cmds::build_inspect_reply(doc, state, &resolution, offset, limit, None).unwrap();
        let reply = prepare(&s, &params, &ctl()).unwrap();
        assert_eq!(reply.command(), "xcf.inspect");
        assert_eq!(reply.value(), &expected);
        assert!(reply.quota().shares_ledger(doc.accounted().unwrap().quota()));
    }
    let reply = prepare(&s, &json!({"offset":1,"limit":1}), &ctl()).unwrap();
    assert_eq!(reply.value()["savedSets"]["entries"][0]["members"]["offset"], 1);
    assert_eq!(reply.value()["savedSets"]["entries"][0]["members"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(reply.value()["resources"]["entries"][0]["accessGranted"], false);
    assert_eq!(reply.value()["resources"]["entries"][0]["ownedBytes"], 5);
}
#[test]
#[ignore = "UNRUN: paid UTF-8 hash/rule full scans and reply lifetime"]
fn untruncated_hash_full_rule_scan_and_real_reply_retention_preserve_session() {
    let s = fixture(true);
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let history = st.history.clone();
    let q = st.doc.accounted().unwrap().quota();
    let baseline = q.usage().unwrap();
    let work = AtomicU64::new(0);
    let debit = |n| {
        work.fetch_add(n, Ordering::Relaxed);
        Ok(())
    };
    let control = ReadControl { interrupt: &Interrupt::NONE, work: &debit };
    let reply = prepare(&s, &json!({"offset":1,"limit":1}), &control).unwrap();
    assert_eq!(reply.value()["resources"]["entries"][0]["contentHash"].as_str().unwrap(), "\u{03bb}".repeat(40000));
    assert_eq!(reply.value()["savedSets"]["entries"][0]["ruleExpression"].as_str().unwrap().chars().count(), 256);
    assert!(work.load(Ordering::Relaxed) > 160000);
    assert!(q.usage().unwrap().index > baseline.index);
    assert!(st.doc.shares_state(&before));
    assert!(st.history.shares_accounted_history(&history));
    assert_eq!(st.doc.xcf_state, before.xcf_state);
    assert_eq!(st.doc.icc_profile, before.icc_profile);
    drop(reply);
    assert_eq!(q.usage().unwrap(), baseline);
}
#[test]
#[ignore = "UNRUN: actual same-ledger work/cancel/quota rollback"]
fn invalid_pagination_and_private_failures_never_mutate_or_leak() {
    let s = fixture(true);
    let st = s.active().unwrap();
    let q = st.doc.accounted().unwrap().quota();
    let before = st.doc.clone();
    let baseline = q.usage().unwrap();
    for p in [json!({"limit":0}), json!({"limit":129}), json!({"offset":100001}), json!({"offset":-1}), json!({"extra":true})] {
        assert!(prepare(&s, &p, &ctl()).is_err());
        assert_eq!(q.usage().unwrap(), baseline);
    }
    let deny = |_| Err(BackingError::Work);
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &deny };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let stop = || true;
    let interrupt = Interrupt::cancel_only(&stop);
    let c = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    // Reject during a bounded full UTF-8 scan after prospective reply admission.
    let admitted = AtomicU64::new(0);
    let late = |n| {
        if n > 1000000 {
            admitted.store(1, Ordering::Relaxed);
        } else if n == 32769 && admitted.load(Ordering::Relaxed) != 0 {
            return Err(BackingError::Cancelled);
        }
        Ok(())
    };
    let c = ReadControl { interrupt: &Interrupt::NONE, work: &late };
    assert!(prepare(&s, &Value::Null, &c).is_err());
    assert_eq!(q.usage().unwrap(), baseline);
    let blocker = q.reserve(QuotaKind::Index, (64 << 20) - baseline.index).unwrap();
    assert!(prepare(&s, &Value::Null, &ctl()).is_err());
    drop(blocker);
    assert_eq!(q.usage().unwrap(), baseline);
    assert!(st.doc.shares_state(&before));
}
#[test]
#[ignore = "UNRUN: actual paged nested-reference planning and closed gate"]
fn plan_counts_actual_128_by_128_refs_and_refuses_unowned_sessions() {
    let s = fixture(false);
    let st = s.active().unwrap();
    let doc = &st.doc;
    let q = doc.accounted().unwrap().quota();
    let _fixture = q.reserve(QuotaKind::Index, 4 << 20).unwrap();
    let mut state = doc.xcf_state.as_ref().unwrap().clone();
    let template = state.item_sets[0].clone();
    state.item_sets.clear();
    for i in 0..128 {
        let mut set = template.clone();
        set.id = ItemSetId(1000 + i);
        set.members = vec![ItemRef::Layer(doc.layers[0].id); 128];
        state.item_sets.push(set);
    }
    let resolution = doc.effective_resolution();
    let p = plan(&state, &resolution, 0, 128, &ctl()).unwrap();
    assert_eq!(p.sets, 128);
    assert!(p.bounds.nodes >= 128 * 128 * 8);
    assert!(p.bytes > 8 << 20);
    assert!(prepare(&Session::new(), &Value::Null, &ctl()).is_err());
    let mut resident = Session::new();
    resident
        .add_document(
            photocraft_doc::Document::new("unowned", photocraft_geom::Size::new(1, 1), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
            None,
        )
        .unwrap();
    assert!(prepare(&resident, &Value::Null, &ctl()).is_err());
}
