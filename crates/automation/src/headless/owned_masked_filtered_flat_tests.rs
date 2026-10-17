//! Genuine masked mixed Source/native12 workflow; no producer numerical oracle.
use super::*;
#[path = "../../../engine/src/accounted_masked_filtered_flat_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-masked-mixed-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN genuine8 source→actualnewclass typedquery/disable/native12 SaveAs/title/reopen/undo/sourceprotect, finalremove-empty purpose repeated continuation"]
fn masked_mixed_headless_keeps_actual_class_and_source() {
    for c in fixture::cases() {
        let dir = Directory::new();
        let source = fixture::path(&c);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::observed(&before, &c);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        let reply = h.command_run("xcf.filter.inspect", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        drop(reply);
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        fixture::payload(&before, &changed);
        assert!(h.save_owned_native_active(Some(&source), Some("pcraft"), &fixture::ctl()).is_err());
        let dest = dir.0.join("masked-mixed.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "masked-mixed.pcraft");
        fixture::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_masked_filtered_flat_point_storage());
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::storage(&before, &h.session.active().unwrap().doc);
        let all: Vec<_> = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect();
        for id in all {
            h.command_run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        }
        let empty = h.session.active().unwrap().doc.clone();
        assert!(empty.accounted().unwrap().is_readonly_masked_filtered_flat_point_storage());
        assert!(empty.xcf_state.as_ref().unwrap().filter_masks.is_empty());
        let dest = dir.0.join("empty-masked.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "empty-masked.pcraft");
        let mut restored = Headless::trusted_local();
        drop(restored.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &restored.session.active().unwrap().doc);
        assert!(restored.session.active().unwrap().doc.accounted().unwrap().is_readonly_masked_filtered_flat_point_storage());
        assert_eq!(fixture::bytes(&c), std::fs::read(source).unwrap());
    }
}
