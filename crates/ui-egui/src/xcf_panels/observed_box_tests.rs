//! Deferred CURRENT-only controls; no producer radius/display acceptance claim.
use super::*;
#[path = "../canvas_streamed_box_fixture.rs"]
#[allow(dead_code)]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
#[path = "../canvas_observed_box_fixture.rs"]
#[allow(dead_code)]
mod observed;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::Value;
#[test]
#[ignore = "UNRUN: actual owned Box checkbox consumer and bounded Signed32 request/run_checked for three CURRENT roles, background setting, stale/auth/history/native/raw/profile custody; visual checks deferred"]
fn owned_observed_box_controls_keep_captured_identity_and_native_custody() {
    for (sample, assigned) in [(photocraft_doc::SampleType::F32, false), (photocraft_doc::SampleType::F64, false), (photocraft_doc::SampleType::F32, true)] {
        let (base, _) = fixture::fixture(sample, assigned, 1);
        let owner = observed::with_raw(&base, [i32::MIN, 0, i32::MAX], &fixture::ctl());
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut app, "observed.pcraft", fixture::archive(&owner), &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let layer = before.layers[0].id;
        let node = &before.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        assert_eq!(radius(&before, layer, node), Some(4));
        assert_eq!(radius(&before, LayerId(layer.0 + 1), node), None);
        assert!(request(node, 4, -1, true).is_none());
        assert!(request(node, 4, 1001, true).is_none());
        assert!(request(node, 4, 4, true).is_none());
        let caption = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).to_owned();
        let mut harness =
            Harness::builder().with_size(egui::vec2(900., 700.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, layer), app);
        harness.run();
        harness.query_all_by_label(&caption).find(|n| !n.accesskit_node().is_disabled()).unwrap().click();
        harness.run();
        assert!(!harness.state().ui.status_error);
        assert!(!harness.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].enabled);
        let app = harness.state_mut();
        app.run("edit.undo", Value::Null).unwrap();
        let snapshot = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let node = &snapshot.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let p = request(node, 4, 0, false).unwrap();
        assert_eq!(p["arguments"][0]["value"]["Number"]["Signed"]["value"], 0);
        let past = app.session.active().unwrap().history.past_len();
        super::super::run_checked(app, &snapshot, revision, "xcf.filter.set", p);
        assert!(!app.ui.status_error);
        let changed = app.session.active().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
        assert_eq!(observed::raw(&changed), [i32::MIN, 0, i32::MAX]);
        assert_eq!(fixture::native(&changed), fixture::native(&before));
        assert_eq!(changed.icc_profile, before.icc_profile);
        assert_eq!(changed.source_color_context, before.source_color_context);
        assert_eq!(changed.xcf_state.as_ref().unwrap().filter_masks, before.xcf_state.as_ref().unwrap().filter_masks);
        assert!(!app.session.has_jobs());
        // A stale snapshot remains stale even when the numeric revision is restored.
        let saved_revision = app.session.active().unwrap().revision;
        app.session.active_mut().unwrap().revision = revision;
        super::super::run_checked(app, &snapshot, revision, "xcf.filter.set", request(node, 4, 1, true).unwrap());
        assert!(app.ui.status_error);
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.session.active_mut().unwrap().revision = saved_revision;
        let node = &changed.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0];
        let p = request(node, 0, 1000, true).unwrap();
        app.session.authorize = Some(|_, _| Err(photocraft_engine::EngineError::Other("observed Box edit denied".into())));
        assert!(app.run("xcf.filter.set", p.clone()).unwrap_err().to_string().contains("observed Box edit denied"));
        assert!(app.session.active().unwrap().doc.shares_state(&changed));
        app.session.authorize = None;
        app.run("xcf.filter.set", p).unwrap();
        let maximum = app.session.active().unwrap().doc.clone();
        let mut reopened = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", fixture::archive(maximum.accounted().unwrap()), &fixture::ctl()).unwrap();
        fixture::exact(&maximum, &reopened.session.active().unwrap().doc);
        assert_eq!(
            radius(
                &reopened.session.active().unwrap().doc,
                layer,
                &reopened.session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0]
            ),
            Some(1000)
        );
    }
}
