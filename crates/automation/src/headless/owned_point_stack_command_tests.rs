//! UNRUN authentic witnessed two-node Point continuation. No arbitrary stack
//! composition, template insertion or new source eligibility is asserted.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::{NativePlaneDescriptor, ReadControl},
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|v| format!("{v:02x}")).collect()
}
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-stack-headless-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn source() -> Vec<u8> {
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap()).join("filter-stack-edits-evidence");
    assert_eq!(hash(&std::fs::read(root.join("SHA256SUMS.json")).unwrap()), "99038395a4cdf4c3b3ea420ffa5aee46f6bf47c679870d76be933d8e5405170a");
    let bytes = std::fs::read(root.join("stack.xcf")).unwrap();
    assert_eq!(hash(&bytes), "f8f1c11303df47a49b817c6cd745c08edd18fbb1045daa8fdf77111c89f4cbdd");
    bytes
}
fn native(h: &Headless) -> (NativePlaneDescriptor, String) {
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::BackedRaster(layer) = &doc.layers[0].content else { panic!("actual native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
        }
    }
    for mask in &doc.xcf_state.as_ref().unwrap().filter_masks {
        digest.update(mask.surface.default_bytes());
        for (_, tile) in mask.surface.tiles() {
            digest.update(tile.bytes());
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn exposure(id: &str, bits: u32) -> Value {
    json!({"filterId":id,"arguments":[{"name":"exposure","value":{"Number":{"Float32Bits":bits}}}]})
}

#[test]
#[ignore = "UNRUN: pinned witnessed two-node source, stable-ID activation/noopredo and native continuation"]
fn authentic_two_node_discovery_nonfirst_edit_and_native_continue_preserve_original() {
    let dir = Directory::new();
    let original = source();
    let source_path = dir.0.join("stack.xcf");
    std::fs::write(&source_path, &original).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let raw = native(&h);
    let nodes = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
    assert_eq!(nodes.len(), 2);
    let exposure_id = nodes.iter().find(|node| node.operation.name == "gegl:exposure").unwrap().id.0.to_string();
    let second_id = nodes[1].id.0.to_string();
    let second_enabled = nodes[1].enabled;
    let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
    assert_eq!(query.value()["filters"]["total"], 2);
    let rows = query.value()["filters"]["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (row, node) in rows.iter().zip(nodes) {
        assert_eq!(row["id"], node.id.0.to_string());
    }
    drop(query);
    for node in nodes {
        let reply = h.inspect_owned_filter_bounded(&json!({"filterId":node.id.0.to_string()}), true).unwrap();
        assert_eq!(reply.value()["operation"]["name"], node.operation.name);
    }
    assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":second_id,"enabled":!second_enabled})).unwrap()["updated"], true);
    let changed = h.session.active().unwrap().doc.clone();
    assert_eq!(h.session.active().unwrap().history.past_len(), 1);
    assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0], nodes[0]);
    h.command_run("edit.undo", json!({})).unwrap();
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    assert_eq!(h.command_run("xcf.filter.set", exposure(&exposure_id, 0.5f32.to_bits())).unwrap()["updated"], false);
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(h.session.active().unwrap().revision, revision);
    h.command_run("edit.redo", json!({})).unwrap();
    assert_eq!(h.session.active().unwrap().doc.xcf_state, changed.xcf_state);
    assert_eq!(native(&h), raw);
    // Values outside the existing witnessed stack recipe do not become valid
    // merely because the stable-ID patch reaches its intended node.
    assert!(h.command_run("xcf.filter.set", exposure(&exposure_id, 0.75f32.to_bits())).is_err());
    let destination = dir.0.join("stack.pcraft");
    assert!(h.save_owned_native_active(Some(&source_path), Some("pcraft"), &ctl()).is_err());
    h.save_owned_native_active(Some(&destination), Some("pcraft"), &ctl()).unwrap();
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current(&destination, &ctl()).unwrap());
    let state = reopened.session.active().unwrap();
    assert!(state.doc.accounted().unwrap().is_readonly_point_storage());
    assert!(!state.source_read_only);
    assert_eq!(state.doc.xcf_state, changed.xcf_state);
    assert_eq!(state.doc.icc_profile, before.icc_profile);
    assert_eq!(state.doc.source_color_context, before.source_color_context);
    assert_eq!(state.doc.working_profile_interpretation, before.working_profile_interpretation);
    assert_eq!(native(&reopened), raw);
    assert_eq!(reopened.command_run("xcf.filter.enable", json!({"filterId":second_id,"enabled":second_enabled})).unwrap()["updated"], true);
    assert_eq!(std::fs::read(source_path).unwrap(), original);
}

