//! Deferred actual canvas tool-event routing, not visual acceptance.
use super::*;
#[path = "../../engine/src/accounted_native_stroke_test_support.rs"]
mod fixture;
fn app() -> PhotocraftApp {
    let input = fixture::import(false, photocraft_color::SampleType::U8, false);
    let mut s = photocraft_engine::Session::new();
    s.add_owned_plain_xcf_import(&input, "native-stroke.xcf", Some("authorized/native-stroke.xcf"), &fixture::ctl()).unwrap();
    s.tools.brush.pressure_size = false;
    s.tools.brush.size = 7.0;
    let mut app = PhotocraftApp::new(s, Default::default());
    app.ui.tool = Tool::Brush;
    app
}
#[test]
#[ignore = "UNRUN/uncompiled actual native canvas Down/Move/Up uses captured target without resident preview, one COW/history, stale zoom/options/tool/source refusal and original gesture admission. No offscreen/visual acceptance."]
fn native_canvas_commits_only_the_down_target() {
    let mut a = app();
    let before = a.session.active().unwrap().doc.clone();
    let raw = fixture::payload(&before);
    let revision = a.session.active().unwrap().revision;
    tool_event(&mut a, ToolEvent::Down { x: 253.0, y: 254.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(a.drag.as_ref().unwrap().native_stroke_target.is_some());
    assert!(a.live_stroke.is_none());
    tool_event(&mut a, ToolEvent::Move { x: 255.0, y: 256.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(a.session.active().unwrap().doc.shares_state(&before));
    tool_event(&mut a, ToolEvent::Up { x: 256.0, y: 257.0 }, egui::Modifiers::NONE);
    assert!(!a.ui.status_error, "{}", a.ui.status);
    assert_eq!(a.session.active().unwrap().revision, revision + 1);
    assert_ne!(fixture::payload(&a.session.active().unwrap().doc), raw);
    for variant in 0..5 {
        let mut a = app();
        let before = a.session.active().unwrap().doc.clone();
        tool_event(&mut a, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let d = a.drag.take().unwrap();
        match variant {
            0 => a.ui.tool = Tool::Pencil,
            1 => a.ui.tool_options.pencil_auto_erase = !a.ui.tool_options.pencil_auto_erase,
            2 => a.session.tools.foreground[0] = 0.03125,
            3 => {
                let index = a.session.active_index().unwrap();
                a.ui.views.resize_with(index + 1, Default::default);
                a.ui.views[index].zoom = 2.0;
            }
            _ => a.session.active_mut().unwrap().source_read_only = false,
        };
        assert!(native_stroke::commit(&mut a, &d));
        assert!(a.ui.status_error);
        assert!(before.shares_state(&a.session.active().unwrap().doc));
    }
}
