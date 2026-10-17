//! Deferred actual zero-argument UI controls; no visual/numerical acceptance.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_invert_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN four authentic packets actual invert checkbox/no-parameters label, exact last-node frame target/stale second action, marker10 native CanvasDisplay continuity; external display clone only, no producer oracle."]
fn mixed_invert_checkbox_and_frame_are_exact() {
    for case in &fixture::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "inverts.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let root = before.layers[1].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id;
        let caption = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let mut h = Harness::builder().with_size(egui::vec2(800., 900.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        assert!(h.query_by_label("No parameters").is_some());
        h.get_by_label(&caption).click();
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let after = app.session.active().unwrap().doc.clone();
        fixture::payload(&before, &after);
        assert!(!after.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert!(after.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments.is_empty());
        app.session.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&before, &app.session.active().unwrap().doc);
        let snapshot = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let root = snapshot.layers[7].id;
        let first = capture_snapshot(&snapshot, revision, root, 4).unwrap();
        let second = capture_snapshot(&snapshot, revision, root, 3).unwrap();
        submit(app, &first, "xcf.filter.enable", serde_json::json!({"filterId":first.filter.0.to_string(),"enabled":false}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(app, &second));
        submit(app, &second, "xcf.filter.enable", serde_json::json!({"filterId":second.filter.0.to_string(),"enabled":false}));
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let display = app.session.color.canvas_display_for(&once, None).unwrap();
        let (image, fee) =
            crate::canvas::test_backed_snapshot_display_image(&once, 64, 64, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "inverts.pcraft", fixture::archive(once.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&once, &reopened.session.active().unwrap().doc);
        let current = reopened.session.active().unwrap().doc.clone();
        let display = reopened.session.color.canvas_display_for(&current, None).unwrap();
        let (image, fee) =
            crate::canvas::test_backed_snapshot_display_image(&current, 64, 64, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        assert_eq!(id, before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id);
    }
}
