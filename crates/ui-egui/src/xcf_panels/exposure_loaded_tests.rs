//! Deferred effective-value UI without passive raw endpoint rewrite.
use super::*;
#[path = "../../../engine/src/accounted_exposure_loaded_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN actual ordinary loadedPoint passiveframe/Exposurewidget rawblack preservation and native continuity; exact helper deliberateblack0 transport versus passive effective0, mixed syntheticCurrent passiveframe only. No claim typing unchanged0 fires widgetchanged or visual acceptance."]
fn loaded_endpoint_ui_preserves_passive_raw_words() {
    for case in &fixture::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "endpoint.xcf", &input, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let root = before.layers[0].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(request(node, [0.5, 0.0], [false, false], true).is_none());
        let explicit = request(node, [0.5, 0.0], [false, true], true).unwrap();
        assert_eq!(explicit.1["arguments"][0]["value"]["Number"]["Float32Bits"], 0);
        let id = node.id;
        let mut h = Harness::builder().with_size(egui::vec2(800., 1100.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(3);
        assert!(h.state_mut().session.active().unwrap().doc.shares_state(&before));
        h.get_by_label("Exposure").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Exposure").type_text("0.75");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        assert_eq!(fixture::words(&changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]), [0.75f32.to_bits(), case.black]);
        fixture::payload(&before, &changed);
        // Explicit zero is an intentional request helper/backend path, not a same-value widget event claim.
        let revision = app.session.active().unwrap().revision;
        run_checked(app, &changed, revision, explicit.0, explicit.1);
        assert!(!app.ui.status_error);
        assert_eq!(fixture::words(&app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0])[1], 0);
        app.session.execute("edit.undo", json!({})).unwrap();
        fixture::same(&changed, &app.session.active().unwrap().doc);
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "point.pcraft", fixture::archive(changed.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&changed, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
        let mixed = fixture::mixed_current(if case.precision == 650 { 0 } else { 1 }, case.black);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "mixed.pcraft", fixture::archive(&mixed), &fixture::ctl()).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let root = original.layers[1].id;
        let mut h = Harness::builder().with_size(egui::vec2(800., 1100.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(3);
        assert!(h.state_mut().session.active().unwrap().doc.shares_state(&original));
        assert_eq!(id, before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id);
    }
}
