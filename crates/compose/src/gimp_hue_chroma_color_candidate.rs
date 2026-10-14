//! Explicit caller-profile RGB Float candidate, not a source renderer.
//!
//! Candidate schedule: analytical ICC RGB -> XYZ in f64, physical CIE Lab using
//! the own CMS D50 constant, polar LCh in f64 rounded once to f32, the measured
//! Float LCh point stage, then f64 inverse polar/Lab/ICC and final RGB f32.
//! Strict preparation admits Identity/Gamma/parametric0 only. The explicit
//! DeclaredSrgbPair policy additionally admits the exact canonical IEC pair or
//! its exact ICC s15f16 representation, without claiming a gap-free inverse.
//! Only the Float LCh point stage is empirically established by retained captures.
//! RGB ingress/egress and this precision schedule have NOT been qualified against
//! GIMP. No absent-profile alias, outer composition, mask, intent/BPC default,
//! premultiplication, or serialized operation admission is supplied here.
use photocraft_algo::gimp::hue_chroma::{self, Parameters};
use photocraft_cms::{
    Intent, control,
    curve::Curve,
    math,
    pipeline::{Pipeline, Stage},
    profile::{ColorSpace, Pcs, Profile, ProfileClass},
};
use photocraft_doc::SampleEncoding;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_WORK: u64 = 1 << 30;
const SCRATCH_BYTES: u64 = 4096;
// Declared conservative algorithm work units, not a libm latency/RSS claim.
const PIXEL_WORK: u64 = 256;

/// The opt-in is a declared forward/analytic inverse pair, not a proof that
/// every value at the standard breakpoint has an exact mathematical inverse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrcPolicy {
    #[default]
    StrictAnalytic,
    DeclaredSrgbPair,
}
fn declared_srgb(curve: &Curve) -> bool {
    let Curve::Parametric { kind: 3, p } = curve else { return false };
    let Curve::Parametric { p: canonical, .. } = photocraft_cms::curve::srgb_trc() else { return false };
    let exact = p.iter().zip(canonical).all(|(a, b)| a.to_bits() == b.to_bits());
    let encoded = p.iter().zip(canonical).all(|(a, b)| a.to_bits() == math::s15f16_round(b).to_bits());
    exact || encoded
}

