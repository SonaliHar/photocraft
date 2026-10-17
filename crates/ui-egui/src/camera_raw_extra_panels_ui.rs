//! Native layouts observed in ACR references 8–11. Unsupported processing is
//! explicitly disabled; these controls cannot write fictitious recipe settings.
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::{icons, theme::Tokens, widgets};

fn tr(text: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", text)
}

/// The observed enhancement controls are unavailable without an enhancement
/// backend. Their checkbox semantics and blank amount field remain explicit.
pub(super) fn detail_enhancements(ui: &mut Ui) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.detail_enhancements_height), Sense::hover());
    for (label, y) in [("Denoise", 15.0), ("Raw Details", 72.0), ("Super Resolution", 93.0)] {
        let row = Rect::from_min_size(rect.min + vec2(0.0, y - 8.0), vec2(rect.width(), 16.0));
        let response = ui.interact(row, ui.id().with(("enhancement", label)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, false, false, tr(label)));
        response.on_hover_text(tr("Not available"));
        ui.painter().rect_stroke(
            Rect::from_center_size(pos2(rect.left() + 7.0, rect.top() + y), vec2(12.0, 12.0)),
            2.0,
            Stroke::new(1.0, t.text_faint.gamma_multiply(0.65)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(pos2(rect.left() + 20.0, rect.top() + y), Align2::LEFT_CENTER, tr(label), FontId::proportional(design.panel_font_size), t.text_dim);
    }
    let field =
        Rect::from_min_size(pos2(rect.right() - design.value_field_width, rect.top() + 26.5), vec2(design.value_field_width, design.slider_field_height));
    ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let y = rect.top() + 49.5;
    ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(3.0, t.field_border));
    let y = rect.top() + 112.5;
    ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, t.separator));
}

pub(super) fn noise_disclosure(ui: &mut Ui, expanded: &mut bool) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), design.slider_label_row_height + design.slider_track_height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), *expanded, tr("Manual Noise Reduction")));
    if response.clicked() {
        *expanded = !*expanded;
    }
    ui.painter().text(row.left_center(), Align2::LEFT_CENTER, tr("Manual Noise Reduction"), FontId::proportional(design.panel_font_size), t.text_dim);
    let center = pos2(row.right() - design.disclosure_button_size / 2.0, row.center().y);
    let points = if *expanded {
        vec![center + vec2(-4.5, -3.0), center + vec2(4.5, -3.0), center + vec2(0.0, 3.0)]
    } else {
        vec![center + vec2(3.0, -4.5), center + vec2(-3.0, 0.0), center + vec2(3.0, 4.5)]
    };
    ui.painter().add(egui::Shape::convex_polygon(points, t.text_dim, Stroke::NONE));
}

fn unavailable(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    ui.add_enabled_ui(false, body).response.on_disabled_hover_text(tr("Not available"));
}

fn checkbox(ui: &mut Ui, label: &str) {
    unavailable(ui, |ui| {
        let mut value = false;
        widgets::checkbox(ui, &mut value, tr(label));
    });
}

fn space(ui: &mut Ui) {
    ui.add_space(crate::camera_raw_design::get(ui.ctx()).extra_control_gap);
}

fn disclosure(ui: &mut Ui, label: &str, icon: &str) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), design.extra_disclosure_height), Sense::hover());
    let rect = row.expand2(vec2(design.extra_disclosure_outset, 0.0));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, tr(label)));
    response.on_disabled_hover_text(tr("Not available"));
    ui.painter().rect_filled(rect, t.radius * 2.0, t.field);
    let glyph = Rect::from_min_size(rect.min + vec2(7.0, 4.0), vec2(22.0, 22.0));
    if icon == "defringe" {
        ui.painter().rect_filled(glyph, t.radius, t.text_faint.gamma_multiply(0.25));
        for index in 0..4 {
            let step = index as f32;
            let rect = Rect::from_center_size(glyph.center() + vec2(-6.0 + step * 4.0, 6.0 - step * 4.0), vec2(4.0, 4.0));
            ui.painter().rect_filled(rect, 0.0, t.text_faint.gamma_multiply(0.45 + step * 0.15));
        }
    } else {
        let mut icon_ui = ui.new_child(egui::UiBuilder::new().id_salt(label).max_rect(glyph));
        icon_ui.disable();
        icons::button(&mut icon_ui, icon, 22.0, false, tr(label));
    }
    ui.painter().text(pos2(rect.left() + 39.0, rect.center().y), Align2::LEFT_CENTER, tr(label), FontId::proportional(design.panel_font_size), t.text_faint);
    let x = rect.right() - 12.0;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(x + 3.0, rect.center().y - 4.5), pos2(x - 3.0, rect.center().y), pos2(x + 3.0, rect.center().y + 4.5)],
        t.text_faint,
        Stroke::NONE,
    ));
}

