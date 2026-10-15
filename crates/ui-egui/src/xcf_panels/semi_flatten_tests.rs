//! Deferred authored storage/control continuity; no producer numerical acceptance.
use super::*;
#[path = "../semi_flatten_fixture.rs"]
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
    let (image, fee) =
        crate::canvas_backed_preview::backed_snapshot_display_image(doc, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(image.pixels.len(), 48);
    let captured = image.clone(); // External display capture, not quota evidence.
    drop((image, fee));
    captured
}

#[test]
#[ignore = "UNRUN: 12 authored RGB F32/F64 Semi-Flatten Linear/NL literal source/edit files, actual checkbox/RGB controls, paid native/query/canvas continuity and raw alpha/encoding/profile/source/history/fences; CSS/ICC/Babl/FMA/F64 numerical HOLDs retained, visual testing deferred"]
fn authored_semi_flatten_controls_and_native_continue_without_literal_aliases() {
    for case in fixture::cases() {
        let input = fixture::import(case.path.file_name().unwrap().to_str().unwrap(), case.bytes.clone());
        assert!(matches!(&input, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "semi-flatten.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, &case);
        let layer = before.layers[0].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert!(candidate(&before, layer, node));
        let original_display = preview(&app, &before);
        let original_color = fixture::color(&before).clone();
        let next = patch(&original_color, [0.125, 0.625, 0.375]).unwrap();
        assert_eq!(fixture::color_bits(&next).0[3], case.color[3]);
        assert_eq!(fixture::color_bits(&next).1, case.encoding);
        // Standalone controller values, not fabricated source/current owners.
        let mut alpha_zero = original_color.clone();
        if let FilterValue::Color { components, .. } = &mut alpha_zero {
            components[3] = ExactNumber::Float32Bits((-0.0f32).to_bits());
        }
        assert_eq!(fixture::color_bits(&patch(&alpha_zero, [0.125, 0.625, 0.375]).unwrap()).0[3], (-0.0f32).to_bits());
        let linear = FilterValue::Color {
            components: [0.5f32, 0.25, 0.75, 1.].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
            encoding: SampleEncoding::Linear,
            profile: None,
        };
        let mut nonlinear = linear.clone();
        if let FilterValue::Color { encoding, .. } = &mut nonlinear {
            *encoding = SampleEncoding::NonlinearProfile;
        }
        assert!((picker(&linear).unwrap()[0] - 0.5).abs() < 2e-6);
        assert!((picker(&nonlinear).unwrap()[0] - picker(&linear).unwrap()[0]).abs() > 0.1);
        let caption = node.source_display_name.as_deref().filter(|v| !v.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let mut harness =
            Harness::builder().with_size(egui::vec2(900., 700.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
        harness.run_steps(2);
        // Open the actual popup without publishing a round-trip edit, then use
        // the real Red DragValue through the same guarded registered request.
        harness
            .query_all_by_label("Color")
            .find(|n| n.accesskit_node().role() == egui::accesskit::Role::ColorWell && !n.accesskit_node().is_disabled())
            .unwrap()
            .click();
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        let past = harness.state().session.active().unwrap().history.past_len();
        harness.get_by_label("Red").click();
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Red").type_text("0.125");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        let actual_widget = harness.state().session.active().unwrap().doc.clone();
        let (mut expected, encoding) = fixture::color_bits(&original_color);
        expected[0] = 0.125f32.to_bits();
        assert_eq!(fixture::color_bits(fixture::color(&actual_widget)), (expected, encoding));
        assert_eq!(harness.state().session.active().unwrap().history.past_len(), past + 1);
        fixture::storage(&before, &actual_widget);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.state_mut().run("edit.undo", Value::Null).unwrap();
        harness.run_steps(2);
        assert!(harness.state().session.active().unwrap().doc.shares_state(&before));
        harness.query_all_by_label(&caption).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        harness.run();
        assert!(!harness.state().ui.status_error);
        let app = harness.state_mut();
        assert!(!app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        fixture::storage(&before, &app.session.active().unwrap().doc);
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let revision = app.session.active().unwrap().revision;
        super::super::run_checked(app, &before, revision, "xcf.filter.set", fixture::wire(&node.id.0.to_string(), &next));
        assert!(!app.ui.status_error);
        let edited = app.session.active().unwrap().doc.clone();
        fixture::storage(&before, &edited);
        assert_eq!(fixture::color(&edited), &next);
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        app.run("edit.undo", Value::Null).unwrap();
        let history = app.session.active().unwrap().history.clone();
        app.run("xcf.filter.set", fixture::wire(&node.id.0.to_string(), &original_color)).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        super::super::run_checked(app, &before, revision, "xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":false}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&edited));
        app.ui.status_error = false;
        app.run("layer.setProps", json!({"name":"Semi CURRENT"})).unwrap();
        app.run("layer.setProps", json!({"visible":false})).unwrap();
        app.run("edit.undo", Value::Null).unwrap();
        app.run("layer.setProps", json!({"name":"Semi combined CURRENT","visible":true,"opacity":0.375})).unwrap();
        let current = app.session.active().unwrap().doc.clone();
        fixture::storage(&edited, &current);
        let expected = preview(app, &current);
        let titled = current.accounted().unwrap().try_with_document_name("Semi continued", &fixture::ctl()).unwrap();
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        let snapshot = reopened.session.active().unwrap().doc.clone();
        fixture::storage(&current, &snapshot);
        assert_eq!(fixture::color(&snapshot), &next);
        assert_eq!(snapshot.name, "Semi continued");
        assert_eq!(preview(&reopened, &snapshot), expected);
        // Original display is an adapter capture only, never a producer oracle.
        assert_eq!(original_display.pixels.len(), 48);
        for (id, p) in [("layer.translate", json!({"dx":1})), ("layer.setProps", json!({"blend":"multiply"})), ("select.all", Value::Null)] {
            assert!(reopened.run(id, p).is_err());
        }
        assert_eq!(std::fs::read(&case.path).unwrap(), case.bytes);
        assert!(!reopened.session.has_jobs());
    }
}
