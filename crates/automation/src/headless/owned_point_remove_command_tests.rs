//! UNRUN genuine witnessed two-to-one removal through registered Headless APIs.
//! Expected-model clones and source file Vecs are external harness oracles;
//! actual imports, paid discovery, native serialization and reload own fees.
use super::*;
use photocraft_doc::{LayerContent, item_identity::ItemRef};
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
        let path = std::env::temp_dir().join(format!("photocraft-point-remove-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

#[test]
#[ignore = "UNRUN: each authentic stable ID removal, exact raw archival/discovery/undo/native continuation"]
fn remove_each_id_archives_its_mask_preserves_remaining_current_state_and_native_continuation() {
    for removed_index in 0..2 {
        let dir = Directory::new();
        let original = source();
        let path = dir.0.join("stack.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let identity = h.session.active().unwrap().source.clone();
        let initial_revision = h.session.active().unwrap().revision;
        let raw = native(&h);
        let state = before.xcf_state.as_ref().unwrap();
        assert_eq!(state.filter_stacks[0].nodes.len(), 2);
        let removed = &state.filter_stacks[0].nodes[removed_index];
        let survivor = &state.filter_stacks[0].nodes[1 - removed_index];
        let id = removed.id.0.to_string();
        let mask = removed.mask.unwrap();
        let ItemRef::Mask(mask_id) = mask else { panic!("independent source has a physical filter mask") };
        assert_eq!(state.filter_masks.iter().filter(|m| m.owner == removed.id).count(), 1);
        let raw_records = state.opaque.iter().filter(|r| r.owner == Some(mask)).count();
        assert!(raw_records > 0);
        // External exact expected metadata derived from the independently
        // observed original, not from the new mutation helper as an oracle.
        let mut expected = state.clone();
        expected.filter_stacks[0].nodes.remove(removed_index);
        expected.filter_stacks[0].revision += 1;
        expected.filter_masks.retain(|m| m.owner != removed.id);
        expected.items.retain(|item| item.item != mask);
        for record in &mut expected.opaque {
            if record.owner == Some(mask) {
                assert_eq!(record.namespace, "gimp/xcf/source-property");
                assert!(!record.rendering_required);
                record.owner = None;
                record.identity = format!("deleted-filter-mask/{}/{}", mask_id.0, record.identity);
            }
        }
        assert_eq!(h.command_run("xcf.filter.remove", json!({"filterId":id})).unwrap(), json!({"filterId":id,"updated":true}));
        let removed_doc = h.session.active().unwrap().doc.clone();
        assert_eq!(removed_doc.xcf_state.as_ref().unwrap(), &expected);
        assert_eq!(h.session.active().unwrap().history.past_len(), 1);
        assert_eq!(h.session.active().unwrap().history.undo_label(), Some("Remove GIMP Stock Filter"));
        assert_eq!(h.session.active().unwrap().revision, initial_revision + 1);
        assert_eq!(native(&h), raw);
        for (old, current) in state.opaque.iter().zip(&removed_doc.xcf_state.as_ref().unwrap().opaque) {
            assert!(Arc::ptr_eq(&old.bytes, &current.bytes));
        }
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 1);
        assert_eq!(query.value()["filters"]["entries"][0]["id"], survivor.id.0.to_string());
        assert_eq!(query.value()["items"]["total"], expected.items.len());
        drop(query);
        let survivor_reply = h.inspect_owned_filter_bounded(&json!({"filterId":survivor.id.0.to_string()}), true).unwrap();
        assert_eq!(survivor_reply.value()["operation"]["name"], survivor.operation.name);
        drop(survivor_reply);
        assert!(h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).is_err());
        let current = h.session.active().unwrap().doc.clone();
        let history = h.session.active().unwrap().history.clone();
        let journal = h.session.journal.len();
        assert!(h.command_run("xcf.filter.remove", json!({"filterId":id})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&current));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.journal.len(), journal);
        h.command_run("edit.undo", json!({})).unwrap();
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        assert_eq!(h.command_run("xcf.filter.enable", json!({"filterId":id,"enabled":removed.enabled})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        h.command_run("edit.redo", json!({})).unwrap();
        assert_eq!(h.session.active().unwrap().doc.xcf_state.as_ref().unwrap(), &expected);
        let output = dir.0.join("removed.pcraft");
        assert!(h.save_owned_native_active(Some(&path), Some("pcraft"), &ctl()).is_err());
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        assert_eq!(h.session.active().unwrap().source, identity);
        assert!(h.session.active().unwrap().source_read_only);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = reopened.session.active().unwrap();
        assert!(current.doc.accounted().unwrap().is_readonly_point_storage());
        assert!(!current.source_read_only);
        assert_eq!(current.doc.xcf_state.as_ref().unwrap(), &expected);
        assert_eq!(current.doc.icc_profile, before.icc_profile);
        assert_eq!(current.doc.source_color_context, before.source_color_context);
        assert_eq!(current.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        assert_eq!(
            reopened.command_run("xcf.filter.enable", json!({"filterId":survivor.id.0.to_string(),"enabled":!survivor.enabled})).unwrap()["updated"],
            true
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
#[ignore = "UNRUN: authentic Point removal keeps waitfalse/authorizers and refuses unknown/last IDs transactionally"]
fn remove_refusals_leave_history_journal_jobs_and_source_untouched() {
    let dir = Directory::new();
    let path = dir.0.join("stack.xcf");
    let original = source();
    std::fs::write(&path, &original).unwrap();
    let mut h = Headless::trusted_local();
    drop(h.open_owned_xcf_keep_current(&path, &ctl()).unwrap());
    let before = h.session.active().unwrap().doc.clone();
    let nodes = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes;
    let id = nodes[0].id.0.to_string();
    let survivor = nodes[1].id.0.to_string();
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    assert!(
        matches!(h.command_start("xcf.filter.remove",json!({"unparsed":[[[[[0]]]]]}),false),Err(AutomationError::BadRequest(ref why))if why.contains("synchronous"))
    );
    assert!(!h.session.has_jobs());
    h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("deny current Point removal".into())));
    assert!(h.command_run("xcf.filter.remove", json!({"filterId":id})).is_err());
    h.session.authorize = None;
    for params in [json!({"filterId":"18446744073709551615"}), json!({"filterId":id,"index":0}), json!({"filterId":id,"coalesce":null})] {
        assert!(h.command_run("xcf.filter.remove", params).is_err());
    }
    assert!(h.session.active().unwrap().doc.shares_state(&before));
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
    h.command_run("xcf.filter.remove", json!({"filterId":id})).unwrap();
    let current = h.session.active().unwrap().doc.clone();
    let history = h.session.active().unwrap().history.clone();
    let revision = h.session.active().unwrap().revision;
    let journal = h.session.journal.len();
    let error = h.command_run("xcf.filter.remove", json!({"filterId":survivor})).unwrap_err();
    assert!(error.to_string().contains("removing the last Point filter is unsupported"));
    assert!(h.session.active().unwrap().doc.shares_state(&current));
    assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
    assert_eq!(h.session.active().unwrap().revision, revision);
    assert_eq!(h.session.journal.len(), journal);
    assert!(!h.session.has_jobs());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