/// Caller-established identity of the supplied immutable profile/model and space.
/// Equality is a caller witness, not an ICC hash check or source authority grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub profile_identity: [u8; 32],
    pub working_space_identity: [u8; 32],
    pub encoding: SampleEncoding,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationRequirements {
    pub borrowed_profile_bytes: u64,
    pub private_pipeline_bytes: u64,
    pub peak_bytes: u64,
    pub inspection_work_units: u64,
    pub ingress_work_units: u64,
    pub egress_work_units: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub borrowed_profile_bytes: u64,
    pub retained_pipeline_bytes: u64,
    pub borrowed_source_bytes: u64,
    pub private_output_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_bytes: u64,
    pub preparation_work_units: u64,
    pub pixel_work_units: u64,
    pub work_units: u64,
}
/// Unassociated encoded RGB,A Float records. No Surface/Gray/Lab alias exists.
/// Selected RGB and alpha must be finite [0,1], including signed zero. Output
/// RGB may be finite HDR/negative; it is never implicitly clamped. Alpha is raw.
#[derive(Clone, Copy)]
pub struct RgbFloatView<'a> {
    pub extent: Rect,
    pub pixels: &'a [[f32; 4]],
    pub context: WorkingContext,
}
pub struct RgbFloatWindow {
    pub roi: Rect,
    pub pixels: Vec<[f32; 4]>,
    pub context: WorkingContext,
}
pub struct PreparedRgb<'a> {
    profile: &'a Profile,
    context: WorkingContext,
    parameters: Parameters,
    ingress: Pipeline,
    egress: Pipeline,
    profile_bytes: u64,
    pipeline_bytes: u64,
    preparation_work: u64,
    limits: Limits,
    trc_policy: TrcPolicy,
    graph_work: AtomicU64,
}
#[derive(Clone, Copy, Debug)]
pub struct GraphPointRequirements {
    pub retained_bytes: u64,
    pub scratch_bytes: u64,
    pub preparation_work_units: u64,
    pub pixel_work_units: u64,
    pub remaining_work_units: u64,
}
fn prepared_header_bytes() -> Result<u64, E> {
    u64::try_from(size_of::<PreparedRgb<'_>>()).map_err(|_| E::LimitExceeded)
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(crate) fn cms_error(error: control::Error) -> E {
    match error {
        control::Error::Cancelled => E::Cancelled,
        control::Error::Allocation => E::AllocationFailed,
        control::Error::Limit(_) => E::LimitExceeded,
        control::Error::Cms(_) => E::UnsupportedConversion,
    }
}
fn limits_checked(limits: Limits) -> Result<(), E> {
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(())
}
fn cms_limits(limits: Limits) -> Result<control::Limits, E> {
    limits_checked(limits)?;
    let maximum = control::Limits::default();
    Ok(control::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes).map_err(|_| E::LimitExceeded)?.min(maximum.resident_bytes),
        work_units: limits.work_units.min(maximum.work_units),
    })
}
fn plus(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn bytes(count: usize, width: usize) -> Result<u64, E> {
    u64::try_from(count.checked_mul(width).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)
}
fn valid_gamma(g: f64) -> bool {
    g.is_finite() && (0.000001..=64.0).contains(&g)
}
/// Closed invertible analytical families; no sampled curves, numerical inverse
/// fallback, LUT/device-link interpretation, or profile equivalence heuristics.
#[cfg(test)]
fn validate_curve(curve: &Curve) -> Result<(), E> {
    validate_curve_with_policy(curve, TrcPolicy::StrictAnalytic)
}
fn validate_curve_with_policy(curve: &Curve, policy: TrcPolicy) -> Result<(), E> {
    match curve {
        Curve::Identity => Ok(()),
        Curve::Gamma(g) if valid_gamma(*g) => Ok(()),
        Curve::Parametric { kind: 0, p } if valid_gamma(p[0]) && p.iter().all(|v| v.is_finite()) => Ok(()),
        _ if policy == TrcPolicy::DeclaredSrgbPair && declared_srgb(curve) => Ok(()),
        // Piecewise kinds3/4 can contain gaps/overlaps, including otherwise
        // finite positive branches. This candidate admits no guessed join
        // tolerance and no numerical inverse fallback; all other such curves refuse.
        _ => Err(E::UnsupportedConversion),
    }
}
fn profile_domain(profile: &Profile, context: WorkingContext, policy: TrcPolicy, c: &Interrupt<'_>) -> Result<(), E> {
    poll(c)?;
    if context.encoding != SampleEncoding::NonlinearProfile
        || context.profile_identity == [0; 32]
        || context.working_space_identity == [0; 32]
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
    if matrix.iter().flatten().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    let inverse = math::invert(matrix).ok_or(E::UnsupportedConversion)?;
    if inverse.iter().flatten().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    for curve in profile.trc.as_ref().ok_or(E::UnsupportedConversion)? {
        poll(c)?;
        validate_curve_with_policy(curve, policy)?;
    }
    poll(c)
}
/// Borrowed analytical profile census before any pipeline allocation. The caller
/// still owns profile parsing/identity establishment and its original storage.
pub fn preparation_requirements(
    profile: &Profile,
    context: WorkingContext,
    parameters: Parameters,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<PreparationRequirements, E> {
    preparation_requirements_with_trc_policy(profile, context, parameters, TrcPolicy::StrictAnalytic, limits, c)
}
/// Explicit policy admission; supplied curves are checked and never normalized.
pub fn preparation_requirements_with_trc_policy(
    profile: &Profile,
    context: WorkingContext,
    parameters: Parameters,
    policy: TrcPolicy,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<PreparationRequirements, E> {
    poll(c)?;
    limits_checked(limits)?;
    let header_and_scratch = plus(prepared_header_bytes()?, SCRATCH_BYTES)?;
    if header_and_scratch > limits.resident_bytes {
        return Err(E::LimitExceeded);
    }
    hue_chroma::validate(parameters)?;
    profile_domain(profile, context, policy, c)?;
    let cancelled = || c.check().is_err();
    let lower = cms_limits(limits)?;
    let input = profile.side_requirements_with_control(true, Intent::RelativeColorimetric, lower, &cancelled).map_err(cms_error)?;
    let output = profile.side_requirements_with_control(false, Intent::RelativeColorimetric, lower, &cancelled).map_err(cms_error)?;
    let borrowed_profile_bytes = u64::try_from(input.retained_profile_bytes.max(output.retained_profile_bytes)).map_err(|_| E::LimitExceeded)?;
    let private_pipeline_bytes =
        plus(u64::try_from(input.staged_bytes).map_err(|_| E::LimitExceeded)?, u64::try_from(output.staged_bytes).map_err(|_| E::LimitExceeded)?)?;
    let peak_bytes = plus(plus(borrowed_profile_bytes, private_pipeline_bytes)?, header_and_scratch)?;
    let side_work = plus(input.work_units, output.work_units)?;
    // Include caller-side census plus this preparation's repeated CMS side
    // requirements, construction and fixed analytical-domain/capacity checks.
    // Each constructor quota includes its own requirements/copying phase.
    let inspection_work_units = plus(side_work.checked_mul(2).ok_or(E::LimitExceeded)?, 256)?;
    let ingress_work_units = input.work_units.checked_mul(2).ok_or(E::LimitExceeded)?;
    let egress_work_units = output.work_units.checked_mul(2).ok_or(E::LimitExceeded)?;
    let work_units = plus(inspection_work_units, plus(ingress_work_units, egress_work_units)?)?;
    if peak_bytes > limits.resident_bytes || work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    poll(c)?;
    Ok(PreparationRequirements {
        borrowed_profile_bytes,
        private_pipeline_bytes,
        peak_bytes,
        inspection_work_units,
        ingress_work_units,
        egress_work_units,
        work_units,
    })
}
pub(crate) fn pipeline_bytes(p: &Pipeline, policy: TrcPolicy) -> Result<u64, E> {
    let mut result = bytes(p.stages.capacity(), size_of::<Stage>())?;
    if p.inputs != 3 || p.outputs != 3 || p.stages.len() != 2 {
        return Err(E::UnsupportedConversion);
    }
    for stage in &p.stages {
        match stage {
            Stage::Curves(curves) | Stage::InvCurves(curves) => {
                if curves.len() != 3 {
                    return Err(E::UnsupportedConversion);
                }
                for curve in curves {
                    validate_curve_with_policy(curve, policy)?;
                }
                result = plus(result, bytes(curves.capacity(), size_of::<Curve>())?)?;
            }
            Stage::Matrix { rows: 3, cols: 3, m, offset } => {
                if m.len() != 9 || offset.len() != 3 || m.iter().chain(offset).any(|v| !v.is_finite()) {
                    return Err(E::UnsupportedConversion);
                }
                result = plus(result, plus(bytes(m.capacity(), size_of::<f64>())?, bytes(offset.capacity(), size_of::<f64>())?)?)?;
            }
            _ => return Err(E::UnsupportedConversion),
        }
    }
    Ok(result)
}
fn count(rect: Rect) -> Result<usize, E> {
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 {
        return Err(E::InvalidBounds);
    }
    let width = usize::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| E::LimitExceeded)?;
    let height = usize::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| E::LimitExceeded)?;
    width.checked_mul(height).ok_or(E::LimitExceeded)
}
fn index(extent: Rect, x: i32, y: i32) -> Result<usize, E> {
    let width = usize::try_from(i64::from(extent.x1) - i64::from(extent.x0)).map_err(|_| E::InvalidBounds)?;
    let row = usize::try_from(i64::from(y) - i64::from(extent.y0)).map_err(|_| E::InvalidBounds)?;
    let column = usize::try_from(i64::from(x) - i64::from(extent.x0)).map_err(|_| E::InvalidBounds)?;
    row.checked_mul(width).and_then(|n| n.checked_add(column)).ok_or(E::LimitExceeded)
}
fn unit_pixel(input: [f32; 4]) -> Result<(), E> {
    if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(E::InvalidSample);
    }
    Ok(())
}
pub(crate) fn narrow(value: f64) -> Result<f32, E> {
    if !value.is_finite() || value.abs() > f64::from(f32::MAX) {
        return Err(E::InvalidSample);
    }
    let out = value as f32;
    if !out.is_finite() {
        return Err(E::InvalidSample);
    }
    Ok(out)
}
/// CIE polar equations in f64, then a single final Float component rounding.
pub(crate) fn lab_to_lch_float(lab: [f64; 3], alpha: f32) -> Result<[f32; 4], E> {
    if lab.iter().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    let [lightness, a, b] = lab;
    let chroma = a.hypot(b);
    let hue = if chroma == 0.0 { 0.0 } else { b.atan2(a).to_degrees().rem_euclid(360.0) };
    Ok([narrow(lightness)?, narrow(chroma)?, narrow(hue)?, alpha])
}
pub(crate) fn lch_to_lab(input: [f32; 4]) -> Result<[f64; 3], E> {
    let [lightness, chroma, hue, _] = input;
    let angle = f64::from(hue).to_radians();
    let output = [f64::from(lightness), f64::from(chroma) * angle.cos(), f64::from(chroma) * angle.sin()];
    if output.iter().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    Ok(output)
}
fn point_stage(input: [f32; 4], parameters: Parameters) -> Result<[f32; 4], E> {
    hue_chroma::evaluate_lch_float(input, parameters)
}
impl<'a> PreparedRgb<'a> {
    /// Relative-colorimetric analytical profile sides only; no BPC/CLUT/cache.
    /// Does not clone the caller profile or infer a profile for absent metadata.
    pub fn prepare(profile: &'a Profile, context: WorkingContext, parameters: Parameters, limits: Limits, c: &Interrupt<'_>) -> Result<Self, E> {
        Self::prepare_with_trc_policy(profile, context, parameters, TrcPolicy::StrictAnalytic, limits, c)
    }
    /// Uses the original supplied curves and their existing analytical paired
    /// inverse, including the IEC/encoded breakpoint gap. No curve is repaired.
    pub fn prepare_with_trc_policy(
        profile: &'a Profile,
        context: WorkingContext,
        parameters: Parameters,
        policy: TrcPolicy,
        limits: Limits,
        c: &Interrupt<'_>,
    ) -> Result<Self, E> {
        let req = preparation_requirements_with_trc_policy(profile, context, parameters, policy, limits, c)?;
        let cancelled = || c.check().is_err();
        let mut remaining_work = limits.work_units.checked_sub(req.inspection_work_units).ok_or(E::LimitExceeded)?;
        if req.ingress_work_units > remaining_work {
            return Err(E::LimitExceeded);
        }
        let resident_for_pipelines = limits.resident_bytes.checked_sub(plus(SCRATCH_BYTES, prepared_header_bytes()?)?).ok_or(E::LimitExceeded)?;
        let lower = cms_limits(Limits { resident_bytes: resident_for_pipelines, work_units: req.ingress_work_units })?;
        let (ingress, in_pcs) = profile.device_to_pcs_with_control(Intent::RelativeColorimetric, lower, &cancelled).map_err(cms_error)?;
        remaining_work = remaining_work.checked_sub(req.ingress_work_units).ok_or(E::LimitExceeded)?;
        poll(c)?;
        let ingress = Pipeline::new(3, ingress);
        let input_retained = pipeline_bytes(&ingress, policy)?;
        let remaining = resident_for_pipelines.checked_sub(input_retained).ok_or(E::LimitExceeded)?;
        if req.egress_work_units > remaining_work {
            return Err(E::LimitExceeded);
        }
        let output_lower = cms_limits(Limits { resident_bytes: remaining, work_units: req.egress_work_units })?;
        let (egress, out_pcs) = profile.pcs_to_device_with_control(Intent::RelativeColorimetric, output_lower, &cancelled).map_err(cms_error)?;
        if in_pcs != Pcs::Xyz || out_pcs != Pcs::Xyz {
            return Err(E::UnsupportedConversion);
        }
        let egress = Pipeline::new(3, egress);
        remaining_work = remaining_work.checked_sub(req.egress_work_units).ok_or(E::LimitExceeded)?;
        let preparation_work = limits.work_units.checked_sub(remaining_work).ok_or(E::LimitExceeded)?;
        if preparation_work != req.work_units {
            return Err(E::LimitExceeded);
        }
        let retained = plus(pipeline_bytes(&ingress, policy)?, pipeline_bytes(&egress, policy)?)?;
        if retained > req.private_pipeline_bytes
            || plus(plus(req.borrowed_profile_bytes, retained)?, plus(SCRATCH_BYTES, prepared_header_bytes()?)?)? > limits.resident_bytes
        {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(Self {
            profile,
            context,
            parameters,
            ingress,
            egress,
            profile_bytes: req.borrowed_profile_bytes,
            pipeline_bytes: retained,
            preparation_work,
            limits,
            trc_policy: policy,
            graph_work: AtomicU64::new(preparation_work),
        })
    }
    pub fn trc_policy(&self) -> TrcPolicy {
        self.trc_policy
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    pub fn profile(&self) -> &'a Profile {
        self.profile
    }
    pub fn parameters(&self) -> Parameters {
        self.parameters
    }
    pub fn preparation_work_units(&self) -> u64 {
        self.preparation_work
    }
    pub fn limits(&self) -> Limits {
        self.limits
    }
    /// Actual known header/profile/pipeline retention, with constructor work
    /// separated from steady graph work. Enclosing scheduler prepays/holds full
    /// source/config/model backing independently, including pruned owners.
    pub fn graph_point_requirements(&self, c: &Interrupt<'_>) -> Result<GraphPointRequirements, E> {
        poll(c)?;
        let retained_bytes = plus(plus(self.profile_bytes, self.pipeline_bytes)?, prepared_header_bytes()?)?;
        if plus(retained_bytes, SCRATCH_BYTES)? > self.limits.resident_bytes {
            return Err(E::LimitExceeded);
        }
        let left = self.limits.work_units.saturating_sub(self.graph_work.load(Ordering::Relaxed));
        Ok(GraphPointRequirements {
            retained_bytes,
            scratch_bytes: SCRATCH_BYTES,
            preparation_work_units: self.preparation_work,
            pixel_work_units: PIXEL_WORK,
            remaining_work_units: left / PIXEL_WORK * PIXEL_WORK,
        })
    }
    /// Uses the original prepared pipelines/policy/equations, no per-ROI
    /// reconstruction. Caller prepays steady graph work; concurrent prospective
    /// checks are not reservations, so final admission remains atomic/private.
    pub fn evaluate_graph_pixel_with_interrupt(&self, input: [f32; 4], actual: WorkingContext, c: &Interrupt<'_>) -> Result<[f32; 4], E> {
        poll(c)?;
        if actual != self.context {
            return Err(E::UnsupportedConversion);
        }
        self.graph_work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(PIXEL_WORK).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| E::LimitExceeded)?;
        self.evaluate(input, c)
    }

