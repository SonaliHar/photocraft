//! Explicit desktop native input. A selected runtime path is the authority;
//! no source locator, profile policy or automation root is reconstructed here.
use photocraft_format::owned_document::OwnedDocumentArchive;
use photocraft_raster::backing::{ReadControl, quota::BackingQuota};

pub(super) fn open(path: &str, quota: Option<&BackingQuota>, control: &ReadControl<'_>) -> Result<OwnedDocumentArchive, String> {
    crate::crash_guard::guard("Open", || {
        control.check(1).map_err(|e| e.to_string())?;
        if path.is_empty() || path.len() > 4096 {
            return Err("owned native open requires a bounded selected file path".into());
        }
        control.check(path.len() as u64).map_err(|e| e.to_string())?;
        if path.contains('\0') {
            return Err("owned native open path contains NUL".into());
        }
        let quota = match quota {
            Some(existing) => existing.clone(),
            None => photocraft_io::accounted_native::new_accounted_native_open_quota(control).map_err(|e| e.to_string())?,
        };
        photocraft_automation::files::open_accounted_native_selected_path(path, Default::default(), quota, control).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
#[path = "owned_native_open_tests.rs"]
mod tests;
