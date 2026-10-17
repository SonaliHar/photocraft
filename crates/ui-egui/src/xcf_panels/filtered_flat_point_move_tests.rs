//! Deferred actual button/target tests, no visual or numerical acceptance.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_ordered_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN actual Move-to-Top widget on both genuine ordered inputs, exact whole-node ordering/stackrevision/undo/native continuity and shared-frame stale secondmove. No numerical/GIMP reordered oracle."]
fn mixed_move_buttons_keep_one_frame_and_exact_nodes() {
    for case in &fixture::data::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "ordered.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let root = before.layers[3].id;
        let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == ItemRef::Layer(root)).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(800., 1000.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        h.query_all_by_label("Move to Top").last().unwrap().click();
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let moved = app.session.active().unwrap().doc.clone();
        fixture::payload(&before, &moved);
        let new = moved.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|st| st.owner == old.owner).unwrap();
        assert_eq!(new.nodes, vec![old.nodes[2].clone(), old.nodes[0].clone(), old.nodes[1].clone()]);
        assert_eq!(new.revision, old.revision + 1);
        app.run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &app.session.active().unwrap().doc);
        app.run("edit.redo", serde_json::json!({})).unwrap();
        fixture::same(&moved, &app.session.active().unwrap().doc);
        let snapshot = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&snapshot, revision, root, 1).unwrap();
        let second = capture_snapshot(&snapshot, revision, root, 2).unwrap();
        submit(app, &first, "xcf.filter.move", serde_json::json!({"filterId":first.filter.0.to_string(),"index":0}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(app, &second));
        submit(app, &second, "xcf.filter.move", serde_json::json!({"filterId":second.filter.0.to_string(),"index":0}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "moved.pcraft", fixture::archive(once.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&once, &reopened.session.active().unwrap().doc);
    }
}
