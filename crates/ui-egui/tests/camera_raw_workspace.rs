//! The screenshot-driven workspace edits real parameters, with view-only navigation and atomic automation.
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 900.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#80a0c0"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    h
}
fn invoke(h: &mut Harness<'_, PhotocraftApp>, value: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui":value})).unwrap()
}
fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}
fn pointer(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn workspace_navigation_is_view_only_and_mixed_invalid_requests_are_atomic() {
    let mut h = fixture();
    let initial = inspect(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    invoke(&mut h, json!({"workspace":{"grid":true,"info":true,"grading":"global","mixer":"color","band":5}})).unwrap();
    h.run_steps(3);
    let current = inspect(&h);
    assert_eq!(current["params"], initial["params"]);
    assert_eq!(current["previewRevision"], initial["previewRevision"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert!(h.query_by_label("File Name").is_some());
    for bad in [
        json!({"set":{"exposure":1},"workspace":{"band":8}}),
        json!({"set":{"exposure":1},"workspace":{"panels":{"open":["Light","Color"],"single":true}}}),
        json!({"workspace":{"gridSize":0}}),
        json!({"reset":"default","set":{"exposure":1}}),
    ] {
        assert!(invoke(&mut h, bad).is_err());
        assert_eq!(inspect(&h), current);
    }
}

#[test]
fn grading_wheel_drag_changes_its_own_tone_and_undo_keeps_the_document_untouched() {
    let mut h = fixture();
    let history = h.state().session.active().unwrap().history.past_len();
    invoke(&mut h, json!({"workspace":{"panels":{"open":["Color Grading"]}}})).unwrap();
    h.run_steps(12);
    let wheel = h.get_by_label("Midtones Color Wheel").rect();
    let center = wheel.center();
    pointer(&mut h, center, true);
    h.event(Event::PointerMoved(center + vec2(wheel.width() * 0.3, 0.0)));
    h.run_steps(3);
    pointer(&mut h, center + vec2(wheel.width() * 0.3, 0.0), false);
    let edited = inspect(&h);
    assert!(edited["params"]["gradeMidtones"]["sat"].as_f64().unwrap() > 50.0);
    assert_eq!(edited["params"]["gradeShadows"]["sat"], 0.0);
    assert_eq!(edited["params"]["gradeHighlights"]["sat"], 0.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    invoke(&mut h, json!({"undo":true})).unwrap();
    assert_eq!(inspect(&h)["params"]["gradeMidtones"]["sat"], 0.0);
    invoke(&mut h, json!({"redo":true})).unwrap();
    assert_eq!(inspect(&h)["params"], edited["params"]);
}

#[test]
fn reset_defaults_and_open_are_reversible_and_radius_double_click_uses_one_not_zero() {
    let mut h = fixture();
    invoke(&mut h, json!({"set":{"sharpenRadius":2.8,"grainSize":70,"gradeBlending":90},"workspace":{"panels":{"open":["Detail"]},"sharpeningDetails":true}}))
        .unwrap();
    h.run_steps(12);
    let label = h.get_by_label("Radius").rect();
    let track = pos2(label.left() + 90.0, label.bottom() + 11.0);
    for _ in 0..2 {
        pointer(&mut h, track, true);
        pointer(&mut h, track, false);
    }
    assert_eq!(inspect(&h)["params"]["sharpenRadius"], 1.0);
    invoke(&mut h, json!({"reset":"default"})).unwrap();
    let defaults = inspect(&h)["params"].clone();
    assert_eq!(defaults["grainSize"], 25.0);
    assert_eq!(defaults["gradeBlending"], 50.0);
    invoke(&mut h, json!({"undo":true})).unwrap();
    assert_eq!(inspect(&h)["params"]["grainSize"], 70.0);
    invoke(&mut h, json!({"reset":"open"})).unwrap();
    assert_eq!(inspect(&h)["params"], defaults);
}

#[test]
fn red_curve_edits_do_not_change_master_green_or_blue_curves() {
    let mut h = fixture();
    invoke(&mut h, json!({"workspace":{"panels":{"open":["Curve"]},"curveChannel":"red"}})).unwrap();
    h.run_steps(12);
    let curve = inspect(&h)["curveRect"].clone();
    let at =
        pos2((curve[0].as_f64().unwrap() + curve[2].as_f64().unwrap()) as f32 * 0.5, (curve[1].as_f64().unwrap() + curve[3].as_f64().unwrap()) as f32 * 0.5);
    pointer(&mut h, at, true);
    pointer(&mut h, at, false);
    let params = inspect(&h)["params"].clone();
    assert_eq!(params["pointCurveRed"].as_array().unwrap().len(), 3);
    for name in ["pointCurve", "pointCurveGreen", "pointCurveBlue"] {
        assert!(params[name].as_array().unwrap().is_empty());
    }
}

#[test]
fn left_aligned_camera_raw_numbers_keep_a_full_width_edit_target_and_accept_typed_values() {
    use egui::accesskit::Role;
    use photocraft_ui_egui::{camera_raw_design::Design, widgets};
    let design = Design::parse(include_str!("../../../design/camera-raw/layout.json")).unwrap();
    let style = design.slider_style();
    let width = design.value_field_width;
    let mut h = Harness::new_ui_state(
        move |ui, value: &mut f32| {
            widgets::slider_row_prec_styled(
                ui,
                "Exposure",
                value,
                -5.0..=5.0,
                "",
                None,
                widgets::RowGestures { reset: Some(0.0), wheel_step: None },
                Some(2),
                width,
                Some(style),
            );
        },
        0.0,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run();
    let field = h.get_by_role(Role::SpinButton).rect();
    assert!(field.width() >= width - 2.0 * style.field_appearance.text_padding - 1.0);
    h.event(Event::PointerMoved(field.right_center() - vec2(1.0, 0.0)));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: field.right_center() - vec2(1.0, 0.0), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.query_by_role(Role::TextInput).is_some());
    h.event(Event::Text("-1.25".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(*h.state(), -1.25);
}
