//! Live Camera Raw visual design. Native debug previews can reload JSON without a rebuild.
//! No document parameters, executable actions or filesystem destinations are accepted here.
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Design {
    pub panel_width: f32,
    pub rail_width: f32,
    pub header_height: f32,
    pub navigation_height: f32,
    pub footer_height: f32,
    pub preview_margin: f32,
    pub section_height: f32,
    pub section_text_inset: f32,
    pub section_icon_inset: f32,
    pub section_font_size: f32,
    pub section_padding: f32,
    pub section_right_padding: f32,
    pub section_vertical_padding: f32,
    pub section_bottom_padding: f32,
    pub panel_font_size: f32,
    pub value_field_width: f32,
    pub value_font_size: f32,
    pub value_monospace: bool,
    pub value_text_left_aligned: bool,
    pub value_text_padding: f32,
    pub value_text_offset_y: f32,
    pub value_field_offset_y: f32,
    pub row_gap: f32,
    pub slider_field_height: f32,
    pub slider_label_row_height: f32,
    pub slider_track_inset: f32,
    pub slider_track_height: f32,
    pub slider_inner_gap: f32,
    pub slider_bottom_gap: f32,
    pub triangle_handles: bool,
    pub wheel_max_size: f32,
    pub disclosure_button_size: f32,
    pub disclosure_button_gap: f32,
    pub navigation_control_height: f32,
    pub navigation_fit_width: f32,
    pub navigation_zoom_width: f32,
    pub navigation_control_gap: f32,
    pub rail_tool_size: f32,
    pub rail_tool_gap: f32,
    pub rail_bottom_inset: f32,
    pub rail_separator_gap: f32,
    pub histogram_height: f32,
    pub histogram_corner_size: f32,
    pub metadata_height: f32,
    pub edit_bar_height: f32,
    pub profile_bar_height: f32,
    pub bar_inset: f32,
    pub title_action_size: f32,
    pub title_action_gap: f32,
    pub curve_selector_size: f32,
    pub curve_selector_gap: f32,
    pub white_balance_label_width: f32,
    pub panel_order: Vec<String>,
    /// Optional view preset, applied only when this value changes in the JSON.
    pub preview: Option<crate::camera_raw_workspace_ui::WorkspaceState>,
}

impl Default for Design {
    fn default() -> Self {
        Self {
            panel_width: 270.0,
            rail_width: 36.0,
            header_height: 32.0,
            navigation_height: 32.0,
            footer_height: 48.0,
            preview_margin: 8.0,
            section_height: 34.0,
            section_text_inset: 44.0,
            section_icon_inset: 27.0,
            section_font_size: 13.0,
            section_padding: 14.0,
            section_right_padding: 16.0,
            section_vertical_padding: 0.0,
            section_bottom_padding: 24.0,
            panel_font_size: 12.0,
            value_field_width: 45.0,
            value_font_size: 12.0,
            value_monospace: false,
            value_text_left_aligned: true,
            value_text_padding: 4.0,
            value_text_offset_y: 1.0,
            value_field_offset_y: 1.0,
            row_gap: 1.0,
            slider_field_height: 18.0,
            slider_label_row_height: 23.0,
            slider_track_inset: 0.0,
            slider_track_height: 10.0,
            slider_inner_gap: 0.0,
            slider_bottom_gap: 0.0,
            triangle_handles: true,
            wheel_max_size: 130.0,
            disclosure_button_size: 16.0,
            disclosure_button_gap: 4.0,
            navigation_control_height: 28.0,
            navigation_fit_width: 92.0,
            navigation_zoom_width: 72.0,
            navigation_control_gap: 0.0,
            rail_tool_size: 28.0,
            rail_tool_gap: 4.0,
            rail_bottom_inset: 10.0,
            rail_separator_gap: 10.0,
            histogram_height: 118.0,
            histogram_corner_size: 16.0,
            metadata_height: 26.0,
            edit_bar_height: 40.0,
            profile_bar_height: 40.0,
            bar_inset: 22.0,
            title_action_size: 24.0,
            title_action_gap: 6.0,
            curve_selector_size: 20.0,
            curve_selector_gap: 6.0,
            white_balance_label_width: 78.0,
            panel_order: crate::camera_raw_workspace_ui::SECTIONS.iter().map(|name| (*name).into()).collect(),
            preview: None,
        }
    }
}

