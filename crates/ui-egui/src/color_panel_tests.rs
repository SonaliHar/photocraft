//! The Color panel's modes (#2569, UI-217-17): sliders in each colour model edit the chip the panel
//! targets, the wheel's square and ring set saturation/brightness and hue, the ramp picks
//! colours, and the panel menu switches modes, ramps and copies the colour.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::PhotocraftApp;
use crate::state::{ColorPanelMode, ColorRamp};

fn new_app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

type Draw = fn(&mut egui::Ui, &mut PhotocraftApp);

fn harness(app: PhotocraftApp, draw: Draw) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(300.0, 520.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                draw(ui, app);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(2);
    h
}

fn panel(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    crate::color_panel::panel(app, ui, false);
}

/// Click the `field`-th numeric field, type `text` one key per frame, and leave it with Tab.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, field: usize, text: &str) {
    h.query_all_by_role(Role::SpinButton).nth(field).expect("numeric field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

fn bytes(c: [f32; 4]) -> [u8; 3] {
    crate::panels::srgb_bytes(c)
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= tol)
}

#[test]
fn rgb_sliders_set_the_foreground() {
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Rgb;
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    assert_eq!(h.query_all_by_role(Role::SpinButton).count(), 3, "R, G and B");
    type_into(&mut h, 0, "255");
    type_into(&mut h, 2, "51");
    assert_eq!(bytes(h.state().session.tools.foreground), [255, 0, 51]);
    assert_eq!(bytes(h.state().session.tools.background), [255, 255, 255], "the background chip is untouched");
}

#[test]
fn cmyk_sliders_lay_ink_on_the_background_chip() {
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Cmyk;
    app.ui.color_panel.background = true;
    app.session.tools.background = [1.0, 1.0, 1.0, 1.0];
    let fg = app.session.tools.foreground;
    let mut h = harness(app, panel);
    assert_eq!(h.query_all_by_role(Role::SpinButton).count(), 4, "C, M, Y and K");
    type_into(&mut h, 0, "100");
    let [r, g, b] = bytes(h.state().session.tools.background);
    assert!(b > r + 60 && g > r + 40, "100% cyan ink through the CMYK profile: {r} {g} {b}");
    assert_eq!(h.state().session.tools.foreground, fg, "the foreground is untouched");
    type_into(&mut h, 3, "100");
    let c = bytes(h.state().session.tools.background);
    assert!(c.iter().all(|v| *v < 60), "with full black ink: {c:?}");
}

#[test]
fn lab_grayscale_and_hsb_sliders() {
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Lab;
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    type_into(&mut h, 0, "100");
    type_into(&mut h, 1, "0");
    type_into(&mut h, 2, "0");
    assert!(near(bytes(h.state().session.tools.foreground), [255, 255, 255], 1), "L 100 a 0 b 0 is white");

    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Grayscale;
    let mut h = harness(app, panel);
    assert_eq!(h.query_all_by_role(Role::SpinButton).count(), 1, "one K slider");
    type_into(&mut h, 0, "100");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 0, 0], "K 100 is black");
    type_into(&mut h, 0, "0");
    assert_eq!(bytes(h.state().session.tools.foreground), [255, 255, 255], "K 0 is white");

    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Hsb;
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    // Black has no hue or saturation of its own: the typed ones are kept until brightness arrives.
    type_into(&mut h, 0, "120");
    type_into(&mut h, 1, "100");
    type_into(&mut h, 2, "100");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 255, 0]);
}

#[test]
fn web_sliders_take_two_digit_hex() {
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Web;
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    let fields: Vec<_> = h.query_all_by_role(Role::TextInput).collect();
    assert_eq!(fields.len(), 3, "R, G and B hex fields");
    h.query_all_by_role(Role::TextInput).next().unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in "cc".chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(bytes(h.state().session.tools.foreground), [0xcc, 0, 0]);
    // A single digit or a non-hex pair changes nothing.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text("zz".into()));
    h.run_steps(2);
    assert_eq!(bytes(h.state().session.tools.foreground), [0xcc, 0, 0]);
}

