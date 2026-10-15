//! Explicit owned native capture from one file beneath an already-held read root.
use super::{AuthorizedWorkspace, DENIED, file_error, request_path};
use crate::AutomationError;
use photocraft_format::owned_document::{OwnedDocumentArchive, OwnedDocumentLimits};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};

fn backing_error(error: BackingError) -> AutomationError {
    if error == BackingError::Cancelled {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::BadRequest(error.to_string())
    }
}
fn native_error(error: photocraft_io::accounted_native::AccountedNativeError) -> AutomationError {
    use photocraft_io::accounted_native::AccountedNativeError;
    match error {
        AccountedNativeError::Backing(error) => backing_error(error),
        AccountedNativeError::Format(photocraft_format::FormatError::Cancelled) => AutomationError::Engine(photocraft_engine::EngineError::Cancelled),
        AccountedNativeError::Format(error) => AutomationError::Format(error),
        other => AutomationError::BadRequest(other.to_string()),
    }
}
impl AuthorizedWorkspace {
    /// Explicitly read a regular native archive under this held read capability.
    ///
    /// `source_limit` may only lower the existing native memory-capture ceiling.
    /// The real caller quota pays bounded path/handle staging before derivation,
    /// then IO admits encoded capacity/model before allocation on the same ledger.
    /// No ambient path is reconstructed and no source locator grants authority.
    /// The reader's exact EOF and held-file length/mtime checks detect visible
    /// mutation; they do not establish an atomic external snapshot or rule out
    /// concurrent same-length/metadata-preserving writes. Blocking OS calls remain
    /// platform boundaries. Returned ownership retains bytes, not read authority.
    /// Profile preservation/decisions and Session publication belong to the caller.
    pub fn open_accounted_native_with_read_control(
        &self,
        path: &str,
        source_limit: u64,
        limits: OwnedDocumentLimits,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<OwnedDocumentArchive, AutomationError> {
        control.check(1).map_err(backing_error)?;
        let maximum = u64::from(photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS)
            .checked_mul(photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64)
            .ok_or(BackingError::Bounds)
            .map_err(backing_error)?;
        if path.is_empty() || path.len() > 4096 || source_limit == 0 || source_limit > maximum {
            return Err(AutomationError::BadRequest("owned native read path or source limit exceeds bounds".into()));
        }
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        // Known relative-path/native-handle staging, including request_path's
        // bounded component collection. No private allocator/RSS assertion.
        let _paths = quota.reserve(QuotaKind::Resident, 65536).map_err(|_| backing_error(BackingError::Quota))?;
        control.check(path.len() as u64 + 1).map_err(backing_error)?;
        let relative = request_path(path, Some(root))?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| backing_error(BackingError::Bounds))?;
            options.custom_flags(flag);
        }
        control.check(1).map_err(backing_error)?;
        let mut file = root.dir.open_with(&relative, &options).map_err(|error| file_error("open owned native", path, error))?;
        control.check(1).map_err(backing_error)?;
        let initial = file.metadata().map_err(|error| file_error("inspect owned native", path, error))?;
        if !initial.is_file() || initial.len() == 0 || initial.len() > source_limit {
            return Err(AutomationError::BadRequest("owned native source must be a regular file within the explicit source limit".into()));
        }
        let modified = initial.modified().map_err(|error| file_error("inspect owned native modification", path, error))?;
        control.check(1).map_err(backing_error)?;
        let archive =
            photocraft_io::accounted_native::open_accounted_native_from_reader(&mut file, initial.len(), limits, quota, control).map_err(native_error)?;
        control.check(1).map_err(backing_error)?;
        let final_metadata = file.metadata().map_err(|error| file_error("recheck owned native", path, error))?;
        let final_modified = final_metadata.modified().map_err(|error| file_error("recheck owned native modification", path, error))?;
        control.check(1).map_err(backing_error)?;
        if !final_metadata.is_file() || final_metadata.len() != initial.len() || final_modified != modified {
            return Err(AutomationError::BadRequest("held owned native source changed during capture".into()));
        }
        Ok(archive)
    }
}
#[cfg(test)]
#[path = "accounted_native_open_tests.rs"]
mod tests;
