//! Deferred native13 CURRENT hierarchy workflow, no authored hierarchy Source claim.
use super::*;
#[path = "../../../engine/src/accounted_normal_hierarchy_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod f;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-hierarchy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
#[ignore = "UNRUN/uncompiled: actual nested native13 adopt/query/metadata/filter/history/disclosure SaveAs/reopen and unchanged class"]
fn hierarchy_headless_continues_actual_tree() {
    for case in f::cases() {
        let dir = Directory::new();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&f::path(&case)).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        f::observed(&before, &case);
        let output = dir.0.join("authored.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &f::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        f::storage(&before, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&output).unwrap());
        f::same(&saved, &reopened.session.active().unwrap().doc);
    }

    for depth in 0..2 {
        let owner = f::build(depth);
        let dir = Directory::new();
        let input = dir.0.join("input.pcraft");
        std::fs::write(&input, f::encoded(&owner)).unwrap();
        let mut h = Headless::trusted_local();
        h.open_owned_native_keep_current_bounded(&input).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        assert!(before.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        let root = f::layer(&owner);
        let n = before.xcf_state.as_ref().unwrap().filter_stacks.last().unwrap().nodes.last().unwrap().id;
        h.command_run("layer.select", serde_json::json!({"layer":root.0})).unwrap();
        assert_eq!(h.session.active().unwrap().active_layer, Some(root));
        let reply = h.command_run("xcf.filter.inspect", serde_json::json!({"filterId":n.0.to_string()})).unwrap();
        drop(reply);
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":n.0.to_string(),"enabled":false})).unwrap();
        h.command_run("layer.setProps", serde_json::json!({"layer":f::INNER.0,"name":"Inner continued","opacity":0.375})).unwrap();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        h.command_run("layer.setExpanded", serde_json::json!({"layer":f::OUTER.0,"expanded":false})).unwrap();
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal.len(), journal);
        let changed = h.session.active().unwrap().doc.clone();
        f::payload(&before, &changed);
        let output = dir.0.join("continued.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &f::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "continued.pcraft");
        f::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        reopened.open_owned_native_keep_current_bounded(&output).unwrap();
        let now = reopened.session.active().unwrap().doc.clone();
        f::same(&saved, &now);
        assert!(now.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        h.command_run("edit.redo", serde_json::json!({})).unwrap();
        let before = h.session.active().unwrap().doc.clone();
        assert!(h.command_run("layer.translate", serde_json::json!({"layer":root.0,"dx":0,"dy":0})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
    }
}
