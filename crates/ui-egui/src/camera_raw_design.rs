//! Live Camera Raw visual design. Native debug previews can reload JSON without a rebuild.
//! No document parameters, executable actions or filesystem destinations are accepted here.
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Design {
    pub red_eye: crate::camera_raw_ui::tool_panels::RedEyeDesign,
    pub panel_width: f32,
    pub rail_width: f32,
    pub header_height: f32,
    pub navigation_height: f32,
    pub footer_height: f32,
    pub preview_margin: f32,
    pub section_height: f32,
    pub section_text_inset: f32,
    pub section_icon_inset: f32,
    pub section_eye_inset: f32,
    pub section_eye_opacity: f32,
    pub section_separator_height: f32,
    pub extra_control_gap: f32,
    pub extra_disclosure_height: f32,
    pub extra_disclosure_outset: f32,
    pub optics_tabs_height: f32,
    pub optics_checkbox_gap: f32,
    pub optics_divider_gap: f32,
    pub bokeh_selector_size: f32,
    pub calibration_title_height: f32,
    pub calibration_title_leading: f32,
    pub calibration_section_gap: f32,
    pub detail_enhancements_height: f32,
    pub detail_noise_gap: f32,
    pub detail_bottom_padding: f32,
    pub section_font_size: f32,
    pub section_padding: f32,
    pub section_right_padding: f32,
    pub settings_body_right_padding: f32,
    pub settings_scrollbar_gutter: f32,
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
    pub grading_lower_wheel_outset: f32,
    pub grading_wheel_fraction: f32,
    pub grading_luminance_height: f32,
    pub grading_top_label_offset_y: f32,
    pub grading_lower_label_offset_y: f32,
    pub grading_selector_label_inset: f32,
    pub info_left_inset: f32,
    pub info_right_inset: f32,
    pub info_text_inset: f32,
    pub info_top_inset: f32,
    pub info_card_height: f32,
    pub info_card_radius: f32,
    pub info_group_gap: f32,
    pub info_field_gap: f32,
    pub info_divider_top_gap: f32,
    pub info_credentials_top_gap: f32,
    pub info_heading_font_size: f32,
    pub tab_heading_font_size: f32,
    pub tab_heading_inset: f32,
    pub tab_heading_offset_y: f32,
    pub wheel_brightness: f32,
    pub grading_wheel_row_gap: f32,
    pub grading_selector_size: f32,
    pub grading_selector_gap: f32,
    pub grading_selector_group_gap: f32,
    pub grading_selector_start: f32,
    pub grading_selector_bottom_gap: f32,
    pub grading_footer_gap: f32,
    pub grading_bottom_padding: f32,
    pub disclosure_button_size: f32,
    pub disclosure_button_gap: f32,
    pub navigation_control_height: f32,
    pub navigation_fit_width: f32,
    pub navigation_zoom_width: f32,
    pub navigation_control_gap: f32,
    pub rail_tool_size: f32,
    pub rail_top_inset: f32,
    pub snapshots_rail_offset: f32,
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
    pub curve_left_padding: f32,
    pub curve_right_padding: f32,
    pub curve_bottom_padding: f32,
    pub curve_selector_height: f32,
    pub curve_graph_top_gap: f32,
    pub curve_splits_top_gap: f32,
    pub curve_splits_height: f32,
    pub curve_splits_bottom_gap: f32,
    pub white_balance_label_width: f32,
    pub white_balance_bottom_gap: f32,
    pub panel_order: Vec<String>,
    /// Optional view preset, applied only when this value changes in the JSON.
    pub preview: Option<crate::camera_raw_workspace_ui::WorkspaceState>,
}

