//! Deferred final Remove widget/dedicated empty class/native preview continuity.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_current_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN genuine2 source→lastactualRemove widget→dedicatedempty UI, sharedframe stale secondaction, actualCanvasDisplay/native10 emptypreview and zeroquery/metadata/undo; no visual/producerempty numeric acceptance."]
fn empty_mixed_ui_never_falls_through() {
    for case in &fixture::data::CASES {
        let input = fixture::import(case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "ordered.xcf", &input, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let original = app.session.active().unwrap().doc.clone();
        let root = original.layers[3].id;
        let revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&original, revision, root, 0).unwrap();
        let second = capture_snapshot(&original, revision, root, 1).unwrap();
        submit(&mut app, &first, "xcf.filter.remove", serde_json::json!({"filterId":first.filter.0.to_string()}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(&app, &second));
        submit(&mut app, &second, "xcf.filter.remove", serde_json::json!({"filterId":second.filter.0.to_string()}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let ids = once.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect::<Vec<_>>();
        let last = *ids.last().unwrap();
        for id in ids.iter().take(ids.len() - 1) {
            let before = app.session.active().unwrap().doc.clone();
            app.run("xcf.filter.remove", serde_json::json!({"filterId":id.0.to_string()})).unwrap();
            fixture::preserved(&before, &app.session.active().unwrap().doc, *id);
        }
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| &s.nodes).count(), 1);
        let mut h = Harness::builder().with_size(egui::vec2(800., 800.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        h.get_by_label("Remove").click();
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let empty = app.session.active().unwrap().doc.clone();
        fixture::preserved(&before, &empty, last);
        assert!(empty.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        let display = app.session.color.canvas_display_for(&empty, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&empty, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.clone();
        drop((image, fee));
        let archive = fixture::archive(empty.accounted().unwrap());
        assert!(photocraft_engine::accounted_native_open::is_owned_current_filtered_flat_candidate(&archive, &fixture::ctl()).unwrap());
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "empty.pcraft", archive, &fixture::ctl()).unwrap();
        fixture::same(&empty, &reopened.session.active().unwrap().doc);
        let snapshot = reopened.session.active().unwrap().doc.clone();
        let display = reopened.session.color.canvas_display_for(&snapshot, None).unwrap();
        let (image, fee) =
            crate::canvas_backed_preview::backed_snapshot_display_image(&snapshot, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        let mut panel =
            Harness::builder().with_size(egui::vec2(800., 600.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), reopened);
        panel.run_steps(2);
        assert!(panel.query_all_by_label("No GIMP filters").next().is_some());
        assert!(panel.query_all_by_label("Remove").next().is_none());
        assert!(panel.query_all_by_label("Add Filter").next().is_none());
        let reopened = panel.state_mut();
        let q = photocraft_engine::accounted_xcf_inspect::prepare(&reopened.session, &serde_json::json!({}), &fixture::ctl()).unwrap();
        assert_eq!(q.value()["filters"]["total"], 0);
        drop(q);
        reopened.run("layer.setProps", serde_json::json!({"layer":root.0,"name":"Empty mixed root","opacity":0.5})).unwrap();
        assert!(reopened.session.active().unwrap().doc.accounted().unwrap().is_readonly_filtered_flat_point_storage());
        reopened.run("edit.undo", serde_json::json!({})).unwrap();
        fixture::same(&empty, &reopened.session.active().unwrap().doc);
    }
}