fn slider(ui: &mut Ui, label: &str, gradient: &[Color32]) {
    let style = crate::camera_raw_design::get(ui.ctx()).slider_style();
    let width = crate::camera_raw_design::get(ui.ctx()).value_field_width;
    unavailable(ui, |ui| {
        let mut value = 0.0;
        widgets::slider_row_prec_styled(ui, tr(label), &mut value, -100.0..=100.0, "", Some(gradient), Default::default(), Some(0), width, Some(style));
    });
}

/// Lens Blur has empty disabled fields, not a fabricated zero-valued adjustment.
fn empty_slider(ui: &mut Ui, label: &str) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width();
    let height = design.slider_label_row_height + design.slider_track_height;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, false, tr(label)));
    ui.painter().text(
        pos2(rect.left(), rect.top() + design.slider_label_row_height / 2.0),
        Align2::LEFT_CENTER,
        tr(label),
        FontId::proportional(design.panel_font_size),
        t.text_faint.gamma_multiply(0.65),
    );
    let field = Rect::from_min_size(
        pos2(rect.right() - design.value_field_width, rect.top() + design.value_field_offset_y),
        vec2(design.value_field_width, design.slider_field_height),
    );
    ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let y = rect.bottom() - design.slider_track_height / 2.0;
    ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(3.0, t.field_border));
}

fn optics(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let design = crate::camera_raw_design::get(ui.ctx());
    let (tabs, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.optics_tabs_height), Sense::hover());
    for (offset, label, selected) in [(0.0, "Profile", true), (47.0, "Manual", false)] {
        let rect = Rect::from_min_size(tabs.min + vec2(offset, 0.0), vec2(44.0, 24.0));
        let response = ui.interact(rect, ui.id().with(label), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, false, selected, tr(label)));
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            tr(label),
            FontId::proportional(design.panel_font_size),
            if selected { t.text } else { t.text_faint },
        );
    }
    ui.painter().line_segment([tabs.left_bottom(), tabs.right_bottom()], Stroke::new(1.0, t.separator));
    ui.painter().line_segment([tabs.left_bottom(), tabs.left_bottom() + vec2(34.0, 0.0)], Stroke::new(1.0, t.text_dim));
    space(ui);
    checkbox(ui, "Remove chromatic aberration");
    ui.add_space(design.optics_checkbox_gap);
    checkbox(ui, "Use profile corrections");
    ui.add_space(design.optics_divider_gap);
    let (divider, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([divider.left_center(), divider.right_center()], Stroke::new(1.0, t.separator));
    ui.add_space(design.optics_divider_gap);
    disclosure(ui, "Defringe", "defringe");
}

