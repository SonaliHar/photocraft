//! Deferred real affine widgets and captured same-frame publication fences.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_affine_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};

#[test]
#[ignore = "UNRUN bothdepth genuine affine nonfirst-Level widgets/passive words/typedtuple/stale secondaction/realdisplay nativecontinuation; no visual or producer numerical acceptance."]
fn affine_widgets_preserve_actual_target_and_native() {
    for c in fixture::CASES.iter().step_by(2) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let input = fixture::import(c);
        crate::owned_xcf_open::adopt_bytes(&mut app, "affine.xcf", &input, &fixture::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, c);
        let stack = before
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .find(|s| s.nodes.len() == 4 && s.nodes.iter().any(|n| n.operation.name == "gegl:levels"))
            .unwrap();
        let ItemRef::Layer(root) = stack.owner else { panic!("root") };
        let index = stack.nodes.iter().position(|n| n.operation.name == "gegl:levels").unwrap();
        let filter = stack.nodes[index].id;
        assert!(index > 0);
        let old = &stack.nodes[index];
        let values = photocraft_compose::gegl_levels_mixed_parameters(&old.arguments).unwrap().raw_words().map(f32::from_bits);
        assert!(affine::levels_request(old, values, old.enabled).is_none());
        let mut h = Harness::builder().with_size(egui::vec2(900., 1500.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        fixture::same(&before, &h.state_mut().session.active().unwrap().doc);
        h.get_by_label("Input low").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Input low").type_text("-0.125");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        fixture::payload(&before, &changed);
        let node = &changed.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == stack.owner).unwrap().nodes[index];
        assert_eq!(node.id, filter);
        assert_eq!(photocraft_compose::gegl_levels_mixed_parameters(&node.arguments).unwrap().values(), [-0.125, 0.875, 0.0625, 0.9375]);
        let revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&changed, revision, root, index).unwrap();
        let stale = capture_snapshot(&changed, revision, root, 0).unwrap();
        submit(app, &first, "xcf.filter.enable", json!({"filterId":first.filter.0.to_string(),"enabled":false}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        submit(app, &stale, "xcf.filter.enable", json!({"filterId":stale.filter.0.to_string(),"enabled":false}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let display = app.session.color.canvas_display_for(&once, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&once, c.width, c.height, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE)
                .unwrap();
        let expected = image.pixels.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "affine.pcraft", fixture::archive(once.accounted().unwrap()), &fixture::ctl()).unwrap();
        let current = reopened.session.active().unwrap().doc.clone();
        fixture::same(&once, &current);
        let display = reopened.session.color.canvas_display_for(&current, None).unwrap();
        let (image, fee) = crate::canvas_backed_preview::backed_snapshot_display_image(
            &current,
            c.width,
            c.height,
            Some(display.as_ref()),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(image.pixels, expected);
        drop((image, fee));
        assert_eq!(fixture::bytes(c), std::fs::read(fixture::path(c)).unwrap());
    }
}
