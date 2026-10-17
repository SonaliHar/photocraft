//! XCF transfer interpretation using ICC primaries and curves without converting source pixels.
use crate::{Builtin, CmsError, ColorSpace, Curve, Profile};
use std::sync::Arc;

mod assigned_effective_plan;
pub use assigned_effective_plan::{assigned_effective_planning_work_ceiling, semantic_analytic_profile_work_ceiling};

mod implicit_planner;
pub use implicit_planner::effective_implicit_rgb_nonlinear_planner_work_with_control;

pub mod prepared_implicit_transfer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Linear,
    NonlinearProfile,
    Perceptual,
}

/// Storage precision is independent of the source transfer function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplePrecision {
    U8,
    U16,
    U32,
    F16,
    F32,
    F64,
}

impl SamplePrecision {
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::F16 => 2,
            Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    pub const fn is_float(self) -> bool {
        matches!(self, Self::F16 | Self::F32 | Self::F64)
    }
}

/// Nonfinite payloads can be retained as original bits, but are never evaluated as colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonFinitePolicy {
    Reject,
    PreserveBits,
}

pub fn validate_sample(value: f64, precision: SamplePrecision, policy: NonFinitePolicy) -> Result<(), CmsError> {
    if !value.is_finite() {
        return if precision.is_float() && policy == NonFinitePolicy::PreserveBits {
            Ok(())
        } else {
            Err(CmsError::Invalid("nonfinite XCF sample requires floating storage and explicit bit preservation".into()))
        };
    }
    if !precision.is_float() && !(0.0..=1.0).contains(&value) {
        return Err(CmsError::Invalid("integer XCF sample outside normalized range".into()));
    }
    // Finite HDR is not clamped. The storage codec, rather than CMS, owns rounding/bit layout.
    Ok(())
}

/// Original assigned/source/working profile identities alongside derived interpretation.
///
/// The caller supplies the effective source explicitly even when no ICC was assigned; no
/// default profile is invented here. This retains the distinction between absent assignment
/// and a built-in effective profile. Construction never converts authoritative samples.
#[derive(Clone, Debug)]
pub struct SourceColorContext {
    assigned: Option<Arc<Vec<u8>>>,
    source: Profile,
    working: Profile,
    source_effective: Profile,
    working_effective: Profile,
    transform: Arc<crate::Transform>,
    source_encoding: Encoding,
    working_encoding: Encoding,
    precision: SamplePrecision,
    nonfinite_policy: NonFinitePolicy,
}

impl SourceColorContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        assigned: Option<&[u8]>,
        source: &[u8],
        working: &[u8],
        space: ColorSpace,
        source_encoding: Encoding,
        working_encoding: Encoding,
        precision: SamplePrecision,
        nonfinite_policy: NonFinitePolicy,
    ) -> Result<Self, CmsError> {
        let source_profile = parse_bounded_profile(source, space)?;
        let working_signature: [u8; 4] = working.get(16..20).ok_or(CmsError::Truncated)?.try_into().map_err(|_| CmsError::Truncated)?;
        let working_space = ColorSpace::from_sig(u32::from_be_bytes(working_signature));
        let working_profile = parse_bounded_profile(working, working_space)?;
        let assigned = assigned.map(|bytes| parse_bounded_profile(bytes, space).map(|p| p.to_bytes())).transpose()?;
        let source_effective = effective_profile(Some(source), space, source_encoding)?;
        let working_effective = effective_profile(Some(working), working_space, working_encoding)?;
        let transform = Arc::new(crate::Transform::new(&source_effective, &working_effective, crate::Intent::RelativeColorimetric, false)?);
        Ok(Self {
            assigned,
            source: source_profile,
            working: working_profile,
            source_effective,
            working_effective,
            transform,
            source_encoding,
            working_encoding,
            precision,
            nonfinite_policy,
        })
    }
    pub fn assigned_bytes(&self) -> Option<Arc<Vec<u8>>> {
        self.assigned.clone()
    }
    pub fn source_bytes(&self) -> Arc<Vec<u8>> {
        self.source.to_bytes()
    }
    pub fn working_bytes(&self) -> Arc<Vec<u8>> {
        self.working.to_bytes()
    }
    pub fn source_profile(&self) -> &Profile {
        &self.source_effective
    }
    pub fn working_profile(&self) -> &Profile {
        &self.working_effective
    }
    pub fn source_encoding(&self) -> Encoding {
        self.source_encoding
    }
    pub fn working_encoding(&self) -> Encoding {
        self.working_encoding
    }
    pub fn precision(&self) -> SamplePrecision {
        self.precision
    }
    pub fn nonfinite_policy(&self) -> NonFinitePolicy {
        self.nonfinite_policy
    }
    pub fn validate_stored_sample(&self, value: f64) -> Result<(), CmsError> {
        validate_sample(value, self.precision, self.nonfinite_policy)
    }
    /// Relative-colorimetric conversion through the exact matrix/TRC pipeline. No BPC or
    /// display quantization is implied, and the source profiles/samples are not modified.
    pub fn to_working_f64(&self, input: &[f64], output: &mut [f64]) -> Result<(), CmsError> {
        for &sample in input {
            self.validate_stored_sample(sample)?;
        }
        self.transform.try_eval_f64(input, output)
    }
}

/// Strict XCF profile admission (unlike the compatibility-oriented general ICC reader).
pub fn parse_bounded_profile(bytes: &[u8], space: ColorSpace) -> Result<Profile, CmsError> {
    if bytes.len() > 16 << 20 {
        return Err(CmsError::Invalid("XCF ICC exceeds 16 MiB".into()));
    }
    let read_u32 = |offset: usize| -> Result<u32, CmsError> {
        let end = offset.checked_add(4).ok_or(CmsError::Truncated)?;
        let value: [u8; 4] = bytes.get(offset..end).ok_or(CmsError::Truncated)?.try_into().map_err(|_| CmsError::Truncated)?;
        Ok(u32::from_be_bytes(value))
    };
    let declared = usize::try_from(read_u32(0)?).map_err(|_| CmsError::Truncated)?;
    if declared < 132 || declared > bytes.len() {
        return Err(CmsError::Invalid("invalid XCF ICC declared size".into()));
    }
    if !matches!(bytes.get(8), Some(2 | 4)) || !matches!(bytes.get(20..24), Some(b"XYZ " | b"Lab ")) {
        return Err(CmsError::Unsupported("XCF ICC version or PCS".into()));
    }
    // The general ICC reader deliberately masks/falls back for legacy intent
    // values. Strict source interpretation must not silently change this field.
    if read_u32(64)? > 3 {
        return Err(CmsError::Invalid("invalid XCF ICC rendering intent".into()));
    }
    let count = usize::try_from(read_u32(128)?).map_err(|_| CmsError::Truncated)?;
    if count > 1024 {
        return Err(CmsError::Invalid("too many XCF ICC tags".into()));
    }
    let table_end = 132_usize.checked_add(count.checked_mul(12).ok_or(CmsError::Truncated)?).ok_or(CmsError::Truncated)?;
    if table_end > declared {
        return Err(CmsError::Truncated);
    }
    for i in 0..count {
        let offset = 132 + i * 12;
        let signature = read_u32(offset)?;
        let start = usize::try_from(read_u32(offset + 4)?).map_err(|_| CmsError::Truncated)?;
        let size = usize::try_from(read_u32(offset + 8)?).map_err(|_| CmsError::Truncated)?;
        let end = start.checked_add(size).ok_or(CmsError::Truncated)?;
        if start < table_end || end > declared {
            return Err(CmsError::Invalid("XCF ICC tag lies outside profile data".into()));
        }
        // Shared complete tag payloads (such as identical RGB TRCs) are valid;
        // duplicate definitions and partial overlaps are ambiguous, not a
        // reason to choose whichever definition the compatibility reader sees first.
        for previous in 0..i {
            let previous_offset = 132 + previous * 12;
            let previous_signature = read_u32(previous_offset)?;
            let previous_start = usize::try_from(read_u32(previous_offset + 4)?).map_err(|_| CmsError::Truncated)?;
            let previous_size = usize::try_from(read_u32(previous_offset + 8)?).map_err(|_| CmsError::Truncated)?;
            let previous_end = previous_start.checked_add(previous_size).ok_or(CmsError::Truncated)?;
            if signature == previous_signature {
                return Err(CmsError::Invalid("duplicate XCF ICC tag definition".into()));
            }
            if start < previous_end && previous_start < end && (start != previous_start || size != previous_size) {
                return Err(CmsError::Invalid("partially overlapping XCF ICC tag data".into()));
            }
        }
        let data = bytes.get(start..end).ok_or(CmsError::Truncated)?;
        // Profile::parse intentionally discards malformed optional bkpt/chad
        // tags for compatibility. Here presence is meaningful: refuse a malformed
        // value rather than treating a source black point/adaptation as absent.
        if signature == u32::from_be_bytes(*b"bkpt") && (size != 20 || data.get(..4) != Some(b"XYZ ".as_slice())) {
            return Err(CmsError::Invalid("invalid XCF ICC black-point tag".into()));
        }
        if signature == u32::from_be_bytes(*b"chad") && (size != 44 || data.get(..4) != Some(b"sf32".as_slice())) {
            return Err(CmsError::Invalid("invalid XCF ICC chromatic-adaptation tag".into()));
        }
    }
    let p = Profile::parse(bytes)?;
    if p.color_space != space {
        return Err(CmsError::Invalid("XCF profile color model mismatch".into()));
    }
    validate_profile(&p)?;
    Ok(p)
}

