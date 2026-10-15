//! Strict isolated Normal group publication on actual paid current storage.
//! The sealed IO carrier supplies original-source custody; this independently
//! validates current ICC interpretation and every canvas band before publication.
use super::*;
use photocraft_compose::{
    backed_assigned_current_profile::{AssignedCurrentProfileLimits, PreparedAssignedCurrentProfile},
    backed_document::{BackedDocumentSinkError, PreparedBackedDocument},
    backed_normal::BackedNormalError,
};
use photocraft_doc::accounted_document::AccountedDocument;
fn render_error(error: BackedNormalError, control: &ReadControl<'_>) -> EngineError {
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        BackedNormalError::Backing(error)
        | BackedNormalError::View(RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error)) => error.into(),
        BackedNormalError::Quota(_) => BackingError::Quota.into(),
        BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned isolated Normal group current recipe: {other}")),
    }
}
pub(crate) fn validate_current_isolated_normal_group_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    let profile = PreparedAssignedCurrentProfile::prepare_isolated_normal_group(owner, AssignedCurrentProfileLimits::default(), control)
        .map_err(|e| render_error(e, control))?;
    let prepared =
        PreparedBackedDocument::new_accounted_with_assigned_profile(owner, &profile, Default::default(), control).map_err(|e| render_error(e, control))?;
    prepared
        .render_bands(owner.document().bounds(), control, |band| -> std::result::Result<(), BackedNormalError> {
            drop(band);
            Ok(())
        })
        .map_err(|e| match e {
            BackedDocumentSinkError::Render(e) | BackedDocumentSinkError::Sink(e) => render_error(e, control),
        })?;
    drop(prepared);
    drop(profile);
    control.check(1)?;
    Ok(())
}
impl Session {
    /// Explicit keep-current isolated group source handoff. Supplied paths are
    /// exclusion identities, never read grants; default open/color policy stays
    /// with the service. All current validation precedes the common paid tab,
    /// history, view and runtime-source publication.
    pub fn add_owned_isolated_normal_group_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedIsolatedNormalGroupXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        validate_current_isolated_normal_group_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }
}
