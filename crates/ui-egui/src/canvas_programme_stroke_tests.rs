//! Deferred common captured canvas over retained programme-owning native leaves.
use super::*;
use photocraft_engine::Session;
#[path = "../../engine/src/accounted_programme_paint_test_support.rs"]
mod f;
fn app(case: f::Case) -> PhotocraftApp {
    let input = f::load(case);
    let mut s = Session::new();
    f::adopt(&mut s, case, &input);
    if s.active().unwrap().doc.accounted().unwrap().is_readonly_normal_hierarchy_storage() {
        let id = f::target(&s.active().unwrap().doc);
        s.execute("layer.select", json!({"layer":id.0})).unwrap();
    }
    s.tools.brush.pressure_size = false;
    s.tools.brush.size = 2.0;
    let mut app = PhotocraftApp::new(s, Default::default());
    app.ui.tool = Tool::Brush;
    app.background_jobs = true;
    app
}
#[test]
#[ignore = "UNRUN/uncompiled genuine Source/explicit paid masked-hierarchy CURRENT canvas Down/Move/Up programme-preserving COW, unchanged masks/source/siblings and captured stale zoom/source refusal; no live preview, producer or visual oracle."]
fn programme_canvas_reuses_one_pressed_native_target() {
    for case in [f::Case::Point, f::Case::FilteredFlat, f::Case::MaskedFilteredFlat, f::Case::Hierarchy, f::Case::MaskedHierarchyCurrent] {
        let mut a = app(case);
        let before = a.session.active().unwrap().doc.clone();
        let id = a.session.active().unwrap().active_layer.unwrap();
        let photocraft_doc::LayerContent::BackedRaster(b) = &f::layer(&before, id).content else { panic!("actual initially focused raster") };
        let r = b.operative_bounds();
        let witness = f::Witness::capture(before.accounted().unwrap(), &f::ctl());
        let raw = f::raw(&before, id);
        let revision = a.session.active().unwrap().revision;
        let past = a.session.active().unwrap().history.past_len();
        tool_event(&mut a, ToolEvent::Down { x: f64::from(r.x0) + 2.0, y: f64::from(r.y0) + 2.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(a.drag.as_ref().unwrap().native_stroke_target.is_some());
        assert!(a.live_stroke.is_none());
        tool_event(&mut a, ToolEvent::Move { x: f64::from(r.x0) + 3.0, y: f64::from(r.y0) + 3.0, pressure: 0.8 }, egui::Modifiers::NONE);
        assert!(before.shares_state(&a.session.active().unwrap().doc));
        tool_event(&mut a, ToolEvent::Up { x: f64::from(r.x0) + 3.0, y: f64::from(r.y0) + 3.0 }, egui::Modifiers::NONE);
        assert!(!a.ui.status_error, "{}", a.ui.status);
        let after = a.session.active().unwrap().doc.clone();
        assert_eq!(a.session.active().unwrap().revision, revision + 1);
        assert_eq!(a.session.active().unwrap().history.past_len(), past + 1);
        assert_ne!(f::raw(&after, id), raw);
        witness.preserved(&after, id, &f::ctl());
        a.session.undo_with_control(&f::ctl()).unwrap();
        assert_eq!(f::raw(&a.session.active().unwrap().doc, id), raw);
    }
    for variant in 0..2 {
        let mut a = app(f::Case::Point);
        let before = a.session.active().unwrap().doc.clone();
        let id = a.session.active().unwrap().active_layer.unwrap();
        let photocraft_doc::LayerContent::BackedRaster(b) = &f::layer(&before, id).content else { panic!("native") };
        let r = b.operative_bounds();
        tool_event(&mut a, ToolEvent::Down { x: f64::from(r.x0) + 2.0, y: f64::from(r.y0) + 2.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let d = a.drag.take().unwrap();
        if variant == 0 {
            let index = a.session.active_index().unwrap();
            a.ui.views.resize_with(index + 1, Default::default);
            a.ui.views[index].zoom = 2.0;
        } else {
            a.session.active_mut().unwrap().source_read_only = false;
        }
        assert!(native_stroke::commit(&mut a, &d));
        assert!(a.ui.status_error);
        assert!(before.shares_state(&a.session.active().unwrap().doc));
    }
}
