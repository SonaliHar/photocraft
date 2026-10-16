//! Native Camera Raw chrome observed in the supplied ACR references.
//! Kept under camera_raw_ui so view controls reuse its existing edit/history path.
use super::CameraRawDialog;
use crate::{camera_raw_workspace_ui::CurveChannel, icons, theme::Tokens};
use egui::{Align2, Color32, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

fn button(ui: &mut Ui, rect: Rect, label: &str, selected: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let response = ui.interact(rect, ui.id().with(label), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label));
    let fill = if enabled && response.hovered() {
        t.hover
    } else if selected {
        t.accent_soft
    } else {
        t.field
    };
    ui.painter().rect(rect, 0.0, fill, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), if enabled { t.text } else { t.text_faint });
    response
}

pub(super) fn metadata(ui: &mut Ui, info: &photocraft_algo::exif::CameraInfo) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), design.metadata_height), Sense::hover());
    response.on_hover_text(crate::camera_raw_workspace_ui::capture_summary(info));
    let fields = metadata_fields(info);
    for (field, center) in fields.iter().zip([0.12, 0.42, 0.68, 0.88]) {
        let x = rect.left() + rect.width() * center;
        ui.painter().text(pos2(x, rect.center().y), Align2::CENTER_CENTER, field, FontId::proportional(10.5), t.text_dim);
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
}

fn metadata_fields(info: &photocraft_algo::exif::CameraInfo) -> [String; 4] {
    let positive = |n: Option<f64>| n.filter(|n| n.is_finite() && *n > 0.0);
    let iso = positive(info.iso).map_or_else(|| "ISO —".into(), |n| format!("ISO {n:.0}"));
    let focal = positive(info.focal_length).map_or_else(
        || "— mm".into(),
        |n| {
            let range =
                info.lens.as_deref().and_then(|s| s.split_whitespace().find(|s| s.contains('-') && s.chars().filter(char::is_ascii_digit).count() >= 2));
            range.map_or_else(|| format!("{n:.0} mm"), |range| format!("{}@{n:.0} mm", range.trim_end_matches("mm")))
        },
    );
    let aperture = positive(info.f_number).map_or_else(|| "f/—".into(), |n| format!("f/{n}"));
    let exposure = positive(info.exposure_time).map_or_else(
        || "— s".into(),
        |n| {
            if n < 1.0 && n.recip().is_finite() { format!("1/{:.0}s", n.recip()) } else { format!("{n}s") }
        },
    );
    [iso, focal, aperture, exposure]
}

pub(super) fn edit_bar(ui: &mut Ui, d: &mut CameraRawDialog, info: bool) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.edit_bar_height), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + design.bar_inset, rect.center().y),
        Align2::LEFT_CENTER,
        if info { tl!("Info") } else { tl!("Edit") },
        FontId::proportional(13.0),
        t.text,
    );
    if !info {
        let y = rect.center().y - 10.0;
        let hdr = Rect::from_min_size(pos2(rect.right() - design.section_right_padding - 32.0, y), vec2(32.0, 20.0));
        button(ui, hdr, "HDR", false, false).on_hover_text(tl!("HDR editing is not available"));
        let bw = hdr.translate(vec2(-40.0, 0.0));
        if button(ui, bw, "B&W", d.params.saturation == -100.0, true).clicked() {
            let id = ui.id().with(("cr-color-saturation", d.layer.0, d.document_name.as_str(), d.raw_open.as_ref().and_then(|raw| raw.path.as_deref())));
            if d.params.saturation == -100.0 {
                d.params.saturation = ui.ctx().data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0);
            } else {
                ui.ctx().data_mut(|data| data.insert_temp(id, d.params.saturation));
                d.params.saturation = -100.0;
            }
            d.dirty = true;
        }
        let auto = bw.translate(vec2(-38.0, 0.0));
        button(ui, auto, tl!("Auto"), false, false).on_hover_text(tl!("Automatic Camera Raw tone adjustment is not available"));
        ui.painter().line_segment([pos2(hdr.left() - 4.0, y), pos2(hdr.left() - 4.0, y + 20.0)], Stroke::new(1.0, t.separator));
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
}

