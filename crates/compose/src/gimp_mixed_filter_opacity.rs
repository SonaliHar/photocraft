//! Declared CURRENT-only mixed opacity recipe; no Source or ordinary Point authority.
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
const PARTIAL_WORK: u64 = 2048; // 1152 transfer + 896 guards, unfused math, finite checks and Float tail.
const ZERO_WORK: u64 = 256;
fn scope() -> E {
    E::Unsupported("mixed CURRENT opacity finite/unit policy")
}
fn length() -> E {
    E::Invalid("mixed opacity preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
pub(crate) fn validate_opacity(bits: u64) -> Result<f64, E> {
    let t = f64::from_bits(bits);
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return Err(scope());
    }
    Ok(t)
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
    /// Only enabled nonunit nodes call this after complete leaf evaluation/finite proof.
    pub(crate) fn apply(&self, before: [f64; 4], after: [f64; 4], bits: u64, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        let t = validate_opacity(bits)?;
        if t == 1.0 {
            return Err(scope());
        } // Exact unit path must stay literal old body.
        c.check(if t == 0.0 { ZERO_WORK } else { PARTIAL_WORK }).map_err(E::Backing)?;
        pixel(before)?;
        pixel(after)?;
        if t == 0.0 {
            let output = float_tail(before)?;
            c.check(0).map_err(E::Backing)?;
            return Ok(output);
        }
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

#[cfg(test)]
#[path = "gimp_mixed_filter_opacity_tests.rs"]
mod tests;
