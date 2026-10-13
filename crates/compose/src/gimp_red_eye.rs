//! Explicit-profile Red Eye working conversion. Source admission is separate.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::point_red_eye::RedEyeRemoval;
use photocraft_cms::{
    Builtin, Intent, Profile, Transform,
    xcf_color::{Encoding, convert_encoding_f64, effective_profile_from_definition},
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::{Interrupt, RasterInputError};
#[cfg(test)]
#[path = "gimp_red_eye_geometry_tests.rs"]
mod geometry_tests;

pub const RED_EYE_PRECISION_EPOCH: &str = crate::gimp_filters::RED_EYE_PRECISION_EPOCH;
pub const RED_EYE_OWNER_GEOMETRY_EPOCH: &str = "xcf-22-stock-red-eye-removal-owner-geometry-v1";
pub(crate) const OWNER_GEOMETRY_RAW_SPACE: i32 = -487751087;

/// Closed observed owner tuples. This metadata predicate supplies no pixel authority.
pub fn qualified_owner_geometry(epoch: &str, canvas: (u32, u32), owner: (u32, u32), offset: (i32, i32)) -> bool {
    match epoch {
        crate::gimp_filters::RED_EYE_PRECISION_EPOCH => (canvas, owner, offset) == ((8, 6), (8, 6), (0, 0)),
        RED_EYE_OWNER_GEOMETRY_EPOCH => matches!((canvas, owner, offset), ((37, 29), (19, 13), (-7, 5)) | ((259, 17), (262, 9), (-3, -4))),
        _ => false,
    }
}
pub(crate) fn claims_geometry(node: &photocraft_doc::xcf_filters::FilterNode) -> bool {
    node.operation.schema_epoch == RED_EYE_OWNER_GEOMETRY_EPOCH
        || node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == RED_EYE_OWNER_GEOMETRY_EPOCH)
}
/// Scalar command candidacy only; rendering retains owner/profile/mask authority.
pub fn current_epoch_parameters(epoch: &str, arguments: &[FilterArgument]) -> bool {
    matches!(epoch, crate::gimp_filters::RED_EYE_PRECISION_EPOCH | RED_EYE_OWNER_GEOMETRY_EPOCH) && typed_threshold(arguments).is_ok()
}
pub(crate) fn canonical_geometry(node: &photocraft_doc::xcf_filters::FilterNode, profile: &Profile, stored: SampleEncoding, version: u32) -> Result<()> {
    if node.operation.name != "gegl:red-eye-removal"
        || node.operation.schema_epoch != RED_EYE_OWNER_GEOMETRY_EPOCH
        || node.operation.stored_version != "0:0"
        || version != 22
        || profile.to_bytes() != Builtin::Srgb.profile().to_bytes()
        || stored != SampleEncoding::NonlinearProfile
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || node.composition.as_ref().is_none_or(|policy| {
            policy.compatibility_epoch != RED_EYE_OWNER_GEOMETRY_EPOCH || policy.source_mode != 62 || policy.legacy || policy.pass_through
                || policy.resolved.is_some()
                || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator)
                    != (OWNER_GEOMETRY_RAW_SPACE, OWNER_GEOMETRY_RAW_SPACE, OWNER_GEOMETRY_RAW_SPACE)
        })
    {
        return Err(CompositionError::Unsupported("Red Eye owner geometry canonical envelope"));
    }
    typed_threshold(&node.arguments)?;
    Ok(())
}

pub fn typed_threshold(arguments: &[FilterArgument]) -> Result<f64> {
    let [argument] = arguments else {
        return Err(CompositionError::Unsupported("Red Eye argument count"));
    };
    if argument.name != "threshold" {
        return Err(CompositionError::Unsupported("Red Eye argument name"));
    }
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else {
        return Err(CompositionError::Unsupported("Red Eye stored threshold type"));
    };
    let value = f64::from(f32::from_bits(bits));
    if !value.is_finite() || !(0.0..=0.8).contains(&value) {
        return Err(CompositionError::Unsupported("Red Eye threshold range"));
    }
    Ok(value)
}

