//! Camera Raw Info layout, measured against the supplied reference 7.
//! Values come from the open document; unavailable metadata stays unspecified.
use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::CameraRawDialog;
use crate::theme::Tokens;

fn file_kind(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map_or_else(|| "—".into(), |ext| ext.chars().take(8).collect::<String>().to_uppercase())
}

/// Paint from the measured left edge, independently of inherited child-layout justification.
pub(super) fn heading(ui: &mut Ui, rect: Rect, label: &str, size: f32) {
    let response = ui.interact(rect, ui.id().with(label), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label));
    ui.painter().with_clip_rect(rect).text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        crate::theme::semibold(size),
        Tokens::get(ui.ctx()).text,
    );
}

pub(super) fn show(ui: &mut Ui, dialog: &CameraRawDialog) {
    let design = crate::camera_raw_design::get(ui.ctx());
    let tokens = Tokens::get(ui.ctx());
    let width = ui.available_width();
    let card_width = (width - design.info_left_inset - design.info_right_inset).max(1.0);
    ui.add_space(design.info_top_inset);
    let (row, _) = ui.allocate_exact_size(vec2(width, design.info_card_height), Sense::hover());
    let card = Rect::from_min_size(row.min + vec2(design.info_left_inset, 0.0), vec2(card_width, row.height()));
    ui.painter().rect_filled(card, design.info_card_radius, tokens.field);
    let model = dialog.camera_info.model.as_deref().filter(|value| !value.trim().is_empty()).unwrap_or("—");
    ui.painter().with_clip_rect(card.shrink(8.0)).text(
        card.min + vec2(12.0, 15.0),
        Align2::LEFT_CENTER,
        model,
        FontId::proportional(design.panel_font_size),
        tokens.text,
    );
    let dimensions = format!("{} x {}", dialog.full_w, dialog.full_h);
    let summary = dialog
        .raw_open
        .as_ref()
        .and_then(|raw| raw.bytes.as_ref())
        .map_or_else(|| dimensions.clone(), |bytes| format!("{dimensions} · {:.1} MB", bytes.len() as f64 / 1_000_000.0));
    let kind = file_kind(dialog.raw_open.as_ref().map_or(dialog.document_name.as_str(), |raw| raw.name.as_str()));
    let badge = Rect::from_min_size(pos2(card.right() - 44.0, card.bottom() - 26.0), vec2(36.0, 18.0));
    ui.painter().rect_filled(badge, tokens.radius * 2.0, tokens.text_faint);
    ui.painter().with_clip_rect(badge).text(badge.center(), Align2::CENTER_CENTER, kind, FontId::proportional(design.panel_font_size), tokens.field);
    let text = ui.painter().with_clip_rect(Rect::from_min_max(card.min, pos2(badge.left() - 5.0, card.bottom())));
    text.text(card.min + vec2(12.0, 33.0), Align2::LEFT_CENTER, summary, FontId::proportional(design.panel_font_size), tokens.text);
    ui.add_space((design.info_group_gap - 4.0).max(0.0));
    let date = dialog.camera_info.date_taken.as_deref().filter(|value| !value.trim().is_empty()).unwrap_or("—");
    for (index, (label, value)) in [(tl!("Capture Date"), date), (tl!("File Name"), dialog.document_name.as_str())].into_iter().enumerate() {
        if index > 0 {
            ui.add_space(design.info_field_gap);
        }
        let (row, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
        let text_rect = Rect::from_min_max(row.min + vec2(design.info_text_inset, 0.0), row.max);
        heading(ui, text_rect, label, design.info_heading_font_size);
        ui.add_space(4.0);
        let (body, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
        ui.painter().with_clip_rect(body.shrink2(vec2(design.info_text_inset, 0.0))).text(
            pos2(body.left() + design.info_text_inset, body.center().y),
            Align2::LEFT_CENTER,
            value,
            FontId::proportional(design.panel_font_size),
            tokens.text,
        );
    }
    ui.add_space(design.info_divider_top_gap);
    let (divider, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().line_segment([divider.left_center(), pos2(divider.right() - design.info_right_inset, divider.center().y)], Stroke::new(1.0, tokens.separator));
    ui.add_space(design.info_credentials_top_gap);
    let (heading, response) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    let label = tl!("Content Credentials");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label));
    let galley = ui.painter().layout_no_wrap(label.into(), crate::theme::semibold(design.info_heading_font_size), tokens.text);
    let at = pos2(heading.left() + design.info_text_inset, heading.center().y - galley.size().y / 2.0);
    let badge = Rect::from_min_size(pos2(at.x + galley.size().x + 8.0, heading.center().y - 7.5), vec2(15.0, 15.0));
    ui.painter().with_clip_rect(heading).galley(at, galley, tokens.text);
    ui.painter().rect_stroke(badge.shrink(1.0), 4.0, Stroke::new(1.2, tokens.text_dim), egui::StrokeKind::Inside);
    ui.painter().with_clip_rect(badge).text(badge.center(), Align2::CENTER_CENTER, "cr", FontId::proportional(9.0), tokens.text_dim);
    ui.add_space(4.0);
    let (body, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    ui.painter().text(
        pos2(body.left() + design.info_text_inset, body.center().y),
        Align2::LEFT_CENTER,
        "—",
        FontId::proportional(design.panel_font_size),
        tokens.text,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_keep_the_measured_left_edge_in_wide_or_justified_layouts() {
        for width in [220.0, 270.0, 400.0] {
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, Default::default());
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().layout(egui::Layout::top_down_justified(egui::Align::Center)));
                heading(&mut child, Rect::from_min_size(pos2(20.0, 40.0), vec2(width, 20.0)), "Capture Date", 12.0);
            });
            output.textures_delta.clear();
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Capture Date" => Some(text),
                    _ => None,
                })
                .expect("heading was painted");
            assert_eq!(text.pos.x, 20.0, "heading centered at dock width {width}");
            assert_eq!(text.galley.job.sections.first().unwrap().format.font_id.family, egui::FontFamily::Name("semibold".into()));
        }
    }

    #[test]
    fn file_badge_uses_actual_extension_without_fabricating_metadata() {
        assert_eq!(file_kind("photo.nef"), "NEF");
        assert_eq!(file_kind("photo.pNg"), "PNG");
        assert_eq!(file_kind("Untitled"), "—");
        assert_eq!(file_kind("image.abcdefghijklmnop"), "ABCDEFGH");
    }
}
