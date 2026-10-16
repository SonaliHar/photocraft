//! Deferred actual source/remove/native lifecycle, not producer numeric acceptance.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_remove_test_support.rs"]
mod fixture;
use std::sync::atomic::{AtomicU64, Ordering};
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!("photocraft-mixed-remove-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn save_reopen(h: &mut Headless, path: &std::path::Path) {
    let before = h.session.active().unwrap().doc.clone();
    h.save_owned_native_active(Some(path), Some("pcraft"), &fixture::ctl()).unwrap();
    let saved = h.session.active().unwrap().doc.clone();
    assert_eq!(saved.name, path.file_name().unwrap().to_str().unwrap());
    fixture::storage(&before, &saved);
    let mut opened = Headless::trusted_local();
    drop(opened.open_owned_native_keep_current_bounded(path).unwrap());
    fixture::same(&saved, &opened.session.active().unwrap().doc);
}
#[test]
#[ignore = "UNRUN bothgenuine sources publicremove3→2→1+unsupportedwire, auth/waitfalse/refusalatomic/failurepreservedredo, source/native/mask/inertarchive custody and actualwriter-nativequery/title; paid single-root3→2→unsupportedwire, no producer removed projection oracle."]
fn safe_remove_headless_keeps_topology_and_source() {
    for case in &fixture::data::CASES {
        let dir = Directory::new();
        let path = fixture::workspace().join(case.source.path);
        let source = fixture::bytes(case.source);
        let mut h = Headless::trusted_local();
        drop(h.open_owned_xcf_keep_current_bounded(&path).unwrap());
        let original = h.session.active().unwrap().doc.clone();
        fixture::observed(&original, case);
        let stack = &original.xcf_state.as_ref().unwrap().filter_stacks[0];
        let ids = stack.nodes.iter().map(|n| n.id).collect::<Vec<_>>();
        let p = serde_json::json!({"filterId":ids[1].0.to_string()});
        assert!(h.command_start("xcf.filter.remove", p.clone(), false).is_err());
        h.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("denied".into())));
        assert!(h.command_run("xcf.filter.remove", p.clone()).is_err());
        h.session.authorize = None;
        assert!(h.session.active().unwrap().doc.shares_state(&original));
        assert_eq!(h.command_start("xcf.filter.remove", p.clone(), true).unwrap()["updated"], true);
        let removed = h.session.active().unwrap().doc.clone();
        fixture::preserved(&original, &removed, ids[1]);
        assert!(h.command_run("xcf.filter.remove", p).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&removed));
        h.command_run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &h.session.active().unwrap().doc);
        assert!(h.command_run("xcf.filter.remove", serde_json::json!({"filterId":"0"})).is_err());
        assert!(h.session.active().unwrap().history.can_redo());
        h.command_run("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&removed, &h.session.active().unwrap().doc);
        save_reopen(&mut h, &dir.0.join("removed.pcraft"));
        let before = h.session.active().unwrap().doc.clone();
        assert_eq!(h.command_run("xcf.filter.remove", serde_json::json!({"filterId":ids[0].0.to_string()})).unwrap()["updated"], true);
        let one = h.session.active().unwrap().doc.clone();
        fixture::preserved(&before, &one, ids[0]);
        assert!(h.command_run("xcf.filter.remove", serde_json::json!({"filterId":ids[2].0.to_string(),"index":0})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&one));
        save_reopen(&mut h, &dir.0.join("one-on-root.pcraft"));
        let query = h.inspect_owned_command_with_read_control("xcf.inspect", &serde_json::json!({"limit":128}), false, &fixture::ctl()).unwrap();
        assert_eq!(query.value()["filters"]["total"], 7);
        drop(query);
        let single = fixture::single_root_three(original.accounted().unwrap());
        let file = dir.0.join("single.pcraft");
        std::fs::write(&file, fixture::encoded(&single)).unwrap();
        let mut h = Headless::trusted_local();
        drop(h.open_owned_native_keep_current_bounded(&file).unwrap());
        let id = single.document().xcf_state.as_ref().unwrap().filter_stacks[0].nodes[1].id;
        h.command_run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
        let two = h.session.active().unwrap().doc.clone();
        fixture::preserved(single.document(), &two, id);
        let id = two.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        assert!(h.command_run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string(),"index":0})).is_err());
        assert!(h.session.active().unwrap().doc.shares_state(&two));
        save_reopen(&mut h, &dir.0.join("single-two.pcraft"));
        assert_eq!(std::fs::read(path).unwrap(), source);
        assert!(!h.session.has_jobs());
    }
}
