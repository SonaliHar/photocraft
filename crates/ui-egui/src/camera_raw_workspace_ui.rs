//! Native Camera Raw workspace controls, observed in the owner's ACR 18.6 captures.
//! Presentation is serde view state; image edits use the existing CameraRaw parameters.
use egui::{Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, Wheel};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{icons, theme::Tokens, widgets};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradingView {
    #[default]
    ThreeWay,
    Shadows,
    Midtones,
    Highlights,
    Global,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixerMode {
    #[default]
    Hsl,
    Color,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CurveChannel {
    Parametric,
    #[default]
    Master,
    Red,
    Green,
    Blue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceTab {
    #[default]
    Edit,
    Snapshots,
    RedEye,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PanelState {
    pub open: Vec<String>,
    pub single: bool,
}
impl Default for PanelState {
    fn default() -> Self {
        Self { open: vec!["Light".into()], single: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct WorkspaceState {
    pub tab: WorkspaceTab,
    pub panels: PanelState,
    pub grading: GradingView,
    pub curve_channel: CurveChannel,
    pub mixer: MixerMode,
    pub band: usize,
    pub hsl_channel: usize,
    pub vignette_details: bool,
    pub grain_details: bool,
    pub sharpening_details: bool,
    pub noise_details: bool,
    pub info: bool,
    pub grid: bool,
    pub grid_size: f32,
    pub grid_opacity: f32,
    pub white_balance_tool: bool,
    /// Temporary preview only. Never part of the saved recipe or committed edits.
    pub bypass_section: Option<String>,
}
impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tab: WorkspaceTab::Edit,
            panels: PanelState::default(),
            grading: GradingView::default(),
            curve_channel: CurveChannel::default(),
            mixer: MixerMode::Color,
            band: 0,
            hsl_channel: 1,
            vignette_details: false,
            grain_details: false,
            sharpening_details: false,
            noise_details: false,
            info: false,
            grid: false,
            grid_size: 40.0,
            grid_opacity: 0.35,
            white_balance_tool: false,
            bypass_section: None,
        }
    }
}

pub const SECTIONS: [&str; 10] = ["Light", "Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail", "Optics", "Lens Blur", "Calibration"];

pub(crate) fn supports_bypass(title: &str) -> bool {
    SECTIONS.iter().take(7).any(|name| *name == title)
}

pub(crate) fn held_bypass_id() -> egui::Id {
    egui::Id::new("camera-raw-held-section-bypass")
}

/// Neutralize one implemented processing stage in a copy, preserving the saved recipe.
pub(crate) fn preview_without(parameters: &CameraRaw, title: Option<&str>) -> CameraRaw {
    let mut p = parameters.clone();
    let defaults = CameraRaw::default();
    match title {
        Some("Light") => {
            p.exposure = defaults.exposure;
            p.contrast = defaults.contrast;
            p.highlights = defaults.highlights;
            p.shadows = defaults.shadows;
            p.whites = defaults.whites;
            p.blacks = defaults.blacks;
        }
        Some("Color") => {
            p.temperature = defaults.temperature;
            p.tint = defaults.tint;
            p.vibrance = defaults.vibrance;
            p.saturation = defaults.saturation;
        }
        Some("Effects") => {
            p.texture = defaults.texture;
            p.clarity = defaults.clarity;
            p.dehaze = defaults.dehaze;
            p.vignette_amount = defaults.vignette_amount;
            p.grain_amount = defaults.grain_amount;
        }
        Some("Curve") => {
            p.curve_highlights = defaults.curve_highlights;
            p.curve_lights = defaults.curve_lights;
            p.curve_darks = defaults.curve_darks;
            p.curve_shadows = defaults.curve_shadows;
            p.curve_splits = defaults.curve_splits;
            p.point_curve = defaults.point_curve;
            p.point_curve_red = defaults.point_curve_red;
            p.point_curve_green = defaults.point_curve_green;
            p.point_curve_blue = defaults.point_curve_blue;
        }
        Some("Color Mixer") => {
            p.hsl_hue = defaults.hsl_hue;
            p.hsl_sat = defaults.hsl_sat;
            p.hsl_lum = defaults.hsl_lum;
        }
        Some("Color Grading") => {
            p.grade_shadows = defaults.grade_shadows;
            p.grade_midtones = defaults.grade_midtones;
            p.grade_highlights = defaults.grade_highlights;
            p.grade_global = defaults.grade_global;
            p.grade_blending = defaults.grade_blending;
            p.grade_balance = defaults.grade_balance;
        }
        Some("Detail") => {
            p.sharpen_amount = defaults.sharpen_amount;
            p.noise_luminance = defaults.noise_luminance;
            p.noise_color = defaults.noise_color;
        }
        _ => {}
    }
    p
}

fn slider_row_with(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
    gestures: widgets::RowGestures,
) -> egui::Response {
    let design = super::camera_raw_design::get(ui.ctx());
    widgets::slider_row_prec_styled(ui, label, value, range, suffix, gradient, gestures, None, design.value_field_width, Some(design.slider_style()))
}

fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, gradient: Option<&[Color32]>) -> egui::Response {
    slider_row_with(ui, label, value, range, suffix, gradient, Default::default())
}

pub(crate) fn capture_summary(info: &photocraft_algo::exif::CameraInfo) -> String {
    let mut fields = Vec::new();
    if let Some(iso) = info.iso.filter(|v| v.is_finite() && *v > 0.0) {
        fields.push(format!("ISO {iso:.0}"));
    }
    if let Some(focal) = info.focal_length.filter(|v| v.is_finite() && *v > 0.0) {
        fields.push(format!("{focal:.0} mm"));
    }
    if let Some(aperture) = info.f_number.filter(|v| v.is_finite() && *v > 0.0) {
        fields.push(format!("f/{aperture:.1}"));
    }
    if let Some(seconds) = info.exposure_time.filter(|v| v.is_finite() && *v > 0.0) {
        fields.push(if seconds < 1.0 && seconds.recip().is_finite() { format!("1/{:.0}s", seconds.recip()) } else { format!("{seconds:.1}s") });
    }
    fields.join("   ")
}

pub(crate) fn disclosed_slider(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, expanded: &mut bool) -> bool {
    let design = super::camera_raw_design::get(ui.ctx());
    widgets::slider_row_prec_styled_trailing(
        ui,
        tr(label),
        value,
        range,
        "",
        None,
        widgets::RowGestures { reset: Some(0.0), wheel_step: None },
        None,
        design.value_field_width,
        Some(design.slider_style()),
        |ui| {
            ui.spacing_mut().item_spacing.x = design.disclosure_button_gap;
            let label = format!("{} {}", tr(label), tl!("Details"));
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(design.disclosure_button_size), Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), *expanded, &label));
            if response.clicked() {
                *expanded = !*expanded;
            }
            response.on_hover_text(label);
            let center = rect.center();
            let points = if *expanded {
                vec![center + vec2(-4.5, -3.0), center + vec2(4.5, -3.0), center + vec2(0.0, 3.0)]
            } else {
                vec![center + vec2(3.0, -4.5), center + vec2(-3.0, 0.0), center + vec2(3.0, 4.5)]
            };
            ui.painter().add(egui::Shape::convex_polygon(points, Tokens::get(ui.ctx()).text_dim, Stroke::NONE));
        },
    )
    .changed()
}

