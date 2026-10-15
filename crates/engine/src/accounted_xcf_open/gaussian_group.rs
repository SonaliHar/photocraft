//! Strict isolated small-Auto Gaussian group publication on actual paid current storage.
//! The sealed IO carrier supplies original-source custody; this independently
//! validates current ICC interpretation and every canvas band before publication.
use super::*;
use photocraft_compose::backed_normal::BackedNormalError;
use photocraft_doc::accounted_document::AccountedDocument;
fn render_error(error: BackedNormalError, control: &ReadControl<'_>) -> EngineError {
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        BackedNormalError::Backing(error)
        | BackedNormalError::View(RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error)) => error.into(),
        BackedNormalError::Quota(_) => BackingError::Quota.into(),
        BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned isolated small-Auto Gaussian group current recipe: {other}")),
    }
}
pub(crate) fn validate_current_isolated_gaussian_group_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    owner.validate_gaussian_group_storage(control)?;
    let prepared = photocraft_compose::backed_gaussian_group::PreparedBackedGaussianGroupContext::new(owner, Default::default(), control)
        .map_err(|e| render_error(e, control))?;
    prepared.validate_operative(control).map_err(|e| render_error(e, control))?;
    drop(prepared);
    control.check(1)?;
    Ok(())
}
impl Session {
    /// Explicit keep-current isolated group source handoff. Supplied paths are
    /// exclusion identities, never read grants; default open/color policy stays
    /// with the service. All current validation precedes the common paid tab,
    /// history, view and runtime-source publication.
    pub fn add_owned_isolated_gaussian_group_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedIsolatedGaussianGroupXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        validate_current_isolated_gaussian_group_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }
}
