//! Sealed ownership handoff for the qualified mixed filtered-flat source family.
//! This carrier preserves genuine codec receipts; it grants no edit or profile authority.
use super::*;

pub struct OwnedMaskedFilteredFlatXcfImport {
    inner: owned_point_source::OwnedPointXcfImport,
}
impl OwnedMaskedFilteredFlatXcfImport {
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
impl std::fmt::Debug for OwnedMaskedFilteredFlatXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedMaskedFilteredFlatXcfImport").field("document_id", &self.document().document().id).finish_non_exhaustive()
    }
}
pub(in crate::xcf_import) fn promote_source<'b, 'a, S: owned_xcf_source::PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: photocraft_raster::backing::quota::BackingQuota,
    control: &photocraft_raster::backing::ReadControl<'_>,
    preparation: &mut Option<owned_plain_source::existing_quota::PreparationHold>,
) -> Result<OwnedMaskedFilteredFlatXcfImport, IoError> {
    Ok(OwnedMaskedFilteredFlatXcfImport { inner: owned_point_source::promote_masked_filtered_flat_source(source, budget, quota, control, preparation)? })
}