pub(crate) fn prepare(current: &WorkspaceState, value: &Value) -> Result<WorkspaceState, String> {
    let Some(fields) = value.as_object() else { return Err("Camera Raw workspace must be an object".into()) };
    let mut merged = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let Some(target) = merged.as_object_mut() else { return Err("invalid Camera Raw workspace".into()) };
    for (key, value) in fields {
        if !target.contains_key(key) {
            return Err(format!("unknown Camera Raw workspace property {key}"));
        }
        if key == "panels"
            && let (Some(old), Some(new)) = (target.get_mut(key).and_then(Value::as_object_mut), value.as_object())
        {
            for (name, item) in new {
                old.insert(name.clone(), item.clone());
            }
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    let next: WorkspaceState = serde_json::from_value(merged).map_err(|e| format!("invalid Camera Raw workspace: {e}"))?;
    if next.hsl_channel >= 3
        || next.band >= 8
        || !next.grid_size.is_finite()
        || !(12.0..=200.0).contains(&next.grid_size)
        || !next.grid_opacity.is_finite()
        || !(0.0..=1.0).contains(&next.grid_opacity)
    {
        return Err("Camera Raw band must be 0–7, grid size 12–200, and opacity 0–1".into());
    }
    if next.panels.open.len() > SECTIONS.len()
        || next.panels.open.iter().any(|name| !SECTIONS.contains(&name.as_str()))
        || next.panels.open.iter().enumerate().any(|(i, name)| next.panels.open.iter().take(i).any(|old| old == name))
        || next.panels.single && next.panels.open.len() > 1
    {
        return Err("Camera Raw panels must be unique supported names (at most one in single-panel mode)".into());
    }
    if next.bypass_section.as_deref().is_some_and(|name| !supports_bypass(name)) {
        return Err("Camera Raw bypassSection must name an implemented processing section".into());
    }
    Ok(next)
}

pub(crate) fn section(ui: &mut Ui, title: &str, state: &mut PanelState, body: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    let design = super::camera_raw_design::get(ui.ctx());
    let expanded = state.open.iter().any(|name| name == title);
    let original_spacing = ui.spacing().clone();
    let original_visuals = ui.visuals().clone();
    let result = ui
        .scope(|ui| {
            ui.visuals_mut().collapsing_header_frame = true;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = t.dock;
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = t.hover;
            ui.visuals_mut().widgets.active.weak_bg_fill = t.pressed;
            ui.spacing_mut().interact_size.y = design.section_height;
            ui.spacing_mut().indent = design.section_text_inset;
            ui.spacing_mut().button_padding = vec2(12.0, 8.0);
            let icon_inset = design.section_icon_inset;
            egui::CollapsingHeader::new(egui::RichText::new(tr(title)).size(design.section_font_size))
                .icon(move |ui, openness, response| {
                    let center = pos2(response.rect.center().x - ui.spacing().indent / 2.0 + icon_inset, response.rect.center().y);
                    let rotation = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
                    let points = [vec2(-2.5, -5.0), vec2(2.5, 0.0), vec2(-2.5, 5.0)].map(|p| center + rotation * p);
                    ui.painter().line_segment([points[0], points[1]], Stroke::new(2.0, t.icon));
                    ui.painter().line_segment([points[1], points[2]], Stroke::new(2.0, t.icon));
                })
                .id_salt(("camera-raw-section", title))
                .open(Some(expanded))
                .show_unindented(ui, |ui| {
                    *ui.spacing_mut() = original_spacing;
                    *ui.visuals_mut() = original_visuals;
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: if title == "Curve" { design.curve_left_padding as i8 } else { design.section_padding as i8 },
                            right: if title == "Curve" { design.curve_right_padding as i8 } else { design.settings_body_right_padding as i8 },
                            top: design.section_vertical_padding as i8,
                            bottom: match title {
                                "Detail" => design.detail_bottom_padding as i8,
                                "Color Grading" => design.grading_bottom_padding as i8,
                                "Curve" => design.curve_bottom_padding as i8,
                                _ => design.section_bottom_padding as i8,
                            },
                        })
                        .show(ui, body);
                })
        })
        .inner;
    let header = &result.header_response;
    // Axis-aligned dividers have hard pixel edges in the reference. A filled
    // Shape::Rect is feathered by the tessellator even with zero corner radius.
    let scale = ui.ctx().pixels_per_point();
    let top = (header.rect.top() * scale).round() / scale;
    let divider = Rect::from_min_size(pos2(header.rect.left(), top), vec2(header.rect.width(), design.section_separator_height));
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(divider, t.separator);
    ui.painter().add(egui::Shape::mesh(mesh));
    let center = pos2(header.rect.right() - design.section_eye_inset, header.rect.center().y);
    let eye_rect = Rect::from_center_size(center, vec2(18.0, 18.0));
    let enabled = ui.is_enabled() && supports_bypass(title);
    let label = format!("{} — {}", tr(title), tl!("Before"));
    let eye = ui.interact(eye_rect, ui.id().with(("camera-raw-section-eye", title)), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    eye.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.clone()));
    let held = enabled && eye.is_pointer_button_down_on();
    let ink = if held { t.text } else { t.text_faint.gamma_multiply(design.section_eye_opacity) };
    let points = (0..=32)
        .map(|i| {
            let phase = i as f32 / 32.0 * std::f32::consts::TAU;
            center + vec2(phase.cos() * 6.0, phase.sin() * (1.0 - phase.cos().abs() * 0.25) * 3.75)
        })
        .collect();
    ui.painter().add(egui::Shape::closed_line(points, Stroke::new(1.2, ink)));
    ui.painter().circle_stroke(center, 2.0, Stroke::new(1.1, ink));
    if held {
        ui.ctx().data_mut(|data| data.insert_temp(held_bypass_id(), title.to_owned()));
        ui.ctx().request_repaint();
    }
    eye.clone().on_hover_text(label);
    // With explicit serde state, egui animates the view but never owns which panels are open.
    if header.clicked() && !eye.hovered() && !held {
        if expanded {
            state.open.retain(|name| name != title);
        } else {
            if state.single {
                state.open.clear();
            }
            state.open.push(title.into());
        }
    }
    header.context_menu(|ui| {
        if ui.button(tl!("Expand All")).clicked() {
            state.single = false;
            state.open = SECTIONS.iter().map(|name| (*name).into()).collect();
            ui.close();
        }
        if ui.button(tl!("Collapse All")).clicked() {
            state.open.clear();
            ui.close();
        }
        if ui.checkbox(&mut state.single, tl!("Single Panel Mode")).changed() && state.single {
            state.open = vec![title.into()];
        }
    });
}