pub(super) fn white_balance(ui: &mut Ui, p: &mut photocraft_algo::camera_raw::CameraRaw, picker: &mut bool) -> bool {
    let design = crate::camera_raw_design::get(ui.ctx());
    let before = (p.temperature, p.tint);
    let mut custom = p.temperature != 0.0 || p.tint != 0.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_sized(vec2(design.white_balance_label_width, 20.0), egui::Label::new(tl!("White balance")));
        egui::ComboBox::from_id_salt("cr-white-balance")
            .width((ui.available_width() - 28.0).max(55.0))
            .selected_text(if custom { tl!("Custom") } else { tl!("As Shot") })
            .show_ui(ui, |ui| {
                if ui.selectable_value(&mut custom, false, tl!("As Shot")).clicked() {
                    p.temperature = 0.0;
                    p.tint = 0.0;
                }
                ui.add_enabled(false, egui::Label::new(tl!("Custom")));
            });
        if icons::button_with_icon_size(ui, "pipette", 22.0, 18.0, *picker, tl!("White Balance Tool")).clicked() {
            *picker = !*picker;
        }
    });
    ui.add_space(6.0);
    before != (p.temperature, p.tint)
}

/// Solve the existing relative white-balance gains for one neutral sample.
/// This uses the document transfer curve and never treats the slider as Kelvin.
fn balance_for_sample(rgb: [f32; 3], gamma: f32) -> Option<(f32, f32)> {
    if rgb.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let [r, g, b] = rgb.map(|v| if gamma.is_finite() && gamma > 0.0 { v.powf(gamma.clamp(0.5, 4.0)) } else { photocraft_color::convert::srgb_to_linear(v) });
    if [r, g, b].iter().any(|v| !v.is_finite() || *v <= f32::EPSILON) {
        return None;
    }
    let temperature = 100.0 * (b - r) / (0.35 * (r + b));
    let common = 2.0 * r * b / (r + b);
    let tint = 400.0 * (1.0 - common / g);
    if !temperature.is_finite() || !tint.is_finite() {
        return None;
    }
    Some((temperature.clamp(-100.0, 100.0), tint.clamp(-100.0, 100.0)))
}

pub(super) fn pick_white_balance(ui: &Ui, d: &mut CameraRawDialog, image: Rect, response: &Response, picker: &mut bool, panning: bool) {
    if !*picker {
        return;
    }
    response.clone().on_hover_cursor(egui::CursorIcon::Crosshair);
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        *picker = false;
        return;
    }
    if panning || !response.clicked_by(egui::PointerButton::Primary) {
        return;
    }
    let Some(at) = response.interact_pointer_pos().filter(|at| image.contains(*at)) else {
        return;
    };
    if d.pw == 0 || d.ph == 0 {
        return;
    }
    let x = (((at.x - image.left()) / image.width().max(1.0)) * d.pw as f32).floor().max(0.0) as usize;
    let y = (((at.y - image.top()) / image.height().max(1.0)) * d.ph as f32).floor().max(0.0) as usize;
    let Some(index) = y.min(d.ph - 1).checked_mul(d.pw).and_then(|row| row.checked_add(x.min(d.pw - 1))) else {
        return;
    };
    let Some([r, g, b, alpha]) = d.proxy.get(index).copied() else {
        return;
    };
    if alpha <= 0.0 || !alpha.is_finite() {
        return;
    }
    if let Some((temperature, tint)) = balance_for_sample([r, g, b], d.params.encoding_gamma) {
        d.params.temperature = temperature;
        d.params.tint = tint;
        d.dirty = true;
        *picker = false;
    }
}

