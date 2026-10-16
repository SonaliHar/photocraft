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

fn anim(h: &Harness<'static, PhotocraftApp>) -> photocraft_doc::FrameAnimation {
    h.state().session.active().unwrap().doc.frame_animation.clone().unwrap()
}

/// Puts the pointer over the panel, as the timeline's keys need.
fn hover_panel(h: &mut Harness<'static, PhotocraftApp>) -> egui::Rect {
    let r = h.state().ui.timeline.panel_rect.expect("the panel was drawn");
    h.event(egui::Event::PointerMoved(r.center()));
    h.run_steps(2);
    r
}

#[test]
fn space_plays_and_pauses_while_the_pointer_is_over_the_timeline() {
    let mut h = harness();
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert!(!h.state().ui.timeline.playing, "Space elsewhere is not the timeline's");
    hover_panel(&mut h);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert!(h.state().ui.timeline.playing);
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    assert!(!h.state().ui.timeline.playing);
}

#[test]
fn arrows_home_and_end_step_through_the_frames() {
    let mut h = harness();
    hover_panel(&mut h);
    h.state_mut().run("animation.selectFrame", json!({"frame": 1})).unwrap();
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(anim(&h).current, 2);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(anim(&h).current, 2, "stops at the last frame");
    h.key_press(egui::Key::Home);
    h.run_steps(2);
    assert_eq!(anim(&h).current, 0);
    h.key_press(egui::Key::End);
    h.run_steps(2);
    assert_eq!(anim(&h).current, 2);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(anim(&h).current, 1);
}

#[test]
fn copy_and_paste_duplicate_a_frame_with_its_delay_and_layers() {
    let mut h = harness();
    hover_panel(&mut h);
    h.state_mut().run("animation.selectFrame", json!({"frame": 1})).unwrap();
    h.state_mut().run("animation.setDelay", json!({"seconds": 1.5})).unwrap();
    let copied = anim(&h).frames[1].clone();
    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert_eq!(h.state().ui.timeline.clipboard, std::slice::from_ref(&copied));
    h.state_mut().run("animation.selectFrame", json!({"frame": 2})).unwrap();
    h.event(egui::Event::Paste(String::new()));
    h.run_steps(3);
    let a = anim(&h);
    assert_eq!(a.frames.len(), 4);
    assert_eq!(a.frames[3], copied, "pasted after the selected frame");
    assert_eq!(a.current, 3);
}

#[test]
fn alt_arrows_move_the_selected_frame() {
    let mut h = harness();
    hover_panel(&mut h);
    h.state_mut().run("animation.selectFrame", json!({"frame": 0})).unwrap();
    h.state_mut().run("animation.setDelay", json!({"seconds": 3})).unwrap();
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run_steps(2);
    let a = anim(&h);
    assert_eq!((a.current, a.frames[1].delay_ms), (1, 3000));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(anim(&h).frames[0].delay_ms, 3000);
}

