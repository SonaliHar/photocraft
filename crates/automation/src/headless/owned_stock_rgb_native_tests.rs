//! Deferred trusted native file workflow over a declared synthetic CURRENT owner.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_stock_rgb_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-stock-rgb-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN declared CURRENT actual mixed native adoption, registered Alien10 compound edit, trusted native SaveAs/title/reopen and post-save undo. No producer Source or native GUI/performance acceptance."]
fn stock_rgb_native_save_reopen_and_undo_keep_actual_state() {
    let owner = fixture::build();
    let dir = Directory::new();
    let mut h = Headless::trusted_local();
    h.session.add_owned_native_filtered_flat_archive(fixture::archive(&owner), "stock.pcraft", None, &fixture::ctl()).unwrap();
    let before = h.session.active().unwrap().doc.clone();
    h.command_run("xcf.filter.set", fixture::patch(&before, fixture::Kind::AlienRgb)).unwrap();
    let changed = h.session.active().unwrap().doc.clone();
    fixture::payload(&before, &changed);
    let dest = dir.0.join("stock-native.pcraft");
    h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
    let saved = h.session.active().unwrap().doc.clone();
    assert_eq!(saved.name, "stock-native.pcraft");
    fixture::storage(&changed, &saved);
    let mut reopened = Headless::trusted_local();
    drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
    fixture::same(&saved, &reopened.session.active().unwrap().doc);
    assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_filtered_flat_point_storage());
    h.command_run("edit.undo", serde_json::json!({})).unwrap();
    fixture::storage(&before, &h.session.active().unwrap().doc);
}