/// Reject malformed/noninvertible interpretation rather than replacing curves with sRGB.
pub fn validate_profile(p: &Profile) -> Result<(), CmsError> {
    if !matches!(p.color_space, ColorSpace::Rgb | ColorSpace::Gray) {
        return Err(CmsError::Unsupported("XCF color model".into()));
    }
    if p.white_point.iter().any(|value| !value.is_finite())
        || p.black_point.is_some_and(|point| point.iter().any(|value| !value.is_finite()))
        || p.chad.as_ref().is_some_and(|matrix| matrix.iter().flatten().any(|value| !value.is_finite()) || crate::math::invert(matrix).is_none())
    {
        return Err(CmsError::Invalid("nonfinite XCF profile point or noninvertible chromatic adaptation".into()));
    }
    if p.a2b.iter().any(Option::is_some) || p.b2a.iter().any(Option::is_some) {
        for lut in p.a2b.iter().flatten() {
            validate_lut(lut, p.channels(), 3)?;
        }
        for lut in p.b2a.iter().flatten() {
            validate_lut(lut, 3, p.channels())?;
        }
        return Ok(());
    }
    match p.color_space {
        ColorSpace::Rgb => {
            let matrix = p.matrix.as_ref().ok_or_else(|| CmsError::Invalid("missing XCF RGB colorants".into()))?;
            if matrix.iter().flatten().any(|v| !v.is_finite()) || crate::math::invert(matrix).is_none() {
                return Err(CmsError::Invalid("nonfinite or singular XCF colorant matrix".into()));
            }
            let curves = p.trc.as_ref().ok_or_else(|| CmsError::Invalid("missing XCF RGB curves".into()))?;
            for curve in curves {
                validate_curve(curve)?;
            }
        }
        ColorSpace::Gray => validate_curve(p.gray_trc.as_ref().ok_or_else(|| CmsError::Invalid("missing XCF gray curve".into()))?)?,
        _ => return Err(CmsError::Unsupported("XCF color model".into())),
    }
    Ok(())
}

fn validate_lut(lut: &crate::profile::Lut, inputs: usize, outputs: usize) -> Result<(), CmsError> {
    use crate::pipeline::Stage;
    let invalid = || CmsError::Invalid("invalid XCF ICC LUT dimensions, curves or nodes".into());
    if lut.inputs != inputs || lut.outputs != outputs {
        return Err(invalid());
    }
    let mut channels = inputs;
    for stage in &lut.stages {
        match stage {
            Stage::Curves(curves) | Stage::InvCurves(curves) => {
                if curves.len() != channels {
                    return Err(invalid());
                }
                for curve in curves {
                    validate_curve_values(curve, matches!(stage, Stage::InvCurves(_)))?;
                }
            }
            Stage::Matrix { rows, cols, m, offset } => {
                if *cols != channels
                    || !(1..=15).contains(rows)
                    || m.len() != rows.checked_mul(*cols).ok_or_else(invalid)?
                    || offset.len() != *rows
                    || m.iter().chain(offset).any(|v| !v.is_finite())
                {
                    return Err(invalid());
                }
                channels = *rows;
            }
            Stage::Clut(table) => {
                if table.inputs != channels || !(1..=15).contains(&table.outputs) || table.data.iter().any(|v| !v.is_finite()) {
                    return Err(invalid());
                }
                let mut out = [0.0_f64; 15];
                let input = [0.5_f64; 15];
                table.try_eval_f64(input.get(..channels).ok_or_else(invalid)?, out.get_mut(..table.outputs).ok_or_else(invalid)?)?;
                channels = table.outputs;
            }
            Stage::Clamp01 => (),
            _ => {
                if channels != 3 {
                    return Err(invalid());
                }
            }
        }
    }
    if channels != outputs {
        return Err(invalid());
    }
    Ok(())
}

fn validate_curve(curve: &Curve) -> Result<(), CmsError> {
    validate_curve_values(curve, true)
}

fn validate_curve_values(curve: &Curve, monotonic: bool) -> Result<(), CmsError> {
    let valid = match curve {
        Curve::Identity => true,
        Curve::Gamma(g) => g.is_finite() && *g > 0.0,
        Curve::Table(t) => (2..=1 << 20).contains(&t.len()) && t.iter().all(|v| v.is_finite()) && (!monotonic || t.windows(2).all(|w| w.first() <= w.get(1))),
        Curve::Table64(t) => (2..=1 << 20).contains(&t.len()) && t.iter().all(|v| v.is_finite()) && (!monotonic || t.windows(2).all(|w| w.first() <= w.get(1))),
        Curve::Parametric { kind, p } => {
            *kind <= 4 && p.iter().all(|v| v.is_finite()) && p[0] > 0.0 && (*kind == 0 || p[1] > 0.0) && (*kind < 3 || p[3] >= 0.0)
        }
    };
    if !valid {
        return Err(CmsError::Invalid("invalid XCF tone curve".into()));
    }
    // Allow the fixed-point rounding at parametric branch joins; do not impose equal RGB TRCs.
    let mut previous = curve.eval64(0.0);
    if !previous.is_finite() {
        return Err(CmsError::Invalid("nonfinite XCF tone-curve endpoint".into()));
    }
    for i in 1..=256 {
        let value = curve.eval64(f64::from(i) / 256.0);
        if !value.is_finite() || (monotonic && value + 0.0001 < previous) {
            return Err(CmsError::Invalid("nonmonotonic XCF tone curve".into()));
        }
        previous = value;
    }
    Ok(())
}

/// A bounded matrix/shaper class for XCF Normal unit-sample qualification.
/// This is deliberately narrower than valid ICC: sampled/LUT curves, non-D50
/// media, unstable primaries and flat/large-mismatch analytic branches remain outside
/// this class. It preserves the parsed definition and original profile bytes.
/// ICC.1 parametricCurveType supplies the equations; bounds below are explicit
/// admission limits, not replacements for the stored parameters or Curve math.
pub fn qualify_analytic_profile(profile: &Profile) -> Result<(), CmsError> {
    use crate::profile::{Pcs, ProfileClass};
    let unsupported = || CmsError::Unsupported("XCF profile is outside the bounded D50 analytic matrix/shaper class".into());
    if let Some(bytes) = &profile.bytes {
        let declared: [u8; 4] = bytes.get(..4).ok_or_else(unsupported)?.try_into().map_err(|_| unsupported())?;
        if !(132..=65536).contains(&bytes.len())
            || usize::try_from(u32::from_be_bytes(declared)).map_err(|_| unsupported())? != bytes.len()
            || bytes.get(20..24) != Some(b"XYZ ".as_slice())
        {
            return Err(unsupported());
        }
    }
    if !matches!(profile.version.0, 2 | 4)
        || profile.class != ProfileClass::Display
        || profile.pcs != Pcs::Xyz
        || profile.a2b.iter().chain(&profile.b2a).any(Option::is_some)
        || profile.white_point.iter().zip(crate::math::D50).any(|(value, white)| !value.is_finite() || (value - white).abs() > 4.0 / 65536.0)
        || profile.black_point.is_some_and(|point| point.iter().any(|value| !value.is_finite() || value.abs() > 1.0 / 65536.0))
        || profile.chad.as_ref().is_some_and(|matrix| matrix.iter().flatten().any(|value| !value.is_finite()) || crate::math::invert(matrix).is_none())
    {
        return Err(unsupported());
    }
    match profile.color_space {
        ColorSpace::Rgb => {
            let matrix = profile.matrix.as_ref().ok_or_else(unsupported)?;
            let inverse = crate::math::invert(matrix).ok_or_else(unsupported)?;
            if matrix.iter().flatten().any(|value| !(0.0..=1.25).contains(value))
                || matrix.iter().zip(crate::math::D50).any(|(row, white)| (row.iter().sum::<f64>() - white).abs() > 4.0 / 65536.0)
                || crate::math::det(matrix) < 0.01
                || inverse.iter().any(|row| row.iter().map(|value| value.abs()).sum::<f64>() > 16.0)
            {
                return Err(unsupported());
            }
            for curve in profile.trc.as_ref().ok_or_else(unsupported)? {
                qualify_analytic_curve(curve)?;
            }
        }
        ColorSpace::Gray => qualify_analytic_curve(profile.gray_trc.as_ref().ok_or_else(unsupported)?)?,
        _ => return Err(unsupported()),
    }
    validate_profile(profile)
}

