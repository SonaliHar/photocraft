use super::*;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}
fn harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    app.run("window.panel.dock", json!({"panel":"navigator","side":"left"})).unwrap();
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 900.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            egui::Panel::top("test-options").exact_size(50.0).show(ui, |_| {});
            begin_frame(ui);
            crate::panels::right_dock(app, ui);
            if app.ui.panels.toolbar && !was_rendered(ui.ctx()) {
                crate::panels::toolbar(app, ui);
            }
            let canvas = ui.available_rect_before_wrap();
            ui.ctx().data_mut(|data| data.insert_temp(key("test-canvas"), canvas));
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(5);
    h
}
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
}

#[test]
fn toolbar_stays_between_modular_column_and_canvas_and_protects_it_from_floats() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"info","x":280,"y":120,"width":280,"height":300})).unwrap();
    let mut h = harness(app);
    let bar = docked_rect(&h.ctx).unwrap();
    let left = egui::containers::panel::PanelState::load(&h.ctx, Id::new("shared-panel-dock-left")).unwrap().outer_rect;
    let canvas = h.ctx.data(|data| data.get_temp::<Rect>(key("test-canvas"))).unwrap();
    assert!(left.right() <= bar.left() + 1.0, "{left:?} / {bar:?}");
    assert!((bar.right() - canvas.left()).abs() <= 1.0, "{bar:?} / {canvas:?}");
    let floating_tab = h.ctx.read_response(Id::new("photocraft-panel-docking").with(("tab", &"info".to_owned()))).unwrap().rect;
    assert!(!floating_tab.intersects(bar), "{floating_tab:?} / {bar:?}");
    if let Some(path) = std::env::var_os("PHOTOCRAFT_TOOLBAR_CAPTURE") {
        h.render().unwrap().save(path).unwrap();
    }
}

#[test]
fn toolbar_grip_floats_redocks_right_and_keeps_tool_buttons_working() {
    let mut h = harness(app());
    let source = h.get_by_label("Drag Toolbar").rect().center();
    drag(&mut h, source, pos2(650.0, 180.0));
    assert_eq!(h.state().ui.toolbar_layout.placement, Placement::Floating);
    assert!(docked_rect(&h.ctx).is_none());
    let source = h.get_by_label("Drag Toolbar").rect().center();
    drag(&mut h, source, pos2(1270.0, 180.0));
    assert_eq!(h.state().ui.toolbar_layout.placement, Placement::Right);
    let bar = docked_rect(&h.ctx).unwrap();
    let canvas = h.ctx.data(|data| data.get_temp::<Rect>(key("test-canvas"))).unwrap();
    assert!((bar.left() - canvas.right()).abs() <= 1.0, "{bar:?} / {canvas:?}");
    let before = h.state().ui.panels.toolbar_double;
    let at = h.get_by_label("Toolbar").rect().center();
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    assert_ne!(h.state().ui.panels.toolbar_double, before);
}

#[test]
fn toolbar_commands_are_atomic_locked_and_saved_per_workspace() {
    let mut app = app();
    app.run("window.toolbar.layout", json!({"placement":"floating","position":[300,140],"height":500})).unwrap();
    let saved = json!({"toolbarLayout":app.ui.toolbar_layout});
    let mut restored = self::app();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(app.ui.toolbar_layout, restored.ui.toolbar_layout);
    for params in [json!({"placement":"bad"}), json!({"height":-1}), json!({"position":[0]}), json!({"unexpected":true})] {
        let before = app.ui.toolbar_layout.clone();
        assert!(app.run("window.toolbar.layout", params).is_err());
        assert_eq!(app.ui.toolbar_layout, before);
    }
    app.session.prefs.edit(|prefs| prefs.workspace_locked = true);
    assert!(app.run("window.toolbar.layout", json!({"placement":"right"})).is_err());
    assert_eq!(app.ui.toolbar_layout.placement, Placement::Floating);
}
