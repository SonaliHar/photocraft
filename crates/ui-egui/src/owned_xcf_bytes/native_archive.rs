//! Portable paid browser capture. Labels confer no filesystem authority.
use super::Capture;
use photocraft_format::{
    LoadOptions,
    canonical_archive::ImmutableCanonicalArchive,
    owned_document::{OwnedDocumentArchive, OwnedDocumentLimits, load_document_from_owned_archive},
};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::sync::Arc;

const MAX_READ: usize = 65_536;

// Payload/provider storage drops before its extra Arc/carrier header charge.
// No callback or borrowed control is retained.
struct NativeCaptureArchive {
    capture: Capture,
    _header: QuotaLease,
}
impl std::fmt::Debug for NativeCaptureArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeCaptureArchive").field("length", &self.capture.bytes.len()).finish_non_exhaustive()
    }
}
impl ImmutableCanonicalArchive for NativeCaptureArchive {
    fn len(&self) -> u64 {
        self.capture.bytes.len() as u64
    }
    fn validate_quota(&self, quota: &BackingQuota) -> Result<(), BackingError> {
        if self.capture.request.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn read_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if output.len() > MAX_READ {
            return Err(BackingError::Bounds);
        }
        let begin = usize::try_from(offset).map_err(|_| BackingError::Bounds)?;
        let end = begin.checked_add(output.len()).ok_or(BackingError::Bounds)?;
        let input = self.capture.bytes.get(begin..end).ok_or(BackingError::Bounds)?;
        // All fallible checks precede this bounded copy, so failed reads do not
        // alter output. Future reads never use the old picker cancellation flag.
        control.check(u64::try_from(output.len()).map_err(|_| BackingError::Bounds)?)?;
        output.copy_from_slice(input);
        Ok(())
    }
}

/// Paid label guard shares the archive's actual payload/provider/header owner.
pub(super) struct NativeCaptureLabel {
    source: Arc<NativeCaptureArchive>,
}
impl NativeCaptureLabel {
    pub(super) fn name(&self) -> &str {
        &self.source.capture.name
    }
}
pub(super) struct NativeArchiveInput {
    archive: OwnedDocumentArchive,
    source: Arc<NativeCaptureArchive>,
}
impl NativeArchiveInput {
    pub(super) fn into_parts(self) -> (NativeCaptureLabel, OwnedDocumentArchive) {
        let Self { archive, source } = self;
        (NativeCaptureLabel { source }, archive)
    }
}

fn source(capture: Capture, control: &ReadControl<'_>) -> Result<Arc<NativeCaptureArchive>, String> {
    control.check(1).map_err(|e| e.to_string())?;
    // Existing browser capture ceiling; no native-large or whole-import-fit claim.
    if capture.bytes.is_empty() || capture.bytes.len() > super::CAP {
        return Err("owned native byte capture exceeds input limits".into());
    }
    let header = std::mem::size_of::<NativeCaptureArchive>()
        .checked_add(std::mem::size_of::<NativeArchiveInput>())
        .and_then(|n| n.checked_add(std::mem::size_of::<NativeCaptureLabel>()))
        .and_then(|n| n.checked_add(128))
        .ok_or("native capture header overflow")?;
    let lease =
        capture.request.quota.reserve(QuotaKind::Index, u64::try_from(header).map_err(|_| "native capture header overflow")?).map_err(|e| e.to_string())?;
    // Arc construction has the existing bounded allocator limitation. Capture
    // already owns all payload capacities; this admission precedes the new Arc.
    control.check(1).map_err(|e| e.to_string())?;
    Ok(Arc::new(NativeCaptureArchive { capture, _header: lease }))
}

/// Consume a genuine paid capture without copying bytes or opening any path.
/// The immutable archive retains full provider/payload charges through all model
/// and native-plane owners. Every read uses its future caller's ReadControl.
pub(super) fn load(capture: Capture, control: &ReadControl<'_>) -> Result<NativeArchiveInput, String> {
    let quota = capture.request.quota.clone();
    let source = source(capture, control)?;
    let archive: Arc<dyn ImmutableCanonicalArchive> = source.clone();
    let archive =
        load_document_from_owned_archive(archive, &LoadOptions::default(), OwnedDocumentLimits::default(), quota, control).map_err(|e| e.to_string())?;
    control.check(0).map_err(|e| e.to_string())?;
    Ok(NativeArchiveInput { archive, source })
}

#[cfg(test)]
#[path = "native_archive_tests.rs"]
mod tests;
