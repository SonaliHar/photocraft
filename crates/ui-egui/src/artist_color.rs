//! Artist-oriented Color panel. The wheel and triangle use sRGB HSV, like the brush colours.
//! Geometry is local to this panel: no dependency on any particular docking implementation.
use crate::{PhotocraftApp, panels, theme::Tokens};
use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, vec2};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Picker {
    #[default]
    Wheel,
    Rectangle,
}

/// RGB hue geometry is independent of the document's storage colour model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarmonyMode {
    #[default]
    Related,
    Rgb,
}

#[derive(Clone, Copy)]
struct HarmonyPalette {
    start: f32,
    colors: [egui::ecolor::Hsva; 12],
    mode: HarmonyMode,
}
impl HarmonyPalette {
    fn new(color: egui::ecolor::Hsva, mode: HarmonyMode) -> Self {
        let start = if mode == HarmonyMode::Related { color.h } else { 0.0 };
        Self { start, mode, colors: std::array::from_fn(|i| egui::ecolor::Hsva::new((start + i as f32 / 12.0).rem_euclid(1.0), color.s, color.v, color.a)) }
    }
    fn index(self, point: Pos2, center: Pos2) -> usize {
        let delta = point - center;
        let hue = delta.y.atan2(delta.x) / TAU + 0.25;
        (((hue - self.start + 1.0 / 24.0).rem_euclid(1.0) * 12.0).floor() as usize).min(11)
    }
    fn label(self, index: usize) -> &'static str {
        if self.mode == HarmonyMode::Rgb {
            return match index % 4 {
                0 => tl!("Primary Color"),
                2 => tl!("Secondary Color"),
                _ => tl!("Tertiary Color"),
            };
        }
        match index {
            0 => tl!("Selected Color"),
            1 | 2 | 10 | 11 => tl!("Analogous Color"),
            6 => tl!("Complementary / Opposite Color"),
            4 | 8 => tl!("Triadic Color"),
            3 | 9 => tl!("Tetradic Color"),
            _ => tl!("Split Complementary Color"),
        }
    }
}

fn active(app: &PhotocraftApp) -> [f32; 4] {
    if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground }
}

fn set_color(app: &mut PhotocraftApp, color: [f32; 4]) {
    if !color.iter().all(|v| v.is_finite()) {
        return;
    }
    let color = color.map(|v| v.clamp(0.0, 1.0));
    if app.ui.color_panel.background {
        app.session.tools.background = color;
    } else {
        app.session.tools.foreground = color;
        crate::type_tool::foreground_changed(app);
    }
}

