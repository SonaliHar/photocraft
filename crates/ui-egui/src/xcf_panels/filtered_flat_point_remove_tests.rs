//! Deferred actual removal widget/capture regression, no visual/numeric claim.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_remove_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN actual Remove widget genuine2 ordered cases, precise lastnode mask/sourcearchive and sibling order, oneframe stale secondaction, undo/native and single-root2 unsupported-wire refusal; no removed producer numeric oracle."]
fn safe_remove_ui_keeps_frame_and_class() {
    for case in &fixture::data::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "ordered.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let root = before.layers[3].id;
        let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap();
        let removed_id = old.nodes[2].id;
        let mut h = Harness::builder().with_size(egui::vec2(800., 1100.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        h.query_all_by_label("Remove").last().unwrap().click();
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let removed = app.session.active().unwrap().doc.clone();
        fixture::preserved(&before, &removed, removed_id);
        app.run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &app.session.active().unwrap().doc);
        app.run("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&removed, &app.session.active().unwrap().doc);
        app.run("edit.undo", serde_json::json!({})).unwrap();
        let snapshot = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&snapshot, revision, root, 0).unwrap();
        let second = capture_snapshot(&snapshot, revision, root, 1).unwrap();
        submit(app, &first, "xcf.filter.remove", serde_json::json!({"filterId":first.filter.0.to_string()}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(app, &second));
        submit(app, &second, "xcf.filter.remove", serde_json::json!({"filterId":second.filter.0.to_string()}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "removed.pcraft", fixture::archive(once.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&once, &reopened.session.active().unwrap().doc);
        let two = fixture::single_root_two(before.accounted().unwrap());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "two.pcraft", fixture::archive(&two), &fixture::ctl()).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let id = original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id;
        assert!(app.run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string(),"index":0})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&original));
    }
}
