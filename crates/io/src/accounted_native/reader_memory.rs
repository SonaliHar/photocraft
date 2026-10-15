//! Explicit held-reader capture into a genuinely owned immutable memory archive.
//! The reader is supplied by an authorized host; this module opens no paths and
//! infers no permission, profile interpretation, or source identity from bytes.
use super::AccountedNativeError;
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentArchive, OwnedDocumentLimits, load_document_from_owned_archive},
};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::{io::Read, sync::Arc};

const CHUNK: usize = 65_536;
// The existing owned native storage profile: 65,536 blocks of 4,096 bytes.
const MAX_CAPTURE_BYTES: u64 = crate::native_plane_store::NATIVE_STORE_MAX_BLOCKS as u64 * crate::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64;
const CAPACITY_SLACK: usize = 64;
const OWNER_SLACK: usize = 128;
const INTERRUPTED_RETRIES: u8 = 8;

/// No public constructor, extraction, mutation or borrowed job/source handles.
/// Physical bytes are destroyed before their same-ledger reservation. The header
/// envelope includes the Arc counters/allocator slack; it is not an RSS claim.
#[derive(Debug)]
struct MemoryArchive {
    bytes: Vec<u8>,
    quota: BackingQuota,
    _lease: QuotaLease,
}

impl MemoryArchive {
    fn capture(reader: &mut (impl Read + ?Sized), exact_length: u64, quota: BackingQuota, control: &ReadControl<'_>) -> Result<Self, AccountedNativeError> {
        control.check(1)?;
        if exact_length == 0 || exact_length > MAX_CAPTURE_BYTES {
            return Err(BackingError::Bounds.into());
        }
        let length = usize::try_from(exact_length).map_err(|_| BackingError::Bounds)?;
        let capacity_limit = length.checked_add(CAPACITY_SLACK).ok_or(BackingError::Bounds)?;
        let header = std::mem::size_of::<Self>().checked_add(OWNER_SLACK).ok_or(BackingError::Bounds)?;
        let envelope = capacity_limit.checked_add(header).ok_or(BackingError::Bounds)?;
        let lease = quota.reserve(QuotaKind::Resident, u64::try_from(envelope).map_err(|_| BackingError::Bounds)?).map_err(|_| BackingError::Quota)?;
        let mut candidate = Self { bytes: Vec::new(), quota, _lease: lease };
        control.check(1)?;
        candidate.bytes.try_reserve_exact(length).map_err(|_| BackingError::Allocation)?;
        if candidate.bytes.capacity() > capacity_limit {
            return Err(BackingError::Allocation.into());
        }
        let actual = candidate.bytes.capacity().checked_add(header).ok_or(BackingError::Bounds)?;
        candidate._lease.reconcile_capacity(u64::try_from(actual).map_err(|_| BackingError::Bounds)?).map_err(|_| BackingError::Quota)?;
        control.check(0)?;
        while candidate.bytes.len() < length {
            let begin = candidate.bytes.len();
            let end = begin.checked_add(CHUNK.min(length - begin)).ok_or(BackingError::Bounds)?;
            let count = end.checked_sub(begin).ok_or(BackingError::Bounds)?;
            // Prepay initialization and reader-written words before either
            // touches storage. Each partial/Interrupted call also pays a unit.
            control.check(u64::try_from(count).map_err(|_| BackingError::Bounds)?.checked_mul(2).ok_or(BackingError::Bounds)?)?;
            candidate.bytes.resize(end, 0);
            let target = candidate.bytes.get_mut(begin..end).ok_or(BackingError::Bounds)?;
            read_exact_chunk(reader, target, control)?;
            control.check(0)?;
        }
        // The caller's length is an admission hint, not a source-consistency
        // proof. Exact EOF rejects a short hint/growth visible at this boundary.
        let mut extra = [0u8; 1];
        let mut interrupted = 0u8;
        loop {
            control.check(1)?;
            match reader.read(&mut extra) {
                Ok(0) => break,
                Ok(1) => return Err(BackingError::Length.into()),
                Ok(_) => return Err(BackingError::Bounds.into()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted && interrupted < INTERRUPTED_RETRIES => interrupted += 1,
                Err(error) => return Err(error.into()),
            }
        }
        control.check(0)?;
        Ok(candidate)
    }
}

fn read_exact_chunk(reader: &mut (impl Read + ?Sized), output: &mut [u8], control: &ReadControl<'_>) -> Result<(), AccountedNativeError> {
    if output.len() > CHUNK {
        return Err(BackingError::Bounds.into());
    }
    let mut filled = 0usize;
    let mut interrupted = 0u8;
    while filled < output.len() {
        control.check(1)?;
        let remaining = output.get_mut(filled..).ok_or(BackingError::Bounds)?;
        match reader.read(remaining) {
            Ok(0) => return Err(BackingError::Length.into()),
            Ok(count) if count <= remaining.len() => {
                filled = filled.checked_add(count).ok_or(BackingError::Bounds)?;
                interrupted = 0;
            }
            Ok(_) => return Err(BackingError::Bounds.into()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted && interrupted < INTERRUPTED_RETRIES => interrupted += 1,
            Err(error) => return Err(error.into()),
        }
        control.check(0)?;
    }
    control.check(0)?;
    Ok(())
}

impl ImmutableCanonicalArchive for MemoryArchive {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }

    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if output.len() > CHUNK {
            return Err(BackingError::Bounds);
        }
        let begin = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = begin.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        let input = self.bytes.get(begin..end).ok_or(BackingError::Bounds)?;
        control.check(u64::try_from(output.len()).map_err(|_| BackingError::Bounds)?)?;
        output.copy_from_slice(input);
        // The decoder owns this staging and discards it on cancellation/error.
        control.check(0)
    }
}

/// Capture exactly `exact_length` bytes starting at the reader's current
/// position, then require EOF. The host supplies and closes its actual authorized
/// reader; no path is opened or retained here. Concurrent same-length source
/// mutation is not ruled out by a length/metadata check: this owns the captured
/// bytes, not a claim about an external file's prior state.
///
/// Encoded capacity/header is reserved before allocation and remains charged
/// through the archive's last owner. Existing manifest/object/model/native-plane
/// admissions use this same ledger. The standard 256 MiB encoded ceiling is not
/// a native_large route or a promise that every file below it fits the quota.
/// Profile policy belongs to the caller; the existing format loader is unchanged.
pub fn open_accounted_native_from_reader(
    reader: &mut (impl Read + ?Sized),
    exact_length: u64,
    limits: OwnedDocumentLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedDocumentArchive, AccountedNativeError> {
    control.check(0)?;
    limits.validate()?;
    let archive = MemoryArchive::capture(reader, exact_length, quota.clone(), control)?;
    control.check(0)?;
    let archive: Arc<dyn ImmutableCanonicalArchive> = Arc::new(archive);
    let candidate = load_document_from_owned_archive(archive, &LoadOptions::default(), limits, quota, control)?;
    control.check(0)?;
    Ok(candidate)
}

#[cfg(test)]
#[path = "reader_memory_tests.rs"]
mod tests;
