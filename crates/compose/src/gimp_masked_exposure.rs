//! Declared ordinary Point scalar-mask Exposure compatibility. No generic mask authority.
use crate::{backed_normal::BackedNormalError as E, backed_stock_point_context::PlanBudget, gimp_composite::WorkingSpace};
use photocraft_doc::{SampleEncoding, import_compositing::GimpSpace};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = 1024;
const MIX_WORK: u64 = 2048; // 1152 transfer + 896 guards, unfused math, finite checks and Float tail.

fn scope() -> E {
    E::Unsupported("masked Exposure scalar finite/unit policy")
}
fn length() -> E {
    E::Invalid("masked Exposure preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
pub const EPOCH: &str = "xcf22-gegl-exposure-masked-wide-v1";
/// Raw mask data is retained unchanged. Checked Float conversion is an explicit
/// scalar policy, including the finite nonnegative rounding preimage of one.
pub(crate) fn coverage(raw: f64) -> Result<f64, E> {
    if !raw.is_finite() || raw < 0.0 {
        return Err(scope());
    }
    let value = raw as f32;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(scope());
    }
    Ok(f64::from(value))
}
pub fn parameters(
    arguments: &[photocraft_doc::xcf_filters::FilterArgument],
) -> Result<crate::gimp_exposure::Parameters, crate::gimp_composite::CompositionError> {
    crate::gimp_exposure::parameters(arguments)
}
pub(crate) fn canonical(
    node: &photocraft_doc::xcf_filters::FilterNode,
    extent: photocraft_geom::Rect,
) -> Result<crate::gimp_exposure::Parameters, crate::gimp_composite::CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    if node.operation.name != "gegl:exposure"
        || node.operation.stored_version != "1:0"
        || node.operation.schema_epoch != EPOCH
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != EPOCH || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(crate::gimp_composite::CompositionError::Unsupported("masked Exposure exact envelope"));
    }
    parameters(&node.arguments)
}
fn pixel(p: [f64; 4]) -> Result<(), E> {
    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
        return Err(scope());
    }
    Ok(())
}
fn finite(v: f64) -> Result<f64, E> {
    if v.is_finite() { Ok(v) } else { Err(scope()) }
}
fn float_tail(p: [f64; 4]) -> Result<[f64; 4], E> {
    pixel(p)?;
    let f = p.map(|v| v as f32);
    if f.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&f[3]) {
        return Err(scope());
    }
    Ok(f.map(f64::from))
}
/// The fixed analytic Space is independently owned/prepaid; current ICC binding remains separate.
pub(crate) struct PreparedMix {
    working: WorkingSpace,
}
impl PreparedMix {
    /// Borrow this actual owned fixed analytic Space; callers pay leaf work and
    /// keep the owning Mix and its real fee live through leaf plus mask mixing.
    pub(crate) fn linear_space(&self) -> &WorkingSpace { &self.working }
    pub(crate) fn prepare<B: PlanBudget>(outer: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        charge(b, i, 512)?;
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
        let bytes = (std::mem::size_of::<Self>() as u64).checked_add(outer).and_then(|n| n.checked_add(PIXEL_SCRATCH_BYTES + 128)).ok_or_else(length)?;
        charge(b, i, semantic.checked_mul(3).and_then(|n| n.checked_add(bytes.checked_mul(64)?)).ok_or_else(length)?)?;
        let fee = b.reserve(bytes)?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, i).map_err(E::Color)?;
        let result = Self { working };
        charge(b, i, 0)?;
        Ok((result, fee))
    }
    /// Caller holds this fee across full leaf evaluation AND mixing, independently of leaf scratch.
    pub(crate) fn reserve_runtime(&self, q: &BackingQuota, c: &ReadControl<'_>) -> Result<QuotaLease, E> {
        c.check(0).map_err(E::Backing)?;
        q.reserve(QuotaKind::Resident, PIXEL_SCRATCH_BYTES).map_err(E::Quota)
    }
    /// Every enabled nonempty-mask sample calls this after the complete leaf, including zero and one.
    pub(crate) fn apply(&self, before: [f64; 4], after: [f64; 4], raw_coverage: f64, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        let t = coverage(raw_coverage)?;
        c.check(MIX_WORK).map_err(E::Backing)?;
        pixel(before)?;
        pixel(after)?;
        c.check(0).map_err(E::Backing)?;
        let b = self.working.convert([before[0], before[1], before[2]], true).map_err(E::Color)?;
        let a = self.working.convert([after[0], after[1], after[2]], true).map_err(E::Color)?;
        if b.iter().chain(a.iter()).any(|v| !v.is_finite()) {
            return Err(scope());
        }
        let delta = finite(after[3] - before[3])?;
        let scaled = finite(t * delta)?;
        let alpha = finite(before[3] + scaled)?;
        if !(0.0..=1.0).contains(&alpha) {
            return Err(scope());
        }
        let mut linear = [0.; 3];
        for ((out, b), a) in linear.iter_mut().zip(b).zip(a) {
            *out = if alpha == 0.0 {
                let delta = finite(a - b)?;
                let scaled = finite(t * delta)?;
                finite(b + scaled)?
            } else {
                let bp = finite(b * before[3])?;
                let ap = finite(a * after[3])?;
                let delta = finite(ap - bp)?;
                let scaled = finite(t * delta)?;
                let mixed = finite(bp + scaled)?;
                finite(mixed / alpha)?
            };
        }
        c.check(0).map_err(E::Backing)?;
        let rgb = self.working.convert(linear, false).map_err(E::Color)?;
        let output = float_tail([rgb[0], rgb[1], rgb[2], alpha])?;
        c.check(0).map_err(E::Backing)?;
        Ok(output)
    }
}

