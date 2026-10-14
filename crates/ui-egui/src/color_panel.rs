//! The Color panel's modes (#2569, UI-217-17). Photoshop's panel menu chooses how the colour is
//! shown and edited: a Hue Cube (a saturation/brightness field beside a hue strip), a Brightness
//! Cube (a hue/saturation field beside a brightness strip), a Color Wheel (a hue ring around a
//! saturation/brightness square), or sliders in Grayscale, RGB, HSB, CMYK, Lab or Web Color
//! form over a colour ramp (the RGB or CMYK spectrum, a grayscale ramp, or the current colours).
//! Every mode edits the chip the panel targets (the foreground, or the background once its chip
//! is clicked: `ColorPanelState::background`). The mode and ramp are `ui.set {colorPanel}` data.
//!
//! Slider values are kept while they still give the colour shown ([`remembered`]): a grey has
//! no hue, CMYK and Lab don't round-trip exactly through sRGB, so recomputing the sliders from
//! the colour after every edit would move what was just typed.

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_color::convert;

use crate::state::{ColorPanelMode, ColorRamp};
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, panels, widgets};

/// The panel body for the current mode. `with_hsb`: the simple themes keep HSB sliders under a
/// cube or wheel (the pro themes show the Photoshop readout there instead).
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, with_hsb: bool) {
    let mode = app.ui.color_panel.mode;
    match mode {
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::Wheel => {
            if mode == ColorPanelMode::Wheel {
                wheel(app, ui);
            } else {
                panels::color_field(app, ui, mode);
            }
            if with_hsb {
                ui.add_space(8.0);
                sliders(app, ui, ColorPanelMode::Hsb);
            }
        }
        _ => {
            sliders(app, ui, mode);
            ui.add_space(6.0);
            ramp(app, ui);
        }
    }
}

/// The Color panel's flyout menu: the slider modes, the hex copies, the field modes, the ramps.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let current = app.ui.color_panel.mode;
    for (mode, label) in [
        (ColorPanelMode::Grayscale, tl!("Grayscale Slider")),
        (ColorPanelMode::Rgb, tl!("RGB Sliders")),
        (ColorPanelMode::Hsb, tl!("HSB Sliders")),
        (ColorPanelMode::Cmyk, tl!("CMYK Sliders")),
        (ColorPanelMode::Lab, tl!("Lab Sliders")),
        (ColorPanelMode::Web, tl!("Web Color Sliders")),
    ] {
        if check_item(ui, current == mode, label) {
            app.ui.color_panel.mode = mode;
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("Copy Color's Hex Code")).clicked() {
        ui.ctx().copy_text(hex_of(app));
        ui.close();
    }
    if ui.button(tl!("Copy Color as HTML")).clicked() {
        ui.ctx().copy_text(html_of(app));
        ui.close();
    }
    ui.separator();
    for (mode, label) in
        [(ColorPanelMode::HueCube, tl!("Hue Cube")), (ColorPanelMode::BrightnessCube, tl!("Brightness Cube")), (ColorPanelMode::Wheel, tl!("Color Wheel"))]
    {
        if check_item(ui, current == mode, label) {
            app.ui.color_panel.mode = mode;
            ui.close();
        }
    }
    ui.separator();
    let ramp_now = app.ui.color_panel.ramp;
    for (ramp, label) in [
        (ColorRamp::RgbSpectrum, tl!("RGB Spectrum")),
        (ColorRamp::CmykSpectrum, tl!("CMYK Spectrum")),
        (ColorRamp::Grayscale, tl!("Grayscale Ramp")),
        (ColorRamp::CurrentColors, tl!("Current Colors")),
    ] {
        if check_item(ui, ramp_now == ramp, label) {
            app.ui.color_panel.ramp = ramp;
            ui.close();
        }
    }
}

/// The targeted chip as six hex digits (Copy Color's Hex Code).
pub fn hex_of(app: &PhotocraftApp) -> String {
    let [r, g, b] = panels::srgb_bytes(active_color(app));
    format!("{r:02x}{g:02x}{b:02x}")
}

/// The targeted chip as an HTML colour attribute (Copy Color as HTML).
pub fn html_of(app: &PhotocraftApp) -> String {
    format!("color=\"#{}\"", hex_of(app))
}

/// A menu row with a check mark when `checked`; true when clicked.
fn check_item(ui: &mut egui::Ui, checked: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (mark, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
        if checked {
            ui.painter().text(mark.center(), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(12.0), t.text);
        }
        ui.button(label).clicked()
    })
    .inner
}

// ----------------------------------------------------------------------------- colour model

/// A slider of a mode: its label, range and unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderSpec {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub suffix: &'static str,
}

