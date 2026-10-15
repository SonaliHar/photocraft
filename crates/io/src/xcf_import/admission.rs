//! Fallible eager-plane admission, kept separate from document publication.
use super::*;

#[derive(Default)]
pub(super) struct AdmissionDomains {
    pub(super) owned_assigned_box_domain: bool,
    pub(super) general_separable_domain: bool,
    pub(super) legacy_separable_domain: bool,
    pub(super) component_separable_domain: bool,
    pub(super) alpha_separable_domain: bool,
    pub(super) lab_separable_domain: bool,
    pub(super) point_replace_domain: Option<crate::xcf_filters::NeighborhoodDomain>,
    // Successful sealed owned scalar admission; does not change old mask rules.
    pub(super) owned_point_domain: Option<crate::xcf_filters::NeighborhoodDomain>,
    pub(super) owned_neighborhood_domain: Option<crate::xcf_filters::NeighborhoodDomain>,
    pub(super) unpremultiply_domain: bool,
    pub(super) finite_normal_domain: bool,
    pub(super) floating_records: Option<(u64, u64)>,
    pub(super) gray_mask_domain: bool,
    pub(super) analytic_geometry_domain: bool,
    pub(super) ordinary_geometry_domain: bool,
    pub(super) normal_mask_domain: bool,
    pub(super) analytic_normal_domain: bool,
    pub(super) gaussian_group_domain: bool,
    pub(super) accounted_group_domain: bool,
    pub(super) finite_gaussian_domain: bool,
    pub(super) finite_box_domain: bool,
    pub(super) finite_median_domain: bool,
    pub(super) gray_median_domain: bool,
    pub(super) opaque_box_domain: bool,
    pub(super) opaque_median_domain: bool,
    pub(super) opaque_motion_domain: bool,
    pub(super) opaque_pixelize_domain: bool,
    pub(super) vector_domain: bool,
    pub(super) stem_text_domain: bool,
    pub(super) masked_group_domain: bool,
    pub(super) multiply_spaces_domain: bool,
    pub(super) composition_command_domain: bool,
}

pub(super) fn classify(file: &XcfFile, ctl: &Interrupt<'_>, preparation: Option<&photocraft_xcf::Budget<'_>>) -> Result<AdmissionDomains, IoError> {
    let point_replace_domain = crate::xcf_point_replace_source::classify(file, ctl, preparation)?;
    let unpremultiply_domain = unpremultiply::qualifies(file, ctl, preparation)?;
    let finite_normal_domain = qualified_finite_normal_source(file);
    for d in &file.layers {
        validate_drawable_alpha(d, finite_normal_domain || point_replace_domain.is_some(), false, ctl, 0)?;
    }
    for d in &file.channels {
        validate_drawable_alpha(d, false, true, ctl, 0)?;
    }
    let floating_records = qualified_floating(file);
    let gray_mask_domain = qualified_gray_mask(file);
    let fixed_normal_mask_domain = qualified_normal_mask(file);
    let analytic_geometry_domain = !fixed_normal_mask_domain && qualified_analytic_normal_geometry(file);
    let ordinary_geometry_domain = if !fixed_normal_mask_domain && !analytic_geometry_domain {
        if let Some(budget) = preparation { crate::xcf_ordinary_source::qualifies(file, ctl, budget)? } else { false }
    } else {
        false
    };
    let normal_mask_domain = fixed_normal_mask_domain || analytic_geometry_domain || ordinary_geometry_domain;
    let analytic_normal_domain = normal_mask_domain && !qualified_known_normal_profile(file) && qualified_analytic_normal_profile(file);
    let gaussian_group_domain = qualified_gaussian_group(file);
    let accounted_group_domain = accounted_group::qualifies(file, ctl)?;
    let finite_gaussian_domain = qualified_finite_raster_filter(file, "gegl:gaussian-blur", Model::Rgb);
    let finite_box_domain = qualified_finite_raster_filter(file, "gegl:box-blur", Model::Rgb);
    let finite_median_domain = qualified_finite_raster_filter(file, "gegl:median-blur", Model::Rgb);
    let gray_median_domain = file.icc.is_empty() && qualified_finite_raster_filter(file, "gegl:median-blur", Model::Gray);
    let opaque_box_domain = qualified_opaque_neighborhood(file, "gegl:box-blur");
    let opaque_median_domain = qualified_opaque_neighborhood(file, "gegl:median-blur");
    let opaque_motion_domain = qualified_opaque_neighborhood(file, "gegl:motion-blur-linear");
    let opaque_pixelize_domain = qualified_opaque_neighborhood(file, "gegl:pixelize");
    let vector_domain = qualified_vector_context(file);
    let stem_text_domain = qualified_rectilinear_text_context(file) || qualified_underline_text_context(file);
    let masked_group_domain = qualified_masked_group(file) || gaussian_group_domain;
    let multiply_spaces_domain = qualified_multiply_spaces(file);
    let composition_command_domain = qualified_composition_command(file);
    Ok(AdmissionDomains {
        owned_assigned_box_domain:false,
        general_separable_domain: false,
        legacy_separable_domain: false,
        component_separable_domain: false,
        alpha_separable_domain: false,
        lab_separable_domain: false,
        point_replace_domain,
        owned_point_domain: None,
        owned_neighborhood_domain: None,
        unpremultiply_domain,
        finite_normal_domain,
        floating_records,
        gray_mask_domain,
        analytic_geometry_domain,
        ordinary_geometry_domain,
        normal_mask_domain,
        analytic_normal_domain,
        gaussian_group_domain,
        accounted_group_domain,
        finite_gaussian_domain,
        finite_box_domain,
        finite_median_domain,
        gray_median_domain,
        opaque_box_domain,
        opaque_median_domain,
        opaque_motion_domain,
        opaque_pixelize_domain,
        vector_domain,
        stem_text_domain,
        masked_group_domain,
        multiply_spaces_domain,
        composition_command_domain,
    })
}

