//! Strict current analytic-geometry publication for a sealed paid XCF owner.
//! Original-source qualification belongs to the typed IO carrier; this stage
//! independently validates the current assigned profile and complete canvas.
use super::*;
use photocraft_compose::backed_normal::BackedNormalError;
use photocraft_doc::accounted_document::AccountedDocument;

fn render_error(error: BackedNormalError, control: &ReadControl<'_>) -> EngineError {
    use photocraft_raster::checked_view::RasterViewError;
    match error {
        BackedNormalError::Backing(e) | BackedNormalError::View(RasterViewError::Backed(e) | RasterViewError::Control(e) | RasterViewError::Descriptor(e)) => {
            e.into()
        }
        BackedNormalError::Quota(_) => BackingError::Quota.into(),
        BackedNormalError::Allocation => BackingError::Allocation.into(),
        _ if control.interrupt.cancelled() => EngineError::Cancelled,
        other => EngineError::Other(format!("owned analytic geometry current recipe: {other}")),
    }
}
/// Current storage/profile/render validation, never an original source grant.
/// The L1 gate retains all original float/integer geometry epochs and raw
/// tuples; the compositor independently binds the exact retained ICC definition.
pub(crate) fn validate_current_analytic_geometry_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<()> {
    photocraft_compose::backed_assigned_current::validate_current_assigned_geometry_storage(owner, control).map_err(|error| render_error(error, control))
}
impl Session {
    /// Explicit keep-current source handoff. Actual assigned ICC/native/mask
    /// validation finishes before the common paid history/view/tab publication.
    /// Source paths remain service-supplied exclusion identities, not read grants.
    pub fn add_owned_analytic_geometry_xcf_import(
        &mut self,
        imported: &photocraft_io::OwnedAnalyticGeometryXcfImport,
        label: &str,
        source_path: Option<&str>,
        control: &ReadControl<'_>,
    ) -> Result<usize> {
        let owner = imported.document();
        validate_current_analytic_geometry_storage(owner, control)?;
        self.add_owned_xcf_model(
            ImportedModel { document: owner, origin: imported.origin(), source_read_only: imported.source_read_only(), preview_only: imported.preview_only() },
            label,
            source_path,
            control,
        )
    }
}
