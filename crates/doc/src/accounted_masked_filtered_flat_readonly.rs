//! Explicit checked-coverage mixed structural custody, without semantic authority.
use super::*;
use crate::xcf_filter_masks::{FilterMaskCoverageClass, MaskCoverageSetLimits};
pub(super) fn measured(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>, source: bool) -> Result<ModelCensus, BackingError> {
    measured_policy(doc,q,c,source,None)
}
pub(super) fn measured_policy(doc:&Document,q:&BackingQuota,c:&ReadControl<'_>,source:bool,selection:Option<&SelectionPermit<'_>>)->Result<ModelCensus,BackingError>{
    doc.checked_f32_filter_mask_coverage_set_requirements(MaskCoverageSetLimits::STANDARD, c)?;
    filtered_flat_readonly::shape_with_purpose_selected(
        doc,
        q,
        c,
        if source { filtered_flat_readonly::TopologyPurpose::Source } else { filtered_flat_readonly::TopologyPurpose::Current }, selection,
    )?;
    let set = doc.checked_f32_filter_mask_coverages_with(MaskCoverageSetLimits::STANDARD, q, c)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut nonempty = false;
    for (ordinal, mask) in state.filter_masks.iter().enumerate() {
        c.check(1)?;
        let coverage = set.classification_for_mask(ordinal, mask)? == FilterMaskCoverageClass::RawNonemptyCoverage;
        nonempty |= coverage;
        for stack in &state.filter_stacks {
            c.check(1)?;
            for node in &stack.nodes {
                c.check(1)?;
                if (source || coverage && node.id == mask.owner) && node.opacity_f64_bits != 1f64.to_bits() {
                    return Err(BackingError::Unsupported);
                }
            }
        }
    }
    if source && !nonempty {
        return Err(BackingError::Unsupported);
    }
    doc.validate_xcf_state_with_checked_filter_masks(&set, c.interrupt)
        .map_err(|_| if c.interrupt.cancelled() { BackingError::Cancelled } else { BackingError::Integrity })?;
    census_with_mask_policy(doc, true, true, selection.and_then(|p|p.guard()), q, c, ReadonlyMaskPolicy::FilteredFlat)
}
impl AccountedDocument {
    pub fn readonly_masked_filtered_flat_point_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, c, true)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn readonly_masked_filtered_flat_point_current_storage_requirements(
        doc: &Document,
        q: &BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<u64, BackingError> {
        measured(doc, q, c, false)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn new_readonly_masked_filtered_flat_point_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::masked_filtered_flat_storage(document, retention, quota, c, true)
    }
    pub fn new_readonly_masked_filtered_flat_point_current_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::masked_filtered_flat_storage(document, retention, quota, c, false)
    }
    fn masked_filtered_flat_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
        source: bool,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        measured(&incoming.document, &quota, c, source)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyMaskedFilteredFlatPoint,
                document,
                _selection: None,
                _selection_view: None,
                retention,
                quota,
                _model: None,
                _allocation: Arc::new(lease),
            }),
        })
    }
    pub fn is_readonly_masked_filtered_flat_point_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyMaskedFilteredFlatPoint
    }
    pub fn validate_readonly_masked_filtered_flat_point_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_masked_filtered_flat_point_storage() {
            return Err(BackingError::Unsupported);
        }
        filtered_flat_readonly::programme_measured(self, c)?;
        Ok(())
    }
}
