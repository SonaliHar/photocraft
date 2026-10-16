//! Accounted native serialization beneath the existing held write root.
use super::{AuthorizedWorkspace, source_write};
use crate::AutomationError;
use photocraft_engine::{DocumentSnapshot, WriteProtection};
use photocraft_raster::backing::{ReadControl, quota::QuotaKind};

impl AuthorizedWorkspace {
    /// Save an actual accounted snapshot using this workspace's existing root authority.
    ///
    /// Source exclusions are borrowed runtime identities, never new read grants.
    /// The same quota holds protection metadata, bounded path bookkeeping and fixed
    /// inspection scratch through every initial/final destination gate. Inspection
    /// retains its existing finite local reader/hash work budget and Interrupt;
    /// blocking filesystem calls are inherited boundaries, not shared-work claims.
    /// Wrappers have no collection headroom and are refused. The successful rename
    /// is the commit point: no subsequent fallible work or cancellation poll occurs.
    pub fn save_accounted_native_with_read_control(
        &self,
        snapshot: &DocumentSnapshot,
        path: &str,
        protection: &WriteProtection,
        limits: photocraft_format::owned_document::OwnedDocumentLimits,
        control: &ReadControl<'_>,
    ) -> Result<(), AutomationError> {
        let cancelled = |error: photocraft_raster::backing::BackingError| {
            if matches!(error, photocraft_raster::backing::BackingError::Cancelled) {
                AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
            } else {
                AutomationError::BadRequest(error.to_string())
            }
        };
        control.check(1).map_err(cancelled)?;
        if path.is_empty() || path.len() > 4096 {
            return Err(AutomationError::BadRequest("accounted native destination path exceeds limit".into()));
        }
        let owner = snapshot.accounted().ok_or_else(|| AutomationError::BadRequest("accounted native save requires the actual owned snapshot".into()))?;
        let quota = owner.quota();
        let held = protection.resident_bytes_with_interrupt(control.interrupt).map_err(AutomationError::Engine)?;
        let _protection = quota.reserve(QuotaKind::Resident, held as u64).map_err(|e| AutomationError::BadRequest(e.to_string()))?;
        // Covers derived relative/parent/leaf/temp names and native handle staging.
        // This is a conservative known bookkeeping envelope, not an RSS census.
        let _paths = quota.reserve(QuotaKind::Resident, 65536).map_err(|e| AutomationError::BadRequest(e.to_string()))?;
        let fixed = photocraft_io::source_protection::SOURCE_PROTECTION_FIXED_BYTES;
        let _inspection = quota.reserve(QuotaKind::Resident, fixed).map_err(|e| AutomationError::BadRequest(e.to_string()))?;
        control.check(1).map_err(cancelled)?;
        source_write::write_stream_with_resident_limit(
            self,
            path,
            source_write::Sources::many(&protection.sources),
            &protection.fingerprints,
            control.interrupt,
            fixed,
            |file| {
                photocraft_io::accounted_current_native::save_accounted_current_archive(owner, file, limits, quota, control).map_err(|error| match error {
                    photocraft_format::FormatError::Cancelled => AutomationError::Engine(photocraft_engine::EngineError::Cancelled),
                    other => AutomationError::BadRequest(other.to_string()),
                })?;
                Ok(())
            },
        )
    }
}

#[cfg(test)]
#[path = "accounted_native_save_tests.rs"]
mod tests;
