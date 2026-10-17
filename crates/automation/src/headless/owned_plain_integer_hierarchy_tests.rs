//! Deferred generated Source -> actual typed hierarchy -> trusted native save.
use super::*;
#[path = "../../../engine/src/accounted_plain_integer_hierarchy_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod f;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-integer-hierarchy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN/uncompiled generated U8/U16 Source open, hierarchy select/metadata/disclosure/history, trusted native13 SaveAs/title/reopen with original source/native/profile custody and closed pixel/filter routes. No producer oracle."]
fn integer_hierarchy_headless_source_and_native_continue() {
    for depth in 0..2 {
        let dir = Directory::new();
        let source = dir.0.join("integer.xcf");
        std::fs::write(&source, f::bytes(depth)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        f::observed(&before, depth);
        assert!(before.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        let (group, leaf, empty) = f::ids(&before);
        let original_source = h.session.active().unwrap().source.clone();
        h.command_run("layer.select", serde_json::json!({"layer":leaf.0})).unwrap();
        h.command_run("layer.setProps", serde_json::json!({"layer":group.0,"name":"Native integer group","opacity":0.375})).unwrap();
        let past = h.session.active().unwrap().history.past_len();
        let rev = h.session.active().unwrap().revision;
        let journal = h.session.journal.len();
        h.command_run("layer.setExpanded", serde_json::json!({"layer":group.0,"expanded":false})).unwrap();
        assert_eq!(h.session.active().unwrap().history.past_len(), past);
        assert_eq!(h.session.active().unwrap().revision, rev);
        assert_eq!(h.session.journal.len(), journal);
        h.command_run("layer.select", serde_json::json!({"layer":empty.0})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        f::payload(&before, &changed);
        let dest = dir.0.join("integer.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &f::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "integer.pcraft");
        assert_eq!(h.session.active().unwrap().source, original_source);
        let mut reopen = Headless::trusted_local();
        drop(reopen.open_owned_native_keep_current_bounded(&dest).unwrap());
        let now = reopen.session.active().unwrap().doc.clone();
        f::same(&saved, &now);
        f::observed(&now, depth);
        assert!(now.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        h.command_run("edit.redo", serde_json::json!({})).unwrap();
        let stable = h.session.active().unwrap().doc.clone();
        for (id, p) in [
            ("paint.stroke", serde_json::json!({"points":[[1,1,1]]})),
            ("xcf.filter.enable", serde_json::json!({"filterId":"1","enabled":false})),
            ("select.all", serde_json::json!({})),
        ] {
            assert!(h.command_run(id, p).is_err());
            assert!(stable.shares_state(&h.session.active().unwrap().doc));
        }
    }
}
