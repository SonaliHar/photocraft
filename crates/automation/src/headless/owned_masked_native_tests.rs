//! Deferred actual trusted-path source/native11 save and forced Point reopen.
use super::*;
#[path = "../../../engine/src/accounted_exposure_masked_test_support.rs"]
mod fixture;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-masked-native-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN actual6 nonempty sourcefiles→native11 trustedSaveAs/reopen/currentPoint exactrawmask/title/source protection andpostsaveundo; no nativeGUI/performance/produceroracle."]
fn masked_native_file_save_reopen_and_source_protection() {
    for case in fixture::cases() {
        let dir = Directory::new();
        let source = fixture::workspace().join(case["source"].as_str().unwrap());
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
