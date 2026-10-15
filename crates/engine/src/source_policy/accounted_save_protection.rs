//! Shared paid all-open exclusions. No source-read or write authority is created.
use super::WriteProtection;

/// All-open exclusions paid before copying, retained until the synchronous
/// writer returns. These inert strings do not grant source-file access.
pub struct AccountedSaveProtection {
    pub protection: WriteProtection,
    _lease: photocraft_raster::backing::quota::QuotaLease,
}
impl AccountedSaveProtection {
    pub fn prepare(session: &crate::Session, control: &photocraft_raster::backing::ReadControl<'_>) -> Result<Self, String> {
        use photocraft_raster::backing::{BackingError, quota::QuotaKind};
        let fail = || "accounted-save.source-protection-limit".to_owned();
        let docs = session.documents();
        if docs.len() > 4096 { return Err(fail()); }
        let quota = session.active().and_then(|s| s.doc.quota()).ok_or_else(fail)?;
        let mut paths = 0usize;
        let mut path_bytes = 0usize;
        let mut hashes = 0usize;
        let mut largest = 0usize;
        for state in docs {
            control.check(1).map_err(|e| e.to_string())?;
            if let Some(path) = state.source.as_ref().filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_ref()) {
                if path.len() > 4096 { return Err(fail()); }
                paths = paths.checked_add(1).ok_or_else(fail)?;
                path_bytes = path_bytes.checked_add(path.len()).ok_or_else(fail)?;
            }
            if let Some(state) = &state.doc.xcf_state {
                let count = state.provenance.protected_source_fingerprints.len().checked_add(usize::from(state.provenance.source_hash.is_some())).ok_or_else(fail)?;
                if count > photocraft_io::source_protection::MAX_FINGERPRINTS { return Err(fail()); }
                hashes = hashes.checked_add(count).ok_or_else(fail)?;
                largest = largest.max(count);
            }
        }
        if hashes > photocraft_io::source_protection::MAX_FINGERPRINTS { return Err(fail()); }
        let string = std::mem::size_of::<String>();
        let retained = paths.checked_add(hashes).and_then(|n| n.checked_mul(string)).and_then(|n| n.checked_add(path_bytes))
            .and_then(|n| hashes.checked_mul(128).and_then(|v| n.checked_add(v))).and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() + 256)).ok_or_else(fail)?;
        let temporary = largest.checked_mul(string + 32 + 128).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(256)).ok_or_else(fail)?;
        let envelope = retained.checked_add(temporary).ok_or_else(fail)?;
        let mut lease = quota.reserve(QuotaKind::Index, envelope as u64).map_err(|e| e.to_string())?;
        let mut protection = WriteProtection::default();
        protection.sources.try_reserve_exact(paths).map_err(|_| BackingError::Allocation.to_string())?;
        protection.fingerprints.try_reserve_exact(hashes).map_err(|_| BackingError::Allocation.to_string())?;
        if protection.sources.capacity() > paths.saturating_mul(2) || protection.fingerprints.capacity() > hashes.saturating_mul(2) { return Err(fail()); }
        for state in docs {
            let count = state.doc.xcf_state.as_ref().map_or(0, |s| s.provenance.protected_source_fingerprints.len() + usize::from(s.provenance.source_hash.is_some()));
            // The existing bounded collector performs two literal passes and
            // at most count*(count-1)/2 comparisons, each on one exact digest.
            let work = (count as u64).checked_mul(count as u64 + 1).and_then(|n| n.checked_add((count as u64).checked_mul(256)?)).ok_or_else(fail)?;
            control.check(work).map_err(|e| e.to_string())?;
            let values = photocraft_io::source_protection::document_fingerprints_with_resident_limit(&state.doc, temporary as u64, control.interrupt).map_err(|e| e.to_string())?;
            for value in values { control.check(1).map_err(|e| e.to_string())?; protection.fingerprints.push(value); }
            if let Some(path) = state.source.as_ref().filter(|s| s.origin.is_read_only()).and_then(|s| s.path.as_ref()) {
                let mut owned = String::new();
                owned.try_reserve_exact(path.len()).map_err(|_| BackingError::Allocation.to_string())?;
                if owned.capacity() > path.len().saturating_mul(2) { return Err(fail()); }
                for ch in path.chars() { control.check(ch.len_utf8() as u64).map_err(|e| e.to_string())?; owned.push(ch); }
                protection.sources.push(owned);
            }
        }
        control.check((hashes as u64).checked_mul(128).ok_or_else(fail)?).map_err(|e| e.to_string())?;
        let actual = protection.resident_bytes_with_interrupt(control.interrupt).map_err(|e| e.to_string())?;
        if actual > retained { return Err(fail()); }
        lease.reconcile_capacity(actual as u64).map_err(|e| e.to_string())?;
        control.check(1).map_err(|e| e.to_string())?;
        Ok(Self { protection, _lease: lease })
    }
}


#[cfg(test)]
#[path = "accounted_save_protection_tests.rs"]
mod tests;
