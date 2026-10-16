//! Deferred actual masked Point UI: passive exact words, intentional edit and stale frame; no visual acceptance.
use super::*;
#[path = "../../../engine/src/accounted_exposure_masked_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN authentic six masked source imports/passivecontrols/intentionalExposurewidget/strictblackhelper/staleframe/undo/native11; no source pixeloracle or monitoridentity"]
fn masked_controls_preserve_raw_mask() {
    for case in fixture::cases() {
        let input = fixture::import(&case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "masked.xcf", &input, &fixture::ctl()).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let root = original.layers[0].id;
        let revision = app.session.active().unwrap().revision;
        let node = &original.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let id = node.id;
        assert!(request(node, [0.5, 0.0625], [false, false], true).is_none());
        assert!(request(node, [0.5, f32::from_bits(0x3dcc_cccd)], [false, true], true).is_none());
        let mut h = Harness::builder().with_size(egui::vec2(800., 1100.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(3);
        assert!(h.state_mut().session.active().unwrap().doc.shares_state(&original));
        h.get_by_label("Exposure").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Exposure").type_text("0.75");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        fixture::payload(&original, &changed);
        assert_eq!(
            photocraft_compose::masked_exposure_parameters(&changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments).unwrap().values(),
            [0.75, 0.0625]
        );
        run_checked(app, &original, revision, "xcf.filter.enable", json!({"filterId":id.0.to_string(),"enabled":false}));
        assert!(app.ui.status_error);
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.session.execute("edit.undo", json!({})).unwrap();
        fixture::same(&original, &app.session.active().unwrap().doc);
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "masked.pcraft", fixture::archive(original.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&original, &reopened.session.active().unwrap().doc);
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_point_storage());
    }
}
