//! Deferred actual affine source -> registered command -> native continuation.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_affine_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-affine-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN all4 genuine affine sources trustedsource/Levels4arg command/native10/title/reopen/undo/protectedsource; no producer numeric/runtime acceptance."]
fn affine_command_and_native_continuation_preserve_sources() {
    for c in &fixture::CASES {
        let dir = Directory::new();
        let source = fixture::path(c);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&source).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, c);
        let id = original.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).find(|n| n.operation.name == "gegl:levels").unwrap().id;
        let args = ["in-low", "in-high", "out-low", "out-high"]
            .into_iter()
            .zip([-0.125f32, 1.25, 0.125, 0.75])
            .map(|(name, value)| serde_json::json!({"name":name,"value":{"Number":{"Float32Bits":value.to_bits()}}}))
            .collect::<Vec<_>>();
        h.command_run("xcf.filter.set", serde_json::json!({"filterId":id.0.to_string(),"arguments":args})).unwrap();
        let changed = h.session.active().unwrap().doc.clone();
        fixture::payload(&original, &changed);
        assert!(h.save_owned_native_active(Some(&source), Some("pcraft"), &fixture::ctl()).is_err());
        let dest = dir.0.join("affine-native.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "affine-native.pcraft");
        fixture::storage(&changed, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::storage(&original, &h.session.active().unwrap().doc);
        assert_eq!(fixture::bytes(c), std::fs::read(source).unwrap());
    }
}
