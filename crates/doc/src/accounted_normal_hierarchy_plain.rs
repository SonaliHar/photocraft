//! Explicit plain integer Source custody; no float/masked Source role widening.
use super::*;
fn measured_plain(doc: &Document, q: &BackingQuota, c: &dyn ModelControl) -> Result<ModelCensus, BackingError> {
    if !crate::normal_hierarchy_mask_index::plain_quantized_header(doc, &|n| c.check(n))? {
        return refused();
    }
    let index = doc.checked_normal_hierarchy_index_with_reservation(NormalHierarchyLimits::STANDARD, c.sync_control().interrupt, &|n| c.check(n), |n| {
        q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
    })?;
    shape_rows(doc, q, c, TopologyPurpose::Source, index.rows(), true)?;
    census_with_mask_policy(doc, true, true, None, q, c, ReadonlyMaskPolicy::FilteredFlat)
}
impl AccountedDocument {
    pub fn readonly_plain_quantized_normal_hierarchy_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured_plain(doc, q, c)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn new_readonly_plain_quantized_normal_hierarchy_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_plain_quantized_normal_hierarchy(document, retention, quota, c)
    }
    fn new_plain_quantized_normal_hierarchy(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &dyn ModelControl,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        measured_plain(&incoming.document, &quota, c)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyNormalHierarchy,
                document: incoming.document,
                _selection: None,
                _selection_view: None,
                retention: incoming.retention,
                quota,
                _model: None,
                _allocation: Arc::new(lease),
            }),
        })
    }
    /// Original local Source work/cancel/deadline checks; no callback is retained.
    pub fn new_readonly_plain_quantized_normal_hierarchy_storage_with_local_work(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
        work: &dyn Fn(u64) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        struct Local<'a, 'b> {
            base: &'a ReadControl<'b>,
            work: &'a dyn Fn(u64) -> Result<(), BackingError>,
        }
        impl ModelControl for Local<'_, '_> {
            fn check(&self, n: u64) -> Result<(), BackingError> {
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)?;
                (self.work)(n)?;
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)
            }
            fn sync_control(&self) -> &ReadControl<'_> {
                self.base
            }
        }
        Self::new_plain_quantized_normal_hierarchy(document, retention, quota, &Local { base: c, work })
    }
}
