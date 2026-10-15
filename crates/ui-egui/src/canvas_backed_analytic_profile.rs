//! Current-profile preparation for the closed assigned-ICC geometry canvas.
//! The actual parsed allocation dies before its same-ledger reservation.
use photocraft_cms::{Profile, control::Limits};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::{
    ReadControl,
    quota::{QuotaKind, QuotaLease},
};

pub(super) struct PaidProfile {
    pub(super) profile: Profile,
    _lease: QuotaLease,
}
pub(super) fn prepare(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<PaidProfile, String> {
    owner.validate_masked_analytic_normal_storage(control).map_err(|e| e.to_string())?;
    let bytes = owner.document().icc_profile.as_deref().ok_or("analytic canvas current profile missing")?;
    let cancelled = || control.interrupt.cancelled();
    let planning = Profile::parse_planning_work_ceiling(bytes.len()).map_err(|e| e.to_string())?;
    control.check(planning).map_err(|e| e.to_string())?;
    let requirements =
        Profile::parse_requirements_with_control(bytes, Limits { work_units: planning, ..Limits::default() }, &cancelled).map_err(|e| e.to_string())?;
    control.check(128).map_err(|e| e.to_string())?;
    let gate = photocraft_cms::xcf_color::encoded_profile_gate_requirements_with_control(bytes, Limits::default(), &cancelled).map_err(|e| e.to_string())?;
    control.check(gate.work_units).map_err(|e| e.to_string())?;
    photocraft_cms::xcf_color::validate_encoded_profile_with_control(
        bytes,
        Limits { resident_bytes: gate.retained_bytes, work_units: gate.work_units },
        &cancelled,
    )
    .map_err(|e| e.to_string())?;
    let peak = u64::try_from(requirements.peak_resident_bytes).map_err(|_| "analytic canvas profile capacity overflow")?;
    let lease = owner.quota().reserve(QuotaKind::Resident, peak).map_err(|e| e.to_string())?;
    control.check(requirements.work_units.checked_mul(2).ok_or("analytic canvas profile work overflow")?).map_err(|e| e.to_string())?;
    let profile =
        Profile::parse_with_control(bytes, Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units }, &cancelled)
            .map_err(|e| e.to_string())?;
    let paid = PaidProfile { profile, _lease: lease };
    if paid.profile.channels() != owner.document().mode.color_channels() {
        return Err("analytic canvas current profile model mismatch".into());
    }
    control.check(1024).map_err(|e| e.to_string())?;
    photocraft_cms::xcf_color::qualify_effective_analytic_profile(&paid.profile, photocraft_cms::xcf_color::Encoding::NonlinearProfile)
        .map_err(|e| e.to_string())?;
    control.check(0).map_err(|e| e.to_string())?;
    Ok(paid)
}