const fn spec(label: &'static str, min: f32, max: f32, suffix: &'static str) -> SliderSpec {
    SliderSpec { label, min, max, suffix }
}

const GRAY_SPECS: [SliderSpec; 1] = [spec("K", 0.0, 100.0, "%")];
const RGB_SPECS: [SliderSpec; 3] = [spec("R", 0.0, 255.0, ""), spec("G", 0.0, 255.0, ""), spec("B", 0.0, 255.0, "")];
const HSB_SPECS: [SliderSpec; 3] = [spec("Hue", 0.0, 360.0, "°"), spec("Saturation", 0.0, 100.0, "%"), spec("Brightness", 0.0, 100.0, "%")];
const CMYK_SPECS: [SliderSpec; 4] = [spec("C", 0.0, 100.0, "%"), spec("M", 0.0, 100.0, "%"), spec("Y", 0.0, 100.0, "%"), spec("K", 0.0, 100.0, "%")];
const LAB_SPECS: [SliderSpec; 3] = [spec("L", 0.0, 100.0, ""), spec("a", -128.0, 127.0, ""), spec("b", -128.0, 127.0, "")];

/// The sliders a mode shows, in Photoshop's order and units.
pub fn slider_specs(mode: ColorPanelMode) -> &'static [SliderSpec] {
    match mode {
        ColorPanelMode::Grayscale => &GRAY_SPECS,
        ColorPanelMode::Rgb | ColorPanelMode::Web => &RGB_SPECS,
        ColorPanelMode::Hsb => &HSB_SPECS,
        ColorPanelMode::Cmyk => &CMYK_SPECS,
        ColorPanelMode::Lab => &LAB_SPECS,
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::Wheel => &[],
    }
}

/// The slider values of `rgb` (sRGB-encoded, 0..1) in `mode`, in the panel's units.
pub fn sliders_of(mode: ColorPanelMode, rgb: [f32; 3]) -> Vec<f32> {
    let rgb = rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) });
    match mode {
        // Photoshop's grayscale slider is the amount of black: 0 is white, 100 black.
        ColorPanelMode::Grayscale => vec![(1.0 - convert::rgb_to_gray(rgb)) * 100.0],
        ColorPanelMode::Rgb | ColorPanelMode::Web => rgb.iter().map(|v| v * 255.0).collect(),
        ColorPanelMode::Hsb => {
            let [h, s, v] = crate::color_picker_ui::rgb_to_hsv(rgb);
            vec![h, s * 100.0, v * 100.0]
        }
        ColorPanelMode::Cmyk => convert::rgb_to_cmyk(rgb).iter().map(|v| v * 100.0).collect(),
        ColorPanelMode::Lab => convert::srgb_to_lab(rgb).to_vec(),
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::Wheel => Vec::new(),
    }
}

/// The colour (sRGB-encoded, 0..1) of slider `values` in `mode`. Missing values read as 0; every
/// value is clamped to its slider's range, and a Lab colour outside sRGB is clipped.
pub fn rgb_of(mode: ColorPanelMode, values: &[f32]) -> [f32; 3] {
    let specs = slider_specs(mode);
    let v = |i: usize| {
        let s = specs.get(i).copied().unwrap_or(spec("", 0.0, 0.0, ""));
        let x = values.get(i).copied().unwrap_or(0.0);
        if x.is_nan() { s.min } else { x.clamp(s.min, s.max) }
    };
    let unit = |c: f32| c.clamp(0.0, 1.0);
    match mode {
        ColorPanelMode::Grayscale => {
            let g = 1.0 - v(0) / 100.0;
            [g, g, g]
        }
        ColorPanelMode::Rgb | ColorPanelMode::Web => [v(0) / 255.0, v(1) / 255.0, v(2) / 255.0],
        ColorPanelMode::Hsb => crate::color_picker_ui::hsv_to_rgb(v(0), v(1) / 100.0, v(2) / 100.0),
        ColorPanelMode::Cmyk => convert::cmyk_to_rgb([v(0) / 100.0, v(1) / 100.0, v(2) / 100.0, v(3) / 100.0]).map(unit),
        ColorPanelMode::Lab => convert::lab_to_srgb([v(0), v(1), v(2)]).map(unit),
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::Wheel => [0.0; 3],
    }
}

/// The slider values to show for `rgb`: `stored` (what was last edited) while it still gives
/// `rgb` to within one 8-bit step per channel (CMYK and Lab round trips drift by that much), else
/// the values computed from `rgb`.
pub fn remembered(mode: ColorPanelMode, rgb: [f32; 3], stored: Option<Vec<f32>>) -> Vec<f32> {
    let q = |c: [f32; 3]| panels::srgb_bytes([c[0], c[1], c[2], 1.0]);
    let same = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| (i16::from(*x) - i16::from(y)).abs() <= 1);
    match stored {
        Some(s) if s.len() == slider_specs(mode).len() && same(q(rgb_of(mode, &s)), q(rgb)) => s,
        _ => sliders_of(mode, rgb),
    }
}

