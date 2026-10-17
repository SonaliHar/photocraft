//! Genuine Source UI continuation, not a GIMP display or numerical oracle.
use super::*;
#[path = "../../../engine/src/accounted_masked_filtered_flat_test_support.rs"]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN genuine8 masked mixed Sources, actual checkbox plus captured shared-frame stale target, native12 pathless actualCanvasDisplay continuity and empty dedicated panel"]
fn masked_mixed_ui_captures_one_frame_and_preserves_native() {
    for c in fixture::cases() {
        let source = fixture::import(&c);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "masked.xcf", &source, &fixture::ctl()).unwrap();
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let st = &before.xcf_state.as_ref().unwrap().filter_stacks[0];
        let ItemRef::Layer(root) = st.owner else { panic!("root") };
        let target = capture_snapshot(&before, revision, root, 0).unwrap();
        assert!(target.route == Route::Masked);
        let other = before
            .xcf_state
            .as_ref()
            .unwrap()
            .filter_stacks
            .iter()
            .flat_map(|s| {
                let ItemRef::Layer(l) = s.owner else { panic!("root") };
                s.nodes.iter().enumerate().map(move |(i, _)| (l, i))
            })
            .nth(1)
            .unwrap();
        let stale = capture_snapshot(&before, revision, other.0, other.1).unwrap();
        let n = &st.nodes[0];
        let caption: String = n.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&n.operation.name).chars().take(128).collect();
        let mut h = Harness::builder().with_size(egui::vec2(800., 600.)).build_ui_state(move |ui, app| properties(app, ui, root), app);
        h.run_steps(2);
        assert!(h.query_all_by_label("Filter editing is unavailable").next().is_none());
        h.get_by_label(caption.as_str()).click();
        h.run_steps(2);
        let app = h.state_mut();
        let after = app.session.active().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().revision, revision + 1);
        fixture::payload(&before, &after);
        assert!(!validate(app, &stale));
        submit(app, &stale, "xcf.filter.enable", json!({"filterId":stale.filter.0.to_string(),"enabled":false}));
        assert!(app.session.active().unwrap().doc.shares_state(&after));
        let display = app.session.color.canvas_display_for(&after, None).unwrap();
        let (image, fee) =
            crate::canvas::test_backed_snapshot_display_image(&after, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.pixels.clone();
        drop((image, fee));
        let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut reopened, "masked.pcraft", fixture::archive(after.accounted().unwrap()), &fixture::ctl()).unwrap();
        let now = reopened.session.active().unwrap().doc.clone();
        fixture::storage(&after, &now);
        let display = reopened.session.color.canvas_display_for(&now, None).unwrap();
        let (image, fee) = crate::canvas::test_backed_snapshot_display_image(&now, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected);
        drop((image, fee));
        app.run("edit.undo", serde_json::Value::Null).unwrap();
        fixture::same(&before, &app.session.active().unwrap().doc);
        let all: Vec<_> = before.xcf_state.as_ref().unwrap().filter_stacks.iter().flat_map(|s| s.nodes.iter().map(|n| n.id)).collect();
        for id in all {
            app.run("xcf.filter.remove", json!({"filterId":id.0.to_string()})).unwrap();
        }
        let empty = app.session.active().unwrap().doc.clone();
        assert!(empty.accounted().unwrap().is_readonly_masked_filtered_flat_point_storage());
        assert!(route(&empty) == Some(Route::Masked));
        assert!(empty.xcf_state.as_ref().unwrap().filter_stacks.iter().all(|s| s.nodes.is_empty()));
        h.run_steps(2);
        assert!(h.query_all_by_label("No GIMP filters").next().is_some());
    }
}
