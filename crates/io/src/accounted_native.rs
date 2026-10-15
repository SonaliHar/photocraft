//! Optional owned native archive IO. Path authority is supplied explicitly by a
//! trusted platform service, never recovered from document/resource locators.
//! This is not a Session/default loader or native_large route.
mod reader_memory;
pub use reader_memory::open_accounted_native_from_reader;
mod standard_quota;
pub use standard_quota::new_accounted_native_open_quota;

use photocraft_doc::Document;
use photocraft_format::owned_document::{OwnedDocumentArchive, OwnedDocumentLimits};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
use std::path::{Path, PathBuf};
#[derive(Debug, thiserror::Error)]
pub enum AccountedNativeError {
    #[error("native backing: {0}")]
    Backing(#[from] BackingError),
    #[error("native format: {0}")]
    Format(#[from] photocraft_format::FormatError),
    #[error("native IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported native path operation: {0}")]
    Unsupported(&'static str),
}
struct PathOwner {
    path: PathBuf,
    quota: BackingQuota,
    _lease: QuotaLease,
}
impl PathOwner {
    fn new(path: PathBuf, quota: &BackingQuota, ctl: &ReadControl<'_>) -> Result<Self, AccountedNativeError> {
        ctl.check(1)?;
        if path.as_os_str().is_empty() || path.capacity() > 8192 || path.as_os_str().len() > 4096 {
            return Err(BackingError::Bounds.into());
        }
        let bytes = (path.capacity() as u64).checked_add(std::mem::size_of::<Self>() as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Bounds)?;
        let lease = quota.reserve(QuotaKind::Resident, bytes).map_err(|_| BackingError::Quota)?;
        let result = Self { path, quota: quota.clone(), _lease: lease };
        ctl.check(0)?;
        Ok(result)
    }
    fn validate(&self, quota: &BackingQuota) -> Result<(), AccountedNativeError> {
        if self.quota.shares_ledger(quota) { Ok(()) } else { Err(BackingError::Quota.into()) }
    }
}
/// Trusted service-issued read authority only. No clone, serialization or path extraction.
/// Namespace/source consistency during capture remains the granting service's contract.
pub struct ServiceAuthorizedNativeReadPath(PathOwner);
impl ServiceAuthorizedNativeReadPath {
    pub fn from_platform_authorized(path: PathBuf, quota: &BackingQuota, ctl: &ReadControl<'_>) -> Result<Self, AccountedNativeError> {
        Ok(Self(PathOwner::new(path, quota, ctl)?))
    }
}
/// Trusted service-issued destination authority only, independent of read grants.
/// Caller maintains namespace/alias identity and supplies its mandatory commit gate.
/// Authorized namespace resolution must also stay within4096 native path units.
pub struct ServiceAuthorizedNativeWritePath(PathOwner);
impl ServiceAuthorizedNativeWritePath {
    pub fn from_platform_authorized(path: PathBuf, quota: &BackingQuota, ctl: &ReadControl<'_>) -> Result<Self, AccountedNativeError> {
        Ok(Self(PathOwner::new(path, quota, ctl)?))
    }
}
impl std::fmt::Debug for ServiceAuthorizedNativeReadPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceAuthorizedNativeReadPath { .. }")
    }
}
impl std::fmt::Debug for ServiceAuthorizedNativeWritePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceAuthorizedNativeWritePath { .. }")
    }
}
/// Capture an explicitly authorized source into immutable kernel-owned storage.
/// The returned owner retains storage/model leases; it never retains this path or job.
/// Source consistency during capture is the trusted granting service's responsibility.
pub fn open_accounted_native(
    capability: &ServiceAuthorizedNativeReadPath,
    scratch: &crate::xcf_source::ServiceAuthorizedScratchDirectory<'_, '_>,
    limits: OwnedDocumentLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedDocumentArchive, AccountedNativeError> {
    capability.0.validate(&quota)?;
    control.check(0)?;
    #[cfg(any(windows, target_os = "linux"))]
    {
        use std::io::{Read, Seek, SeekFrom};
        let handle_lease = quota.reserve(QuotaKind::Resident, 256).map_err(|_| BackingError::Quota)?;
        let mut file = std::fs::File::open(&capability.0.path)?;
        let length = file.metadata()?.len();
        control.check(1)?;
        let storage = scratch.create_owned_byte_storage(length, &quota, control)?;
        let archive =
            crate::native_plane_store::OwnedCanonicalArchive::capture(storage, length, quota.clone(), Default::default(), control, |offset, output, ctl| {
                let count = u64::try_from(output.len()).map_err(|_| BackingError::Bounds)?;
                if offset.checked_add(count).ok_or(BackingError::Bounds)? > length || output.len() > 65536 {
                    return Err(BackingError::Bounds);
                }
                ctl.check(count)?;
                file.seek(SeekFrom::Start(offset)).map_err(|_| BackingError::Io)?;
                let mut filled = 0usize;
                let mut retries = 0u8;
                while filled < output.len() {
                    ctl.check(0)?;
                    let remaining = output.get_mut(filled..).ok_or(BackingError::Bounds)?;
                    match file.read(remaining) {
                        Ok(0) => return Err(BackingError::Length),
                        Ok(n) => {
                            filled = filled.checked_add(n).ok_or(BackingError::Bounds)?;
                            retries = 0;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted && retries < 8 => {
                            retries += 1;
                        }
                        Err(_) => return Err(BackingError::Io),
                    }
                }
                ctl.check(0)
            })?;
        drop(file);
        drop(handle_lease);
        Ok(photocraft_format::owned_document::load_document_from_owned_archive(std::sync::Arc::new(archive), &Default::default(), limits, quota, control)?)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (scratch, limits);
        Err(AccountedNativeError::Unsupported("owned native storage is unavailable on this platform"))
    }
}

fn refuse_protected_destination(path: &Path, quota: &BackingQuota, ctl: &ReadControl<'_>) -> Result<(), AccountedNativeError> {
    use std::io::Read;
    ctl.check(1)?;
    let name = path.to_str().ok_or(AccountedNativeError::Unsupported("non-Unicode native destination"))?;
    if crate::is_xcf_filename(name) {
        return Err(AccountedNativeError::Unsupported("XCF sources are read-only; choose a new .pcraft destination"));
    }
    let lease = quota.reserve(QuotaKind::Resident, 320).map_err(|_| BackingError::Quota)?;
    let file = std::fs::File::open(path);
    match file {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(mut file) => {
            let mut prefix = [0u8; 64];
            let mut filled = 0usize;
            let mut retries = 0u8;
            while filled < prefix.len() {
                ctl.check(0)?;
                match file.read(prefix.get_mut(filled..).ok_or(BackingError::Bounds)?) {
                    Ok(0) => break,
                    Ok(n) => {
                        filled = filled.checked_add(n).ok_or(BackingError::Bounds)?;
                        retries = 0;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted && retries < 8 => retries += 1,
                    Err(e) => return Err(e.into()),
                }
            }
            ctl.check(filled as u64)?;
            let prefix = prefix.get(..filled).ok_or(BackingError::Bounds)?;
            let protected = prefix.starts_with(b"gimp xcf ") || crate::xcf_source::needs_content_inspection(prefix);
            drop(file);
            drop(lease);
            if protected {
                Err(AccountedNativeError::Unsupported("destination contains XCF or a recognized wrapper; choose a new .pcraft destination"))
            } else {
                Ok(())
            }
        }
    }
}

/// Stream current native state into a sibling temporary file and atomically replace
/// only after the mandatory identity gate succeeds again. The caller supplies
/// independent write authority and protects source aliases/namespace stability.
/// Recognized compressed wrappers are refused without whole-file inspection.
/// Output storage and platform allocator overhead are not a total-RSS claim.
pub fn save_accounted_native_path(
    document: &Document,
    capability: &ServiceAuthorizedNativeWritePath,
    limits: OwnedDocumentLimits,
    quota: &BackingQuota,
    control: &ReadControl<'_>,
    before_write: impl Fn(&Path, &ReadControl<'_>) -> Result<(), AccountedNativeError>,
) -> Result<(), AccountedNativeError> {
    capability.0.validate(quota)?;
    control.check(0)?;
    // Prepay bounded atomic path/handle staging before target resolution creates
    // derived path strings. The atomic helper caps its temporary stem to64chars.
    let _atomic_lease = quota.reserve(QuotaKind::Resident, 65536).map_err(|_| BackingError::Quota)?;
    photocraft_format::atomic::atomic_write_stream_guarded(
        &capability.0.path,
        |file| {
            photocraft_format::owned_document::save_current_document_archive(document, file, limits, quota, control)?;
            Ok(())
        },
        |target| {
            if target.as_os_str().len() > 4096 {
                return Err(BackingError::Bounds.into());
            }
            refuse_protected_destination(&capability.0.path, quota, control)?;
            if target != capability.0.path {
                refuse_protected_destination(target, quota, control)?;
            }
            before_write(target, control)?;
            control.check(0)?;
            Ok(())
        },
    )
}
