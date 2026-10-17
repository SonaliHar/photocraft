//! Deferred ordered controls; no visual/runtime or producer numeric acceptance.
use super::*;
#[path = "../../../engine/src/accounted_filtered_flat_ordered_test_support.rs"]
// Keep this test fixture local to its consumer namespace.
#[allow(clippy::duplicate_mod, reason = "Test fixture instances preserve local types, visibility and cache identity")]
mod fixture;
use egui_kittest::{Harness, kittest::Queryable};
fn app(case: &fixture::data::Case) -> PhotocraftApp {
    let input = fixture::import(case);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut app, "ordered.xcf", &input, &fixture::ctl()).unwrap();
    app.background_jobs = true;
    app
}
#[test]
#[ignore = "UNRUN genuine2 ordered sources, actual lastnode scalar widget/IDs+stackrevision, sameframe sharedsnapshot stale control, original/native/raw/typedargs custody and paid synthetic1root2 automaticnative preview continuity. Visual/numeric/operativeorder acceptance deferred."]
fn ordered_ui_nodes_keep_indices_and_one_frame_capture() {
    for case in &fixture::data::CASES {
        let app = app(case);
        let before = app.session.active().unwrap().doc.clone();
        fixture::observed(&before, case);
        let root = before.layers[3].id;
        let stack = before.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap();
        assert_eq!(stack.nodes.len(), 3);
        let ids = stack.nodes.iter().map(|n| n.id).collect::<Vec<_>>();
        assert_eq!(stack.nodes[2].operation.name, "gegl:vibrance");
        let revision = stack.revision;
        for (index, id) in ids.iter().enumerate() {
            let target = capture_at(&app, root, index).unwrap();
            assert_eq!(target.node_index, index);
            assert_eq!(target.filter, *id);
            assert!(validate(&app, &target));
        }
        let mut h = Harness::builder().with_size(egui::vec2(800., 900.)).build_ui_state(move |ui, app| super::super::filter_properties(app, ui, root), app);
        h.run_steps(2);
        h.get_by_label("Vibrance").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("Vibrance").type_text("-12.5");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let app = h.state_mut();
        assert!(!app.ui.status_error);
        let after = app.session.active().unwrap().doc.clone();
        fixture::payload(&before, &after);
        let changed = after.xcf_state.as_ref().unwrap().filter_stacks.iter().find(|s| s.owner == ItemRef::Layer(root)).unwrap();
        assert_eq!(changed.revision, revision + 1);
        assert_eq!(changed.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), ids);
        assert_eq!(&changed.nodes[..2], &stack.nodes[..2]);
        assert_ne!(changed.nodes[2].arguments, stack.nodes[2].arguments);
        app.run("edit.undo", json!({})).unwrap();
        fixture::same(&before, &app.session.active().unwrap().doc);
        app.run("edit.redo", json!({})).unwrap();
        fixture::same(&after, &app.session.active().unwrap().doc);
        let snapshot = app.session.active().unwrap().doc.clone();
        let document_revision = app.session.active().unwrap().revision;
        let first = capture_snapshot(&snapshot, document_revision, root, 0).unwrap();
        let second = capture_snapshot(&snapshot, document_revision, root, 1).unwrap();
        assert!(validate(app, &first) && validate(app, &second));
        submit(app, &first, "xcf.filter.enable", json!({"filterId":first.filter.0.to_string(),"enabled":false}));
        assert!(!app.ui.status_error);
        let once = app.session.active().unwrap().doc.clone();
        assert!(!validate(app, &second));
        submit(app, &second, "xcf.filter.enable", json!({"filterId":second.filter.0.to_string(),"enabled":false}));
        assert_eq!(app.ui.status, "xcf.properties.stale-target");
        assert!(app.session.active().unwrap().doc.shares_state(&once));
        let single = fixture::single_root_two(before.accounted().unwrap());
        let mut one = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut one, "single.pcraft", fixture::archive(&single), &fixture::ctl()).unwrap();
        let single_snapshot = one.session.active().unwrap().doc.clone();
        fixture::same(single.document(), &single_snapshot);
        let root = single_snapshot.layers[0].id;
        let first = capture_at(&one, root, 0).unwrap();
        let last = capture_at(&one, root, 1).unwrap();
        assert_ne!(first.filter, last.filter);
        assert_ne!(first.node_index, last.node_index);
        let display = one.session.color.canvas_display_for(&single_snapshot, None).unwrap();
        let (image, fee) =
            crate::canvas::test_backed_snapshot_display_image(&single_snapshot, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        let expected = image.clone();
        drop((image, fee));
        let mut next = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_native_open::adopt_bytes(&mut next, "continued.pcraft", fixture::archive(single_snapshot.accounted().unwrap()), &fixture::ctl()).unwrap();
        let continued = next.session.active().unwrap().doc.clone();
        fixture::same(&single_snapshot, &continued);
        let display = next.session.color.canvas_display_for(&continued, None).unwrap();
        let (image, fee) =
            crate::canvas::test_backed_snapshot_display_image(&continued, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(image.pixels, expected.pixels);
        drop((image, fee));
        let bad = fixture::malformed(&single);
        let mut failed = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        assert!(crate::owned_native_open::adopt_bytes(&mut failed, "bad.pcraft", fixture::archive(&bad), &fixture::ctl()).is_err());
        assert!(failed.session.active().is_none());
    }
}
