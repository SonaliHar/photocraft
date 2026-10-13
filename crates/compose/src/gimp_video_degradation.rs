//! Explicit native RGB Float nonlinear, coordinate-aware Video Degradation candidate.
//! Saved source admission is separately checked by IO; regeneration never uses cached evidence pixels.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_video_degradation::VideoDegradation;
use photocraft_cms::{Builtin, Profile};
use photocraft_color::PixelFormat;
use photocraft_doc::SampleEncoding;
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
pub const VIDEO_DEGRADATION_EPOCH: &str = "xcf-22-stock-video-degradation-nonlinear-float-coordinate-v1";
pub const VIDEO_DEGRADATION_OPTIONS_EPOCH: &str = "xcf-22-stock-video-degradation-options-rgb-f32-nonlinear-v1";
#[cfg(test)]
#[path = "../tests/common/video_options_canonical.rs"]
mod options_tests;
/// Only saved source/edit/disabled parameter states, without geometry authority.
pub fn witnessed_current_parameters(operation: VideoDegradation, enabled: bool) -> bool {
    operation == (VideoDegradation { pattern: 5, additive: false, rotated: false })
        || enabled && operation == (VideoDegradation { pattern: 2, additive: true, rotated: true })
}
/// Cheap parameter enablement only; native context and policy are checked at preparation.
pub fn current_epoch_parameters(epoch: &str, operation: VideoDegradation, enabled: bool) -> bool {
    match epoch {
        VIDEO_DEGRADATION_EPOCH => witnessed_current_parameters(operation, enabled),
        VIDEO_DEGRADATION_OPTIONS_EPOCH => operation.pattern <= 8,
        _ => false,
    }
}
pub(crate) fn claims_options_epoch(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    node.operation.schema_epoch == VIDEO_DEGRADATION_OPTIONS_EPOCH
        || node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == VIDEO_DEGRADATION_OPTIONS_EPOCH)
}
pub(crate) fn claims_video(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    node.operation.name == "gegl:video-degradation" || claims_options_epoch(node)
}
pub fn typed_arguments(arguments: &[photocraft_doc::xcf_filters::FilterArgument]) -> Result<VideoDegradation> {
    use photocraft_doc::xcf_filters::FilterValue;
    if arguments.len() != 3 {
        return Err(CompositionError::Unsupported("Video Degradation argument count"));
    }
    let (mut pattern, mut additive, mut rotated) = (None, None, None);
    for argument in arguments {
        match (argument.name.as_str(), &argument.value) {
            ("pattern", FilterValue::Enum { domain, value }) if domain == "gegl:video-degradation/pattern" && pattern.is_none() => {
                pattern = Some(u8::try_from(*value).map_err(|_| CompositionError::Unsupported("Video Degradation pattern"))?);
            }
            ("additive", FilterValue::Bool(value)) if additive.is_none() => additive = Some(*value),
            ("rotated", FilterValue::Bool(value)) if rotated.is_none() => rotated = Some(*value),
            _ => return Err(CompositionError::Unsupported("Video Degradation typed argument")),
        }
    }
    let operation = VideoDegradation {
        pattern: pattern.ok_or(CompositionError::Unsupported("Video Degradation pattern missing"))?,
        additive: additive.ok_or(CompositionError::Unsupported("Video Degradation additive missing"))?,
        rotated: rotated.ok_or(CompositionError::Unsupported("Video Degradation rotated missing"))?,
    };
    photocraft_algo::gimp::registry::video_degradation_member(Some("0:0"), operation)?;
    Ok(operation)
}
pub(crate) fn canonical_candidate(
    node: &photocraft_doc::xcf_filters::FilterNode,
    profile: &Profile,
    stored: SampleEncoding,
    owner: Rect,
    version: u32,
) -> Result<VideoDegradationCandidate> {
    let (compatibility, raw_space) = match node.operation.schema_epoch.as_str() {
        VIDEO_DEGRADATION_EPOCH => (crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH, -1877441967),
        VIDEO_DEGRADATION_OPTIONS_EPOCH => (VIDEO_DEGRADATION_OPTIONS_EPOCH, 2026996305),
        _ => return Err(CompositionError::Unsupported("Video Degradation canonical schema epoch")),
    };
    if version != 22
        || owner != Rect::new(0, 0, 16, 16)
        || node.operation.name != "gegl:video-degradation"
        || node.operation.stored_version != "0:0"
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || node.composition.as_ref().is_none_or(|policy| {
            policy.compatibility_epoch != compatibility
                || policy.source_mode != 62
                || policy.legacy
                || policy.pass_through
                || policy.resolved.is_some()
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (raw_space, raw_space, raw_space)
        })
    {
        return Err(CompositionError::Unsupported("Video Degradation canonical singleton envelope"));
    }
    let operation = typed_arguments(&node.arguments)?;
    if !current_epoch_parameters(&node.operation.schema_epoch, operation, node.enabled) {
        return Err(CompositionError::Unsupported("Video Degradation unwitnessed current parameter/enabled combination"));
    }
    VideoDegradationCandidate::new(profile, stored, PixelFormat::RGBA32F, owner, operation)
}
#[derive(Clone, Copy, Debug)]
pub struct VideoDegradationCandidate {
    operation: VideoDegradation,
    owner: Rect,
}
impl VideoDegradationCandidate {
    pub fn new(profile: &Profile, encoding: SampleEncoding, format: PixelFormat, owner: Rect, operation: VideoDegradation) -> Result<Self> {
        if profile != Builtin::Srgb.profile() || encoding != SampleEncoding::NonlinearProfile || format != PixelFormat::RGBA32F {
            return Err(CompositionError::Unsupported("Video Degradation unproved native/profile route"));
        }
        let size = (i64::from(owner.x1) - i64::from(owner.x0), i64::from(owner.y1) - i64::from(owner.y0));
        let origin = (owner.x0, owner.y0);
        // All option combinations have native controls at these two zero-origin
        // sizes. Only the unrotated, subtractive 3x3 pattern has offset controls.
        let zero_origin = origin == (0, 0) && matches!(size, (16, 16) | (32, 24));
        let offset_control = matches!(origin, (2, 1) | (-5, -3)) && size == (16, 16) && operation.pattern == 5 && !operation.additive && !operation.rotated;
        if !zero_origin && !offset_control {
            return Err(CompositionError::Unsupported("Video Degradation unproved owner extent"));
        }
        if operation.pattern > 8 {
            return Err(CompositionError::Unsupported("Video Degradation pattern ordinal"));
        }
        Ok(Self { operation, owner })
    }
    /// Checked world-coordinate to immutable drawable-local pattern phase.
    /// A different owner rectangle is a new qualification, not an ROI phase reset.
    pub fn try_eval_at(&self, point: (i32, i32), input: &[f64], output: &mut [f64], enabled: bool, ctl: &Interrupt<'_>) -> Result<()> {
        ctl.check().map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))?;
        if input.len() != 4 || output.len() != 4 {
            return Err(CompositionError::Invalid("Video Degradation pixel length"));
        }
        if !self.owner.contains(point.0, point.1) {
            return Err(CompositionError::Unsupported("Video Degradation point outside retained owner"));
        }
        let x = i32::try_from(i64::from(point.0) - i64::from(self.owner.x0)).map_err(|_| CompositionError::Invalid("Video Degradation local x"))?;
        let y = i32::try_from(i64::from(point.1) - i64::from(self.owner.y0)).map_err(|_| CompositionError::Invalid("Video Degradation local y"))?;
        if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v) || f64::from(*v as f32) != *v) {
            return Err(CompositionError::Unsupported("Video Degradation exact native Float input"));
        }
        let mut staged = [0.; 4];
        if enabled {
            let mut native = [0.; 4];
            for (d, v) in native.iter_mut().zip(input) {
                *d = *v as f32;
            }
            let mut result = [0.; 4];
            self.operation.try_eval_nonlinear_float(x, y, &native, &mut result)?;
            for (d, v) in staged.iter_mut().zip(result) {
                *d = f64::from(v);
            }
        } else {
            for (d, v) in staged.iter_mut().zip(input) {
                *d = *v;
            }
        }
        ctl.check().map_err(|_| CompositionError::Raster(photocraft_raster::RasterInputError::Cancelled))?;
        for (d, v) in output.iter_mut().zip(staged) {
            *d = v;
        }
        Ok(())
    }
}
