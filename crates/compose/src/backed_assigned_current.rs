//! Complete controlled CURRENT validation of the existing assigned geometry lane.
//! This orchestrates existing paid profile and renderer owners; it grants no
//! original-source, geometry-edit or native write authority.
use crate::{
    backed_assigned_current_profile::{AssignedCurrentProfileLimits, PreparedAssignedCurrentProfile},
    backed_document::{BackedDocumentSinkError, PreparedBackedDocument},
    backed_normal::BackedNormalError,
};
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::ReadControl;

/// Prepare the exact original/effective profile relation, qualify and scan the
/// actual current native/mask recipe, then render its complete canvas. Both
/// retained profile roles remain alive until the renderer and all bands drop.
/// Existing limits, epochs, profile rules and equations are unchanged. Callers
/// must independently enforce command authority and publication fences.
pub fn validate_current_assigned_geometry_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<(), BackedNormalError> {
    let profile = PreparedAssignedCurrentProfile::prepare(owner, AssignedCurrentProfileLimits::default(), control)?;
    let prepared = PreparedBackedDocument::new_accounted_with_assigned_profile(owner, &profile, Default::default(), control)?;
    prepared
        .render_bands(owner.document().bounds(), control, |band| -> Result<(), BackedNormalError> {
            drop(band);
            Ok(())
        })
        .map_err(|error| match error {
            BackedDocumentSinkError::Render(error) | BackedDocumentSinkError::Sink(error) => error,
        })?;
    drop(prepared);
    drop(profile);
    control.check(1).map_err(BackedNormalError::Backing)
}

#[cfg(test)]
#[path = "backed_assigned_current_tests.rs"]
mod tests;
