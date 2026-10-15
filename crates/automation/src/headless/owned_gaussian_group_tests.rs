//! UNRUN real isolated Gaussian group Headless/source/native workflow.
//! External temporary files are harness captures, not quota evidence.
use super::*;
#[path = "../../../ui-egui/src/canvas_gaussian_group_fixture.rs"]
mod fixture;
use fixture::{assets, ctl, observed, retained, state};
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-gaussian-group-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN: authentic mode28 isolated Gaussian source/edit actual Headless paid query/auth/sourceguard/native save/title/reopen tree and masks; no filter mutation"]
fn isolated_gaussian_headless_preserves_source_and_native_tree_custody() {
    let root = assets();
    for phase in ["source", "edit"] {
        let dir = Directory::new();
        let file = if phase == "source" { "group-28-group-mask.xcf" } else { "group-28-group-mask.edit.xcf" };
        let original = std::fs::read(root.join(file)).unwrap();
        let captured = state(&root, phase);
        let source_path = dir.0.join("Original group.xcf");
        std::fs::write(&source_path, &original).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current(&source_path, &ctl()).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        let source = h.session.active().unwrap().source.clone();
        assert!(before.accounted().unwrap().is_readonly_gaussian_group_storage());
        assert!(h.has_owned_filter_inspection());
        assert!(h.has_owned_inspection_command("xcf.inspect"));
        assert!(h.has_owned_inspection_command("xcf.filter.inspect"));
        assert!(h.session.active().unwrap().source_read_only);
        observed(&root, &before, &captured);
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()).unwrap();
        assert_eq!(query.value()["items"]["total"], 6);
        assert_eq!(query.value()["filters"]["total"], 1);
        drop(query);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id.0.to_string();
        let query = h.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(query.value()["operation"]["name"], "gegl:gaussian-blur");
        drop(query);
        // Metadata query wait forms both inspect the installed snapshot inline;
        // this does not start or synchronize an asynchronous mutation job.
        let inline = h.inspect_owned_filter_bounded(&json!({"filterId":id}), false).unwrap();
        assert_eq!(inline.value()["filterId"], id);
        drop(inline);
        assert!(!h.session.has_jobs());
        let history = h.session.active().unwrap().history.clone();
        let revision = h.session.active().unwrap().revision;
        let journal = h.session.journal.clone();
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("group query denied".into())));
        match h.inspect_owned_command_with_read_control("xcf.inspect", &Value::Null, true, &ctl()) {
            Err(e) => assert!(e.to_string().contains("group query denied")),
            Ok(_) => panic!("paid supported query must honor authorization"),
        };
        h.session.authorize = None;
        assert!(h.command_run("xcf.filter.set", json!({"filterId":id,"enabled":false,"arguments":[]})).is_err());
        assert!(h.command_run("layer.setProps", json!({"name":"unsupported"})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        assert!(h.session.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(h.session.active().unwrap().revision, revision);
        assert_eq!(h.session.journal, journal);
        assert!(h.save_owned_native_active(Some(&source_path), Some("pcraft"), &ctl()).is_err());
        let output = dir.0.join("Saved Gaussian group.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        retained(&before, &saved);
        assert_eq!(h.session.active().unwrap().source, source);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current(&output, &ctl()).unwrap());
        let current = &reopened.session.active().unwrap().doc;
        assert_eq!(current.name, saved.name);
        retained(&before, current);
        observed(&root, current, &captured);
        assert!(!reopened.session.active().unwrap().source_read_only);
        assert!(current.accounted().unwrap().is_readonly_gaussian_group_storage());
        assert!(reopened.has_owned_filter_inspection());
        let query = reopened.inspect_owned_filter_bounded(&json!({"filterId":id}), true).unwrap();
        assert_eq!(query.value()["filterId"], id);
        drop(query);
        assert_eq!(std::fs::read(&source_path).unwrap(), original);
        assert_eq!(std::fs::read(root.join(file)).unwrap(), original);
    }
    for file in ["group-61-group-mask.xcf", "group-61-group-mask.edit.xcf"] {
        let dir = Directory::new();
        let original = std::fs::read(root.join(file)).unwrap();
        let path = dir.0.join("Negative pass-through.xcf");
        std::fs::write(&path, &original).unwrap();
        let mut h = Headless::trusted_local();
        assert!(h.open_owned_xcf_keep_current(&path, &ctl()).is_err());
        assert!(h.session.documents().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
