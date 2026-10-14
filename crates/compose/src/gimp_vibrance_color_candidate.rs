//! Caller-established Linear RGB Float ↔ physical LCh Float CMS candidate.
//!
//! Original profile bytes remain borrowed/exact. A separately owned effective
//! linear profile preserves primaries and replaces transfer interpretation via
//! the existing controlled CMS API; Linear never masquerades as NonlinearProfile.
//! The f64 XYZ/Lab/polar schedule with final Float rounding is a candidate, not
//! GEGL/source compositing qualification or an absent-profile alias.
use crate::gimp_hue_chroma_color_candidate as hue;
use photocraft_algo::gimp::{
    configured::Transfer,
    point_vibrance::{Context, ConversionRequirements, FloatConversions},
};
use photocraft_cms::{
    Intent, control, math,
    pipeline::Pipeline,
    profile::{ColorSpace, Pcs, Profile, ProfileClass},
    xcf_color::{self, Encoding},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};

const SCRATCH: u64 = 4096;
const PAIR_WORK: u64 = 256;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES.min(control::Limits::default().resident_bytes as u64), work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PreparationRequirements {
    pub borrowed_original_bytes: u64,
    pub effective_peak_bytes: u64,
    pub pipeline_reserve_bytes: u64,
    pub peak_bytes: u64,
    pub work_units: u64,
    pub effective_work_units: u64,
    pub side_work_units: u64,
}
pub struct PreparedLinearConversions<'a> {
    original: &'a Profile,
    effective: Profile,
    context: Context,
    ingress: Pipeline,
    egress: Pipeline,
    retained: u64,
    preparation_work: u64,
    limits: Limits,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn u64_len(n: usize) -> Result<u64, E> {
    u64::try_from(n).map_err(|_| E::LimitExceeded)
}
fn cms_limits(limits: Limits) -> Result<control::Limits, E> {
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.work_units > max.work_units {
        return Err(E::LimitExceeded);
    }
    Ok(control::Limits { resident_bytes: usize::try_from(limits.resident_bytes).map_err(|_| E::LimitExceeded)?, work_units: limits.work_units })
}
fn domain(profile: &Profile, context: Context, c: &Interrupt<'_>) -> Result<(), E> {
    poll(c)?;
    if context.rgb.transfer != Transfer::Linear
        || context.rgb.color_channels != 3
        || !context.rgb.alpha
        || context.rgb.profile_identity == [0; 32]
        || context.rgb.profile_identity != context.lch.profile_identity
        || context.lch.space_identity == [0; 32]
        || profile.color_space != ColorSpace::Rgb
        || profile.pcs != Pcs::Xyz
        || !matches!(profile.class, ProfileClass::Input | ProfileClass::Display | ProfileClass::ColorSpace)
        || profile.a2b.iter().any(Option::is_some)
        || profile.b2a.iter().any(Option::is_some)
        || profile.gray_trc.is_some()
    {
        return Err(E::UnsupportedConversion);
    }
    let matrix = profile.matrix.as_ref().ok_or(E::UnsupportedConversion)?;
    if matrix.iter().flatten().any(|v| !v.is_finite()) || math::invert(matrix).is_none() {
        return Err(E::UnsupportedConversion);
    }
    poll(c)
}
/// Borrowed, token-controlled census before effective/profile/pipeline copies.
/// Original side reservations conservatively cover their identity-curve linear
/// replacements; repeated validation and construction quotas share one envelope.
pub fn preparation_requirements(profile: &Profile, context: Context, limits: Limits, c: &Interrupt<'_>) -> Result<PreparationRequirements, E> {
    domain(profile, context, c)?;
    let lower = cms_limits(limits)?;
    let cancelled = || c.check().is_err();
    let effective = xcf_color::effective_profile_requirements_with_control(profile, Encoding::Linear, lower, &cancelled).map_err(hue::cms_error)?;
    let input = profile.side_requirements_with_control(true, Intent::RelativeColorimetric, lower, &cancelled).map_err(hue::cms_error)?;
    let output = profile.side_requirements_with_control(false, Intent::RelativeColorimetric, lower, &cancelled).map_err(hue::cms_error)?;
    let borrowed_original_bytes = u64_len(effective.retained_bytes)?;
    let effective_peak_bytes = u64_len(effective.peak_resident_bytes)?;
    let pipeline_reserve_bytes = add(u64_len(input.staged_bytes)?, u64_len(output.staged_bytes)?)?;
    let peak_bytes = add(add(effective_peak_bytes, pipeline_reserve_bytes)?, SCRATCH)?;
    // The effective bound includes validation, cloning and writing. Repeated
    // requirements and both side builders receive separate debited quotas.
    let effective_work_units = effective.work_units.checked_mul(2).ok_or(E::LimitExceeded)?;
    let side_work_units = add(add(input.work_units, output.work_units)?, effective.work_units)?.checked_mul(2).ok_or(E::LimitExceeded)?;
    let work_units = add(add(effective_work_units, side_work_units.checked_mul(2).ok_or(E::LimitExceeded)?)?, 1024)?;
    if peak_bytes > limits.resident_bytes || add(work_units, PAIR_WORK)? > limits.work_units {
        return Err(E::LimitExceeded);
    }
    poll(c)?;
    Ok(PreparationRequirements {
        borrowed_original_bytes,
        effective_peak_bytes,
        pipeline_reserve_bytes,
        peak_bytes,
        work_units,
        effective_work_units,
        side_work_units,
    })
}
impl<'a> PreparedLinearConversions<'a> {
    pub fn prepare(original: &'a Profile, context: Context, limits: Limits, c: &Interrupt<'_>) -> Result<Self, E> {
        let req = preparation_requirements(original, context, limits, c)?;
        let cancelled = || c.check().is_err();
        let effective_limits = cms_limits(Limits {
            resident_bytes: limits.resident_bytes.checked_sub(add(req.pipeline_reserve_bytes, SCRATCH)?).ok_or(E::LimitExceeded)?,
            work_units: req.effective_work_units,
        })?;
        let effective =
            xcf_color::effective_profile_from_definition_with_control(original, Encoding::Linear, effective_limits, &cancelled).map_err(hue::cms_error)?;
        poll(c)?;
        let mut remaining_work = limits.work_units.checked_sub(req.effective_work_units).and_then(|w| w.checked_sub(1024)).ok_or(E::LimitExceeded)?;
        if req.side_work_units > remaining_work {
            return Err(E::LimitExceeded);
        }
        let side_resident = limits.resident_bytes.checked_sub(add(req.borrowed_original_bytes, SCRATCH)?).ok_or(E::LimitExceeded)?;
        let lower = cms_limits(Limits { resident_bytes: side_resident, work_units: req.side_work_units })?;
        let input_req = effective.side_requirements_with_control(true, Intent::RelativeColorimetric, lower, &cancelled).map_err(hue::cms_error)?;
        let output_req = effective.side_requirements_with_control(false, Intent::RelativeColorimetric, lower, &cancelled).map_err(hue::cms_error)?;
        let effective_retained = u64_len(input_req.retained_profile_bytes.max(output_req.retained_profile_bytes))?;
        let retained_profiles = add(req.borrowed_original_bytes, effective_retained)?;
        let input_limits = cms_limits(Limits { resident_bytes: side_resident, work_units: req.side_work_units })?;
        let (input, pcs) = effective.device_to_pcs_with_control(Intent::RelativeColorimetric, input_limits, &cancelled).map_err(hue::cms_error)?;
        if pcs != Pcs::Xyz {
            return Err(E::UnsupportedConversion);
        }
        let ingress = Pipeline::new(3, input);
        let ingress_bytes = hue::pipeline_bytes(&ingress, hue::TrcPolicy::StrictAnalytic)?;
        remaining_work = remaining_work.checked_sub(req.side_work_units).ok_or(E::LimitExceeded)?;
        if req.side_work_units > remaining_work {
            return Err(E::LimitExceeded);
        }
        let output_limits =
            cms_limits(Limits { resident_bytes: side_resident.checked_sub(ingress_bytes).ok_or(E::LimitExceeded)?, work_units: req.side_work_units })?;
        let (output, pcs) = effective.pcs_to_device_with_control(Intent::RelativeColorimetric, output_limits, &cancelled).map_err(hue::cms_error)?;
        if pcs != Pcs::Xyz {
            return Err(E::UnsupportedConversion);
        }
        let egress = Pipeline::new(3, output);
        let pipeline_bytes = add(ingress_bytes, hue::pipeline_bytes(&egress, hue::TrcPolicy::StrictAnalytic)?)?;
        let retained = add(retained_profiles, pipeline_bytes)?;
        if pipeline_bytes > req.pipeline_reserve_bytes || add(retained, SCRATCH)? > req.peak_bytes {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(Self { original, effective, context, ingress, egress, retained, preparation_work: req.work_units, limits })
    }
    pub fn original_profile(&self) -> &'a Profile {
        self.original
    }
    pub fn effective_linear_profile(&self) -> &Profile {
        &self.effective
    }
    pub fn preparation_work_units(&self) -> u64 {
        self.preparation_work
    }
    fn finite(value: [f32; 4]) -> Result<(), E> {
        if value.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&value[3]) {
            return Err(E::InvalidSample);
        }
        Ok(())
    }
}
impl FloatConversions for PreparedLinearConversions<'_> {
    fn context(&self) -> Context {
        self.context
    }
    fn requirements(&self, c: &Interrupt<'_>) -> Result<ConversionRequirements, E> {
        poll(c)?;
        let work_units = add(self.preparation_work, PAIR_WORK)?;
        if add(self.retained, SCRATCH)? > self.limits.resident_bytes || work_units > self.limits.work_units {
            return Err(E::LimitExceeded);
        }
        // The enclosing candidate debits preparation once, then paired pixel
        // work for every selected pixel. Nothing refunds upstream preparation.
        Ok(ConversionRequirements {
            retained_bytes: self.retained,
            scratch_bytes: SCRATCH,
            preparation_work_units: self.preparation_work,
            work_units: PAIR_WORK,
        })
    }
    fn rgb_to_lch(&self, value: [f32; 4], c: &Interrupt<'_>) -> Result<[f32; 4], E> {
        self.requirements(c)?;
        Self::finite(value)?;
        let input = [f64::from(value[0]), f64::from(value[1]), f64::from(value[2])];
        let mut xyz = [0.0; 3];
        self.ingress.try_eval_f64(&input, &mut xyz).map_err(|_| E::InvalidSample)?;
        poll(c)?;
        let output = hue::lab_to_lch_float(math::xyz_to_lab(xyz, math::D50), value[3])?;
        poll(c)?;
        Ok(output)
    }
    fn lch_to_rgb(&self, value: [f32; 4], c: &Interrupt<'_>) -> Result<[f32; 4], E> {
        self.requirements(c)?;
        Self::finite(value)?;
        let xyz = math::lab_to_xyz(hue::lch_to_lab(value)?, math::D50);
        if xyz.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        let mut rgb = [0.0; 3];
        self.egress.try_eval_f64(&xyz, &mut rgb).map_err(|_| E::InvalidSample)?;
        poll(c)?;
        Ok([hue::narrow(rgb[0])?, hue::narrow(rgb[1])?, hue::narrow(rgb[2])?, value[3]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::{
        configured::WorkingContext,
        hue_chroma::LchFloatContext,
        point_vibrance::{self, Parameters},
    };
    fn context() -> Context {
        Context {
            rgb: WorkingContext { transfer: Transfer::Linear, profile_identity: [7; 32], color_channels: 3, alpha: true },
            lch: LchFloatContext { profile_identity: [7; 32], space_identity: [9; 32] },
        }
    }
    #[test]
    fn linear_interpretation_preserves_original_profile_and_alpha() {
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let before = original.to_bytes();
        let prepared = PreparedLinearConversions::prepare(original, context(), Limits::default(), &Interrupt::NONE).unwrap();
        assert!(std::ptr::eq(prepared.original_profile(), original));
        assert_eq!(prepared.context().rgb.transfer, Transfer::Linear);
        assert_eq!(original.to_bytes().as_slice(), before.as_slice());
        assert!(prepared.effective_linear_profile().trc.as_ref().unwrap().iter().all(|c| matches!(c, photocraft_cms::curve::Curve::Identity)));
        let pixel = [0.25, 0.5, 0.75, -0.0];
        let lch = prepared.rgb_to_lch(pixel, &Interrupt::NONE).unwrap();
        let rgb = prepared.lch_to_rgb(lch, &Interrupt::NONE).unwrap();
        assert_eq!(lch[3].to_bits(), pixel[3].to_bits());
        assert_eq!(rgb[3].to_bits(), pixel[3].to_bits());
        assert!(rgb.iter().all(|v| v.is_finite()));
        let output = point_vibrance::evaluate(pixel, Parameters { vibrance: 12.5, saturation: 1.0 }, context(), &prepared, &Interrupt::NONE).unwrap();
        assert_eq!(output[3].to_bits(), pixel[3].to_bits());
        assert!(output.iter().all(|v| v.is_finite()));
    }
    #[test]
    fn no_encoding_alias_or_profile_identity_alias_and_cancel() {
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let mut wrong = context();
        wrong.rgb.transfer = Transfer::NonlinearProfile;
        assert_eq!(PreparedLinearConversions::prepare(original, wrong, Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        wrong = context();
        wrong.lch.profile_identity = [8; 32];
        assert_eq!(PreparedLinearConversions::prepare(original, wrong, Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        assert_eq!(PreparedLinearConversions::prepare(original, context(), Limits::default(), &Interrupt::cancel_only(&|| true)).err(), Some(E::Cancelled));
        let prepared = PreparedLinearConversions::prepare(original, context(), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(prepared.rgb_to_lch([f32::NAN, 0.0, 0.0, 1.0], &Interrupt::NONE).err(), Some(E::InvalidSample));
        assert_eq!(prepared.lch_to_rgb([50.0, 20.0, 60.0, 1.0], &Interrupt::cancel_only(&|| true)).err(), Some(E::Cancelled));
    }
    #[test]
    fn preparation_and_steady_pair_have_separate_immutable_quotas() {
        let original = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let requirements = preparation_requirements(original, context(), Limits::default(), &Interrupt::NONE).unwrap();
        let low = Limits { work_units: requirements.work_units + PAIR_WORK - 1, ..Limits::default() };
        assert_eq!(PreparedLinearConversions::prepare(original, context(), low, &Interrupt::NONE).err(), Some(E::LimitExceeded));
        let limits = Limits { work_units: requirements.work_units + PAIR_WORK, ..Limits::default() };
        let prepared = PreparedLinearConversions::prepare(original, context(), limits, &Interrupt::NONE).unwrap();
        let bound = prepared.requirements(&Interrupt::NONE).unwrap();
        assert_eq!(bound.preparation_work_units, requirements.work_units);
        assert_eq!(bound.work_units, PAIR_WORK);
        assert!(bound.retained_bytes + bound.scratch_bytes <= limits.resident_bytes);
        assert_eq!(
            PreparedLinearConversions::prepare(original, context(), Limits { resident_bytes: 1, ..limits }, &Interrupt::NONE).err(),
            Some(E::LimitExceeded)
        );
    }
}