/// Qualify an active current ICC definition for the stored sample interpretation.
/// No immutable source profile is consulted or substituted. Linear samples require
/// operative identity curves; Perceptual samples require the canonical sRGB
/// parametric transfer, either analytical or exactly rounded to ICC s15Fixed16.
/// Profile-defined nonlinear samples retain the existing analytic class limits.
pub fn qualify_effective_analytic_profile(profile: &Profile, encoding: Encoding) -> Result<(), CmsError> {
    qualify_analytic_profile(profile)?;
    if encoding == Encoding::NonlinearProfile {
        return Ok(());
    }
    let unsupported = || CmsError::Unsupported("analytic XCF current ICC transfer does not match the stored sample encoding".into());
    let Curve::Parametric { kind: expected_kind, p: expected } = crate::curve::srgb_trc() else {
        return Err(unsupported());
    };
    let rounded = expected.map(|value| (value * 65536.0).round() / 65536.0);
    let matches = |curve: &Curve| match encoding {
        Encoding::Linear => match curve {
            Curve::Identity => true,
            Curve::Gamma(gamma) => *gamma == 1.0,
            Curve::Parametric { kind: 0, p } => p.first() == Some(&1.0),
            _ => false,
        },
        Encoding::Perceptual => matches!(curve, Curve::Parametric { kind, p } if *kind == expected_kind && (*p == expected || *p == rounded)),
        Encoding::NonlinearProfile => true,
    };
    let valid = match profile.color_space {
        ColorSpace::Rgb => profile.trc.as_ref().is_some_and(|curves| curves.iter().all(matches)),
        ColorSpace::Gray => profile.gray_trc.as_ref().is_some_and(matches),
        _ => false,
    };
    if valid { Ok(()) } else { Err(unsupported()) }
}

fn qualify_analytic_curve(curve: &Curve) -> Result<(), CmsError> {
    let unsupported = || CmsError::Unsupported("XCF tone curve is outside the bounded normalized analytic class".into());
    let gamma = match curve {
        Curve::Identity => return Ok(()),
        Curve::Gamma(gamma) => *gamma,
        Curve::Parametric { kind: 0, p } if p.iter().all(|value| value.is_finite()) => p[0],
        Curve::Parametric { kind, p } if matches!(kind, 3 | 4) && p.iter().all(|value| value.is_finite()) => {
            let [gamma, a, b, c, d, e, f] = *p;
            let (e, f) = if *kind == 3 { (0.0, 0.0) } else { (e, f) };
            if !(0.25..=2.0).contains(&a)
                || !(-0.25..=0.25).contains(&b)
                || !(0.0..=0.25).contains(&d)
                || !(0.0..=4.0).contains(&c)
                || (d > 0.0 && c < 1.0 / 65536.0)
                || !(-0.25..=0.25).contains(&e)
                || !(-0.25..=0.25).contains(&f)
                || a * d + b < 0.0
            {
                return Err(unsupported());
            }
            // Check the actual join analytically, including branches narrower
            // than a sampling grid. ICC fixed-point rounding gets four LSBs;
            // larger branch mismatches and plateaus are not admitted. The
            // stored fixed-point join is evaluated unchanged in either direction.
            if d > 0.0 && ((a * d + b).powf(gamma) + e - (c * d + f)).abs() > 4.0 / 65536.0 {
                return Err(unsupported());
            }
            gamma
        }
        _ => return Err(unsupported()),
    };
    if !gamma.is_finite() || !(0.75..=3.5).contains(&gamma) || curve.eval64(0.0).abs() > 2.0 / 65536.0 || (curve.eval64(1.0) - 1.0).abs() > 4.0 / 65536.0 {
        return Err(unsupported());
    }
    Ok(())
}

// ICC curves are selected by the declared device model, even when unrelated tags exist.
fn model_curve(profile: &Profile, channel: usize) -> Option<&Curve> {
    match profile.color_space {
        ColorSpace::Rgb => profile.trc.as_ref().and_then(|curves| curves.get(channel)),
        ColorSpace::Gray => profile.gray_trc.as_ref(),
        _ => None,
    }
}

/// Transfer-only conversion in the source primaries; no alpha, narrowing or default curve.
pub fn convert_encoding_f64(p: &Profile, from: Encoding, to: Encoding, samples: &[f64], output: &mut [f64]) -> Result<(), CmsError> {
    validate_profile(p)?;
    convert_encoding_f64_validated(p, from, to, samples, output)
}

// The caller must hold an immutable profile whose full validation succeeded.
// Only this module and its sealed prepared implicit transfer child use this.
fn convert_encoding_f64_validated(p: &Profile, from: Encoding, to: Encoding, samples: &[f64], output: &mut [f64]) -> Result<(), CmsError> {
    if samples.len() != p.channels() || output.len() != p.channels() || samples.iter().any(|v| !v.is_finite()) {
        return Err(CmsError::Invalid("XCF transfer sample count or nonfinite input".into()));
    }
    if from == to {
        output.copy_from_slice(samples);
        return Ok(());
    }
    if p.a2b.iter().any(Option::is_some) || p.b2a.iter().any(Option::is_some) {
        return Err(CmsError::Unsupported("XCF ICC LUT transfer override has no separable tone curves; retain profile-defined nonlinear encoding".into()));
    }
    let mut values = [0.0_f64; 3];
    for (i, &value) in samples.iter().enumerate() {
        let profile_curve = model_curve(p, i).ok_or_else(|| CmsError::Invalid("missing XCF transfer curve".into()))?;
        let perceptual = crate::curve::srgb_trc();
        let curve = |encoding| match encoding {
            Encoding::Linear => None,
            Encoding::Perceptual => Some(&perceptual),
            Encoding::NonlinearProfile => Some(profile_curve),
        };
        let linear = curve(from).map_or(value, |c| c.eval64(value));
        let converted = if from == to { value } else { curve(to).map_or(linear, |c| c.eval_inverse64(linear)) };
        if !converted.is_finite() {
            return Err(CmsError::Invalid("XCF transfer produced nonfinite output".into()));
        }
        *values.get_mut(i).ok_or_else(|| CmsError::Invalid("XCF transfer channel count".into()))? = converted;
    }
    output.copy_from_slice(values.get(..samples.len()).ok_or_else(|| CmsError::Invalid("XCF transfer channel count".into()))?);
    Ok(())
}

/// Recover the analytical implicit GIMP working profile for the current sample
/// encoding, only when its serialized ICC is exactly bound to the generated one.
/// The caller must separately validate active interpretation identity; original
/// source provenance is never evidence that a current profile remains implicit.
pub fn effective_implicit_profile(bound_icc: &[u8], space: ColorSpace, encoding: Encoding) -> Result<Profile, CmsError> {
    if bound_icc.len() > 16 << 20 {
        return Err(CmsError::Invalid("XCF implicit ICC exceeds 16 MiB".into()));
    }
    let effective = effective_profile(None, space, encoding)?;
    if bound_icc != effective.to_bytes().as_slice() {
        return Err(CmsError::Invalid("active implicit profile ICC does not match its analytical interpretation".into()));
    }
    Ok(effective)
}