fn remember(app: &mut PhotocraftApp, color: [f32; 4]) {
    if !color.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
        return;
    }
    let mut recent = [None; 12];
    if let Some(first) = recent.first_mut() {
        *first = Some(color);
    }
    let mut out = recent.iter_mut().skip(1);
    for old in app.ui.color_panel.recent.iter().flatten() {
        if old.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) && panels::srgb_bytes(*old) != panels::srgb_bytes(color) {
            let Some(slot) = out.next() else { break };
            *slot = Some(*old);
        }
    }
    app.ui.color_panel.recent = recent;
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // The dock supplies a bounded body. Reserve the editable footer before sizing the
    // selector: scrolling the entire colour panel must never hide HSB or recent colours.
    let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
    let previous = (app.ui.color_panel.background, active(app));
    header(app, ui);
    let background = app.ui.color_panel.background;
    let before = if previous.0 == background { previous.1 } else { active(app) };
    let observed_id = ui.id().with(("artist-observed-color", background));
    let observed: Option<[f32; 4]> = ui.data(|d| d.get_temp(observed_id));
    let remaining = ui.available_rect_before_wrap().intersect(area);
    if !remaining.is_positive() {
        return;
    }
    let width = remaining.width().max(1.0);
    let numeric_hsb = remaining.height() < 220.0;
    let horizontal = remaining.height() < 180.0 && width >= 260.0;
    let controls_width = if horizontal { 168.0 } else { width };
    let sliders_height = if app.ui.color_panel.sliders { if numeric_hsb { 24.0 } else { 76.0 } } else { 0.0 };
    let recent_height = if app.ui.color_panel.show_recent { 24.0 } else { 0.0 };
    let footer_height: f32 = sliders_height + recent_height + 4.0;
    let selector_height = (remaining.height() - footer_height).max(0.0);
    let selector_rect = if horizontal {
        Rect::from_min_size(remaining.min, vec2((width - controls_width - 6.0).max(1.0), remaining.height().max(0.0)))
    } else {
        Rect::from_min_size(remaining.min, vec2(width, selector_height))
    };
    let footer_rect = if horizontal {
        Rect::from_min_max(egui::pos2(selector_rect.right() + 6.0, remaining.top()), remaining.max)
    } else {
        Rect::from_min_max(egui::pos2(remaining.left(), selector_rect.bottom()), remaining.max)
    };
    let hue_key = Id::new(("color-field-hue", background));
    if selector_rect.height() > 1.0 {
        // Keep the rectangular field clear of the header, history and dock edges.
        let selector_rect = if app.ui.color_panel.picker == Picker::Rectangle {
            selector_rect.shrink2(vec2(6.0_f32.min(selector_rect.width() * 0.1), 6.0_f32.min(selector_rect.height() * 0.1)))
        } else {
            selector_rect
        };
        let mut selector_ui = ui.new_child(egui::UiBuilder::new().id_salt("color-selector").max_rect(selector_rect));
        selector_ui.set_clip_rect(selector_rect.intersect(ui.clip_rect()));
        if app.ui.color_panel.picker == Picker::Wheel {
            let color = active(app);
            let mut hsv = panels::srgb_hsva(color);
            let last: (f32, f32) = ui.data(|d| d.get_temp(hue_key)).unwrap_or((hsv.h, hsv.s));
            if hsv.v < 0.01 {
                hsv.h = last.0;
                hsv.s = last.1;
            } else if hsv.s < 0.01 {
                hsv.h = last.0;
            }
            let diameter = selector_rect.width().min(selector_rect.height());
            selector_ui.add_space(((selector_rect.height() - diameter) * 0.5).max(0.0));
            if wheel(
                &mut selector_ui,
                &mut hsv,
                diameter,
                app.ui.color_panel.rotate_triangle,
                app.ui.color_panel.harmony_ring.then_some(app.ui.color_panel.harmony_mode),
            ) {
                let mut color = panels::hsva_srgb(hsv);
                color[3] = active(app)[3];
                set_color(app, color);
                ui.data_mut(|d| {
                    d.insert_temp(hue_key, (hsv.h, hsv.s));
                    d.remove::<egui::ecolor::Hsva>(Id::new(("color-picker-hsva", background)));
                });
            }
        } else {
            panels::rectangular_color_field(app, &mut selector_ui, selector_rect.height());
        }
    }
    let mut footer_ui = ui.new_child(egui::UiBuilder::new().id_salt("color-controls").max_rect(footer_rect));
    footer_ui.set_clip_rect(footer_rect.intersect(ui.clip_rect()));
    footer_ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
    if app.ui.color_panel.sliders {
        if numeric_hsb {
            panels::hsb_sliders_sized(app, &mut footer_ui, true);
        } else {
            panels::hsb_sliders(app, &mut footer_ui);
        }
    }
    if app.ui.color_panel.show_recent {
        history(app, &mut footer_ui);
    }
    ui.advance_cursor_after_rect(remaining);
    let after = active(app);
    let pending = ui.id().with(("recent-color-pending", background));
    if before != after || observed.is_some_and(|color| color != before) {
        ui.data_mut(|d| d.insert_temp(pending, true));
    }
    // Commit a drag once it ends, instead of filling history with every intermediate colour.
    if !ui.input(|i| i.pointer.primary_down()) && ui.data(|d| d.get_temp::<bool>(pending)).unwrap_or(false) {
        remember(app, after);
        ui.data_mut(|d| d.remove::<bool>(pending));
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(observed_id, after));
}