/// The colours along slider `i` of `mode` with the other sliders at `values` (Preferences ›
/// Interface › Dynamic Color Sliders).
pub fn track(mode: ColorPanelMode, values: &[f32], i: usize) -> Vec<Color32> {
    let Some(s) = slider_specs(mode).get(i) else { return Vec::new() };
    let n = if mode == ColorPanelMode::Hsb && i == 0 { 12 } else { 8 };
    (0..=n)
        .map(|k| {
            let mut v = values.to_vec();
            v.resize(slider_specs(mode).len(), 0.0);
            if let Some(x) = v.get_mut(i) {
                *x = s.min + (s.max - s.min) * k as f32 / n as f32;
            }
            let [r, g, b] = panels::srgb_bytes({
                let c = rgb_of(mode, &v);
                [c[0], c[1], c[2], 1.0]
            });
            Color32::from_rgb(r, g, b)
        })
        .collect()
}

/// The colour at (`x`, `y`) in 0..1 (y down) of a ramp: Photoshop's spectrum ramps run through
/// the hues (RGB) or the inks (CMYK) across, white at the top and black at the bottom; the
/// grayscale ramp from black to white; the current colours from the foreground to the background.
pub fn ramp_color(ramp: ColorRamp, x: f32, y: f32, fg: [f32; 3], bg: [f32; 3]) -> [f32; 3] {
    let unit = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let (x, y) = (unit(x), unit(y));
    let shade = |c: [f32; 3]| {
        // Top half: towards white; bottom half: towards black.
        if y < 0.5 { c.map(|v| v + (1.0 - v) * (1.0 - y * 2.0)) } else { c.map(|v| v * (2.0 - y * 2.0)) }
    };
    match ramp {
        ColorRamp::RgbSpectrum => shade(crate::color_picker_ui::hsv_to_rgb(x * 360.0, 1.0, 1.0)),
        ColorRamp::CmykSpectrum => {
            // C → M → Y → C through the two-ink mixes, in ink space.
            let t = x * 3.0;
            let seg = (t.floor() as usize).min(2);
            let f = t - seg as f32;
            let inks = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
            let (a, b) = (inks[seg], inks[seg + 1]);
            // Rise to the mix, then fall from it: a → a+b → b.
            let mix = |p: f32| {
                if p < 0.5 {
                    [a[0] + b[0] * p * 2.0, a[1] + b[1] * p * 2.0, a[2] + b[2] * p * 2.0]
                } else {
                    [b[0] + a[0] * (2.0 - p * 2.0), b[1] + a[1] * (2.0 - p * 2.0), b[2] + a[2] * (2.0 - p * 2.0)]
                }
            };
            let [c, m, yy] = mix(f);
            shade(convert::cmyk_to_rgb([c, m, yy, 0.0]).map(|v| v.clamp(0.0, 1.0)))
        }
        ColorRamp::Grayscale => [x, x, x],
        ColorRamp::CurrentColors => [fg[0] + (bg[0] - fg[0]) * x, fg[1] + (bg[1] - fg[1]) * x, fg[2] + (bg[2] - fg[2]) * x],
    }
}

// ----------------------------------------------------------------------------- the active chip

fn active_color(app: &PhotocraftApp) -> [f32; 4] {
    if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground }
}

/// Set the chip the panel edits (the foreground recolours selected type, as it does elsewhere).
pub(crate) fn set_active_color(app: &mut PhotocraftApp, rgb: [f32; 3]) {
    let c = [rgb[0], rgb[1], rgb[2], 1.0];
    if app.ui.color_panel.background {
        app.session.tools.background = c;
    } else {
        app.session.tools.foreground = c;
        crate::type_tool::foreground_changed(app);
    }
}

