//! Owned plain-XCF input from an actual held read capability; no ambient fallback.
use super::{AuthorizedWorkspace, DENIED, file_error, request_path};
use crate::{
    AutomationError,
    files::owned_xcf_open::{backing, basename, import_error, memory_policy, validate_path},
};
use photocraft_io::{OwnedPlainXcfImport, OwnedXcfImport};
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind},
};
use photocraft_xcf::BudgetKind;

impl AuthorizedWorkspace {
    /// Explicit source input under the already-held read root. Uses the same
    /// finite memory-only policy as the trusted selected-path service, actual
    /// catalog quota, and caller control. Source locators do not grant access.
    /// Each capture/decode phase is separately prepaid; this is not a proof that
    /// every later preparation fits before the first read. Profile decisions and
    /// Session publication belong to the caller; XCF origin remains read-only.
    pub fn open_owned_plain_xcf_with_read_control(
        &self,
        path: &str,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<OwnedPlainXcfImport, AutomationError> {
        self.open_owned_xcf_with(path, quota, control, |name, file, length, options, limits, quota, control| {
            photocraft_io::import_owned_staged_plain_from_reader_in_quota(name, file, length, options, limits, quota, control)
        })
    }

    /// Single original decode selecting the closed plain/Threshold Alpha family
    /// beneath this actual read root. Failure never falls back to an ambient
    /// path or retries another source/render family. No new grant is persisted.
    pub fn open_owned_current_xcf_with_read_control(
        &self,
        path: &str,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<OwnedXcfImport, AutomationError> {
        self.open_owned_xcf_with(path, quota, control, |name, file, length, options, limits, quota, control| {
            photocraft_io::import_owned_xcf_from_reader_in_quota(name, file, length, options, limits, quota, control)
        })
    }

    fn open_owned_xcf_with<T>(
        &self,
        path: &str,
        quota: BackingQuota,
        control: &ReadControl<'_>,
        decode: impl FnOnce(
            &str,
            &mut cap_std::fs::File,
            u64,
            &photocraft_xcf::DecodeOptions<'_>,
            &photocraft_xcf::ResourceLimits,
            BackingQuota,
            &ReadControl<'_>,
        ) -> Result<T, photocraft_io::IoError>,
    ) -> Result<T, AutomationError> {
        validate_path(path, control)?;
        let (options, limits) = memory_policy()?;
        let root = self.read.as_ref().ok_or_else(|| AutomationError::BadRequest(format!("{DENIED}: read authority is absent")))?;
        // Existing bounded request_path component collection and actual handle
        // metadata staging. Kernel/private allocator storage is not an RSS claim.
        let _host = quota.reserve(QuotaKind::Resident, 65536).map_err(|_| backing(BackingError::Quota))?;
        control.check(path.len() as u64 + 1).map_err(backing)?;
        let relative = request_path(path, Some(root))?;
        let name = basename(&relative)?;
        let mut open = cap_std::fs::OpenOptions::new();
        open.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).map_err(|_| backing(BackingError::Bounds))?;
            open.custom_flags(flag);
        }
        control.check(1).map_err(backing)?;
        let mut file = root.dir.open_with(&relative, &open).map_err(|e| file_error("open owned XCF", path, e))?;
        control.check(1).map_err(backing)?;
        let before = file.metadata().map_err(|e| file_error("inspect owned XCF", path, e))?;
        let maximum = limits.get(BudgetKind::Encoded).min(limits.get(BudgetKind::Logical)).min(options.limits.input_bytes as u64);
        if !before.is_file() || before.len() == 0 || before.len() > maximum {
            return Err(AutomationError::BadRequest("owned XCF input must be a regular file within the memory capture limit".into()));
        }
        let modified = before.modified().map_err(|e| file_error("inspect owned XCF modification", path, e))?;
        let imported = decode(name, &mut file, before.len(), &options, &limits, quota, control).map_err(import_error)?;
        control.check(1).map_err(backing)?;
        let after = file.metadata().map_err(|e| file_error("recheck owned XCF", path, e))?;
        let after_modified = after.modified().map_err(|e| file_error("recheck owned XCF modification", path, e))?;
        control.check(1).map_err(backing)?;
        if !after.is_file() || after.len() != before.len() || after_modified != modified {
            return Err(AutomationError::BadRequest("held owned XCF input changed during capture".into()));
        }
        // Held-file length/mtime and exact EOF are observable-change checks,
        // not an atomic external snapshot or a content equality assertion.
        Ok(imported)
    }
}
#[cfg(test)]
#[path = "owned_xcf_open_tests.rs"]
mod tests;
