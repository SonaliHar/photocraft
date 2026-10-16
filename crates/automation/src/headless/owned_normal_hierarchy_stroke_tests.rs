//! Deferred source-first headless hierarchy native stroke command continuity.
use super::*;
#[path = "../../../engine/src/accounted_plain_hierarchy_paint_test_support.rs"]
mod f;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-hierarchy-stroke-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn patch() -> serde_json::Value {
    serde_json::json!({"points":[[254.0,254.0,0.7],[256.0,257.0,1.0]],"size":7.0,"hardness":0.6,"spacing":0.3,"color":[0.8,0.2,0.1,0.75],"seed":53})
}
#[test]
#[ignore = "UNRUN/uncompiled actual five-depth generated source files, nested leaf registered pencil/Clear with source-protected SaveAs/native13 title and same-class continuity; group/selection/filter refusal and undo. Internal compatibility, not producer oracle."]
fn hierarchy_headless_leaf_commands_preserve_the_source_and_native_class() {
    for depth in 0..5 {
        let dir = Directory::new();
        let source = dir.0.join("nested.xcf");
        std::fs::write(&source, f::bytes(depth)).unwrap();
        let original_bytes = std::fs::read(&source).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        f::observed(&before, depth);
        let (group, leaf, _) = f::ids(&before);
        let identity = h.session.active().unwrap().source.clone();
        h.command_run("layer.select", serde_json::json!({"layer":leaf.0})).unwrap();
        h.session.tools.brush.pressure_size = false;
        let raw = f::payload(&before);
        let rev = h.session.active().unwrap().revision;
        let past = h.session.active().unwrap().history.past_len();
        h.command_run("paint.pencil", patch()).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        assert_eq!(h.session.active().unwrap().revision, rev + 1);
        assert_eq!(h.session.active().unwrap().history.past_len(), past + 1);
        assert_ne!(f::payload(&changed), raw);
        f::source_payload(&before, &changed);
        assert_eq!(changed.depth, f::sample(depth));
        assert_eq!(h.session.active().unwrap().source, identity);
        assert!(identity.as_ref().unwrap().check_destination(source.to_str().unwrap()).is_err());
        let output = dir.0.join("nested.pcraft");
        h.save_owned_native_active(Some(&output), Some("pcraft"), &f::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "nested.pcraft");
        let mut reopen = Headless::trusted_local();
        drop(reopen.open_owned_native_keep_current_bounded(&output).unwrap());
        let now = reopen.session.active().unwrap().doc.clone();
        f::same(&saved, &now);
        assert!(now.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        assert_eq!(f::payload(&now), f::payload(&changed));
        assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(f::payload(&h.session.active().unwrap().doc), raw);
        h.command_run("layer.select", serde_json::json!({"layer":leaf.0})).unwrap();
        let mut clear = patch();
        clear["mode"] = serde_json::json!("clear");
        h.command_run("paint.stroke", clear).unwrap();
        h.command_run("layer.select", serde_json::json!({"layer":group.0})).unwrap();
        let stable = h.session.active().unwrap().doc.clone();
        for (id, p) in
            [("paint.stroke", patch()), ("select.all", serde_json::json!({})), ("xcf.filter.enable", serde_json::json!({"filterId":"1","enabled":false}))]
        {
            assert!(h.command_run(id, p).is_err());
            assert!(stable.shares_state(&h.session.active().unwrap().doc));
        }
    }
}