    /// Allocation-free shape/context census; no sample scan or frame rendering.
    /// Retains the preparation quota in every render admission, conservatively;
    /// the prepared immutable envelope may only be lowered, never refreshed.
    pub fn requirements(&self, source: RgbFloatView<'_>, roi: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements, E> {
        poll(c)?;
        limits_checked(limits)?;
        if limits.resident_bytes > self.limits.resident_bytes || limits.work_units > self.limits.work_units {
            return Err(E::LimitExceeded);
        }
        if source.context != self.context {
            return Err(E::UnsupportedConversion);
        }
        let input = count(source.extent)?;
        let output = count(roi)?;
        if input != source.pixels.len() {
            return Err(E::LengthMismatch);
        }
        if !roi.is_empty() && roi.intersect(&source.extent) != roi {
            return Err(E::InvalidBounds);
        }
        let borrowed_source_bytes = bytes(input, 16)?;
        let private_output_bytes = bytes(output, 16)?;
        let peak_bytes = plus(
            plus(plus(self.profile_bytes, self.pipeline_bytes)?, borrowed_source_bytes)?,
            plus(private_output_bytes, plus(SCRATCH_BYTES, prepared_header_bytes()?)?)?,
        )?;
        let pixel_work_units = u64::try_from(output).map_err(|_| E::LimitExceeded)?.checked_mul(PIXEL_WORK).ok_or(E::LimitExceeded)?;
        let work_units = plus(self.preparation_work, pixel_work_units)?;
        if peak_bytes > limits.resident_bytes || work_units > limits.work_units {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(Requirements {
            borrowed_profile_bytes: self.profile_bytes,
            retained_pipeline_bytes: self.pipeline_bytes,
            borrowed_source_bytes,
            private_output_bytes,
            scratch_bytes: SCRATCH_BYTES,
            peak_bytes,
            preparation_work_units: self.preparation_work,
            pixel_work_units,
            work_units,
        })
    }
    fn evaluate(&self, input: [f32; 4], c: &Interrupt<'_>) -> Result<[f32; 4], E> {
        poll(c)?;
        unit_pixel(input)?;
        let mut xyz = [0.0; 3];
        let encoded = [f64::from(input[0]), f64::from(input[1]), f64::from(input[2])];
        self.ingress.try_eval_f64(&encoded, &mut xyz).map_err(|_| E::InvalidSample)?;
        poll(c)?;
        let lch = lab_to_lch_float(math::xyz_to_lab(xyz, math::D50), input[3])?;
        let changed = point_stage(lch, self.parameters)?;
        let xyz = math::lab_to_xyz(lch_to_lab(changed)?, math::D50);
        if xyz.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        poll(c)?;
        let mut rgb = [0.0; 3];
        self.egress.try_eval_f64(&xyz, &mut rgb).map_err(|_| E::InvalidSample)?;
        poll(c)?;
        Ok([narrow(rgb[0])?, narrow(rgb[1])?, narrow(rgb[2])?, input[3]])
    }
    /// Validates selected numeric input under the actual token BEFORE output
    /// allocation. Private ROI output only; borrowed source/outside data
    /// are never modified. Actual Vec capacity is reconciled before evaluation.
    pub fn render_roi(&self, source: RgbFloatView<'_>, roi: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<RgbFloatWindow, E> {
        let req = self.requirements(source, roi, limits, c)?;
        if roi.is_empty() {
            poll(c)?;
            return Ok(RgbFloatWindow { roi, pixels: Vec::new(), context: self.context });
        }
        for y in roi.y0..roi.y1 {
            poll(c)?;
            for x in roi.x0..roi.x1 {
                poll(c)?;
                unit_pixel(*source.pixels.get(index(source.extent, x, y)?).ok_or(E::LengthMismatch)?)?;
            }
        }
        poll(c)?;
        let length = count(roi)?;
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).map_err(|_| E::AllocationFailed)?;
        let actual = bytes(pixels.capacity(), 16)?;
        if plus(req.peak_bytes.checked_sub(req.private_output_bytes).ok_or(E::LimitExceeded)?, actual)? > limits.resident_bytes {
            return Err(E::LimitExceeded);
        }
        for y in roi.y0..roi.y1 {
            poll(c)?;
            for x in roi.x0..roi.x1 {
                poll(c)?;
                pixels.push(self.evaluate(*source.pixels.get(index(source.extent, x, y)?).ok_or(E::LengthMismatch)?, c)?);
            }
        }
        if pixels.len() != length {
            return Err(E::LengthMismatch);
        }
        poll(c)?;
        Ok(RgbFloatWindow { roi, pixels, context: self.context })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_srgb_is_exact_closed_pair_and_strict_stays_default() {
        let canonical = photocraft_cms::curve::srgb_trc();
        let Curve::Parametric { kind, p } = canonical.clone() else { panic!("canonical kind3") };
        assert_eq!(kind, 3);
        let encoded = Curve::Parametric { kind, p: p.map(math::s15f16_round) };
        for curve in [&canonical, &encoded] {
            assert_eq!(validate_curve(curve), Err(E::UnsupportedConversion));
            assert_eq!(validate_curve_with_policy(curve, TrcPolicy::DeclaredSrgbPair), Ok(()));
            let Curve::Parametric { p: supplied, .. } = curve else { panic!("kind3") };
            for index in 0..7 {
                let mut changed = *supplied;
                changed[index] = f64::from_bits(changed[index].to_bits() ^ 1);
                assert_eq!(validate_curve_with_policy(&Curve::Parametric { kind: 3, p: changed }, TrcPolicy::DeclaredSrgbPair), Err(E::UnsupportedConversion));
            }
            assert_eq!(validate_curve_with_policy(&Curve::Parametric { kind: 4, p: *supplied }, TrcPolicy::DeclaredSrgbPair), Err(E::UnsupportedConversion));
            // Declare actual branch rules, including the gap: no roundtrip
            // equality claim and no invented join tolerance.
            let d = supplied[4];
            let high = (supplied[1] * d + supplied[2]).powf(supplied[0]);
            let low = supplied[3] * d;
            assert!(high.is_finite() && low.is_finite());
            assert_eq!(curve.eval64(d), high);
            assert_eq!(curve.eval_inverse64(low.min(high) * 0.5), low.min(high) * 0.5 / supplied[3]);
            for x in [0.0, d * 0.5, d, 0.5, 1.0] {
                assert!(curve.eval64(x).is_finite());
                assert!(curve.eval_inverse64(x).is_finite());
            }
        }
        let counterexample = Curve::Parametric { kind: 3, p: [1.0, 1.0, 0.0, 2.0, 0.5, 0.0, 0.0] };
        assert_eq!(validate_curve_with_policy(&counterexample, TrcPolicy::DeclaredSrgbPair), Err(E::UnsupportedConversion));
    }
    #[test]
    fn declared_srgb_icc_roundtrip_and_same_work_envelope() {
        let mut profile = photocraft_cms::Builtin::AdobeRgbCompat.profile().clone();
        let curve = photocraft_cms::curve::srgb_trc();
        profile.trc = Some([curve.clone(), curve.clone(), curve]);
        let parsed = Profile::parse(&profile.to_bytes()).unwrap();
        for curve in parsed.trc.as_ref().unwrap() {
            assert!(declared_srgb(curve));
        }
        for profile in [&profile, &parsed] {
            assert_eq!(PreparedRgb::prepare(profile, context(), neutral(), Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
            let req = preparation_requirements_with_trc_policy(profile, context(), neutral(), TrcPolicy::DeclaredSrgbPair, Limits::default(), &Interrupt::NONE)
                .unwrap();
            let low = Limits { work_units: req.work_units - 1, ..Limits::default() };
            assert_eq!(
                PreparedRgb::prepare_with_trc_policy(profile, context(), neutral(), TrcPolicy::DeclaredSrgbPair, low, &Interrupt::NONE).err(),
                Some(E::LimitExceeded)
            );
            let admitted = Limits { work_units: req.work_units + PIXEL_WORK, ..Limits::default() };
            let prepared =
                PreparedRgb::prepare_with_trc_policy(profile, context(), neutral(), TrcPolicy::DeclaredSrgbPair, admitted, &Interrupt::NONE).unwrap();
            assert_eq!(prepared.trc_policy(), TrcPolicy::DeclaredSrgbPair);
            let pixels = [[0.04045, 0.5, 0.75, -0.0]];
            let input = RgbFloatView { extent: Rect::new(0, 0, 1, 1), pixels: &pixels, context: context() };
            let output = prepared.render_roi(input, input.extent, admitted, &Interrupt::NONE).unwrap();
            assert!(output.pixels[0].iter().all(|v| v.is_finite()));
            assert_eq!(output.pixels[0][3].to_bits(), (-0.0_f32).to_bits());
            assert_eq!(
                prepared.render_roi(input, input.extent, Limits { work_units: admitted.work_units - 1, ..admitted }, &Interrupt::NONE).err(),
                Some(E::LimitExceeded)
            );
        }
    }
    fn context() -> WorkingContext {
        WorkingContext { profile_identity: [7; 32], working_space_identity: [9; 32], encoding: SampleEncoding::NonlinearProfile }
    }
    fn neutral() -> Parameters {
        Parameters { hue: 0.0, chroma: 0.0, lightness: 0.0 }
    }
    fn prepared() -> PreparedRgb<'static> {
        PreparedRgb::prepare(photocraft_cms::Builtin::AdobeRgbCompat.profile(), context(), neutral(), Limits::default(), &Interrupt::NONE).unwrap()
    }
    #[test]
    fn discontinuous_piecewise_curves_refuse_before_pipeline_preparation() {
        for kind in [3, 4] {
            let curve = Curve::Parametric { kind, p: [1.0, 1.0, 0.0, 2.0, 0.5, 0.0, 0.0] };
            assert_eq!(curve.eval64(0.375), 0.75);
            assert_eq!(curve.eval_inverse64(0.75), 0.75);
            assert_eq!(validate_curve(&curve), Err(E::UnsupportedConversion));
            let mut profile = photocraft_cms::Builtin::AdobeRgbCompat.profile().clone();
            profile.trc = Some([curve.clone(), curve.clone(), curve]);
            assert_eq!(preparation_requirements(&profile, context(), neutral(), Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        }
        // Standard piecewise curves remain closed too: no fitted join tolerance.
        assert_eq!(validate_curve(&photocraft_cms::curve::srgb_trc()), Err(E::UnsupportedConversion));
    }
    #[test]
    fn preparation_and_render_share_the_immutable_work_envelope() {
        let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let req = preparation_requirements(profile, context(), neutral(), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(req.work_units, req.inspection_work_units + req.ingress_work_units + req.egress_work_units);
        let too_low = Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: req.work_units - 1 };
        assert_eq!(PreparedRgb::prepare(profile, context(), neutral(), too_low, &Interrupt::NONE).err(), Some(E::LimitExceeded));
        let configured = Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: req.work_units + PIXEL_WORK };
        let prepared = PreparedRgb::prepare(profile, context(), neutral(), configured, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.preparation_work_units(), req.work_units);
        let pixels = [[0.0, 0.0, 0.0, -0.0]];
        let view = RgbFloatView { extent: Rect::new(0, 0, 1, 1), pixels: &pixels, context: context() };
        let render = prepared.requirements(view, view.extent, configured, &Interrupt::NONE).unwrap();
        assert_eq!(render.preparation_work_units, req.work_units);
        assert_eq!(render.pixel_work_units, PIXEL_WORK);
        assert_eq!(render.work_units, configured.work_units);
        let lowered = Limits { work_units: configured.work_units - 1, ..configured };
        assert_eq!(prepared.render_roi(view, view.extent, lowered, &Interrupt::NONE).err(), Some(E::LimitExceeded));
        assert_eq!(prepared.requirements(view, view.extent, Limits::default(), &Interrupt::NONE).err(), Some(E::LimitExceeded));
    }
    #[test]
    fn independent_captured_lch_stage_words_remain_exact() {
        // Own literal V4 implicit.source packet, 32 records; sampled indices
        // 0,1,3,4,5,6,7. No claim about this candidate's RGB/ICC conversion.
        let input = [
            [3011510272, 0, 0, 0],
            [1112903246, 0, 0, 0],
            [1113139444, 1121300597, 1109618215, 0],
            [1118806914, 1122151144, 1124491898, 0],
            [1106019840, 1124283720, 1133948591, 0],
            [1115392358, 1113198925, 1134358508, 0],
            [1107324811, 1117333585, 1133624066, 0],
        ];
        let expected = [
            [1090781184, 0, 0, 0],
            [1115065934, 0, 0, 0],
            [1115302132, 1119662197, 1117566740, 0],
            [1119888258, 1120512744, 1126949498, 0],
            [1108820736, 1122855568, 1135177391, 0],
            [1116619955, 1109922125, 1135587308, 0],
            [1109487499, 1115695185, 1134852866, 0],
        ];
        let p = Parameters { hue: 37.5, chroma: -12.5, lightness: 8.25 };
        for (source, target) in input.into_iter().zip(expected) {
            assert_eq!(point_stage(source.map(f32::from_bits), p).unwrap().map(f32::to_bits), target);
        }
    }
    #[test]
    fn public_polar_axes_and_raw_alpha() {
        assert_eq!(
            lab_to_lch_float([50.0, 0.0, 4.0], -0.0).unwrap().map(f32::to_bits),
            [50.0f32.to_bits(), 4.0f32.to_bits(), 90.0f32.to_bits(), (-0.0f32).to_bits()]
        );
        assert_eq!(lab_to_lch_float([50.0, 0.0, -4.0], 1.0).unwrap()[2], 270.0);
        assert!(lab_to_lch_float([f64::INFINITY, 0.0, 0.0], 1.0).is_err());
    }
    #[test]
    fn private_roi_preserves_outside_raw_words_and_alpha() {
        let p = prepared();
        let source = [[0.0, 0.0, 0.0, -0.0], [f32::from_bits(0x7fc00123), 0.0, 0.0, 1.0]];
        let before = source.map(|px| px.map(f32::to_bits));
        let view = RgbFloatView { extent: Rect::new(-3, 7, -1, 8), pixels: &source, context: context() };
        let result = p.render_roi(view, Rect::new(-3, 7, -2, 8), Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(result.pixels.len(), 1);
        assert_eq!(result.pixels[0][3].to_bits(), (-0.0f32).to_bits());
        assert_eq!(source.map(|px| px.map(f32::to_bits)), before);
        assert_eq!(p.render_roi(view, view.extent, Limits::default(), &Interrupt::NONE).err(), Some(E::InvalidSample));
    }
    #[test]
    fn hostile_empty_span_is_constant_work_and_context_is_exact() {
        let p = prepared();
        let view = RgbFloatView { extent: Rect::new(0, 0, 0, 0), pixels: &[], context: context() };
        assert!(p.render_roi(view, Rect::new(0, i32::MIN, 0, i32::MAX), Limits::default(), &Interrupt::NONE).unwrap().pixels.is_empty());
        let mut wrong = view;
        wrong.context.profile_identity = [8; 32];
        assert_eq!(p.requirements(wrong, view.extent, Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        assert_eq!(p.requirements(view, Rect::new(1, 0, 0, 1), Limits::default(), &Interrupt::NONE).err(), Some(E::InvalidBounds));
    }
    #[test]
    fn unsupported_profile_and_numeric_input_refuse_privately() {
        let mut profile = photocraft_cms::builtin::srgb().clone();
        profile.trc = Some([Curve::Table(vec![0.0, 1.0]), Curve::Identity, Curve::Identity]);
        assert_eq!(preparation_requirements(&profile, context(), neutral(), Limits::default(), &Interrupt::NONE).err(), Some(E::UnsupportedConversion));
        assert_eq!(unit_pixel([1.01, 0.0, 0.0, 1.0]), Err(E::InvalidSample));
        assert_eq!(unit_pixel([0.0, 0.0, 0.0, f32::NAN]), Err(E::InvalidSample));
        let p = prepared();
        let source = [[f32::NAN, 0.0, 0.0, 1.0]];
        let view = RgbFloatView { extent: Rect::new(0, 0, 1, 1), pixels: &source, context: context() };
        assert_eq!(p.requirements(view, view.extent, Limits { resident_bytes: 1, work_units: MAX_WORK }, &Interrupt::NONE).err(), Some(E::LimitExceeded));
    }
    #[test]
    fn cancelled_selected_validation_keeps_borrowed_input() {
        let p = prepared();
        let source = [[0.5, 0.5, 0.5, -0.0]; 8];
        let before = source.map(|px| px.map(f32::to_bits));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 6;
        let progress = |_: f32| {};
        let control = Interrupt::new(&cancelled, &progress);
        let view = RgbFloatView { extent: Rect::new(0, 0, 8, 1), pixels: &source, context: context() };
        assert_eq!(p.render_roi(view, view.extent, Limits::default(), &control).err(), Some(E::Cancelled));
        assert_eq!(source.map(|px| px.map(f32::to_bits)), before);
    }
    #[test]
    fn graph_point_reuses_original_pipeline_and_raw_alpha_without_constructor_reset() {
        let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let parameters = Parameters { hue: 37.5, chroma: -12.5, lightness: 8.25 };
        let prepared = PreparedRgb::prepare(profile, context(), parameters, Limits::default(), &Interrupt::NONE).unwrap();
        let pixels = [[0.25, 0.5, 0.75, -0.0], [0.5, 0.25, 0.75, 1.0]];
        let view = RgbFloatView { extent: Rect::new(-1, 2, 1, 3), pixels: &pixels, context: context() };
        let array = prepared.render_roi(view, view.extent, Limits::default(), &Interrupt::NONE).unwrap();
        let before = prepared.graph_point_requirements(&Interrupt::NONE).unwrap();
        for (input, expected) in pixels.into_iter().zip(array.pixels) {
            let actual = prepared.evaluate_graph_pixel_with_interrupt(input, context(), &Interrupt::NONE).unwrap();
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
            assert_eq!(actual[3].to_bits(), input[3].to_bits());
        }
        let after = prepared.graph_point_requirements(&Interrupt::NONE).unwrap();
        assert_eq!(before.preparation_work_units, after.preparation_work_units);
        assert_eq!(before.remaining_work_units - after.remaining_work_units, 2 * PIXEL_WORK);
        assert!(std::ptr::eq(prepared.profile(), profile));
    }
    #[test]
    fn graph_steady_quota_is_shared_across_requests_and_cancel_never_recaptures() {
        let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let req = preparation_requirements(profile, context(), neutral(), Limits::default(), &Interrupt::NONE).unwrap();
        let prepared =
            PreparedRgb::prepare(profile, context(), neutral(), Limits { work_units: req.work_units + PIXEL_WORK, ..Limits::default() }, &Interrupt::NONE)
                .unwrap();
        assert_eq!(prepared.graph_point_requirements(&Interrupt::NONE).unwrap().remaining_work_units, PIXEL_WORK);
        assert_eq!(prepared.evaluate_graph_pixel_with_interrupt([0.25, 0.5, 0.75, -0.0], context(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
        assert_eq!(prepared.graph_point_requirements(&Interrupt::NONE).unwrap().remaining_work_units, PIXEL_WORK);
        prepared.evaluate_graph_pixel_with_interrupt([0.25, 0.5, 0.75, -0.0], context(), &Interrupt::NONE).unwrap();
        assert_eq!(prepared.graph_point_requirements(&Interrupt::NONE).unwrap().remaining_work_units, 0);
        assert_eq!(prepared.evaluate_graph_pixel_with_interrupt([0.25, 0.5, 0.75, -0.0], context(), &Interrupt::NONE), Err(E::LimitExceeded));
        assert!(matches!(
            PreparedRgb::prepare(
                profile,
                context(),
                neutral(),
                Limits { resident_bytes: prepared_header_bytes().unwrap() - 1, ..Limits::default() },
                &Interrupt::NONE
            ),
            Err(E::LimitExceeded)
        ));
    }
}