fn tr(source: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", source)
}

/// Circle geometry uses HSV coordinates: red at the right, increasing hue counter-clockwise.
fn hue_sat(delta: Vec2, radius: f32, previous_hue: f32) -> Option<(f32, f32)> {
    if !delta.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let distance = delta.length();
    let hue = if distance < 0.001 { previous_hue } else { (-delta.y).atan2(delta.x).to_degrees().rem_euclid(360.0) };
    Some((hue, (distance / radius).clamp(0.0, 1.0) * 100.0))
}

/// Display-referred HSV: wheel art must not pass through a linear-light HSV conversion.
fn wheel_rim_color(hue: f32, brightness: f32) -> Color32 {
    let channel = |offset: f32| {
        let k = (offset + hue * 6.0).rem_euclid(6.0);
        (brightness * (1.0 - k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Color32::from_rgb(channel(5.0), channel(3.0), channel(1.0))
}

/// A wheel's luminance track has no visible label or value field in the three-way layout.
fn wheel_luminance(ui: &mut Ui, label: &str, value: &mut f32, width: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, super::camera_raw_design::get(ui.ctx()).grading_luminance_height), Sense::click_and_drag());
    let track = Rect::from_min_max(pos2(rect.left() + 5.0, rect.bottom() - 8.0), pos2(rect.right() - 5.0, rect.bottom() - 8.0));
    let old = *value;
    if (response.clicked() || response.dragged())
        && let Some(at) = response.interact_pointer_pos()
    {
        *value = ((at.x - track.left()) / track.width().max(1.0) * 200.0 - 100.0).clamp(-100.0, 100.0);
        response.request_focus();
    }
    if response.double_clicked() {
        *value = 0.0;
    }
    if response.has_focus() {
        let step = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
        for (key, delta) in [(egui::Key::ArrowLeft, -step), (egui::Key::ArrowRight, step)] {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key) || i.consume_key(egui::Modifiers::SHIFT, key)) {
                *value = (*value + delta).clamp(-100.0, 100.0);
            }
        }
    }
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), format!("{} {}", tr(label), tr("Luminance"))));
    ui.painter().line_segment([track.left_center(), track.right_center()], Stroke::new(3.0, t.text_faint.gamma_multiply(0.45)));
    let center = pos2(egui::lerp(track.x_range(), (*value + 100.0) / 200.0), track.center().y);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![center + vec2(0.0, -5.0), center + vec2(6.0, 1.0), center + vec2(6.0, 5.0), center + vec2(-6.0, 5.0), center + vec2(-6.0, 1.0)],
        t.text_faint.gamma_multiply(0.7),
        Stroke::new(1.0, t.field),
    ));
    if response.has_focus() {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    response.on_hover_text(format!("{} {}: {:.0}", tr(label), tr("Luminance"), *value));
    *value != old
}

