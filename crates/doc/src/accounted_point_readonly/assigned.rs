//! Explicit assigned-profile custody; no profile, operation or source proof.
use super::*;
#[path = "assigned_enabled.rs"]
pub(super) mod enabled;
pub use enabled::AssignedPointEnabledPatch;
impl AccountedDocument {
    /// Extra owner/header only. Existing model, both ICC owners and native
    /// allocations must already be paid on this actual ledger by retention.
    pub fn readonly_assigned_point_storage_requirements(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured_assigned(doc, quota, control)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    /// Consume genuinely retained codec/model storage, without cloning payload.
    pub fn new_readonly_assigned_point_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        let required = Self::readonly_assigned_point_storage_requirements(&incoming.document, &quota, control)?;
        let allocation = quota.reserve(QuotaKind::Index, required).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyAssignedPoint,
                document,
                _selection: None,
                _selection_view: None,
                retention,
                quota,
                _model: None,
                _allocation: Arc::new(allocation),
            }),
        })
    }
    pub fn is_readonly_assigned_point_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyAssignedPoint
    }
    pub fn validate_readonly_assigned_point_storage(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_assigned_point_storage() {
            return refused();
        }
        measured_assigned(self.document(), self.quota(), control)?;
        Ok(())
    }
}
#[cfg(test)]
#[path = "assigned_tests.rs"]
mod tests;