/// Analytical nonlinear source basis of a bound active implicit RGB/Gray profile.
/// Current sample encoding stays a separate runtime value used by the consumer.
pub fn implicit_source_profile(bound_icc: &[u8], space: ColorSpace, encoding: Encoding) -> Result<Profile, CmsError> {
    effective_implicit_profile(bound_icc, space, encoding)?;
    match space {
        ColorSpace::Rgb => Ok(Builtin::Srgb.profile().clone()),
        ColorSpace::Gray => Ok(Builtin::SGray.profile().clone()),
        _ => Err(CmsError::Unsupported("XCF implicit color model".into())),
    }
}

/// Effective profile interpretation. Original source ICC must be retained separately.
/// LUT profiles retain their pipeline under NonlinearProfile and reject transfer overrides.
pub fn effective_profile(original: Option<&[u8]>, space: ColorSpace, encoding: Encoding) -> Result<Profile, CmsError> {
    if !matches!(space, ColorSpace::Rgb | ColorSpace::Gray) {
        return Err(CmsError::Unsupported("XCF color model".into()));
    }
    let p = match original {
        Some(bytes) => {
            if bytes.len() > 16 << 20 {
                return Err(CmsError::Invalid("XCF ICC exceeds 16 MiB".into()));
            }
            parse_bounded_profile(bytes, space)?
        }
        None => match space {
            ColorSpace::Gray => Builtin::SGray.profile().clone(),
            _ => Builtin::Srgb.profile().clone(),
        },
    };
    if p.color_space != space {
        return Err(CmsError::Invalid("XCF profile color model mismatch".into()));
    }
    effective_profile_from_definition(&p, encoding)
}

/// Apply sample interpretation to a validated profile definition without
/// serializing/reparsing it. Analytical implicit transfer/matrix constants
/// remain exact; parsed assigned profiles retain their byte-defined constants.
pub fn effective_profile_from_definition(profile: &Profile, encoding: Encoding) -> Result<Profile, CmsError> {
    if !matches!(profile.color_space, ColorSpace::Rgb | ColorSpace::Gray) {
        return Err(CmsError::Unsupported("XCF color model".into()));
    }
    validate_profile(profile)?;
    let mut p = profile.clone();
    let space = p.color_space;
    if (p.a2b.iter().any(Option::is_some) || p.b2a.iter().any(Option::is_some)) && encoding != Encoding::NonlinearProfile {
        return Err(CmsError::Unsupported("XCF ICC LUT transfer override has no separable tone curves; retain profile-defined nonlinear encoding".into()));
    }
    let curve = match encoding {
        Encoding::Linear => Some(Curve::Identity),
        Encoding::Perceptual => Some(crate::curve::srgb_trc()),
        Encoding::NonlinearProfile => None,
    };
    if let Some(c) = curve {
        if space == ColorSpace::Rgb {
            p.trc = Some([c.clone(), c.clone(), c]);
        } else {
            p.gray_trc = Some(c);
        }
        p.description = format!("{} (XCF {encoding:?} samples)", p.description);
        p = p.with_encoded_bytes();
    }
    Ok(p)
}

/// Checked f32 transfer conversion for derived display/legacy working samples.
/// No fallback curve is supplied for profiles without separable transfer semantics.
pub fn try_convert_encoding_f32(p: &Profile, from: Encoding, to: Encoding, samples: &[f32], output: &mut [f32]) -> Result<(), CmsError> {
    if samples.len() > 3 || output.len() != samples.len() {
        return Err(CmsError::Invalid("XCF transfer sample count".into()));
    }
    let mut input = [0.0_f64; 3];
    for (target, &sample) in input.iter_mut().zip(samples) {
        *target = f64::from(sample);
    }
    let mut wide = [0.0_f64; 3];
    convert_encoding_f64(p, from, to, input.get(..samples.len()).ok_or(CmsError::Truncated)?, wide.get_mut(..samples.len()).ok_or(CmsError::Truncated)?)?;
    if wide.iter().any(|value| !(*value as f32).is_finite()) {
        return Err(CmsError::Invalid("XCF derived transfer output exceeds f32 range".into()));
    }
    for (target, &value) in output.iter_mut().zip(&wide) {
        *target = value as f32;
    }
    Ok(())
}

