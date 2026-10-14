//! Cheap prospective work planning, not profile equality or source authority.
use crate::{
    Builtin, ColorSpace, Curve,
    control::{Budget, Error, Limits, Result, invalid},
};

pub(super) fn base_effective_work(retained: usize) -> Result<u64> {
    u64::try_from(retained).ok().and_then(|n| n.checked_mul(4)).and_then(|n| n.checked_add(200_000)).ok_or(Error::Limit("effective profile work"))
}

/// Allocation-free bounded metadata planning for implicit nonlinear RGB only.
/// The caller must prepay 4096 planning work units before calling. The returned
/// work is checked against the default future-operation ceiling, independently
/// of this call's planning work allowance. The caller subsequently prepays it
/// and supplies real operation limits. It is prospective accounting, not a
/// profile proof. No ICC bytes are read or compared from `bound_len` alone.
/// Builtin OnceLock initialization remains an existing provider boundary, with
/// polls immediately before and after it; it is not made interruptible here.
pub fn effective_implicit_rgb_nonlinear_planner_work_with_control(bound_len: usize, limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<u64> {
    let mut planning = Budget::new(limits, cancelled)?;
    if bound_len > 16 << 20 {
        return Err(invalid("XCF implicit ICC exceeds 16 MiB"));
    }
    // Reject insufficient planning allowance before entering the provider.
    planning.work(4096)?;
    planning.poll()?;
    let original = Builtin::Srgb.profile();
    planning.poll()?;
    if original.color_space != ColorSpace::Rgb
        || original.matrix.as_ref().is_none_or(|m| m.iter().flatten().any(|v| !v.is_finite()))
        || original.gray_trc.is_some()
        || original.a2b.iter().chain(&original.b2a).any(Option::is_some)
    {
        return Err(invalid("implicit RGB planner requires analytic matrix builtin"));
    }
    let curves = original.trc.as_ref().ok_or_else(|| invalid("implicit RGB planner missing curves"))?;
    for curve in curves {
        planning.poll()?;
        let analytic = match curve {
            Curve::Identity => true,
            Curve::Gamma(g) => g.is_finite() && *g > 0.,
            Curve::Parametric { kind, p } => *kind <= 4 && p.iter().all(|v| v.is_finite()),
            Curve::Table(_) | Curve::Table64(_) => false,
        };
        if !analytic {
            return Err(invalid("implicit RGB planner requires constant-size analytic curves"));
        }
    }
    // These existing metadata-only helpers account actual retained capacities.
    // The fixed planning cap refuses future larger cloned metadata rather than
    // silently continuing with an unbounded planner. No curve evaluation occurs.
    let metadata_limits = Limits { resident_bytes: limits.resident_bytes, work_units: 4096 };
    let metadata = Budget::new(metadata_limits, cancelled)?;
    let retained = original.controlled_retained_bytes(&metadata)?;
    let cloned = original.clone_requirements_with_control(metadata_limits, cancelled)?;
    let peak = retained.checked_add(cloned.cloned_model_bytes).and_then(|n| n.checked_add(bound_len)).ok_or(Error::Limit("implicit ICC peak"))?;
    let work = base_effective_work(retained)?.checked_add(bound_len as u64).ok_or(Error::Limit("implicit ICC work"))?;
    if work > Limits::default().work_units {
        return Err(Error::Limit("implicit ICC future work"));
    }
    Budget::new(limits, cancelled)?.admit(peak, 0)?;
    planning.poll()?;
    Ok(work)
}

#[cfg(test)]
#[path = "implicit_planner_tests.rs"]
mod tests;