/// Photoshop's warnings under the chips, each over a chip of the nearest colour that passes
/// (a click takes it): ⚠ when the targeted colour is out of gamut for printing in the working
/// CMYK, a cube when it is not a web colour. `column` is the chips' column; the warnings sit
/// under the two chips, side by side.
pub(crate) fn chip_warnings(app: &mut PhotocraftApp, ui: &mut egui::Ui, column: Rect) {
    let t = Tokens::get(ui.ctx());
    let c = active_color(app);
    let rgb = [c[0], c[1], c[2]];
    let gamut = tl!("Out of gamut for printing: click for the nearest printable colour");
    let web = tl!("Not a web colour: click for the nearest web colour");
    let top = column.top() + 41.0;
    for (slot, warning, tip) in [(0.0, crate::color_picker_ui::out_of_gamut(rgb), gamut), (17.0, crate::color_picker_ui::not_web_safe(rgb), web)] {
        let Some(near) = warning else { continue };
        let icon = Rect::from_min_size(pos2(column.left() + 3.0 + slot, top), vec2(13.0, 13.0));
        let chip = Rect::from_min_size(pos2(icon.left() + 1.5, icon.bottom() + 3.0), vec2(10.0, 10.0));
        let p = ui.painter();
        if slot == 0.0 {
            crate::color_picker_ui::warning_triangle(p, icon, t.text, t.chrome);
        } else {
            crate::color_picker_ui::web_cube(p, icon, t.text);
        }
        let [r, g, b] = panels::srgb_bytes([near[0], near[1], near[2], 1.0]);
        p.rect_filled(chip, 0.0, Color32::from_rgb(r, g, b));
        p.rect_stroke(chip, 0.0, Stroke::new(1.0, t.text_dim), StrokeKind::Outside);
        let resp = ui.interact(icon.union(chip), ui.id().with(("color-panel-warning", slot as i32)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
        if resp.on_hover_text(tip).clicked() {
            set_active_color(app, near);
        }
    }
}

/// Height of the chips column: the two chips and the warnings under them.
pub(crate) const CHIP_COLUMN_H: f32 = 70.0;

// ----------------------------------------------------------------------------- sliders

/// The chips beside the sliders of `mode`, the hex readout under them.
pub(crate) fn sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui, mode: ColorPanelMode) {
    let bg = app.ui.color_panel.background;
    let color = active_color(app);
    let rgb = [color[0], color[1], color[2]];
    let key = egui::Id::new(("color-panel-sliders", mode, bg));
    let stored: Option<Vec<f32>> = ui.data(|d| d.get_temp(key));
    let mut values = remembered(mode, rgb, stored);
    let specs = slider_specs(mode);
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let mut changed = false;
    // Photoshop's rows: the label, the track and the field on one line, the chips beside them.
    let label_w = label_column(ui, specs);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, (ROW_H * specs.len() as f32).max(CHIP_COLUMN_H)), Sense::hover());
        panels::field_chips(app, ui, chips);
        chip_warnings(app, ui, chips);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, s) in specs.iter().enumerate() {
                let stops = dynamic.then(|| track(mode, &values, i));
                let Some(v) = values.get_mut(i) else { continue };
                changed |= if mode == ColorPanelMode::Web {
                    web_row(ui, s.label, label_w, v, stops.as_deref())
                } else {
                    compact_row(ui, s.label, label_w, v, s.min..=s.max, s.suffix, stops.as_deref())
                };
            }
        });
    });
    if changed {
        set_active_color(app, rgb_of(mode, &values));
        ui.data_mut(|d| d.insert_temp(key, values));
    }
}

/// Height of one slider row.
const ROW_H: f32 = 24.0;

/// The width of the label column for `specs`: the widest label, at most 72 pt (a long
/// translation is elided with its full text on hover).
fn label_column(ui: &egui::Ui, specs: &[SliderSpec]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let widest = specs.iter().map(|s| ui.fonts_mut(|f| f.layout_no_wrap(tl!(s.label).to_string(), font.clone(), Color32::WHITE).size().x)).fold(0.0, f32::max);
    (widest + 2.0).clamp(16.0, 72.0)
}

/// The label of a row, `label_w` wide and `ROW_H` tall, registered as a label for assistive
/// technology (and tests).
fn row_label(ui: &mut egui::Ui, label: &str, label_w: f32) {
    ui.allocate_ui_with_layout(vec2(label_w, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        widgets::elided_label(ui, tl!(label), label_w);
    });
}

/// One slider row as Photoshop's Color panel lays it out: the label, the track and the value
/// field on one line. True when the value changed.
fn compact_row(
    ui: &mut egui::Ui,
    label: &str,
    label_w: f32,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        row_label(ui, label, label_w);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed |= widgets::value_field_prec(ui, value, range.clone(), suffix, 48.0, 0).changed();
            changed |= widgets::slider_with(ui, value, range, gradient, widgets::RowGestures::default()).changed();
        });
    });
    changed
}

