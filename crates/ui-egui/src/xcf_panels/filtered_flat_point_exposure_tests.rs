//! Deferred Exposure widgets/requests, not a visual or producer rounding oracle.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_exposure_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN actual Exposure/Black-level widgets on bothgenuine mixedpackets nonfirst target, typedF32/newversion/sourceorder/nativequery and oneframe stale secondaction, realCanvasDisplay/native continuity. BlackUI inwardbound and no arbitraryExposureclamp; visual/numeric/profileHOLDs remain."]
fn mixed_exposure_widgets_keep_target_and_words() {
    for case in &fixture::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "exposure.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, case);
        let root = before.layers[2].id;
        let old = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap();
        assert_eq!(old.nodes[3].operation.name, "gegl:exposure");
        let id = old.nodes[3].id;
        let mut h = Harness::builder().with_size(egui::vec2(800., 1400.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        h.get_by_label("Exposure").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Exposure").type_text("0.75");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        h.get_by_label("Black level").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Black level").type_text("-0.03125");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let after = app.session.active().unwrap().doc.clone();
        fixture::payload(&before, &after);
        let new = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == old.owner).unwrap();
        assert_eq!(new.nodes[3].id, id);
        assert_eq!(&new.nodes[..3], &old.nodes[..3]);
        let node = &new.nodes[3];
        for a in &node.arguments {
            let expected = match a.name.as_str() {
                "exposure" => 0.75f32,
                "black-level" => -0.03125f32,
                _ => panic!("name"),
            };
            assert_eq!(a.value, photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(expected.to_bits())));
        }
        let snapshot = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&snapshot, revision, root, 3).unwrap();
        let second = capture_snapshot(&snapshot, revision, root, 0).unwrap();
        submit(app, &first, "xcf.filter.enable", serde_json::json!({"filterId":first.filter.0.to_string(),"enabled":false}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(app, &second));
        submit(app, &second, "xcf.filter.enable", serde_json::json!({"filterId":second.filter.0.to_string(),"enabled":false}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let display = app.session.color.canvas_display_for(&once, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&once, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "exposure.pcraft", fixture::archive(once.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::same(&once, &reopened.session.active().unwrap().doc);
        let current = reopened.session.active().unwrap().doc.clone();
        let display = reopened.session.color.canvas_display_for(&current, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&current, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
    }
}
