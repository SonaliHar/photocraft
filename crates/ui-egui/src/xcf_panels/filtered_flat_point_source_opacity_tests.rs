//! Deferred original partial Source passive percentage UI.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_source_opacity_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN actualpartialoriginalF32/F64 source→sealedadoption,37.5passiveUI preservesSource rawwords then actual50% edit/undo/nativeclass. No currentownerfabrication/visual/numeric acceptance."]
fn original_partial_source_is_passive_at_37_5_percent() {
    for c in fixture::cases().into_iter().filter(|c| c["id"].as_str().unwrap().ends_with("partial")) {
        let input = fixture::import(&c);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "partial-original.xcf", &input, &fixture::ctl()).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let root = original.layers[0].id;
        assert_eq!(original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits, 0.375f64.to_bits());
        let mut h = Harness::builder().with_size(egui::vec2(800., 1100.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(3);
        assert!(h.state_mut().session.active().unwrap().doc.shares_state(&original));
        h.get_by_label("Filter opacity").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Filter opacity").type_text("50");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        let changed = app.session.active().unwrap().doc.clone();
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].opacity_f64_bits, 0.5f64.to_bits());
        fixture::payload(&original, &changed);
        app.session.execute("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&original, &app.session.active().unwrap().doc);
    }
}
