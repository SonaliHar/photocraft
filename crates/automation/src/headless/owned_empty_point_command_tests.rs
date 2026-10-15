//! UNRUN CURRENT empty-stack lifecycle. Original source remains an independently
//! authored two-node stack; no zero-operation source qualification is asserted.
//! File buffers and expected metadata clones are external test oracles.
use super::*;
use photocraft_doc::{FilterOutputPrecision, LayerContent, item_identity::ItemRef};
use photocraft_raster::{Interrupt, backing::ReadControl, checked_view::NativeTileBytes};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
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
        let path = std::env::temp_dir().join(format!("photocraft-empty-point-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
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
fn native(h: &Headless) -> (photocraft_raster::backing::NativePlaneDescriptor, String) {
    let LayerContent::BackedRaster(layer) = &h.session.active().unwrap().doc.layers[0].content else { panic!("real native owner") };
    let mut digest = Sha256::new();
    digest.update(layer.descriptor().default_bytes());
    for coord in layer.descriptor().bounds().tiles() {
        let pin = layer.checked_view().pin_tile(coord, &ctl()).unwrap();
        match pin.encoded() {
            NativeTileBytes::FullTile(bytes) | NativeTileBytes::DefaultPixel(bytes) => digest.update(bytes),
        }
    }
    (layer.descriptor(), digest.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn query_empty(h: &mut Headless, stack_revision: u64) {
    let reply = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
    assert_eq!(reply.value()["filters"]["total"], 0);
    assert!(reply.value()["filters"]["entries"].as_array().unwrap().is_empty());
    assert_eq!(reply.value()["filterStacks"]["total"], 1);
    assert_eq!(reply.value()["filterStacks"]["entries"][0]["nodes"], 0);
    assert_eq!(reply.value()["filterStacks"]["entries"][0]["revision"], stack_revision.to_string());
    assert_eq!(reply.value()["items"]["total"], 1);
}

#[test]
#[ignore = "UNRUN: genuine removals in both orders preserve empty stack/revision/raw archive, undo and paid native continuation"]
fn empty_current_stack_survives_discovery_undo_metadata_and_native_reopen_without_dummy_node() {
    for order in [[0usize, 1usize], [1usize, 0usize]] {
        let dir = Directory::new();
        let path = dir.0.join("stack.xcf");
        let original = source();
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source_identity = h.session.active().unwrap().source.clone();
        let raw = native(&h);
        let state = before.xcf_state.as_ref().unwrap();
        let nodes = &state.filter_stacks[0].nodes;
        assert_eq!(nodes.len(), 2);
        let owner = state.filter_stacks[0].owner;
        let ids: Vec<_> = nodes.iter().map(|n| n.id.0.to_string()).collect();
        let masks: Vec<_> = state.filter_masks.iter().map(|m| ItemRef::Mask(m.id)).collect();
        assert_eq!(masks.len(), 2);
        let mut expected = state.clone();
        expected.filter_stacks[0].nodes.clear();
        expected.filter_stacks[0].revision = expected.filter_stacks[0].revision.checked_add(2).unwrap();
        expected.filter_masks.clear();
        expected.items.retain(|item| !masks.contains(&item.item));
        expected.current_filter_output_precision = FilterOutputPrecision::Canonical;
        for record in &mut expected.opaque {
            if let Some(ItemRef::Mask(mask)) = record.owner
                && masks.contains(&ItemRef::Mask(mask))
            {
                assert_eq!(record.namespace, "gimp/xcf/source-property");
                assert!(!record.rendering_required);
                record.owner = None;
                record.identity = format!("deleted-filter-mask/{}/{}", mask.0, record.identity);
            }
        }
        for index in order {
            assert_eq!(h.command_run("xcf.filter.remove", json!({"filterId":ids[index]})).unwrap()["updated"], true);
        }
        let empty = h.session.active().unwrap().doc.clone();
        assert_eq!(empty.xcf_state.as_ref().unwrap(), &expected);
        assert_eq!(expected.filter_stacks[0].owner, owner);
        assert!(empty.accounted().unwrap().is_readonly_point_storage());
        assert_eq!(h.session.active().unwrap().history.past_len(), 2);
        assert_eq!(native(&h), raw);
        for (a, b) in state.opaque.iter().zip(&empty.xcf_state.as_ref().unwrap().opaque) {
            assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
        }
        query_empty(&mut h, expected.filter_stacks[0].revision);
        for id in &ids {
            assert!(h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).is_err());
        }
        h.command_run("edit.undo", json!({})).unwrap();
        assert_eq!(h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(), 1);
        let history = h.session.active().unwrap().history.clone();
        let survivor = &h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let noop = json!({"filterId":survivor.id.0.to_string(),"enabled":survivor.enabled});
        assert_eq!(h.command_run("xcf.filter.enable", noop).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        h.command_run("edit.redo", json!({})).unwrap();
        query_empty(&mut h, expected.filter_stacks[0].revision);
        h.command_run("edit.undo", json!({})).unwrap();
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        h.command_run("edit.redo", json!({})).unwrap();
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(h.session.active().unwrap().doc.xcf_state.as_ref().unwrap(), &expected);
        let output = dir.0.join("empty.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, source_identity);
        assert!(h.session.active().unwrap().source_read_only);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(!current.source_read_only);
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert_eq!(current.doc.xcf_state.as_ref().unwrap(), &expected);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        query_empty(&mut reopened, expected.filter_stacks[0].revision);
        assert_eq!(reopened.command_run("layer.setProps", json!({"name":"Empty current layer"})).unwrap(), Value::Null);
        assert_eq!(reopened.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks, expected.filter_stacks);
        assert_eq!(native(&reopened), raw);
        reopened.command_run("edit.undo", json!({})).unwrap();
        query_empty(&mut reopened, expected.filter_stacks[0].revision);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: empty runtime Point retains synchronous/auth/query controls and has no filter target"]
fn empty_current_has_no_filter_target_and_refusals_do_not_publish() {
    let dir = Directory::new();
    let path = dir.0.join("stack.xcf");
    let original = source();
    std::fs::write(&path, &original).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
    let ids: Vec<_> = h.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.iter().map(|n| n.id.0.to_string()).collect();
    for id in &ids {
        h.command_run("xcf.filter.remove", json!({"filterId":id})).unwrap();
    }
    let before = h.session.active().unwrap().doc.clone();
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    for command in ["xcf.filter.set", "xcf.filter.enable", "xcf.filter.move", "xcf.filter.remove"] {
        assert!(
            matches!(h.command_start(command,json!({"unparsed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
        );
    }
    for id in &ids {
        for (command, params) in [
            ("xcf.filter.set", json!({"filterId":id,"enabled":false})),
            ("xcf.filter.enable", json!({"filterId":id,"enabled":false})),
            ("xcf.filter.move", json!({"filterId":id,"index":0})),
            ("xcf.filter.remove", json!({"filterId":id})),
        ] {
            assert!(h.command_run(command, params).is_err());
        }
        assert!(h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).is_err());
    }
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny empty CURRENT metadata".into())));
    assert!(h.command_run("layer.setProps", json!({"name":"denied"})).is_err());
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
    assert!(!h.session.has_jobs());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