/// Side-effect-free existing eager predicates. Kept after profile validation.
#[derive(Default)]
pub(super) struct LateAdmissionDomains {
    pub(super) point_stack_edits_domain: bool,
    pub(super) posterize_domain: bool,
    pub(super) value_invert_domain: bool,
    pub(super) hdr_clip_domain: bool,
    pub(super) value_precision_domain: bool,
    pub(super) brightness_precision_domain: bool,
    pub(super) levels_precision_domain: bool,
    pub(super) exposure_geometry_domain: bool,
    pub(super) exposure_black_domain: bool,
    pub(super) color_balance_depth_domain: bool,
    pub(super) color_balance_domain: bool,
    pub(super) temperature_domain: bool,
    pub(super) desaturate_domain: bool,
    pub(super) saturation_yuv_domain: bool,
    pub(super) saturation_domain: bool,
    pub(super) saturation_gray_domain: bool,
    pub(super) threshold_domain: bool,
    pub(super) threshold_alpha_domain: bool,
    pub(super) sepia_domain: bool,
    pub(super) alien_map_domain: bool,
    pub(super) alien_map_hsl_domain: bool,
    pub(super) mono_mixer_domain: bool,
    pub(super) mono_mixer_edits_domain: bool,
    pub(super) point_context: bool,
    pub(super) levels_context: bool,
    pub(super) multiply_domain: bool,
    pub(super) basic_blend_domain: bool,
}