fn wheel(ui: &mut Ui, label: &str, value: &mut Wheel, side: f32, label_offset: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.vertical_centered(|ui| {
        let design = super::camera_raw_design::get(ui.ctx());
        let (label_rect, response) = ui.allocate_exact_size(vec2(side, ui.text_style_height(&egui::TextStyle::Body)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), tr(label)));
        ui.painter().text(
            label_rect.center() + vec2(0.0, label_offset),
            egui::Align2::CENTER_CENTER,
            tr(label),
            egui::FontId::proportional(design.panel_font_size),
            t.text,
        );
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, ui.is_enabled(), format!("{} {}", tr(label), tl!("Color Wheel"))));
        let radius = (side * 0.5 - 5.0).max(1.0);
        let center = rect.center();
        let pointer = response.interact_pointer_pos();
        if (response.clicked() || response.dragged())
            && let Some(at) = pointer
            && let Some((hue, sat)) = hue_sat(at - center, radius, value.hue)
        {
            changed |= value.hue != hue || value.sat != sat;
            value.hue = hue;
            value.sat = sat;
            response.request_focus();
        }
        if response.double_clicked() {
            changed |= value.sat != 0.0;
            value.sat = 0.0;
        }
        if response.has_focus() {
            let step = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
            for (key, dh, ds) in
                [(egui::Key::ArrowLeft, -step, 0.0), (egui::Key::ArrowRight, step, 0.0), (egui::Key::ArrowDown, 0.0, -step), (egui::Key::ArrowUp, 0.0, step)]
            {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key) || i.consume_key(egui::Modifiers::SHIFT, key)) {
                    let old = *value;
                    value.hue = (value.hue + dh).rem_euclid(360.0);
                    value.sat = (value.sat + ds).clamp(0.0, 100.0);
                    changed |= old != *value;
                }
            }
        }
        if ui.is_rect_visible(rect) {
            let mut mesh = egui::Mesh::default();
            // Fixed small mesh, independent of source image dimensions. No bitmap assets.
            const N: usize = 192;
            mesh.colored_vertex(center, Color32::from_gray(128));
            let brightness = super::camera_raw_design::get(ui.ctx()).wheel_brightness;
            for i in 0..=N {
                let hue = i as f32 / N as f32;
                let angle = hue * std::f32::consts::TAU;
                let direction = vec2(angle.cos(), -angle.sin());
                // An alpha fringe antialiases the procedural wheel's silhouette at any DPI.
                mesh.colored_vertex(center + direction * (radius - 0.5).max(0.0), wheel_rim_color(hue, brightness));
                mesh.colored_vertex(center + direction * (radius + 0.5), Color32::TRANSPARENT);
                if i > 0 {
                    let inner = 1 + 2 * i as u32;
                    mesh.add_triangle(0, inner - 2, inner);
                    mesh.add_triangle(inner - 2, inner - 1, inner);
                    mesh.add_triangle(inner - 1, inner + 1, inner);
                }
            }
            ui.painter().add(egui::Shape::mesh(mesh));
            let angle = value.hue.to_radians();
            let handle = center + vec2(angle.cos(), -angle.sin()) * radius * value.sat.clamp(0.0, 100.0) / 100.0;
            ui.painter().circle_stroke(handle, 5.0, Stroke::new(1.5, t.field));
            ui.painter().circle_filled(center + vec2(radius + 8.0, 0.0), 5.0, t.field);
            ui.painter().circle_filled(handle, 1.5, t.canvas);
            if response.has_focus() {
                ui.painter().circle_stroke(center, radius + 3.0, Stroke::new(1.0, t.accent));
            }
        }
        changed |= wheel_luminance(ui, label, &mut value.lum, side);
    });
    changed
}

