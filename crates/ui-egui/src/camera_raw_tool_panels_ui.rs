//! Native tool-panel presentation. Tool settings here are view state until the
//! Camera Raw transaction can replay the corresponding engine operation.
use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, vec2};
use serde::{Deserialize, Serialize};

use crate::{camera_raw_design, theme::Tokens, widgets};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct RedEyeDesign {
    pub height: f32,
    pub type_top: f32,
    pub pupil_top: f32,
    pub darken_top: f32,
    pub divider_top: f32,
    pub overlay_top: f32,
}

impl Default for RedEyeDesign {
    fn default() -> Self {
        Self { height: 190.0, type_top: 41.5, pupil_top: 71.5, darken_top: 105.5, divider_top: 146.5, overlay_top: 161.5 }
    }
}

impl RedEyeDesign {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if [self.height, self.type_top, self.pupil_top, self.darken_top, self.divider_top, self.overlay_top]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=400.0).contains(v))
            || self.type_top < 28.0
            || self.pupil_top < self.type_top + 20.0
            || self.darken_top < self.pupil_top + 28.0
            || self.divider_top < self.darken_top + 28.0
            || self.overlay_top < self.divider_top + 8.0
            || self.height < self.overlay_top + 20.0
        {
            return Err("Camera Raw Red Eye geometry must be finite, ordered and contained in its panel".into());
        }
        Ok(())
    }
}

fn tr(label: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", label)
}

pub(super) fn red_eye(ui: &mut Ui) {
    let design = camera_raw_design::get(ui.ctx());
    let layout = &design.red_eye;
    let t = Tokens::get(ui.ctx());
    let (panel, _) = ui.allocate_exact_size(vec2(ui.available_width(), layout.height), Sense::hover());
    let at = |x, y| panel.min + vec2(x, y);
    super::info::heading(ui, Rect::from_min_size(at(19.0, 0.0), vec2(160.0, 40.0)), tr("Red Eye"), design.tab_heading_font_size);
    // The underlying engine has click-local correction; the Camera Raw dialog
    // does not yet store/replay it on commit. Do not expose a working Auto action.
    for (label, icon, x) in [("Reset", "undo-2", 210.0), ("Preview", "eye", 242.0)] {
        let r = Rect::from_center_size(at(x, 20.0), vec2(20.0, 20.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(label).max_rect(r));
        child.add_enabled_ui(false, |ui| {
            crate::icons::button_with_icon_size(ui, icon, 20.0, 18.0, false, tr(label));
        });
    }
    let type_y = layout.type_top;
    ui.painter().text(at(23.0, type_y + 10.0), Align2::LEFT_CENTER, tr("Type"), FontId::proportional(design.panel_font_size), t.text);
    let field = Rect::from_min_size(at(53.5, type_y), vec2((panel.width() - 159.0).max(40.0), 19.0));
    ui.painter().rect_filled(field, 1.0, t.field);
    ui.painter().rect_stroke(field, 1.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(field.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, tr("Red Eye"), FontId::proportional(design.panel_font_size), t.text);
    let arrow = field.right_center() - vec2(7.0, 0.0);
    ui.painter().add(egui::Shape::line(vec![arrow + vec2(-3.0, -1.5), arrow + vec2(0.0, 1.5), arrow + vec2(3.0, -1.5)], Stroke::new(1.0, t.text_dim)));
    let response = ui.interact(field, ui.id().with("red-eye-type"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, false, tr("Type")));
    response.on_hover_text(tr("Not available"));
    let auto = Rect::from_min_size(at(panel.width() - 69.0, type_y), vec2(51.0, 20.0));
    ui.painter().rect_filled(auto, 1.0, t.field);
    ui.painter().rect_stroke(auto, 1.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    ui.painter().text(auto.center(), Align2::CENTER_CENTER, tr("Auto"), FontId::proportional(design.panel_font_size), t.text);
    let response = ui.interact(auto, ui.id().with("red-eye-auto"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, tr("Auto")));
    response.on_hover_text(tr("Not available"));
    for (label, y) in [("Pupil Size", layout.pupil_top), ("Darken", layout.darken_top)] {
        let row = Rect::from_min_size(at(22.0, y), vec2((panel.width() - 46.0).max(1.0), 28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(label).max_rect(row));
        child.set_width(row.width());
        // Keep the measured appearance but correctly report unavailable editing
        // to accessibility and prevent pointer/keyboard changes to the recipe.
        child.style_mut().visuals.disabled_alpha = 1.0;
        let mut style = design.slider_style();
        style.label_row_height = 18.0;
        style.field_appearance.box_offset_y = -1.0;
        let track = [t.field_border, t.field_border];
        child
            .add_enabled_ui(false, |ui| {
                let mut value = 50.0;
                widgets::slider_row_prec_styled(
                    ui,
                    tr(label),
                    &mut value,
                    0.0..=100.0,
                    "",
                    Some(&track),
                    Default::default(),
                    Some(0),
                    design.value_field_width,
                    Some(style),
                );
            })
            .response
            .on_disabled_hover_text(tr("Not available"));
    }
    ui.painter().line_segment([at(15.0, layout.divider_top), at(panel.width() - 17.0, layout.divider_top)], Stroke::new(1.0, t.separator));
    let check = Rect::from_min_size(at(19.0, layout.overlay_top), vec2(12.0, 12.0));
    // ACR's selected box is neutral with a dark tick in this tool panel.
    ui.painter().rect_filled(check, 1.0, t.text_dim);
    ui.painter().add(egui::Shape::line(vec![check.min + vec2(2.0, 6.0), check.min + vec2(5.0, 9.0), check.min + vec2(10.0, 3.0)], Stroke::new(2.0, t.dock)));
    ui.painter().text(at(38.0, layout.overlay_top + 6.0), Align2::LEFT_CENTER, tr("Overlay"), FontId::proportional(design.panel_font_size), t.text_dim);
    let response = ui.interact(Rect::from_min_size(check.min, vec2(100.0, 14.0)), ui.id().with("red-eye-overlay"), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, false, true, tr("Overlay")));
    response.on_hover_text(tr("Not available"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_eye_geometry_rejects_reversed_overlapping_and_nonfinite_controls() {
        assert!(RedEyeDesign::default().validate().is_ok());
        for bad in [
            RedEyeDesign { type_top: f32::NAN, ..Default::default() },
            RedEyeDesign { darken_top: 70.0, ..Default::default() },
            RedEyeDesign { divider_top: 110.0, ..Default::default() },
            RedEyeDesign { height: 165.0, ..Default::default() },
        ] {
            assert!(bad.validate().is_err());
        }
        assert!(crate::camera_raw_design::Design::parse(r#"{"redEye":{"pupilTop":1}}"#).is_err());
        assert!(crate::camera_raw_design::Design::parse(r#"{"redEye":{"execute":"paint.redEye"}}"#).is_err());
    }
}