/// Anchor each group independently; narrow docks move only the readout to a new row.
fn header(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let tokens = Tokens::get(ui.ctx());
    let width = ui.available_width().max(1.0);
    const READOUT: f32 = 254.0;
    let inline = width >= 414.0;
    let readout_width = width.min(READOUT);
    let readout_height = if readout_width >= READOUT {
        24.0
    } else if readout_width >= 138.0 {
        50.0
    } else {
        102.0
    };
    let height = if inline { 28.0 } else { 30.0 + readout_height };
    let (bounds, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let rect = Rect::from_min_size(bounds.min, vec2(42.0, 28.0));
    let bg = Rect::from_min_size(rect.min + vec2(16.0, 7.0), vec2(22.0, 20.0));
    let fg = Rect::from_min_size(rect.min + vec2(2.0, 1.0), vec2(25.0, 23.0));
    for (r, color, selected) in
        [(bg, app.session.tools.background, app.ui.color_panel.background), (fg, app.session.tools.foreground, !app.ui.color_panel.background)]
    {
        let [r8, g8, b8] = panels::srgb_bytes(color);
        ui.painter().rect_filled(r, 3.0, Color32::from_rgb(r8, g8, b8));
        ui.painter().rect_stroke(
            r,
            3.0,
            Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { tokens.accent } else { tokens.field_border }),
            StrokeKind::Outside,
        );
    }
    let bg_response = ui.interact(bg, ui.id().with("artist-bg"), Sense::click()).on_hover_text(tl!("Background Color"));
    let fg_response = ui.interact(fg, ui.id().with("artist-fg"), Sense::click()).on_hover_text(tl!("Foreground Color"));
    fg_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Foreground Color")));
    bg_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Background Color")));
    if fg_response.clicked() {
        app.ui.color_panel.background = false;
    } else if bg_response.clicked() {
        app.ui.color_panel.background = true;
    }
    if fg_response.double_clicked() || bg_response.double_clicked() {
        crate::color_picker_ui::open(app, if app.ui.color_panel.background { "background" } else { "foreground" });
    }
    let swap = Rect::from_min_size(bounds.min + vec2(46.0, 2.0), vec2(24.0, 24.0));
    let response = ui.interact(swap, ui.id().with("artist-swap"), Sense::click()).on_hover_text(tl!("Swap colours (X)"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Swap colours (X)")));
    let ink = crate::icons::button_chrome(ui, swap, false, response.hovered());
    crate::swap_arrow::paint(ui.painter(), swap.center() - vec2(6.5, 6.5), ink);
    if response.clicked() && app.run("tools.swapColors", serde_json::json!({})).is_ok() {
        crate::type_tool::foreground_changed(app);
    }
    for (index, picker, label) in [(0, Picker::Wheel, tl!("Triangle")), (1, Picker::Rectangle, tl!("Square"))] {
        let rect = Rect::from_min_size(egui::pos2(bounds.right() - 52.0 + index as f32 * 28.0, bounds.top() + 2.0), vec2(24.0, 24.0));
        let selected = app.ui.color_panel.picker == picker;
        let response = ui.interact(rect, ui.id().with(("artist-picker", index)), Sense::click()).on_hover_text(label);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label));
        let ink = crate::icons::button_chrome(ui, rect, selected, response.hovered());
        let icon = rect.shrink(6.0);
        if picker == Picker::Wheel {
            ui.painter()
                .add(egui::Shape::closed_line(vec![icon.left_bottom(), egui::pos2(icon.center().x, icon.top()), icon.right_bottom()], Stroke::new(1.5, ink)));
        } else {
            ui.painter().rect_stroke(icon, 0.0, Stroke::new(1.5, ink), StrokeKind::Inside);
        }
        if response.clicked() {
            app.ui.color_panel.picker = picker;
        }
    }
    // The centre stays centred in the panel until it would touch either edge group.
    let x = if inline {
        (bounds.center().x - READOUT * 0.5).clamp(bounds.left() + 78.0, bounds.right() - 60.0 - READOUT)
    } else {
        bounds.center().x - readout_width * 0.5
    };
    let readout = Rect::from_min_size(egui::pos2(x, bounds.top() + if inline { 2.0 } else { 30.0 }), vec2(readout_width, readout_height));
    let mut fields = ui.new_child(egui::UiBuilder::new().id_salt("color-header-readout").max_rect(readout));
    fields.set_clip_rect(readout.intersect(ui.clip_rect()));
    let mut color = active(app);
    if panels::color_readout(&mut fields, ui.id().with(("color-field", app.ui.color_panel.background)), &mut color) {
        set_color(app, color);
    }
}

