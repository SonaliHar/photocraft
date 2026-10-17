//! UNRUN exact authored F64 source and actual owned Box control consumers.
use super::*;
#[path = "../assigned_f64_box_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::Value;
#[test]
#[ignore = "UNRUN: two authored assigned-F64 observed Box sources, actual checkbox/Signed32 controls, HDR/signedzero native/source/profile custody, radius/activation/history/metadata/Move/native continuation and refusals; visual/numerical acceptance deferred"]
fn assigned_f64_observed_controls_preserve_authored_storage_and_continue() {
    for (path, bytes, old_radius) in fixture::cases() {
        let input = fixture::import(path.file_name().unwrap().to_str().unwrap(), bytes.clone());
        assert!(matches!(&input, photocraft_io::OwnedXcfImport::Point(_)));
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "assigned-f64.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before);
        assert!(before.accounted().unwrap().is_readonly_assigned_point_storage());
        let layer = before.layers[0].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(radius(&before, layer, node), Some(old_radius));
        let expected = crate::canvas::display_image(None, &fixture::reference(&before));
        let (actual, fee) =
            crate::canvas::test_backed_snapshot_display_image(&before, before.size.width, before.size.height, None, &photocraft_raster::Interrupt::NONE)
                .unwrap();
        assert_eq!(actual.pixels.len(), expected.pixels.len());
        for (a, b) in actual.pixels.iter().zip(&expected.pixels) {
            for (a, b) in a.to_array().into_iter().zip(b.to_array()) {
                assert!((i16::from(a) - i16::from(b)).abs() <= 1);
            }
        }
        drop((actual, fee));
        let caption = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let mut harness =
            Harness::builder().with_size(egui::vec2(900., 700.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
        harness.run();
        harness.query_all_by_label(&caption).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        harness.run();
        assert!(!harness.state().ui.status_error);
        let app = harness.state_mut();
        let disabled = app.session.active().unwrap().doc.clone();
        assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        fixture::native_roles(&before, &disabled);
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        let target = if old_radius == 4 { 5 } else { 4 };
        let p = request(node, old_radius, target, false).unwrap();
        let revision = app.session.active().unwrap().revision;
        let past = app.session.active().unwrap().history.past_len();
        super::super::run_checked(app, &before, revision, "xcf.filter.set", p);
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        fixture::native_roles(&before, &changed);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Edit GIMP Stock Filter"));
        assert_eq!(radius(&changed, layer, &changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]), Some(target));
        app.run("edit.undo", Value::Null).unwrap();
        let history = app.session.active().unwrap().history.clone();
        app.run("xcf.filter.enable", json!({"filterId":node.id.0.to_string(),"enabled":true})).unwrap();
        assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
        app.run("edit.redo", Value::Null).unwrap();
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        // Full prepublication renderer checks still run on disabled/equal state.
        for p in [
            json!({"filterId":node.id.0.to_string(),"arguments":[{"name":"radius","value":{"Number":{"Signed":{"width":32,"value":1001}}}}]}),
            json!({"filterId":node.id.0.to_string(),"opacityF64Bits":"0x3ff0000000000000"}),
        ] {
            assert!(app.run("xcf.filter.set", p).is_err());
        }
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.run("layer.setProps", json!({"name":"Assigned F64 CURRENT","opacity":0.375,"coalesce":"F64"})).unwrap();
        app.run("layer.translate", json!({"dx":1,"dy":-1,"coalesce":"F64"})).unwrap();
        let moved = app.session.active().unwrap().doc.clone();
        fixture::exact(&changed, &moved, 1, -1);
        fixture::observed(&moved);
        let titled = moved.accounted().unwrap().try_with_document_name("Assigned F64 continued", &fixture::ctl()).unwrap();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
        fixture::exact(&moved, &reopened.session.active().unwrap().doc, 0, 0);
        fixture::observed(&reopened.session.active().unwrap().doc);
        assert_eq!(reopened.session.active().unwrap().doc.name, "Assigned F64 continued");
        let current = reopened.session.active().unwrap().doc.clone();
        let retained = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(radius(&current, layer, retained), Some(target));
        reopened.run("xcf.filter.enable", json!({"filterId":retained.id.0.to_string(),"enabled":true})).unwrap();
        fixture::observed(&reopened.session.active().unwrap().doc);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!reopened.session.has_jobs());
    }
}