pub struct RedEyeCandidate {
    profile: Profile,
    stored: Encoding,
    channels: usize,
    gray_output: Option<Transform>,
    operation: RedEyeRemoval,
}
impl RedEyeCandidate {
    pub fn new(profile: &Profile, encoding: SampleEncoding, threshold: f64) -> Result<Self> {
        if profile != Builtin::Srgb.profile() && profile != Builtin::SGray.profile() {
            return Err(CompositionError::Unsupported("Red Eye unproved current profile"));
        }
        if !threshold.is_finite() || !(0.0..=0.8).contains(&threshold) {
            return Err(CompositionError::Unsupported("Red Eye threshold range"));
        }
        let stored = crate::gimp_composite::encoding(encoding);
        let gray_output = if profile.channels() == 1 {
            Some(Transform::new(Builtin::Srgb.profile(), &effective_profile_from_definition(profile, stored)?, Intent::RelativeColorimetric, false)?)
        } else {
            None
        };
        Ok(Self { profile: profile.clone(), stored, channels: profile.channels(), gray_output, operation: RedEyeRemoval { threshold } })
    }
    fn validate(&self, pixel: &[f64]) -> Result<()> {
        if pixel.len() != self.channels.saturating_add(1)
            || pixel.iter().any(|v| !v.is_finite())
            || !pixel.get(self.channels).is_some_and(|v| (0.0..=1.0).contains(v))
        {
            return Err(CompositionError::Unsupported("Red Eye finite native input/unit alpha"));
        }
        Ok(())
    }
    pub fn working_input(&self, pixel: &[f64]) -> Result<[f64; 4]> {
        self.validate(pixel)?;
        let mut rgb = [0.; 3];
        convert_encoding_f64(
            &self.profile,
            self.stored,
            Encoding::NonlinearProfile,
            pixel.get(..self.channels).ok_or(CompositionError::Invalid("Red Eye input channels"))?,
            rgb.get_mut(..self.channels).ok_or(CompositionError::Invalid("Red Eye working channels"))?,
        )?;
        if self.channels == 1 {
            rgb = [rgb[0]; 3];
        }
        if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(&(*v as f32))) {
            return Err(CompositionError::Unsupported("Red Eye nonlinear Float unit working input"));
        }
        Ok([
            f64::from(rgb[0] as f32),
            f64::from(rgb[1] as f32),
            f64::from(rgb[2] as f32),
            f64::from(*pixel.get(self.channels).ok_or(CompositionError::Invalid("Red Eye alpha"))? as f32),
        ])
    }
    pub fn apply_with(
        &self,
        pixel: &[f64],
        enabled: bool,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> std::result::Result<(), RasterInputError>,
    ) -> Result<[f64; 4]> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        self.validate(pixel)?;
        work(if enabled { 16 } else { 1 })?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if !enabled {
            let alpha = *pixel.get(self.channels).ok_or(CompositionError::Invalid("Red Eye alpha"))?;
            return if self.channels == 1 {
                let gray = *pixel.first().ok_or(CompositionError::Invalid("Red Eye gray"))?;
                Ok([gray, gray, gray, alpha])
            } else {
                pixel.try_into().map_err(|_| CompositionError::Invalid("Red Eye RGBA"))
            };
        }
        let input = self.working_input(pixel)?.map(|v| v as f32);
        let mut output = [0.; 4];
        self.operation.try_eval_nonlinear_float(&input, &mut output)?;
        let output = output.map(f64::from);
        let mut native = [0.; 3];
        if let Some(transform) = &self.gray_output {
            transform.try_eval_f64(&output[..3], native.get_mut(..1).ok_or(CompositionError::Invalid("Red Eye gray output"))?)?;
            native = [native[0]; 3];
        } else {
            convert_encoding_f64(&self.profile, Encoding::NonlinearProfile, self.stored, &output[..3], &mut native)?;
        }
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if native.iter().any(|v| !v.is_finite()) {
            return Err(CompositionError::Unsupported("Red Eye nonfinite native output"));
        }
        Ok([native[0], native[1], native[2], output[3]])
    }
}
