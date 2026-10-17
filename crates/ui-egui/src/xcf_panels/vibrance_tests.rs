//! Deferred actual controls and CURRENT continuity, not producer math acceptance.
use super::*;
#[path = "../vibrance_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::Value;

fn preview(app: &PhotocraftApp, doc: &DocumentSnapshot) -> egui::ColorImage {
    let display = app.session.color.canvas_display_for(doc, None).unwrap();
    assert!(!display.encode_srgb);
    let (image, fee) = crate::canvas::test_backed_snapshot_display_image(doc, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(image.pixels.len(), 48);
    let capture = image.clone(); // External adapter capture, never a producer oracle.
    drop((image, fee));
    capture
}

#[test]
#[ignore = "UNRUN: 12 authored Vibrance RGB float source/edit files, deferred actual bounded Vibrance widget/checkbox, atomic two-F32+enable request, raw/native/profile/source/query/history/native continuation; producer RGB/LCh rounding, HDR output, strong F32 producer instability (~.475) and scalar Saturation precision and ICC HOLDs retained; visual testing deferred"]
fn authored_vibrance_controls_preserve_transport_and_current_native_roles() {
    for (path, bytes) in fixture::old_sources() {
        assert!(fixture::refuses(path.file_name().unwrap().to_str().unwrap(), bytes));
    }
    for case in fixture::cases() {
        let input = fixture::import(case.path.file_name().unwrap().to_str().unwrap(), case.bytes.clone());
        assert!(matches!(&input, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "vibrance.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        assert!(app.session.active().unwrap().source_read_only);
        let layer = before.layers[0].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(candidate(&before, layer, node));
        let initial = preview(&app, &before);
        assert!(request(node, case.args.map(f32::from_bits), node.enabled).is_none());
        for invalid in [[101., 1.], [0., 10.01], [0., -0.01], [f32::NAN, 1.]] {
            assert!(request(node, invalid, true).is_none());
        }
        // Helper exercises a single combined transaction, including raw signed zero.
        let values = [-0.0, 1.125];
        let (command, params) = request(node, values, false).unwrap();
        assert_eq!(command, "xcf.filter.set");
        assert_eq!(params["enabled"], false);
        let caption = node.source_display_name.as_deref().filter(|v| !v.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let mut harness =
            Harness::builder().with_size(egui::vec2(900., 700.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
        harness.run_steps(2);
        harness.get_by_label("Vibrance").focus();
        harness.run_steps(2);
        assert!(harness.get_by_label("Vibrance").is_focused(), "the actual numeric editor owns keyboard input");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_steps(1);
        harness.get_by_label("Vibrance").type_text("17.25");
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert!(!harness.state().ui.status_error, "{}", harness.state().ui.status);
        let widget = harness.state().session.active().unwrap().doc.clone();
        assert_eq!(fixture::args(&widget), [17.25f32.to_bits(), case.args[1]]);
        fixture::storage(&before, &widget);
        harness.state_mut().run("edit.undo", Value::Null).unwrap();
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        harness.query_all_by_label(&caption).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        harness.run_steps(2);
        assert!(!harness.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        let app = harness.state_mut();
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let revision = app.session.active().unwrap().revision;
        let past = app.session.active().unwrap().history.past_len();
        super::super::run_checked(app, &before, revision, command, params);
        assert!(!app.ui.status_error);
        let edited = app.session.active().unwrap().doc.clone();
        fixture::storage(&before, &edited);
        assert_eq!(fixture::args(&edited), values.map(f32::to_bits));
        assert!(!edited.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        app.run("edit.undo", Value::Null).unwrap();
        let history = app.session.active().unwrap().history.clone();
        app.run("xcf.filter.set", fixture::wire(&node.id.0.to_string(), case.args, Some(true))).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        super::super::run_checked(app, &before, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":true}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        app.ui.status_error = false;
        app.run("layer.setProps", json!({"name":"Vibrance CURRENT","visible":true,"opacity":0.375})).unwrap();
        let current = app.session.active().unwrap().doc.clone();
        fixture::storage(&before, &current);
        let expected = preview(app, &current);
        let titled = current.accounted().unwrap().try_with_document_name("Vibrance native continuation", &fixture::ctl()).unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        let snapshot = reopened.session.active().unwrap().doc.clone();
        fixture::storage(&current, &snapshot);
        assert_eq!(snapshot.name, "Vibrance native continuation");
        assert_eq!(fixture::args(&snapshot), values.map(f32::to_bits));
        assert_eq!(preview(&reopened, &snapshot), expected);
        for (command, params) in [("layer.translate", json!({"dx":1})), ("layer.setProps", json!({"blend":"multiply"})), ("select.all", Value::Null)] {
            assert!(reopened.run(command, params).is_err());
        }
        assert!(reopened.session.active().unwrap().doc.shares_state(&snapshot));
        assert_eq!(initial.pixels.len(), 48);
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
        assert!(!reopened.session.has_jobs());
    }
}
