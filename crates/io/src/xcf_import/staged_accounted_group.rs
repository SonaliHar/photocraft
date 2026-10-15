//! Exact existing isolated Normal group source selection; no current-render grant.
use super::*;
use photocraft_xcf::BudgetKind;
fn fail(message: &str) -> IoError {
    IoError::Unsupported(message.into())
}
pub(super) fn pay(b: &photocraft_xcf::Budget<'_>, mut units: u64) -> Result<(), IoError> {
    b.check(BudgetKind::Work, b.used(BudgetKind::Work).checked_add(units).ok_or_else(|| fail("isolated group work size"))?, "isolated group source preflight")
        .map_err(accounting_error)?;
    while units != 0 {
        let n = units.min(4096);
        b.consume_work(n, "isolated group source work").map_err(accounting_error)?;
        units -= n;
    }
    Ok(())
}

pub(super) fn hint<P>(file: &XcfFile<P>) -> bool {
    file.layers.len() == 4 && file.layers.first().is_some_and(|layer| layer.group)
}

// The bounded analytic parser and qualifier remain the old semantic authority.
// Same-Budget temporary receipts and provider work precede their calls.
pub(super) fn profile<P>(file: &XcfFile<P>, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    let Some(budget) = budget else {
        return Ok(qualified_analytic_normal_icc(file));
    };
    if !(132..=65536).contains(&file.icc.len()) {
        return Ok(false);
    }
    let stop = || ctl.cancelled();
    let work = photocraft_cms::Profile::parse_planning_work_ceiling(file.icc.len()).map_err(|_| fail("isolated group profile planning"))?;
    pay(budget, work)?;
    let limits = photocraft_cms::control::Limits { work_units: work, ..Default::default() };
    let req = photocraft_cms::Profile::parse_requirements_with_control(&file.icc, limits, &stop)
        .map_err(|e| if matches!(e, photocraft_cms::control::Error::Cancelled) { IoError::Cancelled } else { fail("isolated group profile requirements") })?;
    pay(budget, 128)?;
    let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(&file.icc, Default::default(), &stop)
        .map_err(|e| if matches!(e, photocraft_cms::control::Error::Cancelled) { IoError::Cancelled } else { fail("isolated group profile gate") })?;
    let bytes = (req.peak_resident_bytes as u64).checked_mul(2).and_then(|n| n.checked_add(32768)).ok_or_else(|| fail("isolated group profile bytes"))?;
    let _profile_fee = budget.reserve(BudgetKind::Resident, bytes, "isolated group profile temporary").map_err(accounting_error)?;
    pay(budget, req.work_units.checked_mul(4).and_then(|n| n.checked_add(gate.work_units)).ok_or_else(|| fail("isolated group profile work"))?)?;
    // Keep an actual controlled parsed definition alive for prospective
    // semantic admission; no source authority comes from this temporary model.
    pay(budget, req.work_units)?;
    let semantic_profile =
        photocraft_cms::Profile::parse_with_control(&file.icc, photocraft_cms::control::Limits { work_units: req.work_units, ..limits }, &stop).map_err(cms)?;
    let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(&semantic_profile).map_err(cms)?;
    // parse_bounded_profile validates once; the original analytic qualifier
    // validates a second time. These are separate from parser work above.
    pay(budget, semantic.checked_mul(2).ok_or_else(|| fail("isolated group source semantic work"))?)?;
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let profile = qualified_analytic_normal_icc(file);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    Ok(profile)
}
fn cms(error: photocraft_cms::control::Error) -> IoError {
    match error {
        photocraft_cms::control::Error::Cancelled => IoError::Cancelled,
        photocraft_cms::control::Error::Limit(_) | photocraft_cms::control::Error::Allocation => {
            accounting_error(XcfError { kind: ErrorKind::Limit, offset: 0, item: String::new(), message: "isolated group controlled profile budget".into() })
        }
        _ => fail("isolated group controlled profile"),
    }
}

#[cfg(test)]
#[path = "staged_accounted_group_tests.rs"]
pub(in crate::xcf_import) mod tests;