#[test]
fn dragging_a_frame_reorders_it() {
    let mut h = harness();
    let r = hover_panel(&mut h);
    h.state_mut().run("animation.setDelay", json!({"seconds": 3, "frames": [0]})).unwrap();
    h.run_steps(2);
    let y = r.top() + 62.0;
    let cell = |i: usize| egui::pos2(r.left() + 42.0 + i as f32 * 90.0, y);
    h.event(egui::Event::PointerMoved(cell(0)));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: cell(0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for step in 1..=8 {
        let x = cell(0).x + (cell(2).x + 30.0 - cell(0).x) * step as f32 / 8.0;
        h.event(egui::Event::PointerMoved(egui::pos2(x, y)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(cell(2).x + 30.0, y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_steps(3);
    let a = anim(&h);
    assert_eq!(a.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [200, 200, 3000], "frame 1 was dropped after the last one");
    assert_eq!(a.current, 2, "the selection travelled with it");
}

/// The delay menu's "Other…" field, which starts at the frame's 0.2 seconds.
fn other_field<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    h.query_all_by_role(egui::accesskit::Role::TextInput).find(|n| n.value().as_deref() == Some("0.2")).expect("the Other… field")
}

#[test]
fn clicking_the_other_field_keeps_the_delay_menu_open_for_typing() {
    let mut h = harness();
    h.query_all_by_label_contains("0.2 sec.").next().unwrap().click();
    h.run_steps(4);
    assert!(h.query_by_label("Other…").is_some(), "the menu opened");
    other_field(&h).click();
    h.run_steps(4);
    assert!(h.query_by_label("Other…").is_some(), "a click in the field leaves the menu open");
    other_field(&h).type_text("1.5");
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(anim(&h).frames[0].delay_ms, 1500);
    assert!(h.query_by_label("Other…").is_none(), "Enter applies it and closes the menu");
}

/// Clicks frame `i` of the strip with `modifiers` held.
fn click_frame(h: &mut Harness<'static, PhotocraftApp>, r: egui::Rect, i: usize, modifiers: egui::Modifiers) {
    let pos = egui::pos2(r.left() + 42.0 + i as f32 * 90.0, r.top() + 62.0);
    h.event_modifiers(egui::Event::PointerMoved(pos), modifiers);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event_modifiers(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers }, modifiers);
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn selected(h: &Harness<'static, PhotocraftApp>) -> Vec<usize> {
    crate::timeline_ui::selection(h.state()).into_iter().collect()
}

fn five_frames() -> Harness<'static, PhotocraftApp> {
    let mut h = harness();
    for (i, s) in [1.0, 2.0, 3.0].iter().enumerate() {
        h.state_mut().run("animation.setDelay", json!({"seconds": s, "frames": [i]})).unwrap();
    }
    h.state_mut().run("animation.newFrame", json!({"frames": [1, 2]})).unwrap();
    h.state_mut().run("animation.selectFrame", json!({"frame": 0})).unwrap();
    h.run_steps(3);
    h
}

#[test]
fn ctrl_and_shift_clicks_select_several_frames() {
    let mut h = five_frames();
    let r = hover_panel(&mut h);
    click_frame(&mut h, r, 1, egui::Modifiers::NONE);
    assert_eq!(selected(&h), [1]);
    click_frame(&mut h, r, 3, egui::Modifiers::COMMAND);
    assert_eq!(selected(&h), [1, 3]);
    assert_eq!(anim(&h).current, 3);
    click_frame(&mut h, r, 3, egui::Modifiers::COMMAND);
    assert_eq!(selected(&h), [1], "a second ⌘-click takes it out");
    click_frame(&mut h, r, 4, egui::Modifiers::SHIFT);
    assert_eq!(selected(&h), [1, 2, 3, 4], "⇧-click runs from the anchor");
}

#[test]
fn commands_cover_every_selected_frame() {
    let mut h = five_frames();
    let r = hover_panel(&mut h);
    click_frame(&mut h, r, 1, egui::Modifiers::NONE);
    click_frame(&mut h, r, 2, egui::Modifiers::SHIFT);
    // Copy both, paste after the last selected one.
    h.event(egui::Event::Copy);
    h.run_steps(2);
    assert_eq!(h.state().ui.timeline.clipboard.len(), 2);
    h.event(egui::Event::Paste(String::new()));
    h.run_steps(3);
    let delays = |h: &Harness<'static, PhotocraftApp>| anim(h).frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>();
    assert_eq!(delays(&h), [1000, 2000, 3000, 2000, 3000, 2000, 3000]);
    assert_eq!(selected(&h), [3, 4], "the pasted frames are selected");
    // Delete the selection.
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(delays(&h), [1000, 2000, 3000, 2000, 3000]);
    // Select all: deleting every frame is refused.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    assert_eq!(selected(&h), [0, 1, 2, 3, 4]);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(anim(&h).frames.len(), 5, "deleting every frame is refused");
}

#[test]
fn dragging_one_of_the_selected_frames_moves_them_all() {
    let mut h = five_frames();
    let r = hover_panel(&mut h);
    click_frame(&mut h, r, 0, egui::Modifiers::NONE);
    click_frame(&mut h, r, 1, egui::Modifiers::SHIFT);
    let y = r.top() + 62.0;
    let cell = |i: usize| egui::pos2(r.left() + 42.0 + i as f32 * 90.0, y);
    h.event(egui::Event::PointerMoved(cell(0)));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: cell(0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for step in 1..=8 {
        let x = cell(0).x + (cell(4).x + 30.0 - cell(0).x) * step as f32 / 8.0;
        h.event(egui::Event::PointerMoved(egui::pos2(x, y)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(cell(4).x + 30.0, y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_steps(3);
    let delays: Vec<u32> = anim(&h).frames.iter().map(|f| f.delay_ms).collect();
    assert_eq!(delays, [3000, 2000, 3000, 1000, 2000]);
    assert_eq!(selected(&h), [3, 4]);
}
