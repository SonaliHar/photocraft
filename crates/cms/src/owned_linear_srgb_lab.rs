//! Explicit fixed Linear-sRGB device coordinates to physical D50 Lab owners.
//! This is separate from native/current source profile and encoding admission.
//! No nonlinear encoding tag is used to manufacture a Linear recipe.
//! Fixed builtin initialization remains process-global, outside job receipts.
//! Requirements are facts, not reservations. Callers must retain their actual
//! aggregate peak reservation and prepay work before construction.
use crate::{
    Builtin, Intent, Pcs, Profile, Transform, TransformOptions,
    control::{Budget, Error, Limits, Result},
    owned_lab::Parts,
    pipeline::{Pipeline, Stage},
};
#[cfg(test)]
#[path = "owned_linear_srgb_lab_tests.rs"]
mod tests;
fn add(a: usize, b: usize) -> Result<usize> {
    a.checked_add(b).ok_or(Error::Limit("owned Lab storage"))
}
fn work(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::Limit("owned Lab work"))
}
fn times(a: u64, n: u64) -> Result<u64> {
    a.checked_mul(n).ok_or(Error::Limit("owned Lab work"))
}
fn options() -> TransformOptions {
    TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, ..Default::default() }
}
/// O(1) caller prepayment; this function initializes no builtin profile.
/// LinearSrgb/LabD50 process-global initialization is the established CMS
/// boundary. Borrowed planning after that boundary allocates no job owners.
pub const fn planning_work_ceiling() -> u64 {
    16_384
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub peak_resident_bytes: usize,
    pub work_units: u64,
    source: crate::profile::ProfileCloneRequirements,
    target: crate::profile::ProfileCloneRequirements,
    forward: crate::transform::TransformRequirements,
    reverse: crate::transform::TransformRequirements,
    side_forward: crate::profile::SidePipelineRequirements,
    side_reverse: crate::profile::SidePipelineRequirements,
    forward_nodes: usize,
    reverse_nodes: usize,
}
fn phase(bytes: usize, units: u64) -> Limits {
    Limits { resident_bytes: bytes, work_units: units }
}
/// Prospective facts, not reservations. After fixed process-global builtin
/// initialization, these borrowed traversals allocate no prepared job owner.
/// The explicit basis is LinearSrgb; no caller profile or encoding is aliased.
pub fn requirements_with_control(limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Requirements> {
    let b = Budget::new(limits, cancelled)?;
    let planning = planning_work_ceiling();
    let basis = Builtin::LinearSrgb.profile();
    b.poll()?;
    let source = basis.clone_requirements_with_control(limits, cancelled)?;
    b.poll()?;
    let lab = Builtin::LabD50.profile();
    b.poll()?;
    let target = lab.clone_requirements_with_control(limits, cancelled)?;
    let forward = Transform::requirements_with_control(basis, lab, options(), limits, cancelled)?;
    let reverse = Transform::requirements_with_control(lab, basis, options(), limits, cancelled)?;
    let side_forward = basis.side_requirements_with_control(true, Intent::RelativeColorimetric, limits, cancelled)?;
    let side_reverse = basis.side_requirements_with_control(false, Intent::RelativeColorimetric, limits, cancelled)?;
    let (_, _, forward_nodes) = basis.selected_side_cost(true, Intent::RelativeColorimetric, &b)?;
    let (_, _, reverse_nodes) = basis.selected_side_cost(false, Intent::RelativeColorimetric, &b)?;
    // Appending/prepending may hold old and new inventories together. Nested
    // stages move unchanged; each new Vec is guarded at <=2 requested capacity.
    let inventories = forward_nodes
        .checked_add(1)
        .and_then(|n| reverse_nodes.checked_add(1).and_then(|r| n.checked_add(r)))
        .and_then(|n| n.checked_mul(2))
        .ok_or(Error::Limit("owned Lab stages"))?;
    let extra = inventories.checked_mul(std::mem::size_of::<Stage>()).ok_or(Error::Limit("owned Lab stages"))?;
    let mut peak = add(add(source.retained_bytes, source.cloned_model_bytes)?, add(target.retained_bytes, target.cloned_model_bytes)?)?;
    for n in [
        forward.peak_resident_bytes,
        reverse.peak_resident_bytes,
        side_forward.peak_resident_bytes,
        side_reverse.peak_resident_bytes,
        extra,
        std::mem::size_of::<Parts>() + 2 * std::mem::size_of::<Profile>(),
    ] {
        peak = add(peak, n)?;
    }
    let mut units = work(planning, times(source.work_units, 3)?)?;
    units = work(units, times(target.work_units, 3)?)?;
    units = work(units, times(work(forward.work_units, reverse.work_units)?, 3)?)?;
    units = work(units, times(work(side_forward.work_units, side_reverse.work_units)?, 3)?)?;
    // Final actual nested census and bounded inventory moves are separate work.
    // Parts retains its existing composite-work field meaning. Point callers
    // must derive and prepay their own actual direction/stage schedule.
    units = work(units, work(peak as u64, 128)?)?;
    b.admit(peak, units)?;
    Ok(Requirements {
        peak_resident_bytes: peak,
        work_units: units,
        source,
        target,
        forward,
        reverse,
        side_forward,
        side_reverse,
        forward_nodes,
        reverse_nodes,
    })
}
/// Caller must reserve the full requirements peak and prepay its work before
/// construction. All four independently owned pipelines/Transforms survive in
/// Parts; only its physical directions are physical Lab, not normalized Lab.
pub fn construct_with_control(limits: Limits, cancelled: &dyn Fn() -> bool) -> Result<Parts> {
    let r = requirements_with_control(limits, cancelled)?;
    let check = Budget::new(limits, cancelled)?;
    let basis = Builtin::LinearSrgb.profile();
    let source = basis.try_clone_with_control(phase(add(r.source.retained_bytes, r.source.cloned_model_bytes)?, times(r.source.work_units, 3)?), cancelled)?;
    check.poll()?;
    let target = Builtin::LabD50
        .profile()
        .try_clone_with_control(phase(add(r.target.retained_bytes, r.target.cloned_model_bytes)?, times(r.target.work_units, 3)?), cancelled)?;
    let (mut forward, fp) = source.device_to_pcs_with_control(
        Intent::RelativeColorimetric,
        phase(r.side_forward.peak_resident_bytes, times(r.side_forward.work_units, 3)?),
        cancelled,
    )?;
    let (reverse, rp) = source.pcs_to_device_with_control(
        Intent::RelativeColorimetric,
        phase(r.side_reverse.peak_resident_bytes, times(r.side_reverse.work_units, 3)?),
        cancelled,
    )?;
    if fp != Pcs::Xyz || rp != Pcs::Xyz || forward.len() > r.forward_nodes || reverse.len() > r.reverse_nodes {
        return Err(Error::Limit("owned physical Lab side shape"));
    }
    let count = forward.len().checked_add(1).ok_or(Error::Limit("owned Lab append"))?;
    forward.try_reserve_exact(1).map_err(|_| Error::Allocation)?;
    if forward.capacity() > count.checked_mul(2).ok_or(Error::Limit("owned Lab append"))? {
        return Err(Error::Allocation);
    }
    forward.push(Stage::XyzToLab);
    let count = reverse.len().checked_add(1).ok_or(Error::Limit("owned Lab prepend"))?;
    let mut backward = Vec::new();
    backward.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    if backward.capacity() > count.checked_mul(2).ok_or(Error::Limit("owned Lab prepend"))? {
        return Err(Error::Allocation);
    }
    backward.push(Stage::LabToXyz);
    for stage in reverse {
        check.poll()?;
        backward.push(stage);
    }
    let physical_forward = Pipeline::new(3, forward);
    let physical_reverse = Pipeline::new(3, backward);
    let forward =
        Transform::with_options_with_control(&source, &target, options(), phase(r.forward.peak_resident_bytes, times(r.forward.work_units, 3)?), cancelled)?;
    let reverse =
        Transform::with_options_with_control(&target, &source, options(), phase(r.reverse.peak_resident_bytes, times(r.reverse.work_units, 3)?), cancelled)?;
    let mut per_pixel = times(physical_forward.evaluation_work_units_with_control(limits, cancelled)?, 2)?;
    per_pixel = work(per_pixel, physical_reverse.evaluation_work_units_with_control(limits, cancelled)?)?;
    let curves = basis.trc.as_ref().ok_or(Error::Limit("owned Lab curves"))?;
    let mut transfer = 0;
    for curve in curves {
        transfer = work(transfer, curve.controlled_work().max(128))?;
    }
    per_pixel = work(per_pixel, times(transfer, 3)?)?;
    let out = Parts { forward, reverse, physical_forward, physical_reverse, per_pixel_work_units: per_pixel };
    if out.retained_bytes_with_control(limits, cancelled)? > r.peak_resident_bytes {
        return Err(Error::Allocation);
    }
    check.poll()?;
    Ok(out)
}
