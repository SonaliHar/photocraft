//! Explicit host callbacks for the finite, read-only owned plain-XCF import.
use photocraft_io::{OwnedPlainXcfImport, OwnedXcfImport};
use photocraft_raster::backing::{ReadControl, quota::BackingQuota};

fn quota(existing: Option<&BackingQuota>, control: &ReadControl<'_>) -> Result<BackingQuota, String> {
    control.check(1).map_err(|e| e.to_string())?;
    match existing {
        Some(quota) => Ok(quota.clone()),
        None => photocraft_io::accounted_native::new_accounted_native_open_quota(control).map_err(|e| e.to_string()),
    }
}
pub(super) fn open(path: &str, existing: Option<&BackingQuota>, control: &ReadControl<'_>) -> Result<OwnedPlainXcfImport, String> {
    crate::crash_guard::guard("Open XCF", || {
        let quota = quota(existing, control)?;
        photocraft_automation::files::open_owned_plain_xcf_selected_path(path, quota, control).map_err(|e| e.to_string())
    })
}
pub(super) fn rooted(
    workspace: &photocraft_automation::AuthorizedWorkspace,
    path: &str,
    existing: Option<&BackingQuota>,
    control: &ReadControl<'_>,
) -> Result<OwnedPlainXcfImport, String> {
    crate::crash_guard::guard("Open XCF", || {
        let quota = quota(existing, control)?;
        workspace.open_owned_plain_xcf_with_read_control(path, quota, control).map_err(|e| e.to_string())
    })
}

pub(super) fn open_current(path: &str, existing: Option<&BackingQuota>, control: &ReadControl<'_>) -> Result<OwnedXcfImport, String> {
    crate::crash_guard::guard("Open XCF", || {
        let quota = quota(existing, control)?;
        photocraft_automation::files::open_owned_current_xcf_selected_path(path, quota, control).map_err(|e| e.to_string())
    })
}
pub(super) fn rooted_current(
    workspace: &photocraft_automation::AuthorizedWorkspace,
    path: &str,
    existing: Option<&BackingQuota>,
    control: &ReadControl<'_>,
) -> Result<OwnedXcfImport, String> {
    crate::crash_guard::guard("Open XCF", || {
        let quota = quota(existing, control)?;
        workspace.open_owned_current_xcf_with_read_control(path, quota, control).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
#[path = "owned_xcf_open_tests.rs"]
mod tests;
