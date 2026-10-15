//! Current-profile preparation for the closed assigned-ICC geometry canvas.
//! Shared construction preserves original/derived numerical definitions and fees.
use photocraft_compose::backed_assigned_current_profile::PreparedAssignedCurrentProfile;
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::ReadControl;

pub(super) struct PaidProfile {
    pub(super) binding: PreparedAssignedCurrentProfile,
}
pub(super) fn prepare(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<PaidProfile, String> {
    owner.validate_assigned_analytic_geometry_storage(control).map_err(|e| e.to_string())?;
    let binding = PreparedAssignedCurrentProfile::prepare(owner, Default::default(), control).map_err(|e| e.to_string())?;
    Ok(PaidProfile { binding })
}

pub(super) fn prepare_isolated_normal_group(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<PaidProfile, String> {
    owner.validate_assigned_isolated_normal_group_storage(control).map_err(|e| e.to_string())?;
    let binding = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(owner, Default::default(), control).map_err(|e| e.to_string())?;
    Ok(PaidProfile { binding })
}