/// The existing panel ellipsis owns presentation options in both native and custom docks.
pub(crate) fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.ui.color_panel.sliders, tl!("HSB Sliders"));
    ui.checkbox(&mut app.ui.color_panel.show_recent, tl!("Recent Colors"));
    ui.add_enabled_ui(app.ui.color_panel.picker == Picker::Wheel, |ui| {
        ui.checkbox(&mut app.ui.color_panel.rotate_triangle, tl!("Rotate Triangle"));
        ui.checkbox(&mut app.ui.color_panel.harmony_ring, tl!("12-Color Ring"));
        ui.add_enabled_ui(app.ui.color_panel.harmony_ring, |ui| {
            ui.menu_button(tl!("Harmony Mode"), |ui| {
                ui.selectable_value(&mut app.ui.color_panel.harmony_mode, HarmonyMode::Related, tl!("Related Colors"));
                ui.selectable_value(&mut app.ui.color_panel.harmony_mode, HarmonyMode::Rgb, tl!("RGB Circle"));
            });
        });
    });
}

fn marker(ui: &egui::Ui, position: Pos2, radius: f32) {
    ui.painter().circle_stroke(position, radius + 1.5, Stroke::new(2.0, Color32::from_black_alpha(210)));
    ui.painter().circle_stroke(position, radius, Stroke::new(1.5, Color32::WHITE));
}

fn direction(hue: f32) -> Vec2 {
    // Red starts at the top, matching a traditional painting wheel's orientation.
    let angle = hue * TAU - TAU * 0.25;
    vec2(angle.cos(), angle.sin())
}

fn triangle(center: Pos2, radius: f32, hue: Option<f32>) -> [Pos2; 3] {
    // The pure-hue vertex follows the ring marker; white and black rotate with it.
    // Fixed mode keeps the original right-pointing geometry.
    let axis = hue.map(direction).unwrap_or(vec2(1.0, 0.0));
    let perpendicular = vec2(-axis.y, axis.x);
    [center + axis * radius, center + (axis * -0.5 - perpendicular * 0.8660254) * radius, center + (axis * -0.5 + perpendicular * 0.8660254) * radius]
}

fn weights(point: Pos2, [a, b, c]: [Pos2; 3]) -> [f32; 3] {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = point - a;
    let denominator = v0.x * v1.y - v1.x * v0.y;
    if denominator.abs() < 0.001 {
        return [1.0, 0.0, 0.0];
    }
    let white = (v2.x * v1.y - v1.x * v2.y) / denominator;
    let black = (v0.x * v2.y - v2.x * v0.y) / denominator;
    [1.0 - white - black, white, black]
}

fn nearest_segment(point: Pos2, start: Pos2, end: Pos2) -> Pos2 {
    let delta = end - start;
    let fraction = ((point - start).dot(delta) / delta.length_sq().max(0.001)).clamp(0.0, 1.0);
    start + delta * fraction
}

fn projected_weights(point: Pos2, vertices: [Pos2; 3]) -> [f32; 3] {
    let values = weights(point, vertices);
    if values.iter().all(|v| *v >= 0.0) {
        return values;
    }
    let [a, b, c] = vertices;
    let mut nearest = a;
    let mut distance = f32::INFINITY;
    for (start, end) in [(a, b), (b, c), (c, a)] {
        let candidate = nearest_segment(point, start, end);
        let d = candidate.distance_sq(point);
        if d < distance {
            nearest = candidate;
            distance = d;
        }
    }
    weights(nearest, vertices).map(|v| v.clamp(0.0, 1.0))
}

