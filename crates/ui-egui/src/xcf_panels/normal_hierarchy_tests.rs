//! Deferred actual native CURRENT tree UI, no hierarchy Source or pixel oracle.
use super::*;
#[path = "../../../engine/src/accounted_normal_hierarchy_test_support.rs"]
mod f;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
#[ignore = "UNRUN/uncompiled: nested native13 checkbox/shared-frame capture, group disclosure and actual CanvasDisplay continuation"]
fn hierarchy_ui_targets_real_nested_leaf_and_group() {
    for case in f::cases() {
        let source = f::import(&case);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        crate::owned_xcf_open::adopt_bytes(&mut app, "hierarchy.xcf", &source, &f::ctl()).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        f::observed(&before, &case);
        assert!(before.accounted().unwrap().is_readonly_normal_hierarchy_storage());
    }

    let owner = f::build(1);
    let root = f::layer(&owner);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut app, "hierarchy.pcraft", f::archive(&owner), &f::ctl()).unwrap();
    app.background_jobs = true;
    crate::jobs_ui::run(&mut app, "layer.select", json!({"layer":root.0})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let stale = capture_snapshot(&before, revision, root, 1).unwrap();
    assert!(stale.route == Route::Hierarchy);
    let node = node_at(&before, root, 0).unwrap().0;
    let id = node.id;
    let enabled = node.enabled;
    let caption: String = node.source_display_name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&node.operation.name).chars().take(128).collect();
    let mut h = Harness::builder().with_size(egui::vec2(850., 700.)).build_ui_state(move |ui, app| properties(app, ui, root), app);
    h.run_steps(2);
    assert!(h.query_all_by_label("Filter editing is unavailable").next().is_none());
    h.get_by_label(caption.as_str()).click();
    h.run_steps(2);
    let app = h.state_mut();
    let after = app.session.active().unwrap().doc.clone();
    assert_eq!(app.session.active().unwrap().revision, revision + 1);
    assert_eq!(node_at(&after, root, 0).unwrap().0.id, id);
    assert_eq!(node_at(&after, root, 0).unwrap().0.enabled, !enabled);
    assert!(!validate(app, &stale));
    f::payload(&before, &after);
    let visible = crate::layer_tree_ui::display_rows(&after, false).len();
    let revision = app.session.active().unwrap().revision;
    let journal = app.session.journal.len();
    crate::jobs_ui::run(app, "layer.setExpanded", json!({"layer":f::OUTER.0,"expanded":false})).unwrap();
    let collapsed = app.session.active().unwrap().doc.clone();
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.journal.len(), journal);
    assert!(crate::layer_tree_ui::display_rows(&collapsed, false).len() < visible);
    crate::jobs_ui::run(app, "layer.setExpanded", json!({"layer":f::OUTER.0,"expanded":true})).unwrap();
    crate::jobs_ui::run(app, "layer.select", json!({"layer":f::EMPTY.0})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(f::EMPTY));
    let after = app.session.active().unwrap().doc.clone();
    let display = app.session.color.canvas_display_for(&after, None).unwrap();
    let (image, fee) =
        crate::canvas_backed_preview::backed_snapshot_display_image(&after, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    let expected = image.pixels.clone();
    drop((image, fee));
    let mut reopened = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_native_open::adopt_bytes(&mut reopened, "continued.pcraft", f::archive(after.accounted().unwrap()), &f::ctl()).unwrap();
    let now = reopened.session.active().unwrap().doc.clone();
    f::same(&after, &now);
    let display = reopened.session.color.canvas_display_for(&now, None).unwrap();
    let (image, fee) =
        crate::canvas_backed_preview::backed_snapshot_display_image(&now, 8, 6, Some(display.as_ref()), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(image.pixels, expected);
    drop((image, fee));
}
