//! Deferred genuine source protection and native11 trusted file continuation.
use super::*;
#[path = "../../../engine/src/accounted_masked_scalar_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-masked-scalar-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN genuine16 masked scalar source paths→disable/Source protection/native11 SaveAs/title/reopen/undo exact native-mask-recipe; no native GUI/producer numerical/performance acceptance."]
fn masked_scalar_native_file_save_reopen_and_undo_preserve_source() {
    for case in fixture::cases() {
        let dir = Directory::new();
        let source = fixture::path(&case);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        let id = before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":false})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        fixture::payload(&before, &changed);
        assert!(h.save_owned_native_active(Some(&source), Some("pcraft"), &fixture::ctl()).is_err());
        let dest = dir.0.join("masked-native.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "masked-native.pcraft");
        fixture::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::storage(&before, &h.session.active().unwrap().doc);
        assert_eq!(fixture::bytes(&case), std::fs::read(source).unwrap());
    }
}
