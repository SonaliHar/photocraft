//! Deferred plain integer hierarchy actual UI adoption/disclosure/display.
use super::*;
#[path = "../../../engine/src/accounted_plain_integer_hierarchy_test_support.rs"]
// This test fixture keeps its consumer-local namespace and assertions.
#[allow(clippy::duplicate_mod, reason = "Shared test fixtures retain consumer-local module identities and assertion helpers")]
mod f;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN/uncompiled generated integer hierarchy Source adoption, nested focus/disclosure, no filter widgets, actual CanvasDisplay/native13 continuation. No visual/GIMP/producer acceptance."]
fn integer_hierarchy_ui_keeps_native_depth_and_tree() {
    for depth in 0..2 {
        let input = f::import(depth);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "integer.xcf", &input, &f::ctl()).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        f::observed(&original, depth);
        let (group, leaf, empty) = f::ids(&original);
        app.background_jobs = true;
        crate::jobs_ui::run(&mut app, "layer.select", json!({"layer":leaf.0})).unwrap();
        assert_eq!(app.session.active().unwrap().active_layer, Some(leaf));
        let mut h = Harness::builder().with_size(egui::vec2(600., 420.)).build_ui_state(move |ui, app| properties(app, ui, leaf), app);
        h.run_steps(2);
        assert!(h.query_all_by_label("No GIMP filters").next().is_some());
        assert!(h.query_all_by_label("Filter editing is unavailable").next().is_none());
        let app = h.state_mut();
        let rows = crate::layer_tree_ui::display_rows(&app.session.active().unwrap().doc, false).len();
        let rev = app.session.active().unwrap().revision;
        let journal = app.session.journal.len();
        crate::jobs_ui::run(app, "layer.setExpanded", json!({"layer":group.0,"expanded":false})).unwrap();
        assert!(crate::layer_tree_ui::display_rows(&app.session.active().unwrap().doc, false).len() < rows);
        assert_eq!(app.session.active().unwrap().revision, rev);
        assert_eq!(app.session.journal.len(), journal);
        crate::jobs_ui::run(app, "layer.setExpanded", json!({"layer":group.0,"expanded":true})).unwrap();
        crate::jobs_ui::run(app, "layer.select", json!({"layer":empty.0})).unwrap();
        assert_eq!(app.session.active().unwrap().active_layer, Some(empty));
        let before = app.session.active().unwrap().doc.clone();
        let display = app.session.color.canvas_display_for(&before, None).unwrap();
        let (image, fee) = crate::canvas::test_backed_snapshot_display_image(
            &before,
            before.size.width,
            before.size.height,
            Some(display.as_ref()),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        let expected = image.pixels.clone();
        drop((image, fee));
        let mut reopen = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopen, "integer.pcraft", f::archive(before.accounted().unwrap()), &f::ctl()).unwrap();
        let now = reopen.session.active().unwrap().doc.clone();
        f::same(&before, &now);
        f::observed(&now, depth);
        let display = reopen.session.color.canvas_display_for(&now, None).unwrap();
        let (image, fee) = crate::canvas::test_backed_snapshot_display_image(
            &now,
            now.size.width,
            now.size.height,
            Some(display.as_ref()),
            &photocraft_raster::Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(image.pixels, expected);
        drop((image, fee));
    }
}