/// The five view selectors are a single row, independent of the translated tooltip width.
fn grading_selectors(ui: &mut Ui, current: &mut GradingView) {
    let design = super::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let size = design.grading_selector_size;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size), Sense::hover());
    ui.painter().text(
        pos2(row.left() + design.grading_selector_label_inset, row.center().y),
        egui::Align2::LEFT_CENTER,
        tr("Adjust"),
        egui::FontId::proportional(design.panel_font_size),
        t.text,
    );
    for (index, (mode, label)) in [
        (GradingView::ThreeWay, "Three-way"),
        (GradingView::Shadows, "Shadows"),
        (GradingView::Midtones, "Midtones"),
        (GradingView::Highlights, "Highlights"),
        (GradingView::Global, "Global"),
    ]
    .into_iter()
    .enumerate()
    {
        let grouping_gap = design.grading_selector_group_gap;
        let offset = if index == 0 {
            0.0
        } else if index == 4 {
            2.0 * grouping_gap
        } else {
            grouping_gap
        };
        let center = pos2(row.left() + design.grading_selector_start + index as f32 * (size + design.grading_selector_gap) + offset, row.center().y);
        let rect = Rect::from_center_size(center, vec2(size, size));
        let response = ui.interact(rect, ui.id().with(("grading-view", label)), Sense::click()).on_hover_text(tr(label));
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), *current == mode, tr(label)));
        if response.clicked() {
            *current = mode;
        }
        let ink = if *current == mode { t.text } else { t.icon };
        let radius = size * 0.325;
        if index == 1 || index == 4 {
            let separator_x = center.x - (size + design.grading_selector_gap + grouping_gap) / 2.0;
            ui.painter()
                .line_segment([pos2(separator_x, center.y - radius), pos2(separator_x, center.y + radius)], Stroke::new(0.7, t.icon.gamma_multiply(0.65)));
        }
        if mode == GradingView::ThreeWay {
            for offset in [vec2(0.0, -radius * 0.65), vec2(-radius * 0.65, radius * 0.5), vec2(radius * 0.65, radius * 0.5)] {
                ui.painter().circle_stroke(center + offset, radius * 0.4, Stroke::new(1.5, ink));
            }
        } else {
            ui.painter().circle_stroke(center, radius, Stroke::new(1.5, ink));
            let fraction = match mode {
                GradingView::Shadows => 0.0,
                GradingView::Midtones => 1.0,
                GradingView::Highlights => 1.0,
                _ => 0.5,
            };
            if fraction > 0.0 {
                let points = (0..=24)
                    .map(|i| {
                        let angle = -std::f32::consts::FRAC_PI_2 + i as f32 / 24.0 * std::f32::consts::TAU * fraction;
                        center + vec2(angle.cos(), angle.sin()) * radius
                    })
                    .collect();
                let fill = if mode == GradingView::Midtones { ink.gamma_multiply(0.75) } else { ink };
                ui.painter().add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
            }
            if mode == GradingView::Global {
                ui.painter().line_segment([center - vec2(0.0, radius + 2.0), center + vec2(0.0, radius + 2.0)], Stroke::new(1.5, t.dock));
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
    }
    ui.add_space(design.grading_selector_bottom_gap);
}

