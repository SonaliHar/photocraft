//! Explicit service-authorized selected-path capture for the closed owned plain
//! XCF class. No stored locator, profile decision, or default Open route is used.
use crate::AutomationError;
use photocraft_io::{OwnedPlainXcfImport, OwnedXcfImport};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use photocraft_xcf::{BudgetKind, DecodeOptions, ResourceLimits, ResourcePolicy};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(crate) fn backing(error: BackingError) -> AutomationError {
    if error == BackingError::Cancelled {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::BadRequest(error.to_string())
    }
}
pub(crate) fn import_error(error: photocraft_io::IoError) -> AutomationError {
    if matches!(
        &error,
        photocraft_io::IoError::Cancelled | photocraft_io::IoError::Xcf(photocraft_xcf::XcfError { kind: photocraft_xcf::ErrorKind::Cancelled, .. })
    ) {
        AutomationError::Engine(photocraft_engine::EngineError::Cancelled)
    } else {
        AutomationError::XcfImport(Box::new(error.into()))
    }
}

/// An explicitly finite host subset, not measured capacity or broad XCF support.
/// Capture/path reservations coexist OUTSIDE the preparation envelope. Each IO
/// phase actually admits its own allocations; later private quota refusal is
/// permitted. This recipe does not prove the whole import fits before reading.
pub(crate) fn memory_policy() -> Result<(DecodeOptions<'static>, ResourceLimits), AutomationError> {
    let standard = ResourceLimits::for_policy(ResourcePolicy::default()).map_err(|e| import_error(photocraft_io::IoError::Xcf(e)))?;
    let defaults = DecodeOptions::default();
    let resident = standard.get(BudgetKind::Resident).min(defaults.limits.resident_bytes as u64).min(128 << 20);
    let encoded = standard.get(BudgetKind::Encoded).min(defaults.limits.input_bytes as u64).min(16 << 20);
    let logical = standard.get(BudgetKind::Logical).min(16 << 20);
    let index = standard.get(BudgetKind::Index).min(resident).min(16 << 20);
    let limits = standard
        .lowered(&[("residentBytes", resident), ("encodedBytes", encoded), ("logicalBytes", logical), ("indexBytes", index), ("scratchBytes", 0)])
        .map_err(|e| import_error(photocraft_io::IoError::Xcf(e)))?;
    let mut options = defaults;
    options.limits.input_bytes = usize::try_from(encoded.min(logical)).map_err(|_| backing(BackingError::Bounds))?;
    options.limits.resident_bytes = usize::try_from(resident).map_err(|_| backing(BackingError::Bounds))?;
    Ok((options, limits))
}
pub(crate) fn basename(path: &Path) -> Result<&str, AutomationError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AutomationError::BadRequest("owned XCF input requires a UTF-8 source basename".into()))
}
pub(crate) fn validate_path(path: &str, control: &ReadControl<'_>) -> Result<(), AutomationError> {
    control.check(1).map_err(backing)?;
    if path.is_empty() || path.len() > 4096 {
        return Err(AutomationError::BadRequest("owned XCF input requires a bounded explicit path".into()));
    }
    control.check(path.len() as u64).map_err(backing)?;
    if path.contains('\0') {
        return Err(AutomationError::BadRequest("owned XCF input path contains NUL".into()));
    }
    Ok(())
}

/// The desktop service must already authorize this explicitly selected ambient
/// path. The result preserves read-only XCF origin; it grants neither future
/// reads nor normal rendering/profile admission. Caller supplies the real catalog
/// quota (or a genuine standard initial quota), never a census-derived substitute.
pub fn open_owned_plain_xcf_selected_path(path: &str, quota: BackingQuota, control: &ReadControl<'_>) -> Result<OwnedPlainXcfImport, AutomationError> {
    open_selected(path, quota, control, |name, file, length, options, limits, quota, control| {
        photocraft_io::import_owned_staged_plain_from_reader_in_quota(name, file, length, options, limits, quota, control)
    })
}

/// Explicit original-source selection between the already closed plain and
/// Threshold Alpha owned classes. The original is captured and decoded once by
/// IO; an ineligible or failed selected family is not retried as another class.
/// Current interpretation/application publication remain the caller's decision.
pub fn open_owned_current_xcf_selected_path(path: &str, quota: BackingQuota, control: &ReadControl<'_>) -> Result<OwnedXcfImport, AutomationError> {
    open_selected(path, quota, control, |name, file, length, options, limits, quota, control| {
        photocraft_io::import_owned_xcf_from_reader_in_quota(name, file, length, options, limits, quota, control)
    })
}

fn open_selected<T>(
    path: &str,
    quota: BackingQuota,
    control: &ReadControl<'_>,
    decode: impl FnOnce(&str, &mut File, u64, &DecodeOptions<'_>, &ResourceLimits, BackingQuota, &ReadControl<'_>) -> Result<T, photocraft_io::IoError>,
) -> Result<T, AutomationError> {
    validate_path(path, control)?;
    let (options, limits) = memory_policy()?;
    let name = basename(Path::new(path))?;
    let bytes = path
        .len()
        .checked_mul(4)
        .and_then(|n| n.checked_add(4096 + std::mem::size_of::<File>() + 2 * std::mem::size_of::<std::fs::Metadata>()))
        .ok_or_else(|| backing(BackingError::Bounds))?;
    let _host = quota.reserve(QuotaKind::Resident, bytes as u64).map_err(|_| backing(BackingError::Quota))?;
    let mut open = OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| backing(BackingError::Bounds))?;
        open.custom_flags(flag);
    }
    control.check(1).map_err(backing)?;
    let mut file = open.open(Path::new(path)).map_err(|e| AutomationError::Io(format!("owned XCF open failed: {e}")))?;
    control.check(1).map_err(backing)?;
    let before = file.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
    let maximum = limits.get(BudgetKind::Encoded).min(limits.get(BudgetKind::Logical)).min(options.limits.input_bytes as u64);
    if !before.is_file() || before.len() == 0 || before.len() > maximum {
        return Err(AutomationError::BadRequest("owned XCF input must be a regular file within the memory capture limit".into()));
    }
    let modified = before.modified().map_err(|e| AutomationError::Io(e.to_string()))?;
    let imported = decode(name, &mut file, before.len(), &options, &limits, quota, control).map_err(import_error)?;
    control.check(1).map_err(backing)?;
    let after = file.metadata().map_err(|e| AutomationError::Io(e.to_string()))?;
    if !after.is_file() || after.len() != before.len() || after.modified().map_err(|e| AutomationError::Io(e.to_string()))? != modified {
        return Err(AutomationError::BadRequest("held owned XCF input changed during capture".into()));
    }
    // Exact EOF/held-handle metadata detect observable changes, not an atomic
    // external snapshot or same-length, metadata-preserving concurrent writes.
    control.check(1).map_err(backing)?;
    Ok(imported)
}

#[cfg(test)]
#[path = "owned_xcf_open_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