fn lens_blur(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let design = crate::camera_raw_design::get(ui.ctx());
    checkbox(ui, "Apply");
    space(ui);
    empty_slider(ui, "Blur Amount");
    space(ui);
    ui.label(egui::RichText::new(tr("Bokeh")).color(t.text_faint.gamma_multiply(0.65)));
    let side = design.bokeh_selector_size.min(ui.available_width() / 5.0);
    let gap = ((ui.available_width() - side * 5.0) / 4.0).max(0.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), side + 8.0), Sense::hover());
    for (index, label) in ["Circle", "Bubble", "5-Blade", "Ring", "Cat Eye"].into_iter().enumerate() {
        let rect = Rect::from_min_size(row.min + vec2(index as f32 * (side + gap), 2.0), vec2(side, side));
        let response = ui.interact(rect, ui.id().with(("bokeh", index)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, false, false, tr(label)));
        ui.painter().rect_stroke(rect, t.radius * 3.0, Stroke::new(1.5, t.field_border), egui::StrokeKind::Inside);
        for (offset, radius) in [(vec2(2.0, -1.0), side * 0.27), (vec2(-5.0, 6.0), side * 0.15)] {
            let center = rect.center() + offset;
            if index == 2 {
                let points = (0..5)
                    .map(|n| {
                        let a = n as f32 / 5.0 * std::f32::consts::TAU;
                        center + vec2(a.cos(), a.sin()) * radius
                    })
                    .collect();
                ui.painter().add(egui::Shape::convex_polygon(points, t.text_faint.gamma_multiply(0.65), Stroke::NONE));
            } else if index == 1 || index == 3 {
                ui.painter().circle_stroke(center, radius, Stroke::new(if index == 3 { 3.0 } else { 1.0 }, t.text_faint.gamma_multiply(0.65)));
            } else if index == 4 {
                ui.painter().add(egui::Shape::ellipse_filled(center, vec2(radius * 0.55, radius), t.text_faint.gamma_multiply(0.65)));
            } else {
                ui.painter().circle_filled(center, radius, t.text_faint.gamma_multiply(0.65));
            }
        }
    }
    space(ui);
    empty_slider(ui, "Cat Eye");
    empty_slider(ui, "Bokeh Boost");
    space(ui);
    ui.separator();
    space(ui);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tr("Focus Range")).color(t.text_faint.gamma_multiply(0.65)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.disable();
            focus_icon(ui, false);
            focus_icon(ui, true);
        });
    });
    let (range, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, false, tr("Focus Range")));
    ui.painter().rect_filled(range, 0.0, t.separator);
    space(ui);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tr("Near")).color(t.text_faint.gamma_multiply(0.65)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(tr("Far")).color(t.text_faint.gamma_multiply(0.65)));
        });
    });
    space(ui);
    checkbox(ui, "Visualize Depth");
    space(ui);
    disclosure(ui, "Refinement", "brush");
}

/// Original procedural glyphs: the shared icon set has no crosshair or subject
/// icon, and silently substitutes a square for unknown names.
fn focus_icon(ui: &mut Ui, subject: bool) {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, tr(if subject { "Subject" } else { "Target" })));
    let center = rect.center();
    let stroke = Stroke::new(1.5, t.text_faint.gamma_multiply(0.65));
    if subject {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = center + vec2(x * 9.0, y * 9.0);
            ui.painter().line_segment([corner, corner - vec2(x * 4.0, 0.0)], stroke);
            ui.painter().line_segment([corner, corner - vec2(0.0, y * 4.0)], stroke);
        }
        ui.painter().circle_filled(center - vec2(0.0, 3.0), 3.5, stroke.color);
        ui.painter().add(egui::Shape::ellipse_filled(center + vec2(0.0, 4.0), vec2(6.0, 3.0), stroke.color));
    } else {
        ui.painter().circle_stroke(center, 9.0, stroke);
        for direction in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
            ui.painter().line_segment([center + direction * 3.0, center + direction * 8.0], stroke);
        }
        ui.painter().circle_filled(center, 1.5, stroke.color);
    }
}

fn calibration(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let design = crate::camera_raw_design::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(tr("Process"));
        ui.add_enabled_ui(false, |ui| {
            egui::ComboBox::from_id_salt("raw-calibration-process").width(ui.available_width()).selected_text(tr("Version 6 (Current)")).show_ui(ui, |_| {});
        })
        .response
        .on_disabled_hover_text(tr("Not available"));
    });
    space(ui);
    ui.separator();
    space(ui);
    calibration_heading(ui, "Shadows");
    slider(ui, "Tint", &[super::hue_color(120.0), super::hue_color(300.0)]);
    for (label, hue, ends) in [("Red Primary", 0.0, [300.0, 30.0]), ("Green Primary", 120.0, [60.0, 160.0]), ("Blue Primary", 240.0, [180.0, 270.0])] {
        ui.add_space(design.calibration_section_gap);
        ui.separator();
        space(ui);
        calibration_heading(ui, label);
        slider(ui, "Hue", &[super::hue_color(ends[0]), super::hue_color(ends[1])]);
        slider(ui, "Saturation", &[t.text_faint, super::hue_color(hue)]);
    }
}

fn calibration_heading(ui: &mut Ui, label: &str) {
    let design = crate::camera_raw_design::get(ui.ctx());
    ui.add_space(design.calibration_title_leading);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), design.calibration_title_height), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), tr(label)));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, tr(label), FontId::proportional(design.panel_font_size), Tokens::get(ui.ctx()).text);
}

pub(super) fn show(ui: &mut Ui, title: &str) {
    match title {
        "Optics" => optics(ui),
        "Lens Blur" => lens_blur(ui),
        "Calibration" => calibration(ui),
        _ => {}
    }
}
