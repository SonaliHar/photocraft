//! Exact existing assigned analytic geometry source, NL Union intersection.
use super::*;
fn fail(message: &str) -> IoError {
    IoError::Unsupported(message.into())
}
pub(super) fn pay(b: &photocraft_xcf::Budget<'_>, mut units: u64) -> Result<(), IoError> {
    b.check(BudgetKind::Work, b.used(BudgetKind::Work).checked_add(units).ok_or_else(|| fail("geometry work size"))?, "geometry source preflight")
        .map_err(accounting_error)?;
    while units != 0 {
        let n = units.min(4096);
        b.consume_work(n, "geometry source work").map_err(accounting_error)?;
        units -= n;
    }
    Ok(())
}
pub(super) fn hint<P>(file: &XcfFile<P>) -> bool {
    file.version == 12 && file.layers.len() == 2 && !file.icc.is_empty() && file.layers.iter().all(|d| d.effects.is_empty())
}
pub(super) fn qualify<P: ExactParsedPlane>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    pay(budget, 128)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !hint(file)
        || file.encoding != Encoding::NonlinearProfile
        || file.layers.first().is_none_or(|d| d.composite_mode != 1)
        || !(132..=65536).contains(&file.icc.len())
        || !metadata(file)
    {
        return Ok(false);
    }
    // Exact old profile parser/gate remains the semantic authority. Its controlled
    // allocation-free census admits actual temporary space and all bounded gate/
    // parser work before the legacy bounded provider call, with outer polling.
    let stop = || ctl.cancelled();
    let work = photocraft_cms::Profile::parse_planning_work_ceiling(file.icc.len()).map_err(|_| fail("geometry profile planning"))?;
    pay(budget, work)?;
    let limits = photocraft_cms::control::Limits { work_units: work, ..Default::default() };
    let req = photocraft_cms::Profile::parse_requirements_with_control(&file.icc, limits, &stop)
        .map_err(|e| if matches!(e, photocraft_cms::control::Error::Cancelled) { IoError::Cancelled } else { fail("geometry profile requirements") })?;
    pay(budget, 128)?;
    let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(&file.icc, Default::default(), &stop)
        .map_err(|e| if matches!(e, photocraft_cms::control::Error::Cancelled) { IoError::Cancelled } else { fail("geometry profile gate") })?;
    let bytes = (req.peak_resident_bytes as u64).checked_mul(2).and_then(|n| n.checked_add(32768)).ok_or_else(|| fail("geometry profile bytes"))?;
    let _profile_fee = budget.reserve(BudgetKind::Resident, bytes, "geometry profile temporary").map_err(accounting_error)?;
    pay(budget, req.work_units.checked_mul(4).and_then(|n| n.checked_add(gate.work_units)).ok_or_else(|| fail("geometry profile work"))?)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let profile = qualified_analytic_normal_profile(file);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if !profile {
        return Ok(false);
    }
    for layer in &file.layers {
        if !native(layer, file.sample, ctl, budget)? {
            return Ok(false);
        }
        if let Some(mask) = &layer.mask {
            if !native(mask, file.sample, ctl, budget)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn plane_shape<P: ExactParsedPlane>(d: &Drawable<P>, sample: Sample, channels: usize) -> bool {
    let Some(p) = d.plane.as_ref() else { return false };
    let x = p.descriptor();
    let expected = u64::from(d.width)
        .checked_mul(u64::from(d.height))
        .and_then(|n| n.checked_mul(channels as u64))
        .and_then(|n| n.checked_mul(sample.source_bytes() as u64));
    x.width == d.width && x.height == d.height && x.channels == channels && x.sample == sample && p.native_byte_len() == expected
}
fn native<P: ExactParsedPlane>(d: &Drawable<P>, sample: Sample, ctl: &Interrupt<'_>, budget: &photocraft_xcf::Budget<'_>) -> Result<bool, IoError> {
    let p = d.plane.as_ref().ok_or_else(|| fail("geometry plane missing"))?;
    let desc = p.descriptor();
    let stride = desc.channels.checked_mul(sample.source_bytes()).ok_or_else(|| fail("geometry stride"))?;
    let expected = p.native_byte_len().ok_or_else(|| fail("geometry length"))?;
    let mut seen = 0u64;
    let mut valid = true;
    p.visit_native(ctl, &mut |bytes| {
        if stride == 0 || bytes.is_empty() || bytes.len() > stride * 64 || bytes.len() % stride != 0 {
            return Err(fail("geometry native block"));
        }
        pay(budget, bytes.len() as u64)?;
        seen = seen.checked_add(bytes.len() as u64).filter(|n| *n <= expected).ok_or_else(|| fail("geometry native count"))?;
        if matches!(sample, Sample::F32 | Sample::F64) {
            for raw in bytes.chunks_exact(sample.source_bytes()) {
                let v = if sample == Sample::F64 {
                    f64::from_ne_bytes(raw.try_into().map_err(|_| fail("geometry F64 word"))?)
                } else {
                    f64::from(f32::from_ne_bytes(raw.try_into().map_err(|_| fail("geometry F32 word"))?))
                };
                valid &= v.is_finite() && (0.0..=1.0).contains(&v);
            }
        }
        Ok(())
    })?;
    if seen != expected {
        return Err(fail("geometry incomplete native words"));
    }
    Ok(valid)
}
fn metadata<P: ExactParsedPlane>(file: &XcfFile<P>) -> bool {
    fn bounded(width: u32, height: u32) -> bool {
        width > 0 && height > 0 && width.checked_mul(height).is_some_and(|pixels| pixels <= 4096)
    }
    fn plain<P>(d: &Drawable<P>) -> bool {
        !d.group
            && d.path.len() <= 1
            && d.effects.is_empty()
            && d.text.is_none()
            && d.vector.is_none()
            && d.link.is_none()
            && d.transform.is_none()
            && d.visible
            && d.floating_attachment.is_none()
    }
    file.version == 12
        && matches!(
            (file.precision, file.sample, file.encoding),
            (250, Sample::U16, Encoding::NonlinearProfile)
                | (350, Sample::U32, Encoding::NonlinearProfile)
                | (650, Sample::F32, Encoding::NonlinearProfile)
                | (750, Sample::F64, Encoding::NonlinearProfile)
                | (600, Sample::F32, Encoding::Linear)
                | (700, Sample::F64, Encoding::Linear)
                | (675, Sample::F32, Encoding::Perceptual)
                | (775, Sample::F64, Encoding::Perceptual)
        )
        && matches!(file.model, Model::Rgb | Model::Gray)
        && bounded(file.width, file.height)
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 2
        && file.layers.iter().all(|d| {
            plain(d)
                && d.model == file.model
                && d.alpha
                && d.mode == 28
                && d.blend_space == 0
                && bounded(d.width, d.height)
                && bounds(d).is_ok()
                && plane_shape(d, file.sample, if file.model == Model::Gray { 2 } else { 4 })
        })
        && file.layers.first().is_some_and(|d| {
            d.composite_space == 1
                && matches!(d.composite_mode, 1..=4)
                && d.opacity.is_finite()
                && (0.0..=1.0).contains(&d.opacity)
                && file
                    .width
                    .checked_mul(file.height)
                    .and_then(|canvas| d.width.checked_mul(d.height).and_then(|owner| owner.checked_mul(2)).and_then(|owners| canvas.checked_add(owners)))
                    .is_some_and(|pixels| pixels <= 8192)
                && d.mask_enabled
                && !d.mask_shown
                && !d.mask_editing
                && d.mask.as_ref().is_some_and(|mask| {
                    plain(mask)
                        && (mask.width, mask.height) == (d.width, d.height)
                        && mask.offset == (0, 0)
                        && mask.model == Model::Gray
                        && !mask.alpha
                        && plane_shape(mask, file.sample, 1)
                })
        })
        && file.layers.get(1).is_some_and(|d| {
            (d.width, d.height) == (file.width, file.height)
                && d.offset == (0, 0)
                && d.opacity.to_bits() == 1.0_f32.to_bits()
                && d.mask.is_none()
                && (d.composite_space, d.composite_mode) == (-1, -1)
        })
}

fn cms(error: photocraft_cms::control::Error) -> IoError {
    match error {
        photocraft_cms::control::Error::Cancelled => IoError::Cancelled,
        photocraft_cms::control::Error::Limit(_) | photocraft_cms::control::Error::Allocation => {
            accounting_error(XcfError { kind: ErrorKind::Limit, offset: 0, item: String::new(), message: "geometry controlled profile budget".into() })
        }
        _ => fail("geometry controlled profile"),
    }
}
/// Separate actual mapper reconstruction, paid again; source admission conveys
/// no free later parser/effective-profile work or current-render authority.
pub(super) fn effective<P>(
    file: &XcfFile<P>,
    space: photocraft_cms::ColorSpace,
    ctl: &Interrupt<'_>,
    budget: &photocraft_xcf::Budget<'_>,
) -> Result<photocraft_cms::Profile, IoError> {
    let stop = || ctl.cancelled();
    let planning = photocraft_cms::Profile::parse_planning_work_ceiling(file.icc.len()).map_err(|_| fail("geometry parse planning"))?;
    pay(budget, planning)?;
    let parse_limits = photocraft_cms::control::Limits { work_units: planning, ..Default::default() };
    let req = photocraft_cms::Profile::parse_requirements_with_control(&file.icc, parse_limits, &stop).map_err(cms)?;
    pay(budget, 128)?;
    let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(&file.icc, Default::default(), &stop).map_err(cms)?;
    let bytes = (req.peak_resident_bytes as u64).checked_mul(2).and_then(|n| n.checked_add(32768)).ok_or_else(|| fail("geometry effective profile peak"))?;
    let _fee = budget.reserve(BudgetKind::Resident, bytes, "geometry effective profile temporary").map_err(accounting_error)?;
    pay(budget, req.work_units.checked_add(gate.work_units).ok_or_else(|| fail("geometry effective parse work"))?)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let profile = photocraft_cms::xcf_color::parse_bounded_profile(&file.icc, space).map_err(|_| fail("geometry assigned profile"))?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    // The exact old bounded parser remains the source interpretation boundary.
    // The controlled constructor retains the same NL cached bytes/curve words.
    let defaults = photocraft_cms::control::Limits::default();
    let limits = photocraft_cms::control::Limits {
        work_units: (req.peak_resident_bytes as u64)
            .checked_mul(64)
            .and_then(|n| n.checked_add(65536))
            .ok_or_else(|| fail("geometry effective planner ceiling"))?
            .min(defaults.work_units),
        ..defaults
    };
    pay(budget, limits.work_units)?;
    let effective =
        photocraft_cms::xcf_color::effective_profile_requirements_with_control(&profile, photocraft_cms::xcf_color::Encoding::NonlinearProfile, limits, &stop)
            .map_err(cms)?;
    let _effective_fee =
        budget.reserve(BudgetKind::Resident, effective.peak_resident_bytes as u64, "geometry effective construction").map_err(accounting_error)?;
    pay(budget, effective.work_units)?;
    photocraft_cms::xcf_color::effective_profile_from_definition_with_control(
        &profile,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        photocraft_cms::control::Limits { work_units: effective.work_units, ..limits },
        &stop,
    )
    .map_err(cms)
}

#[cfg(test)]
#[path = "staged_analytic_geometry_tests.rs"]
mod tests;
