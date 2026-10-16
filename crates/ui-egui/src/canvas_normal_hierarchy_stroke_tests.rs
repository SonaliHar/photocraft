//! Deferred actual nested leaf canvas events, not visual acceptance.
use super::*;
#[path = "../../engine/src/accounted_plain_hierarchy_paint_test_support.rs"]
mod f;
fn app(depth: usize) -> PhotocraftApp {
    let input = f::import(depth);
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::owned_xcf_open::adopt_bytes(&mut a, "nested.xcf", &input, &f::ctl()).unwrap();
    let leaf = f::target_leaf(&a.session.active().unwrap().doc);
    crate::jobs_ui::run(&mut a, "layer.select", serde_json::json!({"layer":leaf.0})).unwrap();
    a.session.tools.brush.pressure_size = false;
    a.session.tools.brush.size = 7.0;
    a.ui.tool = Tool::Brush;
    a.background_jobs = true;
    a
}
#[test]
#[ignore = "UNRUN/uncompiled all-five actual generated hierarchy Source nested leaf Down/Move/Up COW and synchronous route, one history action, source/sibling precision, undo; captured zoom/view drift and group refusal. No visual/producer oracle."]
fn hierarchy_canvas_keeps_the_pressed_nested_native_target() {
    for depth in 0..5 {
        let mut a = app(depth);
        let before = a.session.active().unwrap().doc.clone();
        let raw = f::payload(&before);
        let leaf = f::target_leaf(&before);
        let source = a.session.active().unwrap().source.clone();
        let rev = a.session.active().unwrap().revision;
        let past = a.session.active().unwrap().history.past_len();
        tool_event(&mut a, ToolEvent::Down { x: 253.0, y: 254.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert!(a.drag.as_ref().unwrap().native_stroke_target.is_some());
        assert!(a.live_stroke.is_none());
        tool_event(&mut a, ToolEvent::Move { x: 255.0, y: 256.0, pressure: 0.8 }, egui::Modifiers::NONE);
        assert!(before.shares_state(&a.session.active().unwrap().doc));
        tool_event(&mut a, ToolEvent::Up { x: 256.0, y: 257.0 }, egui::Modifiers::NONE);
        assert!(!a.ui.status_error, "{}", a.ui.status);
        let after = a.session.active().unwrap().doc.clone();
        assert_eq!(a.session.active().unwrap().revision, rev + 1);
        assert_eq!(a.session.active().unwrap().history.past_len(), past + 1);
        assert!(after.accounted().unwrap().is_readonly_normal_hierarchy_storage());
        assert_eq!(after.depth, f::sample(depth));
        let now = f::payload(&after);
        for (id, bytes) in &raw {
            let other = &now.iter().find(|(v, _)| v == id).unwrap().1;
            if *id == leaf { assert_ne!(bytes, other) } else { assert_eq!(bytes, other) }
        }
        assert_eq!(a.session.active().unwrap().source, source);
        f::source_payload(&before, &after);
        a.session.undo_with_control(&f::ctl()).unwrap();
        assert_eq!(f::payload(&a.session.active().unwrap().doc), raw);
    }
    for variant in 0..2 {
        let mut a = app(4);
        let before = a.session.active().unwrap().doc.clone();
        tool_event(&mut a, ToolEvent::Down { x: 2.0, y: 2.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let d = a.drag.take().unwrap();
        if variant == 0 {
            let i = a.session.active_index().unwrap();
            a.ui.views.resize_with(i + 1, Default::default);
            a.ui.views[i].zoom = 2.0;
        } else {
            a.session.active_mut().unwrap().view_revision += 1;
        }
        assert!(native_stroke::commit(&mut a, &d));
        assert!(a.ui.status_error);
        assert!(before.shares_state(&a.session.active().unwrap().doc));
    }
    let mut a = app(0);
    let group = f::ids(&a.session.active().unwrap().doc).0;
    crate::jobs_ui::run(&mut a, "layer.select", serde_json::json!({"layer":group.0})).unwrap();
    let before = a.session.active().unwrap().doc.clone();
    assert!(native_stroke::capture(&a, Tool::Brush).is_err());
    assert!(before.shares_state(&a.session.active().unwrap().doc));
    assert!(a.live_stroke.is_none());
}
