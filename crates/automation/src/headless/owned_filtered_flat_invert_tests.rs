//! Deferred genuine mixed invert sources/native writer/control lifecycle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_invert_test_support.rs"]
#[allow(clippy::duplicate_mod, reason = "This workflow deliberately instantiates its shared test helper under its own consumer parent")]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-invert-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN four authentic invert source packets all indexed targets, auth/wait/zeroargs boundaries, activation/noopredo/paidquery/native filename title and source/mask/raw/profile continuation; no producer numerical or profile equivalence."]
fn mixed_inverts_public_native_lifecycle() {
    for case in &fixture::CASES {
        let dir = Directory::new();
        let source = fixture::bytes(case);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let targets = original
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|st| st.nodes.iter().filter(|n| matches!(n.operation.name.as_str(), "gegl:invert-linear" | "gegl:invert-gamma")).map(|n| n.id))
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 8);
        for id in targets {
            let before = h.session.active().unwrap().doc.clone();
            let p = serde_json::json!({"filterId":id.0.to_string(),"enabled":false});
            assert!(h.command_start("xcf.filter.set", p.clone(), false).is_err());
            h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
            assert!(h.command_run("xcf.filter.set", p.clone()).is_err());
            h.session.authorize = None;
            assert!(h.session.active().unwrap().doc.shares_state(&before));
            assert_eq!(h.command_start("xcf.filter.set", p.clone(), true).unwrap()["updated"], true);
            let after = h.session.active().unwrap().doc.clone();
            fixture::payload(&before, &after);
            assert_eq!(h.command_run("xcf.filter.set", p).unwrap()["updated"], false);
            h.command_run("edit.undo", serde_json::json!({})).unwrap();
            fixture::same(&before, &h.session.active().unwrap().doc);
            assert_eq!(h.command_run("xcf.filter.enable", serde_json::json!({"filterId":id.0.to_string(),"enabled":true})).unwrap()["updated"], false);
            assert!(h.session.active().unwrap().history.can_redo());
            h.command_run("edit.redo", serde_json::json!({})).unwrap();
            let q = h
                .inspect_owned_command_with_read_control("xcf.filter.inspect", &serde_json::json!({"filterId":id.0.to_string()}), false, &fixture::ctl())
                .unwrap();
            assert_eq!(q.value()["scalarEditCandidate"], true);
            drop(q);
            let path = dir.0.join(format!("invert-{}.pcraft", id.0));
            h.save_owned_native_active(Some(&path), Some("pcraft"), &fixture::ctl()).unwrap();
            let saved = h.session.active().unwrap().doc.clone();
            assert_eq!(saved.name, path.file_name().unwrap().to_str().unwrap());
            fixture::storage(&after, &saved);
            let mut reopened = Headless::trusted_local();
            drop(reopened.open_owned_native_keep_current_bounded(&path).unwrap());
            fixture::same(&saved, &reopened.session.active().unwrap().doc);
            assert!(
                reopened
                    .command_run(
                        "xcf.filter.set",
                        serde_json::json!({"filterId":id.0.to_string(),"arguments":[{"name":"value","value":{"Number":{"Float32Bits":0}}}]})
                    )
                    .is_err()
            );
            assert!(reopened.session.active().unwrap().doc.shares_state(&saved));
            let mut next = Headless::trusted_local();
            drop(next.open_owned_xcf_keep_current_bounded(&fixture::workspace().join(case.source)).unwrap());
            h = next;
        }
        assert_eq!(source, std::fs::read(fixture::workspace().join(case.source)).unwrap());
        assert!(!h.session.has_jobs());
    }
}