/// Legacy separable-curve conversion; callers must establish that the profile has transfer
/// curves before using it. New/imported profiles use checked conversion instead.
/// Same primaries, different working transfer. Alpha is deliberately absent.
pub fn convert_encoding(p: &Profile, from: Encoding, to: Encoding, rgb: [f32; 3]) -> [f32; 3] {
    if from == to {
        return rgb;
    }
    std::array::from_fn(|i| {
        let profile_curve = model_curve(p, i);
        let curve = |encoding| match encoding {
            Encoding::Linear => Curve::Identity,
            Encoding::Perceptual => crate::curve::srgb_trc(),
            Encoding::NonlinearProfile => profile_curve.cloned().unwrap_or_else(crate::curve::srgb_trc),
        };
        curve(to).eval_inverse(curve(from).eval(rgb[i]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_implicit_profile_binding_preserves_analytical_rgb_gray_transfer() {
        for space in [ColorSpace::Rgb, ColorSpace::Gray] {
            for encoding in [Encoding::Linear, Encoding::NonlinearProfile, Encoding::Perceptual] {
                let effective = effective_profile(None, space, encoding).unwrap();
                let bytes = effective.to_bytes();
                let source = implicit_source_profile(&bytes, space, encoding).unwrap();
                let actual = effective_implicit_profile(&bytes, space, encoding).unwrap();
                assert_eq!(actual, effective);
                assert_eq!(bytes, actual.to_bytes());
                let source_curve = source.trc.as_ref().map(|curves| &curves[0]).or(source.gray_trc.as_ref()).unwrap();
                let exact = crate::curve::srgb_trc();
                for value in [-1.25, 0.02, 0.19, 0.5, 3.25] {
                    assert_eq!(source_curve.eval64(value).to_bits(), exact.eval64(value).to_bits());
                }
                if encoding == Encoding::NonlinearProfile {
                    let parsed = Profile::parse(&bytes).unwrap();
                    let parsed_curve = parsed.trc.as_ref().map(|curves| &curves[0]).or(parsed.gray_trc.as_ref()).unwrap();
                    assert_ne!(source_curve.eval64(0.02).to_bits(), parsed_curve.eval64(0.02).to_bits());
                }
                let mut stale = bytes.as_ref().clone();
                stale[0] ^= 1;
                assert!(effective_implicit_profile(&stale, space, encoding).is_err());
                assert!(implicit_source_profile(&stale, space, encoding).is_err());
                let other = if space == ColorSpace::Rgb { ColorSpace::Gray } else { ColorSpace::Rgb };
                assert!(effective_implicit_profile(&bytes, other, encoding).is_err());
            }
        }
        assert!(implicit_source_profile(&Builtin::DisplayP3.profile().to_bytes(), ColorSpace::Rgb, Encoding::NonlinearProfile).is_err());
        assert!(effective_implicit_profile(&[], ColorSpace::Cmyk, Encoding::Linear).is_err());
    }

    #[test]
    fn definition_interpretation_preserves_assigned_tables_and_analytical_bits() {
        let implicit = Builtin::Srgb.profile();
        let exact = effective_profile_from_definition(implicit, Encoding::NonlinearProfile).unwrap();
        assert_eq!(&exact, implicit);
        let reparsed = effective_profile(Some(&implicit.to_bytes()), ColorSpace::Rgb, Encoding::NonlinearProfile).unwrap();
        assert_ne!(exact.trc.as_ref().unwrap()[0].eval64(0.02).to_bits(), reparsed.trc.as_ref().unwrap()[0].eval64(0.02).to_bits());
        let mut assigned = Builtin::Srgb.profile().clone();
        assigned.trc = Some([Curve::Table64(vec![0.0, 0.02, 0.2, 1.0]), Curve::Gamma(2.3), Curve::Table64(vec![0.0, 0.4, 0.9, 1.0])]);
        assigned = assigned.with_encoded_bytes();
        let parsed = Profile::parse(&assigned.to_bytes()).unwrap();
        let interpreted = effective_profile_from_definition(&parsed, Encoding::NonlinearProfile).unwrap();
        assert_eq!(interpreted, parsed);
        assert_eq!(interpreted.to_bytes(), parsed.to_bytes());
        assert_eq!(interpreted.matrix, parsed.matrix);
        assert_eq!(interpreted.trc, parsed.trc);
        assert!(effective_profile_from_definition(&Builtin::CoatedCmyk.profile().clone(), Encoding::NonlinearProfile).is_err());
        let mut invalid = implicit.clone();
        invalid.trc = Some([Curve::Gamma(f64::NAN), Curve::Identity, Curve::Identity]);
        assert!(effective_profile_from_definition(&invalid, Encoding::Linear).is_err());
        invalid.trc = Some([Curve::Table64(vec![0.0; (1 << 20) + 1]), Curve::Identity, Curve::Identity]);
        assert!(effective_profile_from_definition(&invalid, Encoding::NonlinearProfile).is_err());
    }

    #[test]
    fn transfer_uses_model_curve_despite_extraneous_other_model_tags() {
        let mut gray = Builtin::GrayGamma22.profile().clone();
        gray.gray_trc = Some(Curve::Gamma(2.2));
        gray.trc = Some([Curve::Gamma(1.8), Curve::Gamma(1.4), Curve::Gamma(3.0)]);
        let gray = Profile::parse(&gray.with_encoded_bytes().to_bytes()).unwrap();
        let before = gray.clone();
        let expected = gray.gray_trc.as_ref().unwrap().eval64(0.47);
        let mut output = [0.0];
        convert_encoding_f64(&gray, Encoding::NonlinearProfile, Encoding::Linear, &[0.47], &mut output).unwrap();
        assert_eq!(output[0].to_bits(), expected.to_bits());
        let mut back = [0.0];
        convert_encoding_f64(&gray, Encoding::Linear, Encoding::NonlinearProfile, &output, &mut back).unwrap();
        assert!((back[0] - 0.47).abs() < 1e-12);
        let mut narrow = [0.0_f32];
        try_convert_encoding_f32(&gray, Encoding::NonlinearProfile, Encoding::Linear, &[0.47], &mut narrow).unwrap();
        let expected_narrow = gray.gray_trc.as_ref().unwrap().eval64(f64::from(0.47_f32)) as f32;
        assert_eq!(narrow[0].to_bits(), expected_narrow.to_bits());
        let legacy = convert_encoding(&gray, Encoding::NonlinearProfile, Encoding::Linear, [0.47; 3]);
        assert_eq!(legacy.map(f32::to_bits), [expected_narrow.to_bits(); 3]);
        assert_eq!(gray, before);
        let mut rgb = Builtin::Srgb.profile().clone();
        rgb.trc = Some([Curve::Gamma(1.6), Curve::Gamma(2.3), Curve::Gamma(2.8)]);
        rgb.gray_trc = Some(Curve::Gamma(0.8));
        let before = rgb.clone();
        let mut output = [0.0; 3];
        convert_encoding_f64(&rgb, Encoding::NonlinearProfile, Encoding::Linear, &[0.47; 3], &mut output).unwrap();
        let expected = rgb.trc.as_ref().unwrap().each_ref().map(|curve| curve.eval64(0.47));
        assert_eq!(output.map(f64::to_bits), expected.map(f64::to_bits));
        assert_eq!(rgb, before);
    }

    #[test]
    fn encoding_changes_interpretation_not_source_bytes_or_primaries() {
        let original = Builtin::DisplayP3.profile().to_bytes();
        let linear = effective_profile(Some(&original), ColorSpace::Rgb, Encoding::Linear).unwrap();
        assert_eq!(linear.matrix, Builtin::DisplayP3.profile().matrix);
        assert!(linear.trc.as_ref().unwrap().iter().all(Curve::is_identity));
        assert_eq!(Profile::parse(&original).unwrap(), Profile::parse(&original).unwrap());
        let p = Builtin::Srgb.profile();
        let v = convert_encoding(p, Encoding::NonlinearProfile, Encoding::Linear, [0.5; 3]);
        assert!((v[0] - 0.214041).abs() < 0.00001);
        let back = convert_encoding(p, Encoding::Linear, Encoding::NonlinearProfile, v);
        assert!((back[0] - 0.5).abs() < 0.00001);
        assert!(effective_profile(Some(&original), ColorSpace::Gray, Encoding::Linear).is_err());
        assert!(effective_profile(Some(b"bad"), ColorSpace::Rgb, Encoding::Linear).is_err());
    }
}

/// Requirements for validating and reconstructing a current interpretation.
/// Encoded source bytes remain source-owned; this is not rendering qualification.
#[derive(Clone, Copy, Debug)]
pub struct EffectiveProfileRequirements {
    pub retained_bytes: usize,
    pub peak_resident_bytes: usize,
    pub work_units: u64,
}
fn controlled_curve(curve: &Curve, monotonic: bool, b: &mut crate::control::Budget<'_>) -> crate::control::Result<()> {
    use crate::control::invalid;
    b.poll()?;
    let valid = match curve {
        Curve::Identity => true,
        Curve::Gamma(g) => g.is_finite() && *g > 0.,
        Curve::Table(t) => {
            if !(2..=1 << 20).contains(&t.len()) {
                false
            } else {
                let mut prior = None;
                for &v in t {
                    b.work(1)?;
                    if !v.is_finite() || monotonic && prior.is_some_and(|p| p > v) {
                        return Err(invalid("invalid XCF tone curve"));
                    }
                    prior = Some(v);
                }
                true
            }
        }
        Curve::Table64(t) => {
            if !(2..=1 << 20).contains(&t.len()) {
                false
            } else {
                let mut prior = None;
                for &v in t {
                    b.work(1)?;
                    if !v.is_finite() || monotonic && prior.is_some_and(|p| p > v) {
                        return Err(invalid("invalid XCF tone curve"));
                    }
                    prior = Some(v);
                }
                true
            }
        }
        Curve::Parametric { kind, p } => *kind <= 4 && p.iter().all(|v| v.is_finite()) && p[0] > 0. && (*kind == 0 || p[1] > 0.) && (*kind < 3 || p[3] >= 0.),
    };
    if !valid {
        return Err(invalid("invalid XCF tone curve"));
    }
    b.work(128)?;
    let mut previous = curve.eval64(0.);
    if !previous.is_finite() {
        return Err(invalid("nonfinite XCF tone-curve endpoint"));
    }
    for i in 1..=256 {
        b.work(128)?;
        let value = curve.eval64(f64::from(i) / 256.);
        if !value.is_finite() || (monotonic && value + 0.0001 < previous) {
            return Err(invalid("nonmonotonic XCF tone curve"));
        }
        previous = value;
    }
    Ok(())
}
fn controlled_lut(lut: &crate::profile::Lut, inputs: usize, outputs: usize, b: &mut crate::control::Budget<'_>) -> crate::control::Result<()> {
    use crate::{
        control::{Error, invalid},
        pipeline::Stage,
    };
    let bad = || invalid("invalid XCF ICC LUT dimensions, curves or nodes");
    if lut.inputs != inputs || lut.outputs != outputs {
        return Err(bad());
    }
    if lut.stages.len() > 4096 {
        return Err(Error::Limit("XCF ICC LUT stages"));
    }
    let mut channels = inputs;
    for stage in &lut.stages {
        b.work(1)?;
        match stage {
            Stage::Curves(cs) | Stage::InvCurves(cs) => {
                if cs.len() != channels {
                    return Err(bad());
                }
                for c in cs {
                    controlled_curve(c, matches!(stage, Stage::InvCurves(_)), b)?;
                }
            }
            Stage::Matrix { rows, cols, m, offset } => {
                if *cols != channels || !(1..=15).contains(rows) || m.len() != rows.checked_mul(*cols).ok_or_else(bad)? || offset.len() != *rows {
                    return Err(bad());
                }
                for v in m.iter().chain(offset) {
                    b.work(1)?;
                    if !v.is_finite() {
                        return Err(bad());
                    }
                }
                channels = *rows;
            }
            Stage::Clut(c) => {
                if c.inputs != channels || !(1..=15).contains(&c.outputs) {
                    return Err(bad());
                }
                for v in &c.data {
                    b.work(1)?;
                    if !v.is_finite() {
                        return Err(bad());
                    }
                }
                let mut out = [0.; 15];
                let input = [0.5; 15];
                c.eval_f64_controlled(input.get(..channels).ok_or_else(bad)?, out.get_mut(..c.outputs).ok_or_else(bad)?, b)?;
                channels = c.outputs;
            }
            Stage::Clamp01 => (),
            _ => {
                if channels != 3 {
                    return Err(bad());
                }
            }
        }
    }
    if channels != outputs {
        return Err(bad());
    }
    Ok(())
}
fn controlled_validate(p: &Profile, b: &mut crate::control::Budget<'_>) -> crate::control::Result<()> {
    use crate::control::invalid;
    b.poll()?;
    if !matches!(p.color_space, ColorSpace::Rgb | ColorSpace::Gray) {
        return Err(CmsError::Unsupported("XCF color model".into()).into());
    }
    if p.white_point.iter().any(|v| !v.is_finite())
        || p.black_point.is_some_and(|v| v.iter().any(|v| !v.is_finite()))
        || p.chad.as_ref().is_some_and(|m| m.iter().flatten().any(|v| !v.is_finite()) || crate::math::invert(m).is_none())
    {
        return Err(invalid("nonfinite XCF profile point or noninvertible chromatic adaptation"));
    }
    if p.a2b.iter().any(Option::is_some) || p.b2a.iter().any(Option::is_some) {
        for lut in p.a2b.iter().flatten() {
            controlled_lut(lut, p.channels(), 3, b)?;
        }
        for lut in p.b2a.iter().flatten() {
            controlled_lut(lut, 3, p.channels(), b)?;
        }
        return Ok(());
    }
    match p.color_space {
        ColorSpace::Rgb => {
            let m = p.matrix.as_ref().ok_or_else(|| invalid("missing XCF RGB colorants"))?;
            if m.iter().flatten().any(|v| !v.is_finite()) || crate::math::invert(m).is_none() {
                return Err(invalid("nonfinite or singular XCF colorant matrix"));
            }
            let cs = p.trc.as_ref().ok_or_else(|| invalid("missing XCF RGB curves"))?;
            for c in cs {
                controlled_curve(c, true, b)?;
            }
        }
        ColorSpace::Gray => controlled_curve(p.gray_trc.as_ref().ok_or_else(|| invalid("missing XCF gray curve"))?, true, b)?,
        _ => return Err(CmsError::Unsupported("XCF color model".into()).into()),
    }
    b.poll()?;
    Ok(())
}
/// Cooperative equivalent of the existing validation policy, not a new profile class.
pub fn validate_profile_with_control(profile: &Profile, limits: crate::control::Limits, cancelled: &dyn Fn() -> bool) -> crate::control::Result<()> {
    let mut b = crate::control::Budget::new(limits, cancelled)?;
    b.retained(profile.controlled_retained_bytes(&b)?)?;
    controlled_validate(profile, &mut b)
}
pub(crate) fn transfer_recipe(encoding: Encoding) -> Option<(&'static str, Curve)> {
    match encoding {
        Encoding::NonlinearProfile => None,
        Encoding::Linear => Some((" (XCF Linear samples)", Curve::Identity)),
        Encoding::Perceptual => Some((" (XCF Perceptual samples)", crate::curve::srgb_trc())),
    }
}
/// Borrowed admission before cloning, description append or ICC output allocation.
pub fn effective_profile_requirements_with_control(
    profile: &Profile,
    encoding: Encoding,
    limits: crate::control::Limits,
    cancelled: &dyn Fn() -> bool,
) -> crate::control::Result<EffectiveProfileRequirements> {
    use crate::control::{Budget, Error};
    let mut b = Budget::new(limits, cancelled)?;
    let retained = profile.controlled_retained_bytes(&b)?;
    b.retained(retained)?;
    controlled_validate(profile, &mut b)?;
    if (profile.a2b.iter().any(Option::is_some) || profile.b2a.iter().any(Option::is_some)) && encoding != Encoding::NonlinearProfile {
        return Err(
            CmsError::Unsupported("XCF ICC LUT transfer override has no separable tone curves; retain profile-defined nonlinear encoding".into()).into()
        );
    }
    let clone = profile.clone_requirements_with_control(limits, cancelled)?;
    let mut peak = retained.checked_add(clone.cloned_model_bytes).ok_or(Error::Limit("effective profile model"))?;
    // Validation is repeated by callers; charge a conservative per retained byte
    // plus fixed evaluation schedule for every bounded curve/stage in the model.
    let mut work = implicit_planner::base_effective_work(retained)?;
    for l in profile.a2b.iter().chain(&profile.b2a).flatten() {
        for stage in &l.stages {
            b.poll()?;
            let extra = match stage {
                crate::pipeline::Stage::Curves(c) | crate::pipeline::Stage::InvCurves(c) => (c.len() as u64) * 32_896,
                crate::pipeline::Stage::Clut(c) => {
                    c.interpolation_taps()?.checked_mul((c.inputs + c.outputs + 1) as u64).ok_or(Error::Limit("ICC eval work"))?
                }
                _ => 128,
            };
            work = work.checked_add(extra).ok_or(Error::Limit("ICC validation work"))?;
        }
    }
    if let Some((suffix, curve)) = transfer_recipe(encoding) {
        let text = profile.description.len().checked_add(suffix.len()).ok_or(Error::Limit("ICC description"))?;
        let writer = crate::write::controlled::transfer_requirements(profile, suffix, &curve, limits, cancelled)?;
        peak = peak
            .checked_add(text)
            .and_then(|v| v.checked_add(256))
            .and_then(|v| v.checked_add(writer.peak_resident_bytes.checked_sub(writer.retained_bytes)?))
            .ok_or(Error::Limit("effective ICC peak"))?;
        work = work.checked_add(writer.work_units).and_then(|v| v.checked_add(text as u64)).ok_or(Error::Limit("effective ICC work"))?;
    }
    let check = Budget::new(limits, cancelled)?;
    check.admit(peak, work)?;
    Ok(EffectiveProfileRequirements { retained_bytes: retained, peak_resident_bytes: peak, work_units: work })
}
/// Exact current interpretation reconstruction. Nonlinear keeps cached original
/// ICC/hash; transfer overrides reproduce legacy encoded bytes and reset hash.
pub fn effective_profile_from_definition_with_control(
    profile: &Profile,
    encoding: Encoding,
    limits: crate::control::Limits,
    cancelled: &dyn Fn() -> bool,
) -> crate::control::Result<Profile> {
    use crate::control::{Budget, Error};
    let _ = effective_profile_requirements_with_control(profile, encoding, limits, cancelled)?;
    let mut p = profile.try_clone_with_control(limits, cancelled)?;
    if let Some((suffix, c)) = transfer_recipe(encoding) {
        let mut b = Budget::new(limits, cancelled)?;
        let n = p.description.len().checked_add(suffix.len()).ok_or(Error::Limit("ICC description"))?;
        let mut text = b.reserve(n)?;
        for chunk in p.description.as_bytes().chunks(1024) {
            b.work(chunk.len() as u64)?;
            text.extend_from_slice(chunk);
        }
        text.extend_from_slice(suffix.as_bytes());
        p.description = String::from_utf8(text).map_err(|_| crate::control::invalid("ICC description UTF-8"))?;
        if p.color_space == ColorSpace::Rgb {
            p.trc = Some([c.clone(), c.clone(), c]);
        } else {
            p.gray_trc = Some(c);
        }
        p = p.with_encoded_bytes_with_control(limits, cancelled)?;
    }
    if cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(p)
}
/// Borrowed prospective admission for the exact current implicit RGB/Gray binding.
/// Does not verify equality until construction generates the analytical ICC.
/// Builtin OnceLock initialization remains a provider boundary with outer polls.
pub fn effective_implicit_profile_requirements_with_control(
    bound: &[u8],
    space: ColorSpace,
    encoding: Encoding,
    limits: crate::control::Limits,
    cancelled: &dyn Fn() -> bool,
) -> crate::control::Result<EffectiveProfileRequirements> {
    use crate::control::{Budget, Error, invalid};
    let b = Budget::new(limits, cancelled)?;
    if bound.len() > 16 << 20 {
        return Err(invalid("XCF implicit ICC exceeds 16 MiB"));
    }
    b.admit(bound.len(), bound.len() as u64)?;
    let original = match space {
        ColorSpace::Rgb => Builtin::Srgb.profile(),
        ColorSpace::Gray => Builtin::SGray.profile(),
        _ => return Err(CmsError::Unsupported("XCF implicit color model".into()).into()),
    };
    b.poll()?;
    let r = effective_profile_requirements_with_control(original, encoding, limits, cancelled)?;
    let required = EffectiveProfileRequirements {
        retained_bytes: r.retained_bytes.checked_add(bound.len()).ok_or(Error::Limit("implicit ICC retained"))?,
        peak_resident_bytes: r.peak_resident_bytes.checked_add(bound.len()).ok_or(Error::Limit("implicit ICC peak"))?,
        work_units: r.work_units.checked_add(bound.len() as u64).ok_or(Error::Limit("implicit ICC work"))?,
    };
    b.admit(required.peak_resident_bytes, required.work_units)?;
    b.poll()?;
    Ok(required)
}
/// Verify the actual generated implicit ICC against the caller's binding.
/// Builtin OnceLock initialization remains a provider boundary with outer polls.
pub fn effective_implicit_profile_with_control(
    bound: &[u8],
    space: ColorSpace,
    encoding: Encoding,
    limits: crate::control::Limits,
    cancelled: &dyn Fn() -> bool,
) -> crate::control::Result<Profile> {
    use crate::control::{Budget, invalid};
    let required = effective_implicit_profile_requirements_with_control(bound, space, encoding, limits, cancelled)?;
    let original = match space {
        ColorSpace::Rgb => Builtin::Srgb.profile(),
        ColorSpace::Gray => Builtin::SGray.profile(),
        _ => return Err(CmsError::Unsupported("XCF implicit color model".into()).into()),
    };
    let mut check = Budget::new(limits, cancelled)?;
    check.admit(required.peak_resident_bytes, required.work_units)?;
    let effective = effective_profile_from_definition_with_control(original, encoding, limits, cancelled)?;
    let actual = effective.encoded_bytes().ok_or_else(|| invalid("implicit ICC encoding missing"))?;
    if actual.len() != bound.len() {
        return Err(invalid("active implicit profile ICC does not match its analytical interpretation"));
    }
    for (a, b) in actual.chunks(1024).zip(bound.chunks(1024)) {
        check.work(a.len() as u64)?;
        if a != b {
            return Err(invalid("active implicit profile ICC does not match its analytical interpretation"));
        }
    }
    check.poll()?;
    Ok(effective)
}

#[cfg(test)]
mod controlled_effective_tests {
    use super::*;
    use crate::control::{Error, Limits};
    use std::cell::Cell;
    #[test]
    fn exact_transfer_override_and_nonlinear_source_binding() {
        for b in [Builtin::Srgb, Builtin::SGray, Builtin::DisplayP3] {
            let source = b.profile();
            let bytes = source.to_bytes();
            let hash = source.content_hash();
            for encoding in [Encoding::Linear, Encoding::Perceptual, Encoding::NonlinearProfile] {
                let actual = effective_profile_from_definition_with_control(source, encoding, Limits::default(), &|| false).unwrap();
                let old = effective_profile_from_definition(source, encoding).unwrap();
                assert_eq!(actual.to_bytes(), old.to_bytes());
                assert_eq!(actual.description, old.description);
                if encoding == Encoding::NonlinearProfile {
                    assert!(Arc::ptr_eq(&actual.to_bytes(), &bytes));
                    assert_eq!(actual.content_hash(), hash);
                }
            }
            assert_eq!(source.to_bytes(), bytes);
            assert_eq!(source.content_hash(), hash);
        }
    }
    #[test]
    fn bound_implicit_bytes_checked_and_source_unmodified_on_cancel() {
        let expected = effective_profile(None, ColorSpace::Rgb, Encoding::Linear).unwrap().to_bytes();
        assert!(effective_implicit_profile_with_control(&expected, ColorSpace::Rgb, Encoding::Linear, Limits::default(), &|| false).is_ok());
        let mut wrong = expected.as_ref().clone();
        *wrong.last_mut().unwrap() ^= 1;
        assert!(effective_implicit_profile_with_control(&wrong, ColorSpace::Rgb, Encoding::Linear, Limits::default(), &|| false).is_err());
        let p = Builtin::Srgb.profile();
        let bytes = p.to_bytes();
        let calls = Cell::new(0);
        assert!(matches!(
            effective_profile_from_definition_with_control(p, Encoding::Perceptual, Limits::default(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 100
            }),
            Err(Error::Cancelled)
        ));
        assert!(Arc::ptr_eq(&bytes, &p.to_bytes()));
    }
    #[test]
    fn invalid_curve_and_tiny_budget_refuse_before_copy() {
        let mut p = Builtin::SGray.profile().clone();
        p.gray_trc = Some(Curve::Table64(vec![0., 1., 0.]));
        assert!(validate_profile_with_control(&p, Limits::default(), &|| false).is_err());
        assert!(matches!(
            effective_profile_from_definition_with_control(Builtin::Srgb.profile(), Encoding::Linear, Limits { resident_bytes: 1, work_units: 1 }, &|| false),
            Err(Error::Limit(_))
        ));
    }
}

/// Borrowed strict-header gate cost. This does not parse or qualify a profile.
#[derive(Clone, Copy, Debug)]
pub struct EncodedProfileGateRequirements {
    pub retained_bytes: usize,
    pub work_units: u64,
}
fn encoded_gate_header(bytes: &[u8]) -> crate::control::Result<(usize, usize, usize)> {
    use crate::control::invalid;
    if bytes.len() > 16 << 20 {
        return Err(invalid("XCF ICC exceeds 16 MiB"));
    }
    let declared = usize::try_from(encoded_gate_u32(bytes, 0)?).map_err(|_| CmsError::Truncated)?;
    if declared < 132 || declared > bytes.len() {
        return Err(invalid("invalid XCF ICC declared size"));
    }
    if !matches!(bytes.get(8), Some(2 | 4)) || !matches!(bytes.get(20..24), Some(b"XYZ " | b"Lab ")) {
        return Err(CmsError::Unsupported("XCF ICC version or PCS".into()).into());
    }
    if encoded_gate_u32(bytes, 64)? > 3 {
        return Err(invalid("invalid XCF ICC rendering intent"));
    }
    let count = usize::try_from(encoded_gate_u32(bytes, 128)?).map_err(|_| CmsError::Truncated)?;
    if count > 1024 {
        return Err(invalid("too many XCF ICC tags"));
    }
    let end = 132usize.checked_add(count.checked_mul(12).ok_or(CmsError::Truncated)?).ok_or(CmsError::Truncated)?;
    if end > declared {
        return Err(CmsError::Truncated.into());
    }
    Ok((declared, count, end))
}
fn encoded_gate_u32(bytes: &[u8], offset: usize) -> crate::control::Result<u32> {
    let end = offset.checked_add(4).ok_or(CmsError::Truncated)?;
    let word: [u8; 4] = bytes.get(offset..end).ok_or(CmsError::Truncated)?.try_into().map_err(|_| CmsError::Truncated)?;
    Ok(u32::from_be_bytes(word))
}
fn encoded_gate_record(bytes: &[u8], index: usize) -> crate::control::Result<(u32, usize, usize, usize)> {
    let offset = 132usize.checked_add(index.checked_mul(12).ok_or(CmsError::Truncated)?).ok_or(CmsError::Truncated)?;
    let signature = encoded_gate_u32(bytes, offset)?;
    let start = usize::try_from(encoded_gate_u32(bytes, offset.checked_add(4).ok_or(CmsError::Truncated)?)?).map_err(|_| CmsError::Truncated)?;
    let size = usize::try_from(encoded_gate_u32(bytes, offset.checked_add(8).ok_or(CmsError::Truncated)?)?).map_err(|_| CmsError::Truncated)?;
    let end = start.checked_add(size).ok_or(CmsError::Truncated)?;
    Ok((signature, start, size, end))
}
/// Checks the same bounded header as parse_bounded_profile and admits the
/// complete prospective duplicate/overlap work before visiting tag pairs.
pub fn encoded_profile_gate_requirements_with_control(
    bytes: &[u8],
    limits: crate::control::Limits,
    cancelled: &dyn Fn() -> bool,
) -> crate::control::Result<EncodedProfileGateRequirements> {
    use crate::control::{Budget, Error};
    let budget = Budget::new(limits, cancelled)?;
    let (_, count, _) = encoded_gate_header(bytes)?;
    let pairs = count.checked_mul(count.saturating_sub(1)).and_then(|v| v.checked_div(2)).ok_or(Error::Limit("ICC gate pair work"))?;
    let work = count.checked_add(pairs).and_then(|v| v.checked_mul(64)).and_then(|v| v.checked_add(128)).ok_or(Error::Limit("ICC gate work"))? as u64;
    budget.admit(bytes.len(), work)?;
    Ok(EncodedProfileGateRequirements { retained_bytes: bytes.len(), work_units: work })
}
/// Exact borrowed companion to the pre-parse gate in parse_bounded_profile.
/// Preserves declaration tolerance, tag order, shared complete payloads and
/// optional-tag rejection. General parsing and profile validation remain separate.
pub fn validate_encoded_profile_with_control(bytes: &[u8], limits: crate::control::Limits, cancelled: &dyn Fn() -> bool) -> crate::control::Result<()> {
    use crate::control::{Budget, invalid};
    let required = encoded_profile_gate_requirements_with_control(bytes, limits, cancelled)?;
    let mut budget = Budget::new(limits, cancelled)?;
    budget.retained(required.retained_bytes)?;
    budget.work(128)?;
    let (declared, count, table_end) = encoded_gate_header(bytes)?;
    for i in 0..count {
        budget.work(64)?;
        let (signature, start, size, end) = encoded_gate_record(bytes, i)?;
        if start < table_end || end > declared {
            return Err(invalid("XCF ICC tag lies outside profile data"));
        }
        for previous in 0..i {
            budget.work(64)?;
            let (ps, begin, length, finish) = encoded_gate_record(bytes, previous)?;
            if signature == ps {
                return Err(invalid("duplicate XCF ICC tag definition"));
            }
            if start < finish && begin < end && (start != begin || size != length) {
                return Err(invalid("partially overlapping XCF ICC tag data"));
            }
        }
        let data = bytes.get(start..end).ok_or(CmsError::Truncated)?;
        if signature == u32::from_be_bytes(*b"bkpt") && (size != 20 || data.get(..4) != Some(b"XYZ ".as_slice())) {
            return Err(invalid("invalid XCF ICC black-point tag"));
        }
        if signature == u32::from_be_bytes(*b"chad") && (size != 44 || data.get(..4) != Some(b"sf32".as_slice())) {
            return Err(invalid("invalid XCF ICC chromatic-adaptation tag"));
        }
    }
    budget.poll()?;
    Ok(())
}
#[cfg(test)]
mod controlled_encoded_gate_tests {
    use super::*;
    use crate::control::{Error, Limits};
    use std::cell::Cell;
    #[test]
    fn strict_intent_gate_preserves_compatibility_fallback() {
        let mut bytes = Builtin::Srgb.profile().to_bytes().as_ref().clone();
        bytes.get_mut(64..68).unwrap().copy_from_slice(&5u32.to_be_bytes());
        assert!(Profile::parse(&bytes).is_ok());
        assert!(parse_bounded_profile(&bytes, ColorSpace::Rgb).is_err());
        assert!(matches!(validate_encoded_profile_with_control(&bytes, Limits::default(), &|| false), Err(Error::Cms(_))));
    }
    #[test]
    fn exact_shared_payloads_duplicate_and_partial_overlap() {
        let bytes = Builtin::Srgb.profile().to_bytes();
        validate_encoded_profile_with_control(&bytes, Limits::default(), &|| false).unwrap();
        assert!(parse_bounded_profile(&bytes, ColorSpace::Rgb).is_ok());
        let mut duplicate = bytes.as_ref().clone();
        let sig: Vec<u8> = duplicate.get(132..136).unwrap().into();
        duplicate.get_mut(144..148).unwrap().copy_from_slice(&sig);
        assert!(parse_bounded_profile(&duplicate, ColorSpace::Rgb).is_err());
        assert!(validate_encoded_profile_with_control(&duplicate, Limits::default(), &|| false).is_err());
        let mut overlap = bytes.as_ref().clone();
        let first = encoded_gate_u32(&overlap, 136).unwrap();
        overlap.get_mut(148..152).unwrap().copy_from_slice(&(first + 1).to_be_bytes());
        assert!(parse_bounded_profile(&overlap, ColorSpace::Rgb).is_err());
        assert!(validate_encoded_profile_with_control(&overlap, Limits::default(), &|| false).is_err());
    }
    #[test]
    fn borrowed_gate_cancel_resource_and_raw_tail_unchanged() {
        let mut bytes = Builtin::DisplayP3.profile().to_bytes().as_ref().clone();
        bytes.extend_from_slice(b"retained undeclared tail");
        let before = bytes.clone();
        validate_encoded_profile_with_control(&bytes, Limits::default(), &|| false).unwrap();
        assert!(parse_bounded_profile(&bytes, ColorSpace::Rgb).is_ok());
        assert!(matches!(validate_encoded_profile_with_control(&bytes, Limits { resident_bytes: 1, work_units: 1 }, &|| false), Err(Error::Limit(_))));
        let visits = Cell::new(0);
        assert!(matches!(
            validate_encoded_profile_with_control(&bytes, Limits::default(), &|| {
                visits.set(visits.get() + 1);
                visits.get() > 12
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(bytes, before);
    }
}

#[cfg(test)]
mod implicit_requirements_tests {
    use super::*;
    use crate::control::{Error, Limits};
    #[test]
    fn implicit_requirements_match_existing_envelope_and_constructor() {
        for space in [ColorSpace::Rgb, ColorSpace::Gray] {
            let base = match space {
                ColorSpace::Rgb => Builtin::Srgb.profile(),
                _ => Builtin::SGray.profile(),
            };
            for encoding in [Encoding::NonlinearProfile, Encoding::Linear, Encoding::Perceptual] {
                let bound = effective_profile_from_definition(base, encoding).unwrap().to_bytes();
                let r = effective_profile_requirements_with_control(base, encoding, Limits::default(), &|| false).unwrap();
                let got = effective_implicit_profile_requirements_with_control(&bound, space, encoding, Limits::default(), &|| false).unwrap();
                assert_eq!(got.retained_bytes, r.retained_bytes + bound.len());
                assert_eq!(got.peak_resident_bytes, r.peak_resident_bytes + bound.len());
                assert_eq!(got.work_units, r.work_units + bound.len() as u64);
                let actual = effective_implicit_profile_with_control(&bound, space, encoding, Limits::default(), &|| false).unwrap();
                assert_eq!(actual.to_bytes(), bound);
                assert!(
                    effective_implicit_profile_requirements_with_control(
                        &bound,
                        space,
                        encoding,
                        Limits { resident_bytes: got.peak_resident_bytes - 1, work_units: got.work_units },
                        &|| false
                    )
                    .is_err()
                );
                assert!(
                    effective_implicit_profile_requirements_with_control(
                        &bound,
                        space,
                        encoding,
                        Limits { resident_bytes: got.peak_resident_bytes, work_units: got.work_units - 1 },
                        &|| false
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn implicit_requirements_do_not_claim_equality_or_admit_wrong_space_cancel() {
        let bound = effective_profile(None, ColorSpace::Rgb, Encoding::Linear).unwrap().to_bytes();
        assert!(matches!(
            effective_implicit_profile_requirements_with_control(&bound, ColorSpace::Rgb, Encoding::Linear, Limits::default(), &|| true),
            Err(Error::Cancelled)
        ));
        assert!(effective_implicit_profile_requirements_with_control(&bound, ColorSpace::Cmyk, Encoding::Linear, Limits::default(), &|| false).is_err());
        let mut wrong = bound.as_ref().clone();
        *wrong.last_mut().unwrap() ^= 1;
        assert!(effective_implicit_profile_requirements_with_control(&wrong, ColorSpace::Rgb, Encoding::Linear, Limits::default(), &|| false).is_ok());
        assert!(effective_implicit_profile_with_control(&wrong, ColorSpace::Rgb, Encoding::Linear, Limits::default(), &|| false).is_err());
        assert_eq!(bound, effective_profile(None, ColorSpace::Rgb, Encoding::Linear).unwrap().to_bytes());
    }
}
