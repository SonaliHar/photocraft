use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

/// An app with a three-layer document that has a frame animation (one frame per layer).
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
        s.execute("animation.makeFramesFromLayers", json!({"seconds": 0.2})).unwrap();
        PhotocraftApp::new(s, Default::default())
    });
    h.state_mut().ui.timeline.open = true;
    h.run_steps(6);
    h
}

#[test]
fn the_timeline_docks_along_the_bottom_by_default() {
    let h = harness();
    assert!(!h.state().ui.timeline.floating);
    // Docked, the header offers to float it; the strip shows each frame's delay.
    assert!(h.query_by_label("Float Panel").is_some());
    assert!(h.query_by_label("Dock Panel").is_none());
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3);
}

#[test]
fn the_dock_button_floats_the_panel_and_back() {
    let mut h = harness();
    h.get_by_label("Float Panel").click();
    h.run_steps(6);
    assert!(h.state().ui.timeline.floating);
    assert!(h.query_by_label("Dock Panel").is_some(), "a floating panel offers to dock");
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3, "the strip is the same floating");
    h.get_by_label("Dock Panel").click();
    h.run_steps(6);
    assert!(!h.state().ui.timeline.floating);
}

#[test]
fn clicking_a_frame_selects_it_when_docked() {
    let mut h = harness();
    h.state_mut().run("animation.selectFrame", json!({"frame": 0})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.frame_animation.as_ref().unwrap().current, 0);
    h.get_by_label("Next frame").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.frame_animation.as_ref().unwrap().current, 1);
}

#[test]
fn the_window_menu_hides_and_shows_the_docked_timeline() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(4);
    assert!(!h.state().ui.timeline.open);
    assert!(h.query_by_label("Float Panel").is_none());
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(6);
    assert!(h.query_by_label("Float Panel").is_some(), "it comes back docked");
}
