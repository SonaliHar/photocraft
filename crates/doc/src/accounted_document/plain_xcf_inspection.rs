//! Read-only CURRENT XCF metadata shape over a genuine accounted plain owner.
//! This does not prove original-XCF lineage, source eligibility, rendering,
//! profile interpretation, an editable class or resource/locator authority.
use super::*;
impl AccountedDocument {
    /// Validate coherent paid current metadata only. A generic Legacy owner with
    /// such metadata may pass; no original producer/source claim is established.
    /// Uses the actual owner's ledger and selection guard, performs no clone or
    /// allocation, and debits/polls all existing bounded census/reference work.
    pub fn validate_plain_xcf_inspection(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if self.owner._storage_class != StorageClass::Legacy || self.document().xcf_state.as_ref().is_none_or(|state| state.provenance.format != "xcf") {
            return Err(BackingError::Unsupported);
        }
        self.selection_noop(None, control)?;
        selection_census(self.document(), true, self.owner._selection.as_ref(), self.quota(), control)?;
        control.check(1)?;
        Ok(())
    }
}