impl Design {
    pub fn slider_style(&self) -> crate::widgets::SliderRowStyle {
        crate::widgets::SliderRowStyle {
            field_appearance: crate::widgets::ValueFieldAppearance {
                font_size: self.value_font_size,
                monospace: self.value_monospace,
                left_aligned: self.value_text_left_aligned,
                text_padding: self.value_text_padding,
                text_offset_y: self.value_text_offset_y,
                box_offset_y: self.value_field_offset_y,
            },
            field_height: self.slider_field_height,
            label_row_height: self.slider_label_row_height,
            track_inset: self.slider_track_inset,
            track_height: self.slider_track_height,
            inner_gap: self.slider_inner_gap,
            bottom_gap: self.slider_bottom_gap,
            triangle_handle: self.triangle_handles,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        for (label, number, min, max) in [
            ("panelWidth", value.panel_width, 220.0, 600.0),
            ("railWidth", value.rail_width, 32.0, 64.0),
            ("headerHeight", value.header_height, 24.0, 80.0),
            ("navigationHeight", value.navigation_height, 28.0, 80.0),
            ("footerHeight", value.footer_height, 36.0, 96.0),
            ("previewMargin", value.preview_margin, 0.0, 48.0),
            ("sectionHeight", value.section_height, 24.0, 64.0),
            ("sectionTextInset", value.section_text_inset, 16.0, 80.0),
            ("sectionIconInset", value.section_icon_inset, 8.0, 60.0),
            ("sectionFontSize", value.section_font_size, 10.0, 24.0),
            ("sectionPadding", value.section_padding, 0.0, 40.0),
            ("sectionRightPadding", value.section_right_padding, 0.0, 40.0),
            ("sectionVerticalPadding", value.section_vertical_padding, 0.0, 32.0),
            ("sectionBottomPadding", value.section_bottom_padding, 0.0, 64.0),
            ("panelFontSize", value.panel_font_size, 10.0, 24.0),
            ("valueFieldWidth", value.value_field_width, 40.0, 120.0),
            ("valueFontSize", value.value_font_size, 8.0, 24.0),
            ("valueTextPadding", value.value_text_padding, 0.0, 12.0),
            ("valueTextOffsetY", value.value_text_offset_y, -4.0, 4.0),
            ("valueFieldOffsetY", value.value_field_offset_y, -4.0, 4.0),
            ("rowGap", value.row_gap, 0.0, 24.0),
            ("sliderFieldHeight", value.slider_field_height, 16.0, 32.0),
            ("sliderLabelRowHeight", value.slider_label_row_height, 16.0, 32.0),
            ("sliderTrackInset", value.slider_track_inset, 0.0, 16.0),
            ("sliderTrackHeight", value.slider_track_height, 10.0, 24.0),
            ("sliderInnerGap", value.slider_inner_gap, 0.0, 12.0),
            ("sliderBottomGap", value.slider_bottom_gap, 0.0, 12.0),
            ("wheelMaxSize", value.wheel_max_size, 60.0, 240.0),
            ("disclosureButtonSize", value.disclosure_button_size, 12.0, 28.0),
            ("disclosureButtonGap", value.disclosure_button_gap, 0.0, 12.0),
            ("navigationControlHeight", value.navigation_control_height, 20.0, 48.0),
            ("navigationFitWidth", value.navigation_fit_width, 64.0, 140.0),
            ("navigationZoomWidth", value.navigation_zoom_width, 52.0, 120.0),
            ("navigationControlGap", value.navigation_control_gap, 0.0, 16.0),
            ("railToolSize", value.rail_tool_size, 16.0, 40.0),
            ("railToolGap", value.rail_tool_gap, 0.0, 16.0),
            ("railBottomInset", value.rail_bottom_inset, 0.0, 48.0),
            ("railSeparatorGap", value.rail_separator_gap, 0.0, 24.0),
            ("histogramHeight", value.histogram_height, 64.0, 240.0),
            ("histogramCornerSize", value.histogram_corner_size, 12.0, 28.0),
            ("metadataHeight", value.metadata_height, 18.0, 40.0),
            ("editBarHeight", value.edit_bar_height, 28.0, 60.0),
            ("profileBarHeight", value.profile_bar_height, 28.0, 60.0),
            ("barInset", value.bar_inset, 8.0, 32.0),
            ("titleActionSize", value.title_action_size, 18.0, 32.0),
            ("titleActionGap", value.title_action_gap, 0.0, 12.0),
            ("curveSelectorSize", value.curve_selector_size, 16.0, 28.0),
            ("curveSelectorGap", value.curve_selector_gap, 0.0, 12.0),
            ("whiteBalanceLabelWidth", value.white_balance_label_width, 65.0, 110.0),
        ] {
            if !number.is_finite() || !(min..=max).contains(&number) {
                return Err(format!("{label} must be finite and between {min} and {max}"));
            }
        }
        if value.panel_order.is_empty()
            || value.panel_order.len() > crate::camera_raw_workspace_ui::SECTIONS.len()
            || value.panel_order.iter().any(|name| !crate::camera_raw_workspace_ui::SECTIONS.contains(&name.as_str()))
            || value.panel_order.iter().enumerate().any(|(i, name)| value.panel_order.iter().take(i).any(|old| old == name))
        {
            return Err("panelOrder must contain unique supported panel names".into());
        }
        if let Some(preview) = &value.preview {
            crate::camera_raw_workspace_ui::prepare(&Default::default(), &serde_json::to_value(preview).map_err(|e| e.to_string())?)?;
        }
        Ok(value)
    }
}

fn embedded() -> Arc<Design> {
    static DEFAULT: OnceLock<Arc<Design>> = OnceLock::new();
    DEFAULT
        .get_or_init(|| {
            let mut design = Design::parse(include_str!("../../../design/camera-raw/layout.json")).unwrap_or_default();
            // Preview panel selection is for the explicitly opted-in live session, not ordinary editor startup.
            design.preview = None;
            Arc::new(design)
        })
        .clone()
}

/// Shared by drawing and ui.viewport: the host overlay must follow a live layout edit too.
pub(crate) fn get(ctx: &egui::Context) -> Arc<Design> {
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    {
        live::get(ctx)
    }
    #[cfg(not(all(debug_assertions, not(target_arch = "wasm32"))))]
    {
        let _ = ctx;
        embedded()
    }
}

pub(crate) fn apply_preview(ctx: &egui::Context, design: &Design, workspace: &mut crate::camera_raw_workspace_ui::WorkspaceState) {
    let id = egui::Id::new("camera-raw-applied-design-preview");
    let previous = ctx.data(|data| data.get_temp::<Option<crate::camera_raw_workspace_ui::WorkspaceState>>(id)).flatten();
    if previous != design.preview {
        if let Some(preview) = &design.preview {
            *workspace = preview.clone();
        }
        ctx.data_mut(|data| data.insert_temp(id, design.preview.clone()));
    }
}

#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
mod live {
    use super::*;
    use std::{
        path::PathBuf,
        time::{Duration, SystemTime},
    };

