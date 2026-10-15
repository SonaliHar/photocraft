//! Trusted desktop selected-path input. Rooted requests use AuthorizedWorkspace
//! instead; neither source metadata nor this API creates an automation grant.
use crate::AutomationError;
use photocraft_format::owned_document::{OwnedDocumentArchive, OwnedDocumentLimits};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

fn backing(error: BackingError) -> AutomationError {
    if error == BackingError::Cancelled {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::BadRequest(error.to_string())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn selected_fifo_with_no_writer_is_rejected_without_blocking_open() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!("photocraft-selected-fifo-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&directory).unwrap();
        let fifo = directory.join("input.pcraft");
        rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
        let control = ReadControl { interrupt: &photocraft_raster::Interrupt::NONE, work: &|_| Ok(()) };
        let quota = photocraft_io::accounted_native::new_accounted_native_open_quota(&control).unwrap();
        let baseline = quota.usage().unwrap();
        let result = open_accounted_native_selected_path(fifo.to_str().unwrap(), Default::default(), quota.clone(), &control);
        assert!(matches!(result, Err(AutomationError::BadRequest(_))));
        assert_eq!(quota.usage().unwrap(), baseline);
        std::fs::remove_file(fifo).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
fn native(error: photocraft_io::accounted_native::AccountedNativeError) -> AutomationError {
    use photocraft_io::accounted_native::AccountedNativeError as E;
    match error {
        E::Backing(error) => backing(error),
        E::Format(photocraft_format::FormatError::Cancelled) => AutomationError::Engine(photocraft_engine::EngineError::Cancelled),
        E::Format(error) => AutomationError::Format(error),
        other => AutomationError::Io(other.to_string()),
    }
}

/// The host must already authorize this explicitly selected ambient path.
/// Opens it once and holds the actual regular reader through exact EOF capture.
/// This is not a root capability or a normal/default Open/profile-policy route.
pub fn open_accounted_native_selected_path(
    path: &str,
    limits: OwnedDocumentLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedDocumentArchive, AutomationError> {
    control.check(1).map_err(backing)?;
    limits.validate().map_err(AutomationError::Format)?;
    if path.is_empty() || path.len() > 4096 {
        return Err(AutomationError::BadRequest("owned native input requires a bounded selected path".into()));
    }
    control.check(path.len() as u64).map_err(backing)?;
    if path.contains('\0') {
        return Err(AutomationError::BadRequest("owned native input path contains NUL".into()));
    }
    // Borrowed path and actual handle/metadata staging, including bounded OS
    // path conversion. Kernel/private allocator storage is not an RSS claim.
    let bytes = path
        .len()
        .checked_mul(4)
        .and_then(|n| n.checked_add(4096 + std::mem::size_of::<File>() + 2 * std::mem::size_of::<std::fs::Metadata>()))
        .ok_or_else(|| backing(BackingError::Bounds))?;
    let _io = quota.reserve(QuotaKind::Resident, bytes as u64).map_err(|_| backing(BackingError::Quota))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| backing(BackingError::Bounds))?;
        options.custom_flags(flag);
    }
    control.check(1).map_err(backing)?;
    let mut file = options.open(Path::new(path)).map_err(|e| AutomationError::Io(format!("owned native open failed: {e}")))?;
    control.check(1).map_err(backing)?;
    let before = file.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
    let maximum = (photocraft_io::native_plane_store::NATIVE_STORE_MAX_BLOCKS as u64)
        .checked_mul(photocraft_io::native_plane_store::NATIVE_STORE_BLOCK_BYTES as u64)
        .ok_or_else(|| backing(BackingError::Bounds))?;
    if !before.is_file() || before.len() == 0 || before.len() > maximum {
        return Err(AutomationError::BadRequest("owned native input must be a regular file within the native capture limit".into()));
    }
    let modified = before.modified().map_err(|e| AutomationError::Io(e.to_string()))?;
    let archive = photocraft_io::accounted_native::open_accounted_native_from_reader(&mut file, before.len(), limits, quota, control).map_err(native)?;
    control.check(1).map_err(backing)?;
    let after = file.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
    if !after.is_file() || after.len() != before.len() || after.modified().map_err(|e| AutomationError::Io(e.to_string()))? != modified {
        return Err(AutomationError::BadRequest("owned native input changed while opening".into()));
    }
    // Exact EOF plus same-handle length/time checks detect observable changes,
    // not an atomic external snapshot or same-length metadata-preserving writes.
    // NONBLOCK avoids FIFO-open waits on Unix; other filesystem syscalls retain
    // their existing blocking host boundary.
    control.check(1).map_err(backing)?;
    Ok(archive)
}