#[test]
fn wheel_square_and_ring_set_the_colour() {
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Wheel;
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    let (center, outer, inner, square): (egui::Pos2, f32, f32, egui::Rect) =
        h.ctx.data(|d| d.get_temp(crate::color_panel::wheel_geometry_id())).expect("the wheel was drawn");
    // Without a dock the wheel is 120 pt: a 61 pt square inside a 94 pt ring.
    assert!((outer - 60.0).abs() < 0.01 && inner > 40.0 && square.width() < 70.0 && square.center() == center);
    // The middle of the square: half saturation, half brightness of the red hue.
    h.event(egui::Event::PointerMoved(center));
    h.event(egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let got = bytes(h.state().session.tools.foreground);
    assert!(near(got, [128, 64, 64], 6), "s 50% b 50% of red: {got:?}");
    // On the ring at the right: a quarter turn clockwise from red at the top is hue 90°.
    let on_ring = egui::pos2(center.x + (inner + outer) / 2.0, center.y);
    h.event(egui::Event::PointerMoved(on_ring));
    h.event(egui::Event::PointerButton { pos: on_ring, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: on_ring, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let got = bytes(h.state().session.tools.foreground);
    let hsv = crate::color_picker_ui::rgb_to_hsv([f32::from(got[0]) / 255.0, f32::from(got[1]) / 255.0, f32::from(got[2]) / 255.0]);
    assert!((hsv[0] - 90.0).abs() < 4.0 && (hsv[1] - 0.5).abs() < 0.05 && (hsv[2] - 0.5).abs() < 0.05, "{got:?} = {hsv:?}");
}

#[test]
fn ramp_click_picks_the_foreground_and_alt_click_the_background() {
    let mut app = new_app();
    app.ui.color_panel.ramp = ColorRamp::Grayscale;
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    app.session.tools.background = [0.0, 0.0, 1.0, 1.0];
    let mut h = harness(app, |ui, app| crate::color_panel::ramp(app, ui));
    let w = 300.0; // the harness width
    let mid = egui::pos2(w / 2.0, 11.0);
    h.event(egui::Event::PointerMoved(mid));
    h.event(egui::Event::PointerButton { pos: mid, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: mid, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let fg = bytes(h.state().session.tools.foreground);
    assert!(fg[0] == fg[1] && fg[1] == fg[2] && (100..=156).contains(&fg[0]), "the middle of the grayscale ramp: {fg:?}");
    assert_eq!(bytes(h.state().session.tools.background), [0, 0, 255]);
    let right = egui::pos2(w * 0.9, 11.0);
    let alt = egui::Modifiers::ALT;
    h.event(egui::Event::PointerMoved(right));
    h.event(egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed: true, modifiers: alt });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: right, button: egui::PointerButton::Primary, pressed: false, modifiers: alt });
    h.run_steps(2);
    let bg = bytes(h.state().session.tools.background);
    let fg_now = bytes(h.state().session.tools.foreground);
    assert!(
        bg[0] == bg[1] && bg[1] == bg[2] && (200..=245).contains(&bg[0]),
        "Alt-click near the white end sets the background: {bg:?} (foreground {fg_now:?})"
    );
    assert_eq!(bytes(h.state().session.tools.foreground), fg, "and leaves the foreground");
}

/// Each menu row is one click in a fresh menu, as a popup closes after it.
fn menu_click(label: &str, set_up: fn(&mut PhotocraftApp)) -> PhotocraftApp {
    let mut app = new_app();
    set_up(&mut app);
    let mut h = harness(app, |ui, app| crate::color_panel::panel_menu(app, ui));
    h.get_by_label(label).click();
    h.run_steps(2);
    h.into_state()
}

#[test]
fn panel_menu_switches_modes_and_ramps() {
    for (label, mode) in [
        ("Lab Sliders", ColorPanelMode::Lab),
        ("Color Wheel", ColorPanelMode::Wheel),
        ("Brightness Cube", ColorPanelMode::BrightnessCube),
        ("Web Color Sliders", ColorPanelMode::Web),
        ("Grayscale Slider", ColorPanelMode::Grayscale),
    ] {
        assert_eq!(menu_click(label, |_| {}).ui.color_panel.mode, mode, "{label}");
    }
    assert_eq!(menu_click("CMYK Spectrum", |_| {}).ui.color_panel.ramp, ColorRamp::CmykSpectrum);
    assert_eq!(menu_click("Current Colors", |_| {}).ui.color_panel.ramp, ColorRamp::CurrentColors);
    // The copies are of the targeted chip.
    let mut app = new_app();
    app.session.tools.foreground = [1.0, 0.0, 0.2, 1.0];
    app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
    assert_eq!(crate::color_panel::hex_of(&app), "ff0033");
    assert_eq!(crate::color_panel::html_of(&app), "color=\"#ff0033\"");
    app.ui.color_panel.background = true;
    assert_eq!(crate::color_panel::hex_of(&app), "000000");
    let app = menu_click("Copy Color's Hex Code", |a| a.session.tools.foreground = [1.0, 0.0, 0.2, 1.0]);
    assert_eq!(bytes(app.session.tools.foreground), [255, 0, 51], "copying changes nothing");
    menu_click("Copy Color as HTML", |_| {});
}

/// Photoshop's ⚠ and cube under the chips: a click takes the nearest printable or web colour.
#[test]
fn gamut_and_web_warnings_snap_to_the_nearest_colour() {
    const GAMUT: &str = "Out of gamut for printing: click for the nearest printable colour";
    const WEB: &str = "Not a web colour: click for the nearest web colour";
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::Rgb;
    app.session.tools.foreground = [0xfd as f32 / 255.0, 3.0 / 255.0, 3.0 / 255.0, 1.0];
    let mut h = harness(app, panel);
    assert!(h.query_by_label(GAMUT).is_some() && h.query_by_label(WEB).is_some(), "a vivid red is neither printable nor a web colour");
    h.get_by_label(GAMUT).click();
    h.run_steps(2);
    let c = h.state().session.tools.foreground;
    assert!(crate::color_picker_ui::out_of_gamut([c[0], c[1], c[2]]).is_none(), "snapped to a printable colour: {c:?}");
    assert!(h.query_by_label(GAMUT).is_none(), "and the warning is gone");
    h.get_by_label(WEB).click();
    h.run_steps(2);
    let c = bytes(h.state().session.tools.foreground);
    assert!(c.iter().all(|v| v % 0x33 == 0), "snapped to a web colour: {c:?}");
    assert!(h.query_by_label(WEB).is_none() && h.query_by_label(GAMUT).is_none(), "{c:?}");
    // The warnings follow the targeted chip and show in the field modes too.
    let mut app = new_app();
    app.ui.color_panel.mode = ColorPanelMode::HueCube;
    app.ui.color_panel.background = true;
    app.session.tools.background = [0.0, 1.0, 0.0, 1.0];
    let mut h = harness(app, panel);
    assert!(h.query_by_label(GAMUT).is_some(), "pure green doesn't print");
    h.get_by_label(GAMUT).click();
    h.run_steps(2);
    let b = h.state().session.tools.background;
    assert!(crate::color_picker_ui::out_of_gamut([b[0], b[1], b[2]]).is_none() && b[1] > 0.5, "{b:?}");
    assert_eq!(bytes(h.state().session.tools.foreground), [0, 0, 0], "the foreground is untouched");
}

#[test]
fn every_mode_draws_at_dock_widths_without_panicking() {
    for mode in [
        ColorPanelMode::HueCube,
        ColorPanelMode::BrightnessCube,
        ColorPanelMode::Wheel,
        ColorPanelMode::Grayscale,
        ColorPanelMode::Rgb,
        ColorPanelMode::Hsb,
        ColorPanelMode::Cmyk,
        ColorPanelMode::Lab,
        ColorPanelMode::Web,
    ] {
        for width in [120.0, 190.0, 420.0] {
            let mut app = new_app();
            app.ui.color_panel.mode = mode;
            let mut h = Harness::builder().with_size(vec2(width, 300.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    crate::color_panel::panel(app, ui, true);
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
            h.run_steps(3);
        }
    }
}
