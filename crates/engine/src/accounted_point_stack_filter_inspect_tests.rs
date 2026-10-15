//! UNRUN metadata-only two-node storage queries. These synthetic operation names
//! grant no source, renderer or command qualification.
use super::point_tests::{ctl, fixture};
use super::*;
use photocraft_doc::{
    ColorMode, SampleEncoding, SampleType,
    accounted_document::AccountedDocument,
    item_identity::{FilterId, ItemRef, MaskId},
};
use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering};

fn session() -> (Session, BackingQuota, Arc<std::sync::atomic::AtomicU64>) {
    let (mut doc, quota, guard, reads, _, _) = fixture(ColorMode::Rgb, SampleType::F64, SampleEncoding::NonlinearProfile);
    // The fixture's controlled source/model capture and this extra metadata
    // reservation are test setup, not production writer-accounting evidence.
    let fee = quota.reserve(QuotaKind::Index, 65536).unwrap();
    let xcf = doc.xcf_state.as_mut().unwrap();
    let first = &mut xcf.filter_stacks[0].nodes[0];
    first.operation.name = "current:metadata-only-first".into();
    first.operation.schema_epoch = "current-storage-only".into();
    first.composition.as_mut().unwrap().compatibility_epoch = "current-storage-only".into();
    let mut second = first.clone();
    second.id = FilterId(200);
    second.mask = Some(ItemRef::Mask(MaskId(201)));
    second.operation.name = "current:metadata-only-second".into();
    second.enabled = false;
    xcf.filter_stacks[0].nodes.push(second);
    let mut mask = xcf.filter_masks[0].clone();
    mask.id = MaskId(201);
    mask.owner = FilterId(200);
    xcf.filter_masks.push(mask);
    let mut item = xcf.items.iter().find(|item| item.item == ItemRef::Mask(MaskId(101))).unwrap().clone();
    item.item = ItemRef::Mask(MaskId(201));
    xcf.items.push(item);
    let owner = AccountedDocument::new_readonly_point_storage(doc, Arc::new((guard, fee)), quota.clone(), &ctl()).unwrap();
    let state = crate::DocState::new_accounted(owner, None, 16, &ctl()).unwrap();
    let mut session = Session::new();
    // Explicit private metadata setup; generic public Point adoption stays closed.
    session.publish_document_state(state, &ctl()).unwrap();
    (session, quota, reads)
}

#[test]
fn two_node_inspection_targets_each_stable_id_and_retains_paid_reply_without_native_reads() {
    let (s, quota, reads) = session();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.clone();
    let journal = s.journal.clone();
    let baseline = quota.usage().unwrap();
    for (id, name, enabled) in [(100, "current:metadata-only-first", true), (200, "current:metadata-only-second", false)] {
        let reply = prepare(&s, &json!({"filterId":id.to_string()}), &ctl()).unwrap();
        let xcf = s.active().unwrap().doc.xcf_state.as_ref().unwrap();
        let stack = &xcf.filter_stacks[0];
        let node = stack.nodes.iter().find(|node| node.id == FilterId(id)).unwrap();
        assert_eq!(node.operation.name, name);
        assert_eq!(node.enabled, enabled);
        let oracle_fee = quota.reserve(QuotaKind::Index, point_envelope(point_metadata(node, &ctl()).unwrap()).unwrap()).unwrap();
        let expected = crate::xcf_filter_cmds::build_point_storage_inspect_reply(node.id, stack, node).unwrap();
        assert_eq!(reply.value(), &expected);
        assert_eq!(reply.value()["filterId"], id.to_string());
        assert!(quota.usage().unwrap().index > baseline.index);
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        drop(expected);
        drop(oracle_fee);
        drop(reply);
        assert_eq!(quota.usage().unwrap(), baseline);
    }
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert!(s.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(s.journal, journal);
}

#[test]
fn two_node_inspection_rejects_unknown_noncanonical_and_interrupted_requests_without_mutation() {
    let (s, quota, reads) = session();
    let before = s.active().unwrap().doc.clone();
    let baseline = quota.usage().unwrap();
    for params in [json!({"filterId":"999"}), json!({"filterId":"0200"}), json!({"filterId":200}), json!({"filterId":"200","index":1})] {
        assert!(prepare(&s, &params, &ctl()).is_err());
        assert_eq!(quota.usage().unwrap(), baseline);
    }
    let deny = |_| Err(BackingError::Work);
    assert!(prepare(&s, &json!({"filterId":"200"}), &ReadControl { interrupt: &Interrupt::NONE, work: &deny }).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(quota.usage().unwrap(), baseline);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    // Other actual storage classes still use the original singleton bounds.
    assert_eq!((FILTER_BOUNDS.nodes, FILTER_BOUNDS.objects, FILTER_BOUNDS.depth), (256, 32, 8));
}

#[test]
fn point_move_dispatch_hint_preserves_wire_and_background_fences_without_granting_evaluation() {
    let (mut s, quota, reads) = session();
    let before = s.active().unwrap().doc.clone();
    let params = json!({"filterId":"200","index":0});
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_move(&s, "xcf.filter.move", &params, false).is_ok());
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_move(&s, "xcf.filter.move", &params, true).is_err());
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_move(&s, "xcf.filter.remove", &json!({"filterId":"200"}), false).is_err());
    for params in [
        json!({"filterId":"200","index":1024}),
        json!({"filterId":"200","index":-1}),
        json!({"filterId":"0200","index":0}),
        json!({"filterId":"200","index":0,"coalesce":null}),
    ] {
        assert!(crate::xcf_filter_cmds::pre_dispatch_point_move(&s, "xcf.filter.move", &params, false).is_err());
    }
    // The synthetic metadata fixture cannot become editable from the hint.
    // The real dispatch still requires a complete qualified current pipeline.
    assert!(s.execute("xcf.filter.move", params).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(quota.usage().is_ok());
}

#[test]
fn point_remove_dispatch_hint_keeps_exact_wire_and_cannot_authorize_an_unqualified_recipe() {
    let (mut s, quota, reads) = session();
    let before = s.active().unwrap().doc.clone();
    let params = json!({"filterId":"200"});
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_remove(&s, "xcf.filter.remove", &params, false).is_ok());
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_remove(&s, "xcf.filter.remove", &params, true).is_err());
    assert!(crate::xcf_filter_cmds::pre_dispatch_point_remove(&s, "xcf.filter.move", &params, false).is_err());
    for params in [json!({"filterId":"0200"}), json!({"filterId":200}), json!({"filterId":"200","index":0}), json!({"filterId":"200","coalesce":null})] {
        assert!(crate::xcf_filter_cmds::pre_dispatch_point_remove(&s, "xcf.filter.remove", &params, false).is_err());
    }
    assert!(s.execute("xcf.filter.remove", params).is_err());
    assert!(s.active().unwrap().doc.shares_state(&before));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert!(quota.usage().is_ok());
}