#[test]
#[ignore = "UNRUN: two-node waitfalse, semantic and engine-authorizer fences"]
fn two_node_host_refusals_do_not_publish_or_start_jobs() {
    let dir = Directory::new();
    let path = dir.0.join("stack.xcf");
    std::fs::write(&path, source()).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].id.0.to_string();
    for command in ["layer.setProps", "xcf.filter.set", "xcf.filter.enable"] {
        assert!(
            matches!(h.command_start(command,json!({"unparsed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
        );
        assert!(!h.session.has_jobs());
    }
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny two-node activation".into())));
    assert!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":false})).is_err());
    assert!(h.command_run("layer.setProps", json!({"name":"denied"})).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
}

#[test]
#[ignore = "UNRUN: authentic witnessed stack move, exact no-op redo and native continuation"]
fn two_node_move_reverses_once_preserves_noop_redo_and_continues_after_native_save() {
    let dir = Directory::new();
    let original = source();
    let source_path = dir.0.join("stack.xcf");
    std::fs::write(&source_path, &original).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let source_identity = h.session.active().unwrap().source.clone();
    let raw = native(&h);
    let nodes = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
    assert_eq!(nodes.len(), 2);
    let moved = nodes[0].id.0.to_string();
    let request = json!({"filterId":moved,"index":1});
    assert!(matches!(h.command_start("xcf.filter.move", request.clone(), false), Err(AutomationError::BadRequest(ref why)) if why.contains("synchronous")));
    assert!(!h.session.has_jobs());
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny current stack move".into())));
    assert!(h.command_run("xcf.filter.move", request.clone()).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    h.session.authorize = None;
    assert_eq!(h.command_run("xcf.filter.move", request).unwrap(), json!({"filterId":moved,"updated":true}));
    let reversed = h.session.active().unwrap().doc.clone();
    let stack = &reversed.xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(stack.nodes[0], nodes[1]);
    assert_eq!(stack.nodes[1], nodes[0]);
    assert_eq!(stack.revision, before.xcf_state.as_ref().unwrap().filter_stacks[0].revision + 1);
    assert_eq!(h.session.active().unwrap().history.past_len(), 1);
    assert_eq!(native(&h), raw);
    h.command_run("edit.undo", json!({})).unwrap();
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    assert_eq!(h.command_run("xcf.filter.move", json!({"filterId":moved,"index":0})).unwrap()["updated"], false);
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    h.command_run("edit.redo", json!({})).unwrap();
    assert_eq!(h.session.active().unwrap().doc.xcf_state, reversed.xcf_state);
    let destination = dir.0.join("moved.pcraft");
    h.save_owned_native_active(Some(&destination), Some("pcraft"), &ctl()).unwrap();
    assert_eq!(h.session.active().unwrap().source, source_identity);
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current(&destination, &ctl()).unwrap());
    assert_eq!(reopened.session.active().unwrap().doc.xcf_state, reversed.xcf_state);
    assert_eq!(native(&reopened), raw);
    assert_eq!(reopened.command_run("xcf.filter.move", json!({"filterId":moved,"index":0})).unwrap()["updated"], true);
    let restored = &reopened.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0];
    assert_eq!(restored.nodes, *nodes);
    assert_eq!(std::fs::read(source_path).unwrap(), original);
}