fn wheel(ui: &mut egui::Ui, hsv: &mut egui::ecolor::Hsva, diameter: f32, rotate: bool, harmony: Option<HarmonyMode>) -> bool {
    let width = ui.available_width();
    let (area, response) = ui.allocate_exact_size(vec2(width, diameter), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tl!("Color Wheel")));
    let center = area.center();
    let outer = (diameter * 0.5 - 4.0).max(1.0);
    let inner = (outer - (diameter * 0.075).clamp(10.0, 25.0)).max(1.0);
    let palette_outer = (inner - 3.0).max(1.0);
    let palette_inner = (palette_outer - (diameter * 0.09).clamp(14.0, 30.0)).max(1.0);
    let radius = (if harmony.is_some() { palette_inner - 6.0 } else { inner - 7.0 }).max(1.0);
    let vertices = triangle(center, radius, rotate.then_some(hsv.h));
    let drag_id = response.id.with("component");
    let palette_id = response.id.with("harmony-gesture");
    let mut changed = false;
    // Fix the component at pointer-down. Crossing the ring during a triangle drag must not
    // unexpectedly change the hue (and vice versa). Starting in the gap does nothing.
    if ui.input(|i| i.pointer.primary_pressed()) && response.contains_pointer() {
        if let Some(point) = response.interact_pointer_pos().filter(|p| p.is_finite()) {
            let distance = point.distance(center);
            let component = if distance >= inner && distance <= outer + 3.0 {
                1_u8
            } else if harmony.is_some() && distance >= palette_inner && distance <= palette_outer {
                3
            } else if weights(point, vertices).iter().all(|v| *v >= 0.0) {
                2
            } else {
                0
            };
            ui.data_mut(|d| {
                d.insert_temp(drag_id, component);
                if component == 3
                    && let Some(mode) = harmony
                {
                    d.insert_temp(palette_id, HarmonyPalette::new(*hsv, mode));
                }
            });
        }
    }
    if (response.dragged() || response.clicked() || ui.input(|i| i.pointer.primary_pressed()) && response.contains_pointer())
        && let Some(point) = response.interact_pointer_pos().filter(|p| p.is_finite())
    {
        match ui.data(|d| d.get_temp::<u8>(drag_id)).unwrap_or(0) {
            1 => {
                let delta = point - center;
                if delta.length_sq() > 0.01 {
                    hsv.h = (delta.y.atan2(delta.x) / TAU + 0.25).rem_euclid(1.0);
                    changed = true;
                }
            }
            2 => {
                let [hue, white, _] = projected_weights(point, vertices);
                hsv.v = (hue + white).clamp(0.0, 1.0);
                if hsv.v > 0.0001 {
                    hsv.s = (hue / hsv.v).clamp(0.0, 1.0);
                }
                changed = true;
            }
            3 => {
                if let Some(palette) = ui.data(|d| d.get_temp::<HarmonyPalette>(palette_id))
                    && let Some(color) = palette.colors.get(palette.index(point, center))
                {
                    *hsv = *color;
                    changed = true;
                }
            }
            _ => {}
        }
    }
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| {
            d.remove::<u8>(drag_id);
            d.remove::<HarmonyPalette>(palette_id);
        });
    }
    // Ring drags may have changed hue this frame. Use the new orientation for both
    // the mesh and selection marker immediately, without a one-frame lag.
    let vertices = triangle(center, radius, rotate.then_some(hsv.h));
    let mut ring = egui::Mesh::default();
    for step in 0..=144 {
        let hue = step as f32 / 144.0;
        let color = panels::hsva_color32(egui::ecolor::Hsva::new(hue, 1.0, 1.0, 1.0));
        ring.colored_vertex(center + direction(hue) * inner, color);
        ring.colored_vertex(center + direction(hue) * outer, color);
    }
    for step in 0..144_u32 {
        let a = step * 2;
        ring.add_triangle(a, a + 1, a + 3);
        ring.add_triangle(a, a + 3, a + 2);
    }
    ui.painter().add(ring);
    if let Some(mode) = harmony {
        let palette = ui.data(|d| d.get_temp::<HarmonyPalette>(palette_id)).unwrap_or_else(|| HarmonyPalette::new(*hsv, mode));
        paint_harmonies(ui, center, palette_inner, palette_outer, palette);
        if let Some(point) = response.hover_pos().filter(|p| p.is_finite())
            && (palette_inner..=palette_outer).contains(&point.distance(center))
        {
            let index = palette.index(point, center);
            if let Some(color) = palette.colors.get(index) {
                let [r, g, b] = panels::srgb_bytes(panels::hsva_srgb(*color));
                response.clone().on_hover_text(format!("{}\n#{r:02X}{g:02X}{b:02X}", palette.label(index)));
            }
        }
    }
    let [hue_vertex, white_vertex, black_vertex] = vertices;
    let at = |hue: f32, white: f32| black_vertex + (hue_vertex - black_vertex) * hue + (white_vertex - black_vertex) * white;
    let mut mesh = egui::Mesh::default();
    // Separate small triangles interpolate encoded RGB (the same space as the readouts).
    // Fixed subdivision bounds both allocations and per-frame rendering cost.
    for row in 0..32 {
        for column in 0..(32 - row) {
            let h = row as f32 / 32.0;
            let w = column as f32 / 32.0;
            let step = 1.0 / 32.0;
            let mut draw = |points: [(f32, f32); 3]| {
                let offset = mesh.vertices.len() as u32;
                for (hue, white) in points {
                    let value = (hue + white).clamp(0.0, 1.0);
                    let saturation = if value > 0.0001 { hue / value } else { hsv.s };
                    mesh.colored_vertex(at(hue, white), panels::hsva_color32(egui::ecolor::Hsva::new(hsv.h, saturation, value, 1.0)));
                }
                mesh.add_triangle(offset, offset + 1, offset + 2);
            };
            draw([(h, w), (h + step, w), (h, w + step)]);
            if row + column < 31 {
                draw([(h + step, w), (h + step, w + step), (h, w + step)]);
            }
        }
    }
    ui.painter().add(mesh);
    let tokens = Tokens::get(ui.ctx());
    ui.painter().add(egui::Shape::closed_line(vertices.to_vec(), Stroke::new(1.0, tokens.separator)));
    let hue_marker = center + direction(hsv.h) * ((inner + outer) * 0.5);
    if rotate {
        // Draw beneath the marker, linking the pure-hue vertex to its exact hue on the ring.
        let pointer = [hue_vertex, hue_marker];
        ui.painter().line_segment(pointer, Stroke::new(3.5, Color32::from_black_alpha(210)));
        ui.painter().line_segment(pointer, Stroke::new(1.5, Color32::WHITE));
    }
    marker(ui, hue_marker, ((outer - inner) * 0.34).clamp(3.0, 7.0));
    marker(ui, at(hsv.s * hsv.v, (1.0 - hsv.s) * hsv.v), 4.5);
    changed
}

