//! Deferred genuine mixed Source-opacity originals/native writer/control lifecycle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_source_opacity_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-opacity-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN actual8partial/zero/disabled/full originalSources trustedreader→readonlysource/native10/title/query/enable-restoration/undo; Sourcepurposeproofrequired, no fabricatedmodel/numericoracle."]
fn genuine_source_states_save_and_reopen() {
    for c in fixture::cases() {
        let dir = Directory::new();
        let sourcepath = fixture::path(&c);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&sourcepath).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, &c);
        let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id;
        let enabled = node.enabled;
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":!enabled})).unwrap();
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &h.session.active().unwrap().doc);
        assert!(h.save_owned_native_active(Some(&sourcepath), Some("pcraft"), &fixture::ctl()).is_err());
        let dest = dir.0.join("original-native.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "original-native.pcraft");
        fixture::storage(&original, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        assert_eq!(fixture::bytes(&c), std::fs::read(sourcepath).unwrap());
    }
}