    #[derive(Clone)]
    struct State {
        path: Option<PathBuf>,
        stamp: Option<(SystemTime, u64)>,
        checked: Option<f64>,
        design: Arc<Design>,
        error: Option<String>,
    }

    fn update(state: &mut State) {
        let Some(path) = &state.path else { return };
        let result = (|| {
            let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
            let stamp = (metadata.modified().map_err(|e| e.to_string())?, metadata.len());
            if Some(stamp) == state.stamp {
                return Ok(false);
            }
            state.stamp = Some(stamp);
            if metadata.len() > 131_072 {
                return Err("layout JSON exceeds 128 KiB".into());
            }
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let next = Design::parse(&text)?;
            state.design = Arc::new(next);
            Ok(true)
        })();
        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                if state.error.as_ref() != Some(&error) {
                    log::warn!("Camera Raw layout: {error}");
                }
                state.error = Some(error);
                return;
            }
        }
        // A rejected file remains rejected until a newer write succeeds.
        if state.stamp.is_some() {
            state.error = None;
        }
    }

    pub(super) fn get(ctx: &egui::Context) -> Arc<Design> {
        let id = egui::Id::new("camera-raw-live-design");
        let mut state = ctx.data(|data| data.get_temp::<State>(id)).unwrap_or_else(|| State {
            path: std::env::var_os("PHOTOCRAFT_CAMERA_RAW_LAYOUT_FILE").map(PathBuf::from),
            stamp: None,
            checked: None,
            design: embedded(),
            error: None,
        });
        if state.path.is_some() {
            ctx.request_repaint_after(Duration::from_millis(400));
            let now = ctx.input(|input| input.time);
            if state.checked.is_none_or(|last| now < last || now - last >= 0.4) {
                state.checked = Some(now);
                update(&mut state);
            }
        }
        let design = state.design.clone();
        ctx.data_mut(|data| data.insert_temp(id, state));
        design
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn saved_layout_reloads_without_restarting_and_partial_json_keeps_the_last_good_layout() {
            let path = std::env::temp_dir().join(format!(
                "photocraft-layout-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            let mut state = State { path: Some(path.clone()), stamp: None, checked: None, design: embedded(), error: None };
            std::fs::write(&path, r#"{"panelWidth":310}"#).unwrap();
            update(&mut state);
            assert_eq!(state.design.panel_width, 310.0);
            std::fs::write(&path, r#"{"panelWidth":3"#).unwrap();
            update(&mut state);
            assert_eq!(state.design.panel_width, 310.0);
            assert!(state.error.is_some());
            std::fs::write(&path, r#"{"panelWidth":335,"rowGap":6}"#).unwrap();
            update(&mut state);
            assert_eq!(state.design.panel_width, 335.0);
            assert_eq!(state.design.row_gap, 6.0);
            assert!(state.error.is_none());
            std::fs::remove_file(&path).unwrap();
        }

        #[test]
        fn a_live_file_edit_changes_the_actual_workspace_geometry_in_the_same_context() {
            let path = std::env::temp_dir().join(format!(
                "photocraft-layout-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::write(&path, r#"{"panelWidth":270}"#).unwrap();
            let ctx = egui::Context::default();
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("camera-raw-live-design"),
                    State { path: Some(path.clone()), stamp: None, checked: None, design: embedded(), error: None },
                )
            });
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
            let mut first = None;
            let mut output = ctx.run_ui(egui::RawInput { time: Some(1.0), ..Default::default() }, |ui| {
                first = Some(crate::camera_raw_ui::layout(ui.ctx(), screen, false));
            });
            output.textures_delta.clear();
            std::fs::write(&path, r#"{"panelWidth":330,"headerHeight":40}"#).unwrap();
            let mut second = None;
            let mut output = ctx.run_ui(egui::RawInput { time: Some(2.0), ..Default::default() }, |ui| {
                second = Some(crate::camera_raw_ui::layout(ui.ctx(), screen, false));
            });
            output.textures_delta.clear();
            let (first, second) = (first.unwrap(), second.unwrap());
            assert_eq!(first.right.width(), 270.0);
            assert_eq!(second.right.width(), 330.0);
            assert_eq!(first.view.right() - second.view.right(), 60.0);
            assert_eq!(second.title.height(), 40.0);
            std::fs::remove_file(&path).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_json_rejects_unusable_geometry_duplicates_and_behavior_fields() {
        for bad in [
            r#"{"panelWidth":0}"#,
            r#"{"rowGap":-1}"#,
            r#"{"panelOrder":["Light","Light"]}"#,
            r#"{"panelOrder":["Execute"]}"#,
            r#"{"exposure":2}"#,
            r#"{"panelWidth":1e100}"#,
            r#"{"histogramHeight":0}"#,
            r#"{"curveSelectorSize":1e100}"#,
            r#"{"whiteBalanceLabelWidth":-1}"#,
        ] {
            assert!(Design::parse(bad).is_err(), "{bad}");
        }
        assert!(Design::parse(include_str!("../../../design/camera-raw/layout.json")).is_ok());
    }
}
