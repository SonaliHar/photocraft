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
}
impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
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
        }
    }
}

pub const SECTIONS: [&str; 7] = ["Light", "Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail"];

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
            let name = if *expanded { "chevron-down" } else { "chevron-left" };
            if icons::button_with_icon_size(
                ui,
                name,
                design.disclosure_button_size,
                design.disclosure_button_size * 0.5,
                false,
                &format!("{} {}", tr(label), tl!("Details")),
            )
            .clicked()
            {
                *expanded = !*expanded;
            }
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
                            left: design.section_padding as i8,
                            right: design.section_right_padding as i8,
                            top: design.section_vertical_padding as i8,
                            bottom: design.section_bottom_padding as i8,
                        })
                        .show(ui, body);
                })
        })
        .inner;
    let header = &result.header_response;
    ui.painter().line_segment([header.rect.left_top(), header.rect.right_top()], Stroke::new(1.0, t.separator));
    // With explicit serde state, egui animates the view but never owns which panels are open.
    if header.clicked() {
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

fn wheel(ui: &mut Ui, label: &str, value: &mut Wheel, side: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.vertical_centered(|ui| {
        ui.label(tr(label));
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
            const N: usize = 96;
            mesh.colored_vertex(center, Color32::from_gray(128));
            for i in 0..=N {
                let hue = i as f32 / N as f32;
                let angle = hue * std::f32::consts::TAU;
                let at = center + vec2(angle.cos(), -angle.sin()) * radius;
                mesh.colored_vertex(at, Color32::from(egui::ecolor::Hsva::new(hue, 1.0, 0.8, 1.0)));
                if i > 0 {
                    mesh.add_triangle(0, i as u32, i as u32 + 1);
                }
            }
            ui.painter().add(egui::Shape::mesh(mesh));
            let angle = value.hue.to_radians();
            let handle = center + vec2(angle.cos(), -angle.sin()) * radius * value.sat.clamp(0.0, 100.0) / 100.0;
            ui.painter().circle_stroke(handle, 5.0, Stroke::new(1.5, t.text));
            ui.painter().circle_filled(handle, 1.5, t.canvas);
            if response.has_focus() {
                ui.painter().circle_stroke(center, radius + 3.0, Stroke::new(1.0, t.accent));
            }
        }
        let r = slider_row_with(ui, tr("Luminance"), &mut value.lum, -100.0..=100.0, "", None, widgets::RowGestures { reset: Some(0.0), wheel_step: None });
        changed |= r.changed();
    });
    changed
}

pub(crate) fn grading(ui: &mut Ui, p: &mut CameraRaw, state: &mut GradingView) -> bool {
    let mut dirty = false;
    ui.horizontal_wrapped(|ui| {
        for (mode, label) in [
            (GradingView::ThreeWay, "Three-way"),
            (GradingView::Shadows, "Shadows"),
            (GradingView::Midtones, "Midtones"),
            (GradingView::Highlights, "Highlights"),
            (GradingView::Global, "Global"),
        ] {
            if widgets::pill_tab(ui, tr(label), *state == mode).clicked() {
                *state = mode;
            }
        }
    });
    let width = ui.available_width();
    match state {
        GradingView::ThreeWay => {
            let side = (width * 0.48).clamp(50.0, super::camera_raw_design::get(ui.ctx()).wheel_max_size);
            ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_max_width(side);
                dirty |= wheel(ui, "Midtones", &mut p.grade_midtones, side);
            });
            ui.columns(2, |columns| {
                if let Some(left) = columns.first_mut() {
                    dirty |= wheel(left, "Shadows", &mut p.grade_shadows, side.min(left.available_width()));
                }
                if let Some(right) = columns.get_mut(1) {
                    dirty |= wheel(right, "Highlights", &mut p.grade_highlights, side.min(right.available_width()));
                }
            });
        }
        mode => {
            let (label, value) = match mode {
                GradingView::Shadows => ("Shadows", &mut p.grade_shadows),
                GradingView::Midtones => ("Midtones", &mut p.grade_midtones),
                GradingView::Highlights => ("Highlights", &mut p.grade_highlights),
                _ => ("Global", &mut p.grade_global),
            };
            dirty |= wheel(ui, label, value, width.min(220.0));
            dirty |= slider_row(ui, tr("Hue"), &mut value.hue, 0.0..=360.0, "", Some(&widgets::hue_stops())).changed();
            dirty |= slider_row(ui, tr("Saturation"), &mut value.sat, 0.0..=100.0, "", None).changed();
        }
    }
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
    let mut top = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 8.0))));
    top.set_clip_rect(rect);
    if icons::button(&mut top, "sliders-horizontal", 28.0, !state.info, tl!("Edit")).clicked() {
        state.info = false;
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
    fn wheel_geometry_keeps_neutral_hue_and_clamps_to_the_rim() {
        assert_eq!(hue_sat(Vec2::ZERO, 50.0, 123.0), Some((123.0, 0.0)));
        assert_eq!(hue_sat(vec2(100.0, 0.0), 50.0, 123.0), Some((0.0, 100.0)));
        assert_eq!(hue_sat(vec2(0.0, -25.0), 50.0, 0.0), Some((90.0, 50.0)));
        assert!(hue_sat(vec2(f32::NAN, 0.0), 50.0, 0.0).is_none());
        assert!(hue_sat(Vec2::ZERO, 0.0, 0.0).is_none());
    }
}