pub(super) fn profile_bar(ui: &mut Ui) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.profile_bar_height), Sense::hover());
    ui.painter().text(pos2(rect.left() + design.bar_inset, rect.center().y), Align2::LEFT_CENTER, tl!("Profile"), FontId::proportional(12.0), t.text);
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(rect.left() + 65.0, rect.center().y - 10.0), vec2((rect.width() - 132.0).max(60.0), 20.0))),
    );
    child.add_enabled_ui(false, |ui| {
        egui::ComboBox::from_id_salt("cr-profile")
            .width((rect.width() - 132.0).max(60.0))
            .selected_text(tl!("Embedded"))
            .show_ui(ui, |ui| {
                ui.label(tl!("Embedded"));
            })
            .response
            .on_hover_text(tl!("Additional camera profiles are not available"));
    });
    let browser = Rect::from_center_size(pos2(rect.right() - design.section_right_padding - 10.0, rect.center().y), vec2(20.0, 20.0));
    let response = ui
        .interact(browser, ui.id().with("cr-profile-browser"), Sense::hover())
        .on_hover_text(tl!("Browse Profiles — additional camera profiles are not available"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, tl!("Browse Profiles")));
    let min = browser.min + vec2(2.0, 2.0);
    for delta in [vec2(0.0, 0.0), vec2(7.0, 0.0), vec2(0.0, 7.0)] {
        ui.painter().rect_filled(Rect::from_min_size(min + delta, vec2(5.0, 5.0)), 0.0, t.icon);
    }
    let center = min + vec2(10.0, 10.0);
    ui.painter().circle_stroke(center, 3.0, Stroke::new(1.5, t.icon));
    ui.painter().line_segment([center + vec2(2.0, 2.0), center + vec2(5.0, 5.0)], Stroke::new(1.5, t.icon));
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
}

