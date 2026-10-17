//! Synthetic CURRENT geometry controls. No authored larger source/GIMP oracle.
use super::*;
#[path = "../pure_point_geometry_fixture.rs"]
#[allow(dead_code)]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod fixture;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use fixture::Family;
use photocraft_engine::DocumentSnapshot;
use serde_json::Value;
fn candidate(family: Family, doc: &DocumentSnapshot) -> bool {
    let layer = doc.layers[0].id;
    let node = &doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
    match family {
        Family::Semi => semi_flatten::candidate(doc, layer, node),
        Family::Hue => hue_chroma::candidate(doc, layer, node),
        Family::Vibrance => vibrance::candidate(doc, layer, node),
    }
}
fn preview(app: &PhotocraftApp, doc: &DocumentSnapshot) -> egui::ColorImage {
    let display = app.session.color.canvas_display_for(doc, None).unwrap();
    assert!(!display.encode_srgb);
    let (image, fee) =
        crate::canvas::test_backed_snapshot_display_image(doc, doc.size.width, doc.size.height, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE)
            .unwrap();
    assert_eq!(image.pixels.len(), doc.size.area() as usize);
    let capture = image.clone(); // External CURRENT adapter continuity capture only.
    drop((image, fee));
    capture
}
#[test]
#[ignore = "UNRUN: prospectively paid synthetic origin257x259/1x257 CURRENT native owners for Semi/Hue/Vibrance F32/F64, strict UI candidates/actual checkbox/registered params+activation/props/history/query/title/native/remove-empty; no authored larger source, producer display oracle or new numerical/profile acceptance; visual testing deferred"]
fn bounded_origin_pure_point_controls_and_native_current_continue() {
    for family in [Family::Semi, Family::Hue, Family::Vibrance] {
        for sample in [photocraft_doc::SampleType::F32, photocraft_doc::SampleType::F64] {
            for variant in 0..2 {
                let owner = fixture::fixture(family, sample, variant);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                crate::owned_native_open::adopt_bytes(&mut app, "synthetic-current.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
                app.background_jobs = true;
                let before = app.session.active().unwrap().doc.clone();
                assert!(before.accounted().unwrap().is_readonly_point_storage());
                assert!(before.accounted().unwrap().quota().shares_ledger(owner.quota()));
                assert!(candidate(family, &before));
                fixture::storage(owner.document(), &before);
                let layer = before.layers[0].id;
                let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
                let id = node.id.0.to_string();
                let initial = preview(&app, &before);
                let query = photocraft_engine::accounted_filter_inspect::prepare(&app.session, &json!({"filterId":id}), &fixture::ctl()).unwrap();
                assert_eq!(query.value()["operation"]["name"], family.name());
                drop(query);
                let past = app.session.active().unwrap().history.past_len();
                app.run("xcf.filter.set", family.changed(&id)).unwrap();
                let disabled = app.session.active().unwrap().doc.clone();
                fixture::storage(&before, &disabled);
                assert!(!disabled.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
                assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
                app.run("edit.undo", Value::Null).unwrap();
                let history = app.session.active().unwrap().history.clone();
                assert_eq!(app.run("xcf.filter.set", json!({"filterId":id,"arguments":family.arguments(),"enabled":true})).unwrap()["updated"], false);
                assert!(app.session.active().unwrap().history.shares_accounted_history(&history));
                app.run("edit.redo", Value::Null).unwrap();
                assert!(app.session.active().unwrap().doc.shares_state(&disabled));
                let mut harness =
                    Harness::builder().with_size(egui::vec2(900., 700.)).build_ui_state(move |ui, app| super::filter_properties(app, ui, layer), app);
                harness.run_steps(2);
                harness.query_all_by_label(family.name()).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
                harness.run_steps(2);
                assert!(harness.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
                let app = harness.state_mut();
                app.run("layer.setProps", json!({"name":"Origin CURRENT metadata","visible":true,"opacity":0.375})).unwrap();
                let current = app.session.active().unwrap().doc.clone();
                fixture::storage(&before, &current);
                let expected = preview(app, &current);
                let titled = current.accounted().unwrap().try_with_document_name("Origin CURRENT continuation", &fixture::ctl()).unwrap();
                let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(&titled), &fixture::ctl()).unwrap();
                let snapshot = reopened.session.active().unwrap().doc.clone();
                fixture::storage(&current, &snapshot);
                assert_eq!(snapshot.name, "Origin CURRENT continuation");
                assert_eq!(preview(&reopened, &snapshot), expected);
                let args = &current.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments;
                assert_eq!(&snapshot.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].arguments, args);
                for (command, params) in [("layer.translate", json!({"dx":1})), ("layer.setProps", json!({"blend":"multiply"})), ("select.all", Value::Null)] {
                    assert!(reopened.run(command, params).is_err());
                }
                assert!(reopened.session.active().unwrap().doc.shares_state(&snapshot));
                reopened.run("xcf.filter.remove", json!({"filterId":id})).unwrap();
                let empty = reopened.session.active().unwrap().doc.clone();
                assert!(empty.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.is_empty());
                assert!(empty.xcf_state.as_ref().unwrap().filter_masks.is_empty());
                assert_eq!(fixture::raw(&empty), fixture::raw(&before));
                let image = preview(&reopened, &empty);
                let mut empty_app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                crate::owned_native_open::adopt_bytes(&mut empty_app, "empty.pcraft", fixture::archive(empty.accounted().unwrap()), &fixture::ctl()).unwrap();
                assert_eq!(preview(&empty_app, &empty_app.session.active().unwrap().doc), image);
                assert_eq!(empty_app.session.active().unwrap().doc.xcf_state, empty.xcf_state);
                assert_eq!(initial.pixels.len(), before.size.area() as usize);
                assert!(!empty_app.session.has_jobs());
            }
            for invalid in [fixture::offset_fixture(family, sample), fixture::malformed_fixture(family, sample)] {
                let mut refused = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                assert!(crate::owned_native_open::adopt_bytes(&mut refused, "invalid.pcraft", fixture::archive(&invalid), &fixture::ctl()).is_err());
                assert!(refused.session.active().is_none());
            }
        }
    }
}