/// Full leaf plus independent scalar-mask mixing. The existing Exposure
/// constructor proves scale/denominator; identity parameters still traverse
/// the actual Linear working conversion and operation here.
pub(crate) struct PreparedMaskedExposure {
    mix: PreparedMix,
    operation: photocraft_algo::gimp::point::PreparedPointOperation,
}
impl PreparedMaskedExposure {
    pub(crate) fn prepare<B: PlanBudget>(p: crate::gimp_exposure::Parameters, outer: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        let outer = outer.checked_add(std::mem::size_of::<Self>() as u64).ok_or_else(length)?;
        let fee;
        let mix;
        (mix, fee) = PreparedMix::prepare(outer, i, b)?;
        charge(b, i, 512)?;
        let [stops, black_level] = p.values();
        let operation = photocraft_algo::gimp::point::PointOperation::Exposure { stops, black_level }.prepare().map_err(|e| E::Color(e.into()))?;
        let result = Self { mix, operation };
        charge(b, i, 0)?;
        Ok((result, fee))
    }
    pub(crate) fn reserve_runtime(&self, q: &BackingQuota, c: &ReadControl<'_>) -> Result<QuotaLease, E> {
        self.mix.reserve_runtime(q, c)
    }
    /// Caller holds mix scratch before decoding before/mask and through this call.
    pub(crate) fn evaluate(&self, before: [f64; 4], enabled: bool, raw_coverage: f64, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        c.check(128).map_err(E::Backing)?;
        pixel(before)?;
        if before.iter().take(3).any(|v| !(0.0..=1.0).contains(v)) {
            return Err(scope());
        }
        coverage(raw_coverage)?;
        if !enabled {
            c.check(0).map_err(E::Backing)?;
            return Ok(before);
        }
        c.check(1408).map_err(E::Backing)?;
        let _leaf_scratch = q.reserve(QuotaKind::Resident, crate::gimp_exposure::PIXEL_SCRATCH_BYTES).map_err(E::Quota)?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.mix.working.convert([before[0], before[1], before[2]], true).map_err(E::Color)?;
        let mut leaf = [0.; 4];
        c.check(0).map_err(E::Backing)?;
        self.operation.try_eval_pixel(&[rgb[0], rgb[1], rgb[2], before[3]], 3, &mut leaf).map_err(|e| E::Color(e.into()))?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.mix.working.convert([leaf[0], leaf[1], leaf[2]], false).map_err(E::Color)?;
        let after = [rgb[0], rgb[1], rgb[2], before[3]];
        pixel(after)?;
        // No local zero or unit shortcut; leaf scratch remains held during mix.
        self.mix.apply(before, after, raw_coverage, c)
    }
    /// Additive ordered-stage lane: finite derived RGB, unit alpha; old unit entry stays exact.
    pub(crate) fn evaluate_derived(&self, before: [f64; 4], enabled: bool, raw_coverage: f64, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        c.check(128).map_err(E::Backing)?;
        pixel(before)?;
        coverage(raw_coverage)?;
        if !enabled {
            c.check(0).map_err(E::Backing)?;
            return Ok(before);
        }
        c.check(1408).map_err(E::Backing)?;
        let _leaf_scratch = q.reserve(QuotaKind::Resident, crate::gimp_exposure::PIXEL_SCRATCH_BYTES).map_err(E::Quota)?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.mix.working.convert([before[0], before[1], before[2]], true).map_err(E::Color)?;
        let mut leaf = [0.; 4];
        c.check(0).map_err(E::Backing)?;
        self.operation.try_eval_pixel(&[rgb[0], rgb[1], rgb[2], before[3]], 3, &mut leaf).map_err(|e| E::Color(e.into()))?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.mix.working.convert([leaf[0], leaf[1], leaf[2]], false).map_err(E::Color)?;
        let after = [rgb[0], rgb[1], rgb[2], before[3]];
        pixel(after)?;
        // No local zero or unit shortcut; leaf scratch remains held during mix.
        self.mix.apply(before, after, raw_coverage, c)
    }
}


#[cfg(test)]
#[path="gimp_masked_exposure_tests.rs"]
mod tests;
