//! Owned promotion of the exact original Histogram Threshold source family.
//! No renderer, original source grant, edit eligibility or default enrollment
//! follows from the runtime storage class or its retained accounting receipts.
use super::owned_plain_source::existing_quota::PreparationHold;
use super::owned_threshold_source::{StorageClass, StorageParts};
use super::owned_xcf_source::PromotionSource;
use super::*;
use photocraft_doc::accounted_document::AccountedDocument;
use photocraft_raster::backing::{ReadControl, quota::BackingQuota};

pub struct OwnedHistogramThresholdXcfImport {
    inner: StorageParts,
}
impl std::fmt::Debug for OwnedHistogramThresholdXcfImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedHistogramThresholdXcfImport").field("document_id", &self.document().document().id).finish_non_exhaustive()
    }
}
impl OwnedHistogramThresholdXcfImport {
    pub fn document(&self) -> &AccountedDocument {
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
    pub fn into_accounted_document(self) -> AccountedDocument {
        self.inner.into_accounted_document()
    }
}
/// Only actual sealed completion custody can enter. The common transfer retains
/// original encoded/decoder/model/native/mask fees on the exact caller quota.
pub(in crate::xcf_import) fn promote_source<'b, 'a, S: PromotionSource<'b, 'a>>(
    source: S,
    budget: &'b photocraft_xcf::Budget<'a>,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    preparation: &mut Option<PreparationHold>,
) -> Result<OwnedHistogramThresholdXcfImport, IoError> {
    let inner = super::owned_threshold_source::promote_storage(source, budget, quota, control, preparation, StorageClass::Histogram)?;
    Ok(OwnedHistogramThresholdXcfImport { inner })
}

#[cfg(test)]
#[path = "owned_histogram_threshold_source/tests.rs"]
mod tests;
