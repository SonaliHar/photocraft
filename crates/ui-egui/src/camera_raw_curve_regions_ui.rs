//! Native parametric-curve region boundaries. These edit the existing Camera Raw
//! parameters; merely painting or navigating this control never changes them.
use egui::{Rect, Sense, Ui, pos2, vec2};

use crate::{camera_raw_design, theme::Tokens};

pub(super) fn show(ui: &mut Ui, splits: &mut [f32; 3]) -> bool {
    let design = camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    ui.add_space(design.curve_splits_top_gap);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.curve_splits_height), Sense::hover());
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), t.canvas);
    mesh.colored_vertex(rect.right_top(), t.text_faint);
    mesh.colored_vertex(rect.right_bottom(), t.text_faint);
    mesh.colored_vertex(rect.left_bottom(), t.canvas);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
    let mut changed = false;
    for index in 0..3 {
        let Some(value) = splits.get(index).copied() else { continue };
        let value = if value.is_finite() { value.clamp(5.0, 95.0) } else { 50.0 };
        let center = pos2(egui::lerp(rect.x_range(), value / 100.0), rect.center().y);
        let hit = Rect::from_center_size(center, vec2(14.0, rect.height().max(14.0)));
        let response = ui.interact(hit, ui.id().with(("curve-region", index)), Sense::click_and_drag());
        let (left, right) = match index {
            0 => (tl!("Shadows"), tl!("Darks")),
            1 => (tl!("Darks"), tl!("Lights")),
            _ => (tl!("Lights"), tl!("Highlights")),
        };
        response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(value), format!("{left} / {right}")));
        let mut next = value;
        if (response.clicked() || response.dragged())
            && let Some(at) = response.interact_pointer_pos()
        {
            next = (at.x - rect.left()) / rect.width().max(1.0) * 100.0;
            response.request_focus();
        }
        if response.double_clicked() {
            next = (index as f32 + 1.0) * 25.0;
        }
        if response.has_focus() {
            let step = if ui.input(|i| i.modifiers.shift) { 10.0 } else { 1.0 };
            for (key, delta) in [(egui::Key::ArrowLeft, -step), (egui::Key::ArrowRight, step)] {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key) || i.consume_key(egui::Modifiers::SHIFT, key)) {
                    next += delta;
                }
            }
        }
        let low = index.checked_sub(1).and_then(|i| splits.get(i)).copied().filter(|v| v.is_finite()).map_or(5.0, |v| (v + 1.0).clamp(5.0, 94.0));
        let high = splits.get(index + 1).copied().filter(|v| v.is_finite()).map_or(95.0, |v| (v - 1.0).clamp(6.0, 95.0));
        if next.is_finite()
            && low <= high
            && (next - value).abs() > f32::EPSILON
            && let Some(target) = splits.get_mut(index)
        {
            let bounded = next.clamp(low, high);
            if (*target - bounded).abs() > f32::EPSILON {
                *target = bounded;
                changed = true;
            }
        }
        let x = splits.get(index).copied().filter(|v| v.is_finite()).unwrap_or(value).clamp(5.0, 95.0);
        let center = pos2(egui::lerp(rect.x_range(), x / 100.0), rect.center().y);
        ui.painter().circle_filled(center, 4.5, if response.has_focus() { t.text } else { t.text_faint });
        ui.painter().circle_filled(center, 2.75, t.shadow.to_opaque());
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal).on_hover_text(format!("{left} / {right}: {x:.0}%"));
    }
    ui.add_space(design.curve_splits_bottom_gap);
    changed
}