impl Default for Design {
    fn default() -> Self {
        Self {
            red_eye: Default::default(),
            panel_width: 270.0,
            rail_width: 36.0,
            header_height: 32.0,
            navigation_height: 32.0,
            footer_height: 48.0,
            preview_margin: 8.0,
            section_height: 34.0,
            section_text_inset: 44.0,
            section_icon_inset: 27.0,
            section_eye_inset: 18.0,
            section_eye_opacity: 0.36,
            section_separator_height: 2.0,
            extra_control_gap: 10.0,
            extra_disclosure_height: 32.0,
            extra_disclosure_outset: 11.0,
            optics_tabs_height: 26.0,
            optics_checkbox_gap: 2.0,
            optics_divider_gap: 4.0,
            bokeh_selector_size: 34.0,
            calibration_title_height: 8.0,
            calibration_title_leading: 3.0,
            calibration_section_gap: 6.25,
            detail_enhancements_height: 126.0,
            detail_noise_gap: 5.0,
            detail_bottom_padding: 12.0,
            section_font_size: 13.0,
            section_padding: 14.0,
            section_right_padding: 16.0,
            settings_body_right_padding: 21.0,
            settings_scrollbar_gutter: 15.0,
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
            grading_lower_wheel_outset: 17.0,
            grading_wheel_fraction: 0.487,
            grading_luminance_height: 25.0,
            grading_top_label_offset_y: -9.0,
            grading_lower_label_offset_y: -5.0,
            grading_selector_label_inset: 17.0,
            info_left_inset: 15.0,
            info_right_inset: 29.0,
            info_text_inset: 20.0,
            info_top_inset: 16.0,
            info_card_height: 52.0,
            info_card_radius: 8.0,
            info_group_gap: 20.0,
            info_field_gap: 16.0,
            info_divider_top_gap: 24.0,
            info_credentials_top_gap: 14.0,
            info_heading_font_size: 13.4,
            tab_heading_font_size: 13.4,
            tab_heading_inset: 20.0,
            tab_heading_offset_y: 1.5,
            wheel_brightness: 0.5,
            grading_wheel_row_gap: 21.5,
            grading_selector_size: 20.0,
            grading_selector_gap: 6.0,
            grading_selector_group_gap: 6.0,
            grading_selector_start: 68.0,
            grading_selector_bottom_gap: 14.0,
            grading_footer_gap: 7.0,
            grading_bottom_padding: 23.0,
            disclosure_button_size: 16.0,
            disclosure_button_gap: 4.0,
            navigation_control_height: 28.0,
            navigation_fit_width: 92.0,
            navigation_zoom_width: 72.0,
            navigation_control_gap: 0.0,
            rail_tool_size: 28.0,
            rail_top_inset: 3.5,
            snapshots_rail_offset: 187.5,
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
            curve_left_padding: 18.0,
            curve_right_padding: 15.0,
            curve_bottom_padding: 13.0,
            curve_selector_height: 24.0,
            curve_graph_top_gap: 15.0,
            curve_splits_top_gap: 4.0,
            curve_splits_height: 12.0,
            curve_splits_bottom_gap: 12.0,
            white_balance_label_width: 78.0,
            white_balance_bottom_gap: 6.0,
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
        value.red_eye.validate()?;
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
            ("sectionEyeInset", value.section_eye_inset, 9.0, 40.0),
            ("sectionEyeOpacity", value.section_eye_opacity, 0.1, 1.0),
            ("sectionSeparatorHeight", value.section_separator_height, 0.5, 4.0),
            ("extraControlGap", value.extra_control_gap, 0.0, 24.0),
            ("extraDisclosureHeight", value.extra_disclosure_height, 24.0, 48.0),
            ("extraDisclosureOutset", value.extra_disclosure_outset, 0.0, 20.0),
            ("opticsTabsHeight", value.optics_tabs_height, 20.0, 48.0),
            ("opticsCheckboxGap", value.optics_checkbox_gap, 0.0, 20.0),
            ("opticsDividerGap", value.optics_divider_gap, 0.0, 20.0),
            ("bokehSelectorSize", value.bokeh_selector_size, 20.0, 40.0),
            ("calibrationTitleHeight", value.calibration_title_height, 2.0, 24.0),
            ("calibrationTitleLeading", value.calibration_title_leading, 0.0, 12.0),
            ("calibrationSectionGap", value.calibration_section_gap, 0.0, 24.0),
            ("detailEnhancementsHeight", value.detail_enhancements_height, 116.0, 160.0),
            ("detailNoiseGap", value.detail_noise_gap, 0.0, 24.0),
            ("detailBottomPadding", value.detail_bottom_padding, 0.0, 64.0),
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
            ("railTopInset", value.rail_top_inset, 0.0, 16.0),
            ("snapshotsRailOffset", value.snapshots_rail_offset, 100.0, 260.0),
            ("railToolGap", value.rail_tool_gap, 0.0, 16.0),
            ("railBottomInset", value.rail_bottom_inset, 0.0, 48.0),
            ("railSeparatorGap", value.rail_separator_gap, 0.0, 24.0),
            ("gradingSelectorSize", value.grading_selector_size, 16.0, 28.0),
            ("gradingSelectorGap", value.grading_selector_gap, 0.0, 12.0),
            ("gradingSelectorGroupGap", value.grading_selector_group_gap, 0.0, 12.0),
            ("gradingSelectorStart", value.grading_selector_start, 40.0, 90.0),
            ("gradingSelectorBottomGap", value.grading_selector_bottom_gap, 0.0, 40.0),
            ("gradingFooterGap", value.grading_footer_gap, 0.0, 40.0),
            ("gradingBottomPadding", value.grading_bottom_padding, 0.0, 64.0),
            ("wheelBrightness", value.wheel_brightness, 0.1, 1.0),
            ("gradingWheelRowGap", value.grading_wheel_row_gap, 0.0, 40.0),
            ("gradingLowerWheelOutset", value.grading_lower_wheel_outset, 0.0, 30.0),
            ("gradingWheelFraction", value.grading_wheel_fraction, 0.3, 0.6),
            ("gradingLuminanceHeight", value.grading_luminance_height, 18.0, 40.0),
            ("gradingTopLabelOffsetY", value.grading_top_label_offset_y, -16.0, 16.0),
            ("gradingLowerLabelOffsetY", value.grading_lower_label_offset_y, -16.0, 16.0),
            ("gradingSelectorLabelInset", value.grading_selector_label_inset, 0.0, 30.0),
            ("infoLeftInset", value.info_left_inset, 0.0, 40.0),
            ("infoRightInset", value.info_right_inset, 0.0, 48.0),
            ("infoTextInset", value.info_text_inset, 0.0, 40.0),
            ("infoTopInset", value.info_top_inset, 0.0, 40.0),
            ("infoCardHeight", value.info_card_height, 36.0, 90.0),
            ("infoCardRadius", value.info_card_radius, 0.0, 16.0),
            ("infoGroupGap", value.info_group_gap, 0.0, 40.0),
            ("infoFieldGap", value.info_field_gap, 0.0, 40.0),
            ("infoDividerTopGap", value.info_divider_top_gap, 0.0, 40.0),
            ("infoCredentialsTopGap", value.info_credentials_top_gap, 0.0, 40.0),
            ("infoHeadingFontSize", value.info_heading_font_size, 10.0, 24.0),
            ("tabHeadingFontSize", value.tab_heading_font_size, 10.0, 24.0),
            ("tabHeadingInset", value.tab_heading_inset, 0.0, 40.0),
            ("tabHeadingOffsetY", value.tab_heading_offset_y, -8.0, 8.0),
            ("settingsBodyRightPadding", value.settings_body_right_padding, 0.0, 64.0),
            ("settingsScrollbarGutter", value.settings_scrollbar_gutter, 0.0, 40.0),
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
            ("curveLeftPadding", value.curve_left_padding, 0.0, 40.0),
            ("curveRightPadding", value.curve_right_padding, 0.0, 40.0),
            ("curveBottomPadding", value.curve_bottom_padding, 0.0, 64.0),
            ("curveSelectorHeight", value.curve_selector_height, 20.0, 48.0),
            ("curveGraphTopGap", value.curve_graph_top_gap, 0.0, 32.0),
            ("curveSplitsTopGap", value.curve_splits_top_gap, 0.0, 24.0),
            ("curveSplitsHeight", value.curve_splits_height, 8.0, 24.0),
            ("curveSplitsBottomGap", value.curve_splits_bottom_gap, 0.0, 32.0),
            ("whiteBalanceLabelWidth", value.white_balance_label_width, 65.0, 110.0),
            ("whiteBalanceBottomGap", value.white_balance_bottom_gap, 0.0, 24.0),
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
            r#"{"sectionEyeInset":0}"#,
            r#"{"sectionEyeOpacity":2}"#,
            r#"{"calibrationTitleHeight":0}"#,
            r#"{"detailEnhancementsHeight":0}"#,
            r#"{"detailNoiseGap":-1}"#,
            r#"{"detailBottomPadding":-1}"#,
            r#"{"exposure":2}"#,
            r#"{"panelWidth":1e100}"#,
            r#"{"histogramHeight":0}"#,
            r#"{"curveSelectorSize":1e100}"#,
            r#"{"whiteBalanceLabelWidth":-1}"#,
            r#"{"gradingLowerWheelOutset":1000}"#,
            r#"{"gradingLuminanceHeight":0}"#,
            r#"{"infoCardHeight":-1}"#,
            r#"{"infoLeftInset":1e100}"#,
            r#"{"infoFieldGap":-1}"#,
            r#"{"infoHeadingFontSize":0}"#,
            r#"{"tabHeadingOffsetY":1e100}"#,
            r#"{"wheelBrightness":-1}"#,
            r#"{"gradingWheelRowGap":-1}"#,
            r#"{"gradingSelectorSize":0}"#,
            r#"{"gradingSelectorGap":-1}"#,
            r#"{"gradingSelectorGroupGap":-1}"#,
            r#"{"gradingFooterGap":-1}"#,
            r#"{"gradingSelectorStart":1e100}"#,
            r#"{"gradingSelectorBottomGap":-1}"#,
        ] {
            assert!(Design::parse(bad).is_err(), "{bad}");
        }
        assert!(Design::parse(include_str!("../../../design/camera-raw/layout.json")).is_ok());
    }
}