pub(super) fn classify_late(file: &XcfFile) -> LateAdmissionDomains {
    let point_stack_edits_domain = qualified_point_stack_edits(file);
    let posterize_domain = qualified_opaque_neighborhood(file, "gimp:posterize");
    let value_invert_domain = qualified_opaque_neighborhood(file, "gegl:value-invert");
    let hdr_clip_domain = qualified_hdr_clip(file);
    let value_precision_domain = qualified_precision_value_invert(file);
    let brightness_precision_domain = qualified_precision_brightness_contrast(file);
    let levels_precision_domain = qualified_precision_gegl_levels(file);
    let exposure_geometry_domain = qualified_precision_point_impl(file, "gegl:exposure", false, true);
    let exposure_black_domain = qualified_precision_exposure_black(file);
    let color_balance_depth_domain = qualified_precision_point_impl(file, "gimp:color-balance", true, false);
    let color_balance_domain = color_balance_depth_domain || qualified_precision_point(file, "gimp:color-balance");
    let temperature_domain = qualified_precision_point(file, "gegl:color-temperature");
    let desaturate_domain = qualified_precision_point(file, "gimp:desaturate");
    let saturation_yuv_domain = file.model == Model::Rgb
        && qualified_precision_point(file, "gegl:saturation")
        && file
            .layers
            .first()
            .and_then(|layer| layer.effects.first())
            .is_some_and(|effect| effect.arguments.iter().any(|argument| argument.name == "colorspace" && argument.value == ArgumentValue::Enum(2)));
    let saturation_domain = file.model == Model::Rgb && !saturation_yuv_domain && qualified_precision_point(file, "gegl:saturation");
    let saturation_gray_domain = file.model == Model::Gray && qualified_precision_point(file, "gegl:saturation");
    let threshold_domain = qualified_precision_point(file, "gimp:threshold");
    let threshold_alpha_domain = qualified_precision_point(file, "gimp:threshold-alpha");
    let sepia_domain = file.model == Model::Rgb && qualified_precision_point(file, "gegl:sepia");
    let alien_map_domain = file.model == Model::Rgb && qualified_precision_point(file, "gegl:alien-map");
    let alien_map_hsl_domain = alien_map_domain
        && file.layers.first().is_some_and(|layer| {
            layer
                .effects
                .first()
                .is_some_and(|effect| effect.arguments.iter().any(|argument| argument.name == "color-model" && argument.value == ArgumentValue::Enum(1)))
        });
    let mono_mixer_domain = qualified_opaque_neighborhood(file, "gegl:mono-mixer");
    let mono_mixer_edits_domain = mono_mixer_domain
        && file.layers.first().and_then(|layer| layer.effects.first()).is_some_and(|effect| {
            (effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode) == (Some(107971153), Some(107971153), Some(107971153))
        });
    let point_context =
        file.version == 22 && file.model == Model::Rgb && file.sample == Sample::U8 && file.encoding == Encoding::NonlinearProfile && file.icc.is_empty();
    let levels_context = file.version == 22
        && file.model == Model::Rgb
        && file.sample == Sample::F32
        && matches!(file.encoding, Encoding::Linear | Encoding::NonlinearProfile)
        && file.icc.is_empty()
        && file.width == 12
        && file.height == 9;
    // Auto is admitted only for the source/edit domain independently observed in GIMP.
    let multiply_domain = file.version == 12
        && file.model == Model::Rgb
        && file.sample == Sample::F32
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 16
        && file.height == 12
        && file.layers.len() == 2
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.iter().all(|d| !d.group && d.path.len() <= 1 && d.effects.is_empty() && d.visible && d.floating_attachment.is_none())
        && file.layers.first().is_some_and(|d| {
            d.mode == 30
                && d.blend_space == -1
                && d.composite_space == -1
                && (d.composite_mode == -2 && d.mask.is_none()
                    || matches!(d.composite_mode, 1..=4)
                        && d.mask_enabled
                        && !d.mask_shown
                        && d.mask_editing
                        && d.mask.as_ref().is_some_and(|m| m.width == 10 && m.height == 9 && m.offset == (0, 0) && m.model == Model::Gray && !m.alpha))
                && d.width == 10
                && d.height == 9
                && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                    || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
        })
        && file.layers.get(1).is_some_and(|d| {
            d.mode == 28
                && d.width == 16
                && d.height == 12
                && d.offset == (0, 0)
                && d.opacity == 1.0
                && d.mask.is_none()
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
        });
    let basic_blend_domain = (file.model == Model::Rgb || file.model == Model::Gray && file.layers.first().is_some_and(|d| matches!(d.mode, 57..=60)))
        && file.sample == Sample::F32
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 16
        && file.height == 12
        && file.layers.len() == 2
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.iter().all(|d| {
            !d.group
                && d.path.len() <= 1
                && d.effects.is_empty()
                && d.text.is_none()
                && d.vector.is_none()
                && d.link.is_none()
                && d.transform.is_none()
                && d.visible
                && d.floating_attachment.is_none()
                && d.mask.is_none()
                && d.model == file.model
                && d.alpha
                && d.plane.as_ref().is_some_and(|p| p.sample == Sample::F32 && p.channels == if file.model == Model::Gray { 2 } else { 4 })
        })
        && file.layers.first().is_some_and(|d| {
            ((file.version == 23 && matches!(d.mode, 23 | 31 | 32 | 42..=51 | 52 | 53) && d.blend_space == -4)
                || (file.version == 12 && matches!(d.mode, 33 | 34 | 41 | 54..=57) && d.blend_space == -1)
                || (file.version == 12 && matches!(d.mode, 35 | 36) && d.blend_space == 0)
                || (file.version == 12 && matches!(d.mode, 58..=60) && d.blend_space == 0)
                || (file.version == 12 && matches!(d.mode, 37..=40) && d.blend_space == -2)
                || (file.version == 12 && matches!(d.mode, 24..=27) && d.blend_space == -3)
                || (file.version == 12 && matches!(d.mode, 3 | 4 | 6..=21) && d.blend_space == -2))
                && d.composite_space
                    == if matches!(d.mode, 3 | 4 | 6..=21) {
                        -2
                    } else if d.mode == 60 {
                        0
                    } else {
                        -1
                    }
                && d.composite_mode == if d.mode == 59 { -1 } else { -2 }
                && d.width == 10
                && d.height == 9
                && ((d.offset == (2, -1) && d.opacity.to_bits() == 0.7125_f32.to_bits())
                    || (d.offset == (-1, 2) && d.opacity.to_bits() == 0.4375_f32.to_bits()))
        })
        && file.layers.get(1).is_some_and(|d| {
            d.mode == 28
                && d.width == 16
                && d.height == 12
                && d.offset == (0, 0)
                && d.opacity == 1.0
                && d.blend_space == 0
                && d.composite_space == -1
                && d.composite_mode == -1
        });
    let basic_blend_domain = basic_blend_domain
        && (file.layers.first().is_none_or(|d| !matches!(d.mode, 3 | 4 | 6..=21 | 24..=27 | 37..=49 | 51 | 53..=60))
            || file.layers.iter().all(|d| {
                d.plane.as_ref().is_some_and(|p| {
                    p.bytes.as_chunks::<4>().0.iter().all(|bytes| {
                        let value = f32::from_ne_bytes(*bytes);
                        value.is_finite() && (0.0..=1.0).contains(&value)
                    })
                })
            }))
        && file.layers.first().is_none_or(|d| {
            !matches!(d.mode, 41..=43 | 48)
                || d.plane.as_ref().is_some_and(|p| {
                    p.bytes.as_chunks::<4>().0.iter().enumerate().all(|(index, bytes)| {
                        if index % 4 == 3 {
                            return true;
                        }
                        let value = f32::from_ne_bytes(*bytes);
                        match d.mode {
                            42 => value < 1.0,
                            48 => value > 0.0 && value < 1.0,
                            _ => value > 0.0,
                        }
                    })
                })
        });
    LateAdmissionDomains {
        point_stack_edits_domain,
        posterize_domain,
        value_invert_domain,
        hdr_clip_domain,
        value_precision_domain,
        brightness_precision_domain,
        levels_precision_domain,
        exposure_geometry_domain,
        exposure_black_domain,
        color_balance_depth_domain,
        color_balance_domain,
        temperature_domain,
        desaturate_domain,
        saturation_yuv_domain,
        saturation_domain,
        saturation_gray_domain,
        threshold_domain,
        threshold_alpha_domain,
        sepia_domain,
        alien_map_domain,
        alien_map_hsl_domain,
        mono_mixer_domain,
        mono_mixer_edits_domain,
        point_context,
        levels_context,
        multiply_domain,
        basic_blend_domain,
    }
}
