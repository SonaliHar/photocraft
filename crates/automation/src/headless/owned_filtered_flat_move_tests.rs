//! Deferred public mixed move path; source and raw payload remain separate from display.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_ordered_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-move-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
#[ignore = "UNRUN genuine2 sourcepath→actual mixed move lastnode/front, auth/wait/invalidwire atomic refusal, noopredo/undo/source-native-mask-profile preservation and realwriter→automaticnative query/title. No reordered producer numeric oracle."]
fn mixed_move_public_lifecycle_is_atomic() {
    for case in &fixture::data::CASES {
        let directory = Directory::new();
        let path = fixture::workspace().join(case.source.path);
        let source = fixture::bytes(case.source);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let before = h.session.active().unwrap().doc.clone();
        fixture::observed(&before, case);
        let st = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
        let id = st.nodes[2].id;
        let params = serde_json::json!({"filterId":id.0.to_string(),"index":0});
        assert!(h.command_start("xcf.filter.move", params.clone(), false).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
        assert!(h.command_run("xcf.filter.move", params.clone()).is_err());
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&before));
        for invalid in [
            serde_json::json!({"filterId":id.0.to_string(),"index":1024}),
            serde_json::json!({"filterId":id.0.to_string(),"index":3}),
            serde_json::json!({"filterId":id.0.to_string(),"index":0,"enabled":true}),
        ] {
            assert!(h.command_run("xcf.filter.move", invalid).is_err());
            assert!(h.session.active().unwrap().doc.shares_state(&before));
        }
        assert_eq!(h.command_start("xcf.filter.move", params.clone(), true).unwrap()["updated"], true);
        let moved = h.session.active().unwrap().doc.clone();
        fixture::payload(&before, &moved);
        let actual = moved.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == st.owner).unwrap();
        assert_eq!(actual.nodes, vec![st.nodes[2].clone(), st.nodes[0].clone(), st.nodes[1].clone()]);
        assert_eq!(actual.revision, st.revision + 1);
        assert_eq!(h.command_run("xcf.filter.move", params).unwrap()["updated"], false);
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &h.session.active().unwrap().doc);
        assert_eq!(h.command_run("xcf.filter.move", serde_json::json!({"filterId":id.0.to_string(),"index":2})).unwrap()["updated"], false);
        assert!(h.session.active().unwrap().history.can_redo());
        h.command_run("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&moved, &h.session.active().unwrap().doc);
        let dest = directory.0.join("moved.pcraft");
        h.save_owned_native_active(Some(&dest), Some("pcraft"), &fixture::ctl()).unwrap();
        let saved = h.session.active().unwrap().doc.clone();
        assert_eq!(saved.name, "moved.pcraft");
        fixture::storage(&moved, &saved);
        let mut reopened = Headless::trusted_local();
        drop(reopened.open_owned_native_keep_current_bounded(&dest).unwrap());
        fixture::same(&saved, &reopened.session.active().unwrap().doc);
        let query = reopened.inspect_owned_command_with_read_control("xcf.inspect", &serde_json::json!({"limit":128}), false, &fixture::ctl()).unwrap();
        let expected = saved.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id.0.to_string())).collect::<Vec<_>>();
        let found = query.value()["filters"]["entries"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(found, expected);
        drop(query);
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(!reopened.session.has_jobs());
    }
}