pub(super) fn curve_selectors(ui: &mut Ui, current: &mut CurveChannel) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), design.curve_selector_size + 8.0), Sense::hover());
    ui.painter().text(pos2(row.left(), row.center().y), Align2::LEFT_CENTER, tl!("Adjust"), FontId::proportional(12.0), t.text);
    for (index, (channel, name, color)) in [
        (CurveChannel::Parametric, "Parametric", t.icon),
        (CurveChannel::Master, "RGB", t.icon),
        (CurveChannel::Red, "Red", Color32::from_rgb(195, 36, 50)),
        (CurveChannel::Green, "Green", Color32::from_rgb(52, 160, 67)),
        (CurveChannel::Blue, "Blue", Color32::from_rgb(25, 120, 229)),
    ]
    .into_iter()
    .enumerate()
    {
        let center = pos2(row.left() + 42.0 + index as f32 * (design.curve_selector_size + design.curve_selector_gap), row.center().y);
        let rect = Rect::from_center_size(center, vec2(design.curve_selector_size, design.curve_selector_size));
        let label = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", name);
        let response = ui.interact(rect, ui.id().with(name), Sense::click()).on_hover_text(label);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), *current == channel, label));
        if response.clicked() {
            *current = channel;
        }
        if channel == CurveChannel::Parametric {
            for offset in [-3.0, 0.0, 3.0] {
                let points = (0..=12)
                    .map(|i| {
                        let f = i as f32 / 12.0;
                        center + vec2(-6.0 + 12.0 * f + offset, 7.0 - 14.0 * f + (f * std::f32::consts::TAU).sin() * 2.0)
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(points, Stroke::new(1.4, color)));
            }
        } else {
            ui.painter().circle_stroke(center, design.curve_selector_size * 0.34, Stroke::new(2.0, color));
            if *current == channel {
                ui.painter().circle_filled(center, 2.0, t.text);
            }
        }
        if response.has_focus() {
            ui.painter().rect_stroke(rect, 1.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
    }
}

pub(super) fn top_actions(ui: &mut Ui, title: Rect, action: &mut Option<&'static str>, scope: &mut crate::state::CameraRawScopeState) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let width = 4.0 * design.title_action_size + 3.0 * design.title_action_gap + 8.0;
    let rect = Rect::from_min_size(pos2(title.right() - width, title.top()), vec2(width, title.height()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = design.title_action_gap;
    // Draw the observed download glyph independently; there is no bundled download icon.
    let (download, response) = child.allocate_exact_size(vec2(design.title_action_size, design.title_action_size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, child.is_enabled(), tl!("Save")));
    let center = download.center();
    child.painter().line_segment([center + vec2(0.0, -7.0), center + vec2(0.0, 2.0)], Stroke::new(1.8, t.icon));
    child.painter().add(egui::Shape::line(vec![center + vec2(-4.0, -1.0), center + vec2(0.0, 3.0), center + vec2(4.0, -1.0)], Stroke::new(1.8, t.icon)));
    child.painter().add(egui::Shape::line(
        vec![center + vec2(-7.0, 0.0), center + vec2(-7.0, 7.0), center + vec2(7.0, 7.0), center + vec2(7.0, 0.0)],
        Stroke::new(1.8, t.icon),
    ));
    egui::Popup::menu(&response).show(|ui| settings_menu(ui, action));
    let settings = icons::button_with_icon_size(&mut child, "settings", design.title_action_size, 18.0, false, tl!("Camera Raw Settings"));
    let divider = settings.rect.right() + design.title_action_gap * 0.5;
    child.painter().line_segment([pos2(divider, title.center().y - 10.0), pos2(divider, title.center().y + 10.0)], Stroke::new(1.0, t.separator));
    egui::Popup::menu(&settings).show(|ui| {
        ui.checkbox(&mut scope.shadows, tl!("Shadow Clipping Warning (U)"));
        ui.checkbox(&mut scope.highlights, tl!("Highlight Clipping Warning (O)"));
        ui.checkbox(&mut scope.lab, tl!("Show Lab Color Readouts"));
        ui.checkbox(&mut scope.vectorscope, tl!("Show Vectorscope"));
        ui.separator();
        settings_menu(ui, action);
    });
    child.add_enabled_ui(false, |ui| {
        icons::button_with_icon_size(ui, "play", design.title_action_size, 18.0, false, tl!("Tutorials are not available"));
    });
    let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
    if icons::button_with_icon_size(&mut child, "maximize-2", design.title_action_size, 18.0, fullscreen, tl!("Toggle Full Screen")).clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
    }
}

fn settings_menu(ui: &mut Ui, action: &mut Option<&'static str>) {
    for (label, id) in
        [(tl!("Reset to Open"), "reset-open"), (tl!("Reset to Default"), "reset-default"), (tl!("Load Settings…"), "load"), (tl!("Save Settings…"), "save")]
    {
        if ui.button(label).clicked() {
            *action = Some(id);
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_row_preserves_empty_columns_without_fabricating_camera_data() {
        assert_eq!(metadata_fields(&Default::default()), ["ISO —", "— mm", "f/—", "— s"]);
        let info = photocraft_algo::exif::CameraInfo {
            iso: Some(160.0),
            focal_length: Some(30.0),
            lens: Some("24-70mm".into()),
            f_number: Some(8.0),
            exposure_time: Some(1.0 / 320.0),
            ..Default::default()
        };
        assert_eq!(metadata_fields(&info), ["ISO 160", "24-70@30 mm", "f/8", "1/320s"]);
        let hostile = photocraft_algo::exif::CameraInfo {
            iso: Some(f64::NAN),
            focal_length: Some(-1.0),
            f_number: Some(f64::INFINITY),
            exposure_time: Some(0.0),
            ..Default::default()
        };
        assert_eq!(metadata_fields(&hostile), ["ISO —", "— mm", "f/—", "— s"]);
    }
    #[test]
    fn neutral_picker_solves_the_existing_gain_model_and_rejects_bad_samples() {
        let rgb = [0.4, 0.5, 0.6];
        let (temperature, tint) = balance_for_sample(rgb, 1.0).unwrap();
        let gains = photocraft_algo::camera_raw::white_balance_gains(temperature, tint);
        let balanced = std::array::from_fn::<_, 3, _>(|i| rgb[i] * gains[i]);
        assert!((balanced[0] - balanced[1]).abs() < 1e-5);
        assert!((balanced[1] - balanced[2]).abs() < 1e-5);
        assert!(balance_for_sample([f32::NAN, 0.5, 0.5], 0.0).is_none());
        assert!(balance_for_sample([0.0, 0.0, 0.0], 0.0).is_none());
        let (temperature, tint) = balance_for_sample([0.5; 3], 0.0).unwrap();
        assert!(temperature.abs() < 1e-4 && tint.abs() < 1e-4);
    }
}