fn paint_harmonies(ui: &egui::Ui, center: Pos2, inner: f32, outer: f32, palette: HarmonyPalette) {
    let tokens = Tokens::get(ui.ctx());
    for (index, color) in palette.colors.into_iter().enumerate() {
        let middle = palette.start + index as f32 / 12.0;
        let start = middle - 1.0 / 24.0;
        let mut mesh = egui::Mesh::default();
        let color = panels::hsva_color32(color);
        for step in 0..=12 {
            let angle = start + step as f32 / 144.0;
            mesh.colored_vertex(center + direction(angle) * inner, color);
            mesh.colored_vertex(center + direction(angle) * outer, color);
        }
        for step in 0..12_u32 {
            let a = step * 2;
            mesh.add_triangle(a, a + 1, a + 3);
            mesh.add_triangle(a, a + 3, a + 2);
        }
        ui.painter().add(mesh);
        ui.painter().line_segment([center + direction(start) * inner, center + direction(start) * outer], Stroke::new(1.0, tokens.separator));
    }
    ui.painter().circle_stroke(center, inner, Stroke::new(1.0, tokens.separator));
    ui.painter().circle_stroke(center, outer, Stroke::new(1.0, tokens.separator));
}

fn history(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let tokens = Tokens::get(ui.ctx());

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        save_recent_menu(app, ui);
        let size = ((ui.available_width() - 22.0) / 12.0).clamp(1.0, 64.0);
        let latest = app.ui.color_panel.recent.iter().position(|color| valid_recent(*color).is_some());
        for (index, color) in app.ui.color_panel.recent.into_iter().enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(size, 24.0), Sense::click());
            let valid = valid_recent(color);
            let fill = valid
                .map(|c| {
                    let [r, g, b] = panels::srgb_bytes(c);
                    Color32::from_rgb(r, g, b)
                })
                .unwrap_or(tokens.field);
            ui.painter().rect_filled(rect, 3.0, fill);
            ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);
            if latest == Some(index) {
                // A dark inset separates the accent outline even from a matching swatch colour.
                ui.painter().rect_stroke(rect.shrink(1.0), 3.0, Stroke::new(3.0, tokens.field), StrokeKind::Inside);
                ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, tokens.accent), StrokeKind::Inside);
            }
            if let Some(color) = valid {
                let [r, g, b] = panels::srgb_bytes(color);
                let hex = format!("#{r:02X}{g:02X}{b:02X}");
                let tip = if latest == Some(index) { format!("{hex} — {}", tl!("Latest color: saved with +")) } else { hex };
                let response = response.on_hover_text(tip);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("#{r:02X}{g:02X}{b:02X}")));
                if response.clicked() {
                    set_color(app, color);
                }
            }
        }
    });
}

