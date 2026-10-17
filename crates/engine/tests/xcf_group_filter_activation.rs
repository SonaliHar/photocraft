#![cfg(feature = "corpus")]
//! Unrun registered-command drafts using the retained authored group witness.
//! No new source operation/epoch/reference enrollment is performed here.
use photocraft_doc::{Document, item_identity::FilterId};
use photocraft_engine::{Session, xcf_filter_cmds};
use photocraft_raster::Interrupt;
use serde_json::json;
use sha2::{Digest, Sha256};
fn original() -> Document {
    let root = std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/xcf/full"))
        .join("composition-matrix");
    let bytes = std::fs::read(root.join("group-28-group-mask.xcf")).unwrap();
    let hashes: serde_json::Value = serde_json::from_str(include_str!("../../compose/tests/gimp_gaussian_group_hashes.json")).unwrap();
    let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(sha, hashes["group-28-group-mask.xcf"].as_str().unwrap());
    photocraft_io::import("group-28-group-mask.xcf", &bytes).unwrap().document
}
fn target(doc: &Document) -> FilterId {
    doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.operation.schema_epoch == "xcf22-isolated-small-auto-v1").unwrap().id
}
fn bytes(doc: &Document) -> Vec<u8> {
    photocraft_io::save_native_bytes(doc, photocraft_io::NativeSaveLimits::default(), &Interrupt::NONE).unwrap().0
}
#[test]
#[ignore = "requires retained unpublished authored group XCF; execution deferred"]
fn registered_group_activation_preserves_source_native_words_and_real_redo() {
    let doc = original();
    let id = target(&doc);
    let original_bytes = bytes(&doc);
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    let before = session.active().unwrap().doc.clone();
    let before_state = before.xcf_state.as_ref().unwrap().clone();
    let result = session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
    assert_eq!(result["updated"], true);
    assert_eq!(session.active().unwrap().history.past_len(), 1);
    let current = session.active().unwrap().doc.xcf_state.as_ref().unwrap();
    assert_eq!(current.provenance, before_state.provenance);
    assert_eq!(current.opaque, before_state.opaque);
    assert_eq!(current.source_items, before_state.source_items);
    assert_eq!(current.resources, before_state.resources);
    assert_eq!(current.filter_masks, before_state.filter_masks);
    let mut expected = before_state.clone();
    let stack = expected.filter_stacks.iter_mut().find(|s| s.nodes.iter().any(|n| n.id == id)).unwrap();
    stack.nodes[0].enabled = false;
    stack.revision += 1;
    assert_eq!(current, &expected);
    let disabled = session.active().unwrap().doc.clone();
    let disabled_bytes = bytes(&disabled);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bytes(&session.active().unwrap().doc), original_bytes);
    let noop = session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":true})).unwrap();
    assert_eq!(noop["updated"], false);
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    session.execute("edit.redo", json!({})).unwrap();
    assert_eq!(bytes(&session.active().unwrap().doc), disabled_bytes);
    session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":true})).unwrap();
    assert!(session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.id == id).unwrap().enabled);
}
#[test]
#[ignore = "requires retained unpublished authored group XCF; execution deferred"]
fn group_activation_does_not_grant_arguments_opacity_or_disabled_tuple_normalization() {
    let mut doc = original();
    let id = target(&doc);
    let mut session = Session::new();
    session.add_document(doc.clone(), None).unwrap();
    let before = session.active().unwrap().doc.clone();
    for payload in [
        json!({"filterId":id.0.to_string(),"enabled":false,"arguments":[]}),
        json!({"filterId":id.0.to_string(),"enabled":false,"opacityF64Bits":"0x3ff0000000000000"}),
    ] {
        assert!(session.execute("xcf.filter.set", payload).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.past_len(), 0);
    }
    let node = doc.xcf_state.as_mut().unwrap().filter_stacks.iter_mut().flat_map(|s| &mut s.nodes).find(|n| n.id == id).unwrap();
    node.enabled = false;
    let policy = node.composition.as_mut().unwrap();
    policy.stored_blend_space = -1474592175;
    policy.stored_composite_space = -1474592175;
    policy.stored_operator = -1474592175;
    let mut session = Session::new();
    session.add_document(doc, None).unwrap();
    let before = session.active().unwrap().doc.clone();
    let noop = session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
    assert_eq!(noop["updated"], false);
    assert!(session.execute("xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":true})).is_err());
    assert!(session.active().unwrap().doc.shares_state(&before));
    assert_eq!(session.active().unwrap().history.past_len(), 0);
    let stop = || true;
    assert!(matches!(
        xcf_filter_cmds::enable_with_interrupt(&mut session, &json!({"filterId":id.0.to_string(),"enabled":true}), &Interrupt::cancel_only(&stop)),
        Err(photocraft_engine::EngineError::Cancelled)
    ));
    assert!(session.active().unwrap().doc.shares_state(&before));
}