/// A Web Color Sliders row: the channel as a two-digit hex field beside its track, with ticks at
/// the web-safe values (00, 33, 66, 99, CC, FF), as Photoshop shows them.
fn web_row(ui: &mut egui::Ui, label: &str, label_w: f32, value: &mut f32, gradient: Option<&[Color32]>) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut track = Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        row_label(ui, label, label_w);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let id = ui.id().with(("web-hex", label));
            let (rect, _) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::hover());
            widgets::surface(ui, rect, t.field, false);
            if !t.bevel {
                ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            }
            let byte = value.round().clamp(0.0, 255.0) as u8;
            let typing = if ui.memory(|m| m.has_focus(id)) { ui.data(|d| d.get_temp::<String>(id)) } else { None };
            let mut text = typing.unwrap_or_else(|| format!("{byte:02X}"));
            let field = rect.shrink2(vec2(5.0, 2.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let resp = child
                .add(egui::TextEdit::singleline(&mut text).id(id).char_limit(2).desired_width(field.width()).frame(egui::Frame::NONE).font(theme::mono(12.0)));
            if resp.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
            } else {
                ui.data_mut(|d| d.remove::<String>(id));
            }
            if resp.changed()
                && text.len() == 2
                && let Ok(b) = u8::from_str_radix(&text, 16)
            {
                *value = f32::from(b);
                changed = true;
            }
            let r = widgets::slider_with(ui, value, 0.0..=255.0, gradient, widgets::RowGestures::default());
            changed |= r.changed();
            track = r.rect;
        });
    });
    // Web-safe ticks under the track (the track is inset 7 pt at each end of its widget).
    if track.is_positive() {
        let (x0, x1) = (track.left() + 7.0, track.right() - 7.0);
        for k in 0..6 {
            let x = x0 + (x1 - x0) * k as f32 / 5.0;
            ui.painter().line_segment([pos2(x, track.bottom() - 2.0), pos2(x, track.bottom() + 1.0)], Stroke::new(1.0, t.text_faint));
        }
    }
    changed
}

// ----------------------------------------------------------------------------- the ramp

/// The colour ramp under the sliders: click or drag picks the foreground (Alt: the background).
pub(crate) fn ramp(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let kind = app.ui.color_panel.ramp;
    let fg = app.session.tools.foreground;
    let bg = app.session.tools.background;
    let (fg3, bg3) = ([fg[0], fg[1], fg[2]], [bg[0], bg[1], bg[2]]);
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click_and_drag());
    let mut mesh = egui::Mesh::default();
    let (nx, ny) = (48, 4);
    for j in 0..=ny {
        for i in 0..=nx {
            let (x, y) = (i as f32 / nx as f32, j as f32 / ny as f32);
            let [r, g, b] = panels::srgb_bytes({
                let c = ramp_color(kind, x, y, fg3, bg3);
                [c[0], c[1], c[2], 1.0]
            });
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), Color32::from_rgb(r, g, b));
        }
    }
    for j in 0..ny {
        for i in 0..nx {
            let a = (j * (nx + 1) + i) as u32;
            let (b, c) = (a + 1, a + (nx + 1) as u32);
            mesh.add_triangle(a, b, c + 1);
            mesh.add_triangle(a, c + 1, c);
        }
    }
    ui.painter().add(mesh);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let c = ramp_color(kind, (p.x - rect.left()) / rect.width(), (p.y - rect.top()) / rect.height(), fg3, bg3);
        // Alt held, or on the click's own events (a driven click carries its modifiers there).
        let alt = ui.input(|i| i.modifiers.alt || i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { modifiers, .. } if modifiers.alt)));
        if alt {
            app.session.tools.background = [c[0], c[1], c[2], 1.0];
        } else {
            app.session.tools.foreground = [c[0], c[1], c[2], 1.0];
            crate::type_tool::foreground_changed(app);
        }
    }
}

// ----------------------------------------------------------------------------- the wheel

/// Which part of the wheel a drag started on, so leaving the ring mid-drag keeps turning the hue.
#[derive(Clone, Copy, Debug, PartialEq)]
enum WheelPart {
    Ring,
    Square,
}

/// The wheel's geometry in a square of side `side` at `center`: the hue ring's radii and the
/// saturation/brightness square inside it.
pub fn wheel_geometry(center: Pos2, side: f32) -> (f32, f32, Rect) {
    let outer = side / 2.0;
    let inner = outer * 0.78;
    let half = inner * std::f32::consts::FRAC_1_SQRT_2 * 0.92;
    (outer, inner, Rect::from_center_size(center, vec2(half * 2.0, half * 2.0)))
}

/// Where the last drawn wheel is: `(center, outer, inner, square)`.
pub fn wheel_geometry_id() -> egui::Id {
    egui::Id::new("color-wheel-geometry")
}

/// The hue (0..1) at angle `a` from the centre: red at the top, hues clockwise.
pub fn wheel_hue(dx: f32, dy: f32) -> f32 {
    let a = dx.atan2(-dy); // 0 at the top, clockwise positive
    (a / std::f32::consts::TAU).rem_euclid(1.0)
}

