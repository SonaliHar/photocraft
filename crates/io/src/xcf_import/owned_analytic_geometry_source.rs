//! Original assigned-ICC four-operator geometry, separately source-selected.
//! This wrapper preserves shared charged custody; it grants no default policy.
use super::*;
pub struct OwnedAnalyticGeometryXcfImport {
    inner: owned_plain_source::OwnedPlainXcfImport,
}
impl OwnedAnalyticGeometryXcfImport {
    pub fn document(&self) -> &photocraft_doc::accounted_document::AccountedDocument {
        self.inner.document()
    }
    pub fn warnings(&self) -> &[String] {
        self.inner.warnings()
    }
    pub fn origin(&self) -> ImportOrigin {
        self.inner.origin()
    }
    pub fn assessment(&self) -> Option<&crate::xcf_assessment::ImportAssessment> {
        self.inner.assessment()
    }
    pub fn source_read_only(&self) -> bool {
        self.inner.source_read_only()
    }
    pub fn preview_only(&self) -> bool {
        self.inner.preview_only()
    }
    pub fn into_accounted_document(self) -> photocraft_doc::accounted_document::AccountedDocument {
        self.inner.into_accounted_document()
    }
}
impl std::fmt::Debug for OwnedAnalyticGeometryXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedAnalyticGeometryXcfImport").field("document_id", &self.document().document().id).finish_non_exhaustive()
    }
}
pub(in crate::xcf_import) fn promote_source<'b, 'a, S: owned_xcf_source::PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    limits: &photocraft_xcf::ResourceLimits,
    aggregate: u64,
    context: &owned_plain_source::existing_quota::Context<'_, '_>,
    preparation: &mut Option<owned_plain_source::existing_quota::PreparationHold>,
    control: &Interrupt<'_>,
) -> Result<OwnedAnalyticGeometryXcfImport, IoError> {
    let inner = owned_plain_source::promote_analytic_geometry_source(source, budget, limits, aggregate, Some(context), preparation, control)?;
    Ok(OwnedAnalyticGeometryXcfImport { inner })
}