pub(crate) fn grading(ui: &mut Ui, p: &mut CameraRaw, state: &mut GradingView) -> bool {
    let mut dirty = false;
    grading_selectors(ui, state);
    let width = ui.available_width();
    let design = super::camera_raw_design::get(ui.ctx());
    match state {
        GradingView::ThreeWay => {
            let side = (width * design.grading_wheel_fraction).clamp(50.0, design.wheel_max_size);
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_max_width(side);
                dirty |= wheel(ui, "Midtones", &mut p.grade_midtones, side, design.grading_top_label_offset_y);
            });
            ui.add_space(super::camera_raw_design::get(ui.ctx()).grading_wheel_row_gap);
            let lower_rect =
                Rect::from_min_size(ui.cursor().min - vec2(design.grading_lower_wheel_outset, 0.0), vec2(width + 2.0 * design.grading_lower_wheel_outset, 0.0));
            let mut lower = ui.new_child(egui::UiBuilder::new().id_salt("grading-lower-wheels").max_rect(lower_rect));
            lower.set_width(lower_rect.width());
            lower.columns(2, |columns| {
                if let Some(left) = columns.first_mut() {
                    dirty |= wheel(left, "Shadows", &mut p.grade_shadows, side.min(left.available_width()), design.grading_lower_label_offset_y);
                }
                if let Some(right) = columns.get_mut(1) {
                    dirty |= wheel(right, "Highlights", &mut p.grade_highlights, side.min(right.available_width()), design.grading_lower_label_offset_y);
                }
            });
            ui.allocate_space(vec2(width, lower.min_rect().height()));
        }
        mode => {
            let (label, value) = match mode {
                GradingView::Shadows => ("Shadows", &mut p.grade_shadows),
                GradingView::Midtones => ("Midtones", &mut p.grade_midtones),
                GradingView::Highlights => ("Highlights", &mut p.grade_highlights),
                _ => ("Global", &mut p.grade_global),
            };
            dirty |= wheel(ui, label, value, width.min(220.0), 0.0);
            dirty |= slider_row(ui, tr("Hue"), &mut value.hue, 0.0..=360.0, "", Some(&widgets::hue_stops())).changed();
            dirty |= slider_row(ui, tr("Saturation"), &mut value.sat, 0.0..=100.0, "", None).changed();
        }
    }
    ui.add_space(design.grading_footer_gap);
    ui.separator();
    dirty |= slider_row_with(ui, tr("Blending"), &mut p.grade_blending, 0.0..=100.0, "", None, widgets::RowGestures { reset: Some(50.0), wheel_step: None })
        .changed();
    dirty |= slider_row_with(ui, tr("Balance"), &mut p.grade_balance, -100.0..=100.0, "", None, widgets::RowGestures { reset: Some(0.0), wheel_step: None })
        .changed();
    dirty
}

pub(crate) fn grid(ui: &Ui, image: Rect, viewport: Rect, state: &WorkspaceState) {
    if !state.grid {
        return;
    }
    let clip = image.intersect(viewport);
    if !clip.is_positive() {
        return;
    }
    let painter = ui.painter_at(clip);
    let t = Tokens::get(ui.ctx());
    let stroke = Stroke::new(1.0, t.text.gamma_multiply(state.grid_opacity.clamp(0.0, 1.0)));
    let step = if state.grid_size.is_finite() { state.grid_size.clamp(12.0, 200.0) } else { 40.0 };
    let mut x = clip.left() + (image.center().x - clip.left()).rem_euclid(step);
    for _ in 0..1024 {
        if x > clip.right() {
            break;
        }
        painter.line_segment([pos2(x, clip.top()), pos2(x, clip.bottom())], stroke);
        x += step;
    }
    let mut y = clip.top() + (image.center().y - clip.top()).rem_euclid(step);
    for _ in 0..1024 {
        if y > clip.bottom() {
            break;
        }
        painter.line_segment([pos2(clip.left(), y), pos2(clip.right(), y)], stroke);
        y += step;
    }
}