fn valid_recent(color: Option<[f32; 4]>) -> Option<[f32; 4]> {
    color.filter(|c| c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)))
}

/// Save exactly the most recent swatch, using the existing preset commands and persistence.
fn save_recent_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let latest = app.ui.color_panel.recent.iter().find_map(|color| valid_recent(*color));
    ui.add_enabled_ui(latest.is_some(), |ui| {
        ui.menu_button("+", |ui| {
            let Some(color) = latest else { return };
            let [r, g, b] = panels::srgb_bytes(color);
            let hex = format!("#{r:02X}{g:02X}{b:02X}");
            ui.label(tl!("Save latest color to Swatches"));
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(r, g, b));
                ui.label(&hex);
            });
            ui.separator();
            ui.label(tl!("Existing group"));
            let groups: Vec<String> = app.session.presets.swatches.iter().map(|group| group.name.clone()).collect();
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for group in groups {
                    if ui.button(&group).clicked() && save_recent_swatch(app, &group, &hex) {
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.label(tl!("New group"));
            let id = ui.id().with("recent-swatch-new-group");
            let mut name = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_default();
            ui.add(egui::TextEdit::singleline(&mut name).hint_text(tl!("Group name")).char_limit(128).desired_width(180.0));
            ui.data_mut(|data| data.insert_temp(id, name.clone()));
            if ui.add_enabled(!name.trim().is_empty(), egui::Button::new(tl!("Create group and save color"))).clicked()
                && let Some(result) = crate::preset_panels::run(app, "swatches.newGroup", serde_json::json!({"name": name.trim()}))
                && let Some(group) = result.get("group").and_then(serde_json::Value::as_str)
                && save_recent_swatch(app, group, &hex)
            {
                ui.data_mut(|data| data.remove::<String>(id));
                ui.close();
            }
        })
        .response
        .on_hover_text(tl!("Save latest color to Swatches"));
    });
}

fn save_recent_swatch(app: &mut PhotocraftApp, group: &str, hex: &str) -> bool {
    let Some(result) = crate::preset_panels::run(app, "swatches.add", serde_json::json!({"group": group, "name": hex, "color": hex})) else { return false };
    if let Some(name) = result.get("name").and_then(serde_json::Value::as_str) {
        app.ui.presets_ui.selected.insert(crate::swatches_ui::PANEL.into(), name.to_string());
    }
    app.ui.presets_ui.toggled.remove(&format!("swatches/{group}"));
    app.ui.status_error = false;
    app.ui.status = crate::i18n::fmt(tl!("Saved color {color} to group {group}"), &[("color", hex), ("group", group)]);
    true
}
