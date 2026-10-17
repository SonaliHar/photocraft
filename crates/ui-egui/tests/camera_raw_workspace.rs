//! The screenshot-driven workspace edits real parameters, with view-only navigation and atomic automation.
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
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
fn parametric_curve_split_drag_edits_only_the_boundary_and_cannot_cross_its_neighbor() {
    let mut h = fixture();
    h.set_size(vec2(1200.0, 1400.0));
    invoke(&mut h, json!({"workspace":{"panels":{"open":["Curve"],"single":true},"curveChannel":"parametric"}})).unwrap();
    h.run_steps(12);
    let before = inspect(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    let middle = h.get_by_label("Darks / Lights").rect().center();
    let left = h.get_by_label("Shadows / Darks").rect().center();
    let width = (middle.x - left.x) * 4.0;
    pointer(&mut h, middle, true);
    h.event(Event::PointerMoved(middle + vec2(width * 0.1, 0.0)));
    h.run_steps(2);
    pointer(&mut h, middle + vec2(width * 0.1, 0.0), false);
    let edited = inspect(&h);
    assert!((edited["params"]["curveSplits"][1].as_f64().unwrap() - 60.0).abs() < 1.0);
    let mut expected = before["params"].clone();
    expected["curveSplits"] = edited["params"]["curveSplits"].clone();
    assert_eq!(edited["params"], expected);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    let middle = h.get_by_label("Darks / Lights").rect().center();
    pointer(&mut h, middle, true);
    h.event(Event::PointerMoved(middle + vec2(width, 0.0)));
    h.run_steps(2);
    pointer(&mut h, middle + vec2(width, 0.0), false);
    let bounded = inspect(&h);
    assert_eq!(bounded["params"]["curveSplits"], json!([25.0, 74.0, 75.0]));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn red_eye_panel_is_reachable_and_unavailable_processing_preserves_the_photo() {
    let mut h = fixture();
    let before = inspect(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    invoke(&mut h, json!({"workspace":{"tab":"redEye"}})).unwrap();
    h.run_steps(4);
    let after = inspect(&h);
    assert_eq!(after["workspace"]["tab"], "redEye");
    assert_eq!(after["params"], before["params"]);
    assert_eq!(after["previewRevision"], before["previewRevision"]);
    for label in ["Type", "Auto", "Pupil Size", "Darken", "Overlay"] {
        let control =
            h.query_all_by_label(label).find(|node| node.accesskit_node().is_disabled()).unwrap_or_else(|| panic!("missing unavailable control {label}"));
        assert!(control.accesskit_node().is_disabled(), "{label} must not promise an unimplemented Camera Raw edit");
    }
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    invoke(&mut h, json!({"workspace":{"tab":"edit"}})).unwrap();
    h.run_steps(3);
    assert_eq!(inspect(&h)["params"], before["params"]);
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
fn bypass_changes_only_the_preview_and_preserves_recipe_and_document_history() {
    let mut h = fixture();
    invoke(&mut h, json!({"set":{"exposure":1.0,"saturation":-20.0}})).unwrap();
    h.run_steps(5);
    let adjusted = inspect(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    invoke(&mut h, json!({"workspace":{"bypassSection":"Light"}})).unwrap();
    h.run_steps(5);
    let bypassed = inspect(&h);
    assert_eq!(bypassed["params"], adjusted["params"]);
    assert_eq!(bypassed["previewBypass"], "Light");
    assert_ne!(bypassed["histogram"]["red"], adjusted["histogram"]["red"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    invoke(&mut h, json!({"workspace":{"bypassSection":null}})).unwrap();
    h.run_steps(5);
    let restored = inspect(&h);
    assert_eq!(restored["histogram"], adjusted["histogram"]);
    assert_eq!(restored["params"], adjusted["params"]);
}

#[test]
fn all_ten_sections_are_accessible_and_unavailable_optics_controls_do_not_edit() {
    let mut h = fixture();
    invoke(&mut h, json!({"workspace":{"panels":{"open":[]}}})).unwrap();
    h.run_steps(12);
    for name in ["Light", "Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail", "Optics", "Lens Blur", "Calibration"] {
        assert!(h.query_all_by_label(name).next().is_some(), "missing native section {name}");
    }
    invoke(&mut h, json!({"workspace":{"panels":{"open":["Optics"]}}})).unwrap();
    h.run_steps(12);
    let before = inspect(&h)["params"].clone();
    let checkbox = h.get_by_label("Use profile corrections").rect();
    pointer(&mut h, checkbox.center(), true);
    pointer(&mut h, checkbox.center(), false);
    assert_eq!(inspect(&h)["params"], before);
}

#[test]
fn detail_enhancements_are_explicitly_unavailable_and_manual_noise_retains_its_recipe() {
    let mut h = fixture();
    invoke(&mut h, json!({"set":{"noiseLuminance":27.0},"workspace":{"panels":{"open":["Detail"]}}})).unwrap();
    h.run_steps(12);
    let initial = inspect(&h);
    for label in ["Denoise", "Raw Details", "Super Resolution"] {
        let rect = h.get_by_label(label).rect();
        pointer(&mut h, rect.center(), true);
        pointer(&mut h, rect.center(), false);
        assert_eq!(inspect(&h)["params"], initial["params"]);
    }
    assert!(h.query_by_label("Luminance").is_none());
    h.get_by_label("Manual Noise Reduction").click();
    h.run_steps(8);
    assert_eq!(inspect(&h)["workspace"]["noiseDetails"], true);
    assert!(h.query_by_label("Luminance").is_some());
    assert_eq!(inspect(&h)["params"]["noiseLuminance"], 27.0);
    h.get_by_label("Manual Noise Reduction").click();
    h.run_steps(8);
    assert!(h.query_by_label("Luminance").is_none());
    assert_eq!(inspect(&h)["params"], initial["params"]);
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
    let digits = h
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "0.00" => Some(text),
            _ => None,
        })
        .expect("inactive numeric value was painted");
    // Allow the button's one-point text inset and pixel snapping, not centered spare space.
    assert!(
        (digits.pos.x - field.left()).abs() <= 1.5,
        "inactive digits were centered despite requesting left alignment: text x={}, field={field:?}",
        digits.pos.x
    );
    h.event(Event::PointerMoved(field.right_center() - vec2(1.0, 0.0)));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: field.right_center() - vec2(1.0, 0.0), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    // egui exposes DragValue as a SpinButton in both display and text-edit modes.
    assert!(h.ctx.memory(|m| m.focused()).is_some());
    h.event(Event::Text("-1.25".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(*h.state(), -1.25);
}
