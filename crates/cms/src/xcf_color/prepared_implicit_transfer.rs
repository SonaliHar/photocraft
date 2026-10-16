//! One validated implicit RGB/Gray transfer basis per source operation.
//!
//! These are the same actual builtins used by legacy source qualification.
//! This factory accepts neither a caller profile nor an assigned-profile claim.
//! Builtin OnceLock initialization is a fixed process-global provider boundary;
//! borrowed profile bytes are disclosed separately from new inline/scratch state.
use super::Encoding;
use crate::{
    Builtin, CmsError, ColorSpace, Profile,
    control::{Budget, Error, Limits, Result},
};

/// O(1) facts admission, before accessing any lazy builtin provider.
pub const fn planning_work_ceiling() -> u64 {
    8192
}

#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub preparation_work_units: u64,
    pub conversion_work_units: u64,
    /// Actual existing immutable builtin model, not a new job-owned allocation.
    pub borrowed_profile_bytes: usize,
    pub inline_state_bytes: usize,
    pub conversion_scratch_bytes: usize,
    /// Additional fixed state plus maximum conversion scratch for the caller.
    pub peak_resident_bytes: usize,
    /// Component admission includes the borrowed model as well as that new peak.
    pub component_resident_bytes: usize,
    semantic_work_units: u64,
}

pub struct PreparedImplicitTransfer {
    profile: &'static Profile,
    from: Encoding,
    to: Encoding,
    requirements: Requirements,
}

fn unsupported() -> Error {
    CmsError::Unsupported("prepared implicit transfer requires the actual RGB/Gray sRGB-curve builtin basis".into()).into()
}
fn basis(space: ColorSpace) -> Result<&'static Profile> {
    match space {
        ColorSpace::Rgb => Ok(Builtin::Srgb.profile()),
        ColorSpace::Gray => Ok(Builtin::SGray.profile()),
        _ => Err(unsupported()),
    }
}
fn shape(profile: &Profile, space: ColorSpace) -> Result<()> {
    if profile.color_space != space || profile.a2b.iter().chain(&profile.b2a).any(Option::is_some) {
        return Err(unsupported());
    }
    // Exact canonical curves have direct analytic inverses. Arbitrary analytic
    // parametric curves can enter a 60-iteration inverse fallback and are not
    // silently assigned the fixed per-conversion work facts used here.
    let expected = crate::curve::srgb_trc();
    let valid = match space {
        ColorSpace::Rgb => profile.trc.as_ref().is_some_and(|curves| curves.iter().all(|curve| curve == &expected)),
        ColorSpace::Gray => profile.gray_trc.as_ref() == Some(&expected),
        _ => false,
    };
    if valid { Ok(()) } else { Err(unsupported()) }
}

/// Borrowed facts only; callers prepay planning before this function and the
/// returned constructor work before construction on their original operation.
pub fn requirements_with_control(space: ColorSpace, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Requirements> {
    let mut budget = Budget::new(limits, cancelled)?;
    budget.work(128)?;
    let profile = basis(space)?;
    budget.poll()?;
    shape(profile, space)?;
    let borrowed_profile_bytes = profile.controlled_retained_bytes(&budget)?;
    budget.work(128)?;
    let semantic_work_units = super::semantic_analytic_profile_work_ceiling(profile)?;
    let preparation_work_units = semantic_work_units.checked_add(2 * planning_work_ceiling()).ok_or(Error::Limit("implicit transfer preparation work"))?;
    let conversion_work_units = u64::try_from(profile.channels())
        .map_err(|_| Error::Limit("implicit transfer channels"))?
        .checked_mul(2 * 128)
        .and_then(|work| work.checked_add(128))
        .ok_or(Error::Limit("implicit transfer conversion work"))?;
    let inline_state_bytes = std::mem::size_of::<PreparedImplicitTransfer>();
    // Private output, the unchanged legacy numeric body's local output/curve,
    // controlled-call state, and fixed scalar temporaries coexist.
    let conversion_scratch_bytes =
        2 * std::mem::size_of::<[f64; 3]>() + std::mem::size_of::<crate::Curve>() + std::mem::size_of::<Budget<'static>>() + 8 * std::mem::size_of::<f64>();
    let peak_resident_bytes = inline_state_bytes.checked_add(conversion_scratch_bytes).ok_or(Error::Limit("implicit transfer state/scratch"))?;
    let component_resident_bytes = borrowed_profile_bytes.checked_add(peak_resident_bytes).ok_or(Error::Limit("implicit transfer component peak"))?;
    budget.admit(component_resident_bytes, 0)?;
    if preparation_work_units > limits.work_units {
        return Err(Error::Limit("implicit transfer preparation work"));
    }
    budget.poll()?;
    Ok(Requirements {
        preparation_work_units,
        conversion_work_units,
        borrowed_profile_bytes,
        inline_state_bytes,
        conversion_scratch_bytes,
        peak_resident_bytes,
        component_resident_bytes,
        semantic_work_units,
    })
}

/// Validate the immutable actual builtin once. No profile clone, writer,
/// transform, heap buffer, encoding alias or source eligibility is created.
pub fn construct_with_control(
    space: ColorSpace,
    from: Encoding,
    to: Encoding,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedImplicitTransfer> {
    let requirements = requirements_with_control(space, limits, cancelled)?;
    let mut budget = Budget::new(limits, cancelled)?;
    budget.retained(requirements.component_resident_bytes)?;
    budget.work(requirements.preparation_work_units)?;
    let profile = basis(space)?;
    super::validate_profile_with_control(
        profile,
        Limits { resident_bytes: requirements.borrowed_profile_bytes, work_units: requirements.semantic_work_units },
        cancelled,
    )?;
    budget.poll()?;
    Ok(PreparedImplicitTransfer { profile, from, to, requirements })
}

impl PreparedImplicitTransfer {
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }

    /// Keep the exact legacy arithmetic/finite/identity rules. Only the already
    /// completed immutable profile validation is omitted. Output is published
    /// after the final cancellation poll, including for identity conversion.
    pub fn convert_with_control(&self, samples: &[f64], output: &mut [f64], limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let mut budget = Budget::new(limits, cancelled)?;
        budget.retained(self.requirements.conversion_scratch_bytes)?;
        budget.work(self.requirements.conversion_work_units)?;
        let channels = self.profile.channels();
        if samples.len() != channels || output.len() != channels || samples.iter().any(|value| !value.is_finite()) {
            return Err(CmsError::Invalid("XCF transfer sample count or nonfinite input".into()).into());
        }
        let mut temporary = [0.0_f64; 3];
        let temporary = temporary.get_mut(..channels).ok_or_else(unsupported)?;
        super::convert_encoding_f64_validated(self.profile, self.from, self.to, samples, temporary)?;
        budget.poll()?;
        output.copy_from_slice(temporary);
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_implicit_transfer_tests.rs"]
mod tests;