/// Only tools with working behavior are enabled. Remaining screens stay tracked in the parity brief.
pub(crate) fn rail(ui: &mut Ui, rect: Rect, state: &mut WorkspaceState, navigation: &mut crate::state::CameraRawPreviewState, sampler: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.chrome);
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, t.separator));
    let design = super::camera_raw_design::get(ui.ctx());
    let top_rect = Rect::from_min_size(pos2(rect.center().x - 14.0, rect.top() + design.rail_top_inset), vec2(28.0, 28.0));
    let mut top = ui.new_child(egui::UiBuilder::new().max_rect(top_rect));
    top.set_clip_rect(rect);
    if icons::button(&mut top, "sliders-horizontal", 28.0, !state.info && state.tab == WorkspaceTab::Edit, tl!("Edit")).clicked() {
        state.info = false;
        state.tab = WorkspaceTab::Edit;
    }
    let red_eye_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + design.snapshots_rail_offset - 36.0), vec2(28.0, 28.0));
    let mut red_eye_ui = ui.new_child(egui::UiBuilder::new().id_salt("camera-raw-red-eye-tab").max_rect(red_eye_rect));
    if icons::button(&mut red_eye_ui, "eye", 28.0, !state.info && state.tab == WorkspaceTab::RedEye, tl!("Red Eye")).clicked() {
        state.info = false;
        state.tab = WorkspaceTab::RedEye;
    }
    let snapshots_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + design.snapshots_rail_offset), vec2(28.0, 28.0));
    let mut snapshots_ui = ui.new_child(egui::UiBuilder::new().id_salt("camera-raw-snapshots-tab").max_rect(snapshots_rect));
    if icons::button(&mut snapshots_ui, "layers", 28.0, !state.info && state.tab == WorkspaceTab::Snapshots, tl!("Snapshots")).clicked() {
        state.info = false;
        state.tab = WorkspaceTab::Snapshots;
    }
    let design = super::camera_raw_design::get(ui.ctx());
    let available = (rect.bottom() - design.rail_bottom_inset - top.min_rect().bottom() - 8.0).max(1.0);
    let gap = design.rail_tool_gap.min(available / 20.0);
    let separator = design.rail_separator_gap.min(available / 10.0);
    let size = design.rail_tool_size.min((rect.width() - 8.0).max(1.0)).min(((available - 6.0 * gap - 2.0 * separator) / 5.0).max(1.0));
    let height = 5.0 * size + 6.0 * gap + 2.0 * separator;
    let bottom = Rect::from_min_size(pos2(rect.center().x - size / 2.0, rect.bottom() - design.rail_bottom_inset - height), vec2(size, height));
    let mut tools = ui.new_child(egui::UiBuilder::new().max_rect(bottom));
    tools.spacing_mut().item_spacing.y = gap;
    tools.set_clip_rect(bottom.intersect(rect));
    let separator_row = |ui: &mut Ui| {
        let (r, _) = ui.allocate_exact_size(vec2(size, separator), Sense::hover());
        ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, t.separator));
    };
    if icons::button(&mut tools, "info", size, state.info, tl!("Info")).clicked() {
        state.info = !state.info;
    }
    separator_row(&mut tools);
    let zoom_tool = icons::button(&mut tools, "zoom-in", size, !navigation.hand && !*sampler && !state.white_balance_tool, tl!("Zoom Tool"));
    if zoom_tool.double_clicked() {
        navigation.fit();
    }
    if zoom_tool.clicked() {
        navigation.hand = false;
        *sampler = false;
        state.white_balance_tool = false;
    }
    let hand_tool = icons::button(&mut tools, "hand", size, navigation.hand && !*sampler && !state.white_balance_tool, tl!("Hand Tool"));
    if hand_tool.double_clicked() {
        navigation.fit();
    }
    if hand_tool.clicked() {
        navigation.hand = true;
        *sampler = false;
        state.white_balance_tool = false;
    }
    separator_row(&mut tools);
    if icons::button(&mut tools, "pipette", size, *sampler, tl!("Color Sampler")).clicked() {
        *sampler = !*sampler;
        state.white_balance_tool = false;
    }
    if icons::button(&mut tools, "grid-3x3", size, state.grid, tl!("Grid")).clicked() {
        state.grid = !state.grid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_rejects_bad_values_without_changing_the_previous_view() {
        let initial = WorkspaceState::default();
        for bad in [
            json!({"band":8}),
            json!({"gridSize":0}),
            json!({"gridOpacity":2}),
            json!({"grading":"invalid"}),
            json!({"bypassSection":"Optics"}),
            json!({"bypassSection":"Execute"}),
            json!({"noSuchField":true}),
            json!({"panels":{"open":["Light","Light"]}}),
            json!({"panels":{"single":true,"open":["Light","Color"]}}),
        ] {
            assert!(prepare(&initial, &bad).is_err(), "{bad}");
            assert_eq!(initial, WorkspaceState::default());
        }
        let edited = prepare(&initial, &json!({"grading":"global","mixer":"color","band":5,"grid":true})).unwrap();
        assert_eq!(edited.grading, GradingView::Global);
        assert_eq!(edited.panels, initial.panels);
        assert_eq!(serde_json::from_value::<WorkspaceState>(serde_json::to_value(&edited).unwrap()).unwrap(), edited);
    }

    #[test]
    fn grading_view_selectors_fit_one_row_without_changing_the_view() {
        use egui_kittest::{Harness, kittest::Queryable};
        for width in [218.0, 270.0, 330.0] {
            let mut h = Harness::new_ui_state(
                move |ui, state: &mut GradingView| {
                    ui.set_width(width);
                    grading_selectors(ui, state);
                },
                GradingView::Highlights,
            );
            h.run();
            let first = h.get_by_label("Three-way").rect();
            for label in ["Shadows", "Midtones", "Highlights", "Global"] {
                let rect = h.get_by_label(label).rect();
                assert_eq!(rect.center().y, first.center().y, "{label} wrapped at {width}");
                assert!(rect.right() - first.left() <= width, "{label} overflowed at {width}");
            }
            let shadows = h.get_by_label("Shadows").rect();
            let midtones = h.get_by_label("Midtones").rect();
            let global = h.get_by_label("Global").rect();
            let highlights = h.get_by_label("Highlights").rect();
            assert!((shadows.center().x - first.center().x - 32.0).abs() < 0.01);
            assert!((midtones.center().x - shadows.center().x - 26.0).abs() < 0.01);
            assert!((global.center().x - highlights.center().x - 32.0).abs() < 0.01);
            assert_eq!(*h.state(), GradingView::Highlights);
        }
    }

    #[test]
    fn section_bypass_only_neutralizes_the_requested_stage_in_a_copy() {
        let mut original = CameraRaw {
            exposure: 1.5,
            contrast: 20.0,
            temperature: 12.0,
            texture: 30.0,
            curve_lights: 40.0,
            grade_balance: 25.0,
            sharpen_amount: 35.0,
            encoding_gamma: 2.2,
            pixel_scale: 0.5,
            ..Default::default()
        };
        original.hsl_hue[0] = 10.0;
        let light = preview_without(&original, Some("Light"));
        assert_eq!(light.exposure, 0.0);
        assert_eq!(light.contrast, 0.0);
        assert_eq!(light.temperature, original.temperature);
        assert_eq!(light.texture, original.texture);
        assert_eq!(light.encoding_gamma, original.encoding_gamma);
        assert_eq!(light.pixel_scale, original.pixel_scale);
        assert_eq!(original.exposure, 1.5);
        assert_eq!(preview_without(&original, Some("Color")).temperature, 0.0);
        assert_eq!(preview_without(&original, Some("Effects")).texture, 0.0);
        assert_eq!(preview_without(&original, Some("Curve")).curve_lights, 0.0);
        assert_eq!(preview_without(&original, Some("Color Mixer")).hsl_hue[0], 0.0);
        assert_eq!(preview_without(&original, Some("Color Grading")).grade_balance, 0.0);
        assert_eq!(preview_without(&original, Some("Detail")).sharpen_amount, 0.0);
        assert_eq!(preview_without(&original, None), original);
    }

    #[test]
    fn holding_a_section_eye_does_not_expand_or_edit_its_section() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut h = Harness::new_ui_state(
            |ui, state: &mut PanelState| {
                ui.set_width(255.0);
                section(ui, "Light", state, |_| {});
            },
            PanelState { open: vec![], single: false },
        );
        h.run();
        let eye = h.get_by_label("Light — Before").rect();
        h.event(egui::Event::PointerMoved(eye.center()));
        h.event(egui::Event::PointerButton { pos: eye.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.ctx.data(|data| data.get_temp::<String>(held_bypass_id())), Some("Light".into()));
        assert!(h.state().open.is_empty());
        h.event(egui::Event::PointerButton { pos: eye.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
        assert!(h.state().open.is_empty());
    }

    #[test]
    fn grading_glyphs_select_views_and_luminance_remains_keyboard_editable() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut views = Harness::new_ui_state(|ui, state: &mut GradingView| grading_selectors(ui, state), GradingView::ThreeWay);
        views.run();
        views.get_by_label("Global").click();
        views.run();
        assert_eq!(*views.state(), GradingView::Global);
        let mut track = Harness::new_ui_state(
            |ui, value: &mut f32| {
                wheel_luminance(ui, "Midtones", value, 100.0);
            },
            0.0,
        );
        track.run();
        track.get_by_label("Midtones Luminance").click();
        track.run();
        let before = *track.state();
        track.key_press(egui::Key::ArrowRight);
        track.run();
        assert_eq!(*track.state(), (before + 1.0).min(100.0));
    }

    #[test]
    fn grading_wheel_uses_display_referred_midgray_rim_without_changing_hue() {
        assert_eq!(wheel_rim_color(0.0, 0.5), Color32::from_rgb(128, 0, 0));
        assert_eq!(wheel_rim_color(1.0 / 3.0, 0.5), Color32::from_rgb(0, 128, 0));
        assert_eq!(wheel_rim_color(2.0 / 3.0, 0.5), Color32::from_rgb(0, 0, 128));
        assert_eq!(wheel_rim_color(1.0, 0.5), wheel_rim_color(0.0, 0.5));
    }

    #[test]
    fn wheel_geometry_keeps_neutral_hue_and_clamps_to_the_rim() {
        assert_eq!(hue_sat(Vec2::ZERO, 50.0, 123.0), Some((123.0, 0.0)));
        assert_eq!(hue_sat(vec2(100.0, 0.0), 50.0, 123.0), Some((0.0, 100.0)));
        assert_eq!(hue_sat(vec2(0.0, -25.0), 50.0, 0.0), Some((90.0, 50.0)));
        assert!(hue_sat(vec2(f32::NAN, 0.0), 50.0, 0.0).is_none());
        assert!(hue_sat(Vec2::ZERO, 0.0, 0.0).is_none());
    }
}