/// Where hue `h` sits on the ring, as a unit vector from the centre.
pub fn wheel_point(h: f32) -> (f32, f32) {
    let a = h * std::f32::consts::TAU;
    (a.sin(), -a.cos())
}

fn wheel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let bg_active = app.ui.color_panel.background;
    let key = egui::Id::new(("color-field-hue", bg_active));
    let drag_key = egui::Id::new("color-wheel-drag");
    let mut hsva = panels::srgb_hsva(active_color(app));
    let remembered: (f32, f32) = ui.data(|d| d.get_temp(key)).unwrap_or((hsva.h, hsva.s));
    hsva = panels::field_hsva(hsva, remembered);
    let w = ui.available_width();
    let fill = if t.pro { ui.data(|d| d.get_temp::<f32>(panels::color_field_fill_id())) } else { None };
    let h = panels::color_field_height(fill, w);
    let side = (w - 38.0 - 16.0).min(h).max(80.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, h), Sense::hover());
        panels::field_chips(app, ui, chips);
        chip_warnings(app, ui, chips);
        let (area, resp) = ui.allocate_exact_size(vec2(w - 38.0 - 16.0, h), Sense::click_and_drag());
        let center = area.center();
        let (outer, inner, square) = wheel_geometry(center, side);
        // Where the wheel is, for tests that drive it.
        ui.data_mut(|d| d.insert_temp(wheel_geometry_id(), (center, outer, inner, square)));
        // The ring.
        let mut mesh = egui::Mesh::default();
        let n = 96;
        for k in 0..=n {
            let hue = k as f32 / n as f32;
            let (dx, dy) = wheel_point(hue);
            let c = panels::hsva_color32(egui::ecolor::Hsva::new(hue % 1.0, 1.0, 1.0, 1.0));
            mesh.colored_vertex(pos2(center.x + dx * inner, center.y + dy * inner), c);
            mesh.colored_vertex(pos2(center.x + dx * outer, center.y + dy * outer), c);
        }
        for k in 0..n {
            let a = (k * 2) as u32;
            mesh.add_triangle(a, a + 1, a + 3);
            mesh.add_triangle(a, a + 3, a + 2);
        }
        ui.painter().add(mesh);
        ui.painter().circle_stroke(center, outer, Stroke::new(1.0, t.separator));
        ui.painter().circle_stroke(center, inner, Stroke::new(1.0, t.separator));
        // The saturation/brightness square.
        let mut m2 = egui::Mesh::default();
        let g = 16;
        for j in 0..=g {
            for i in 0..=g {
                let (sx, vy) = (i as f32 / g as f32, j as f32 / g as f32);
                let c = panels::hsva_color32(egui::ecolor::Hsva::new(hsva.h, sx, 1.0 - vy, 1.0));
                m2.colored_vertex(pos2(square.left() + sx * square.width(), square.top() + vy * square.height()), c);
            }
        }
        for j in 0..g {
            for i in 0..g {
                let a = (j * (g + 1) + i) as u32;
                let (b, c) = (a + 1, a + (g + 1) as u32);
                m2.add_triangle(a, b, c + 1);
                m2.add_triangle(a, c + 1, c);
            }
        }
        ui.painter().add(m2);
        ui.painter().rect_stroke(square, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        // Pointer: the part pressed first keeps the drag.
        let mut edited = false;
        if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked() || resp.dragged() || resp.drag_started()) {
            let d = p - center;
            let dist = d.length();
            let part: Option<WheelPart> = if resp.drag_started() || resp.clicked() {
                let part = if square.contains(p) {
                    Some(WheelPart::Square)
                } else if dist >= inner * 0.9 && dist <= outer * 1.1 {
                    Some(WheelPart::Ring)
                } else {
                    None
                };
                ui.data_mut(|m| m.insert_temp(drag_key, part));
                part
            } else {
                ui.data(|m| m.get_temp::<Option<WheelPart>>(drag_key)).flatten()
            };
            match part {
                Some(WheelPart::Ring) => {
                    hsva.h = wheel_hue(d.x, d.y).min(0.9999);
                    edited = true;
                }
                Some(WheelPart::Square) => {
                    hsva.s = ((p.x - square.left()) / square.width()).clamp(0.0, 1.0);
                    hsva.v = 1.0 - ((p.y - square.top()) / square.height()).clamp(0.0, 1.0);
                    edited = true;
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            ui.data_mut(|m| m.remove::<Option<WheelPart>>(drag_key));
        }
        // Markers.
        let (dx, dy) = wheel_point(hsva.h);
        let on_ring = pos2(center.x + dx * (inner + outer) / 2.0, center.y + dy * (inner + outer) / 2.0);
        ui.painter().circle_stroke(on_ring, (outer - inner) / 2.0 - 1.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(on_ring, (outer - inner) / 2.0 + 0.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
        let knob = pos2(square.left() + hsva.s * square.width(), square.top() + (1.0 - hsva.v) * square.height());
        ui.painter().circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
        if edited {
            let c = panels::hsva_srgb(hsva);
            set_active_color(app, [c[0], c[1], c[2]]);
            ui.data_mut(|d| d.insert_temp(key, (hsva.h, hsva.s)));
        }
    });
    ui.add_space(6.0);
    panels::color_readout_row(app, ui, key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(c: [f32; 3]) -> [u8; 3] {
        panels::srgb_bytes([c[0], c[1], c[2], 1.0])
    }

    #[test]
    fn slider_modes_round_trip_their_own_values() {
        for mode in [ColorPanelMode::Grayscale, ColorPanelMode::Rgb, ColorPanelMode::Hsb, ColorPanelMode::Cmyk, ColorPanelMode::Lab, ColorPanelMode::Web] {
            for rgb in [[0.847, 0.271, 0.180], [0.5, 0.5, 0.5], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.1, 0.8, 0.3], [0.6, 0.4, 0.3]] {
                let v = sliders_of(mode, rgb);
                assert_eq!(v.len(), slider_specs(mode).len(), "{mode:?}");
                let back = rgb_of(mode, &v);
                let tol = match mode {
                    ColorPanelMode::Grayscale => continue, // a grey of the colour, not the colour
                    // Through the coated CMYK profile and back: a vivid green is outside its gamut
                    // and comes back clipped, as Photoshop shows it; the clipped colour is stable.
                    ColorPanelMode::Cmyk if rgb == [0.1, 0.8, 0.3] => {
                        let again = rgb_of(mode, &sliders_of(mode, back));
                        let err = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (bytes(a)[i] as i32 - bytes(b)[i] as i32).abs()).max().unwrap_or(0);
                        assert!(err(rgb, back) > 8 && err(back, again) < err(rgb, back), "{v:?} -> {back:?} -> {again:?}");
                        continue;
                    }
                    ColorPanelMode::Cmyk => 6,
                    _ => 1,
                };
                for i in 0..3 {
                    let (a, b) = (bytes(rgb)[i] as i32, bytes(back)[i] as i32);
                    assert!((a - b).abs() <= tol, "{mode:?} {rgb:?} -> {v:?} -> {back:?}");
                }
            }
        }
        assert_eq!(sliders_of(ColorPanelMode::Grayscale, [1.0, 1.0, 1.0]), vec![0.0], "white is K 0");
        assert_eq!(sliders_of(ColorPanelMode::Grayscale, [0.0, 0.0, 0.0]), vec![100.0], "black is K 100");
        assert_eq!(bytes(rgb_of(ColorPanelMode::Grayscale, &[50.0])), [128, 128, 128]);
        assert_eq!(bytes(rgb_of(ColorPanelMode::Rgb, &[255.0, 0.0, 51.0])), [255, 0, 51]);
        assert_eq!(bytes(rgb_of(ColorPanelMode::Lab, &[100.0, 0.0, 0.0])), [255, 255, 255]);
        assert_eq!(bytes(rgb_of(ColorPanelMode::Cmyk, &[0.0, 0.0, 0.0, 0.0])), [255, 255, 255], "no ink is paper white");
    }

    #[test]
    fn bad_values_are_clamped_never_nan() {
        for mode in [ColorPanelMode::Grayscale, ColorPanelMode::Rgb, ColorPanelMode::Hsb, ColorPanelMode::Cmyk, ColorPanelMode::Lab, ColorPanelMode::Web] {
            for values in [vec![], vec![f32::NAN; 4], vec![1e9; 4], vec![-1e9; 4], vec![f32::INFINITY, f32::NEG_INFINITY]] {
                let c = rgb_of(mode, &values);
                assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{mode:?} {values:?} -> {c:?}");
            }
            let v = sliders_of(mode, [f32::NAN, 2.0, -1.0]);
            assert!(v.iter().all(|x| x.is_finite()), "{mode:?} {v:?}");
        }
        assert_eq!(rgb_of(ColorPanelMode::HueCube, &[1.0]), [0.0; 3]);
    }

    #[test]
    fn remembered_values_survive_while_they_still_give_the_colour() {
        // Black has no hue or saturation: the typed H and S stay until the colour changes.
        let stored = Some(vec![120.0, 100.0, 0.0]);
        assert_eq!(remembered(ColorPanelMode::Hsb, [0.0, 0.0, 0.0], stored.clone()), vec![120.0, 100.0, 0.0]);
        assert_eq!(remembered(ColorPanelMode::Hsb, [1.0, 0.0, 0.0], stored), vec![0.0, 100.0, 100.0]);
        // A stored value of the wrong shape is ignored.
        assert_eq!(remembered(ColorPanelMode::Rgb, [1.0, 0.0, 0.0], Some(vec![1.0])), vec![255.0, 0.0, 0.0]);
        // Lab a/b of a grey: whatever rounds to the grey stays.
        let grey = [0.5, 0.5, 0.5];
        let l = sliders_of(ColorPanelMode::Lab, grey)[0];
        assert_eq!(remembered(ColorPanelMode::Lab, grey, Some(vec![l, 0.05, -0.05])), vec![l, 0.05, -0.05]);
        let v = sliders_of(ColorPanelMode::Cmyk, [0.6, 0.4, 0.3]);
        assert_eq!(remembered(ColorPanelMode::Cmyk, rgb_of(ColorPanelMode::Cmyk, &v), Some(v.clone())), v, "its own values always count as the colour");
        assert_eq!(remembered(ColorPanelMode::Lab, grey, Some(vec![l, 40.0, 0.0])).len(), 3, "a different colour's values are replaced");
        assert!(remembered(ColorPanelMode::Lab, grey, Some(vec![l, 40.0, 0.0]))[1].abs() < 0.01);
    }

    #[test]
    fn tracks_sweep_one_slider() {
        let t = track(ColorPanelMode::Rgb, &[0.0, 0.0, 0.0], 0);
        assert_eq!((t.first().copied(), t.last().copied()), (Some(Color32::BLACK), Some(Color32::from_rgb(255, 0, 0))));
        let t = track(ColorPanelMode::Grayscale, &[0.0], 0);
        assert_eq!((t.first().copied(), t.last().copied()), (Some(Color32::WHITE), Some(Color32::BLACK)));
        assert!(track(ColorPanelMode::Lab, &[50.0, 0.0, 0.0], 9).is_empty(), "no such slider");
        assert_eq!(track(ColorPanelMode::Hsb, &[0.0, 100.0, 100.0], 0).len(), 13, "the hue track has every sixth hue");
    }

    #[test]
    fn ramps_have_photoshops_corners() {
        let (fg, bg) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(bytes(ramp_color(ColorRamp::RgbSpectrum, 0.0, 0.5, fg, bg)), [255, 0, 0], "red at the left, full in the middle");
        assert_eq!(bytes(ramp_color(ColorRamp::RgbSpectrum, 0.3, 0.0, fg, bg)), [255, 255, 255], "white along the top");
        assert_eq!(bytes(ramp_color(ColorRamp::RgbSpectrum, 0.3, 1.0, fg, bg)), [0, 0, 0], "black along the bottom");
        assert_eq!(bytes(ramp_color(ColorRamp::Grayscale, 0.0, 0.5, fg, bg)), [0, 0, 0]);
        assert_eq!(bytes(ramp_color(ColorRamp::Grayscale, 1.0, 0.5, fg, bg)), [255, 255, 255]);
        assert_eq!(bytes(ramp_color(ColorRamp::CurrentColors, 0.0, 0.5, fg, bg)), [255, 0, 0]);
        assert_eq!(bytes(ramp_color(ColorRamp::CurrentColors, 1.0, 0.5, fg, bg)), [0, 0, 255]);
        let c = ramp_color(ColorRamp::CmykSpectrum, 0.0, 0.5, fg, bg);
        assert!(c[2] > c[0] && c[1] > c[0], "cyan ink at the left: {c:?}");
        for x in [f32::NAN, -5.0, 7.0] {
            let c = ramp_color(ColorRamp::RgbSpectrum, x, x, fg, bg);
            assert!(c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{x}: {c:?}");
        }
    }

    #[test]
    fn wheel_hues_go_clockwise_from_the_top() {
        assert!(wheel_hue(0.0, -1.0).abs() < 1e-6, "red at the top");
        assert!((wheel_hue(1.0, 0.0) - 0.25).abs() < 1e-6, "a quarter turn clockwise at the right");
        assert!((wheel_hue(0.0, 1.0) - 0.5).abs() < 1e-6, "cyan at the bottom");
        for h in [0.0, 0.1, 0.5, 0.9] {
            let (dx, dy) = wheel_point(h);
            assert!((wheel_hue(dx, dy) - h).abs() < 1e-5, "{h}");
        }
        let (outer, inner, square) = wheel_geometry(pos2(100.0, 100.0), 100.0);
        assert!(inner < outer && square.width() < inner * 2.0 && square.center() == pos2(100.0, 100.0));
    }
}
