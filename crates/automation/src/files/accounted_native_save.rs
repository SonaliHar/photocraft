//! Trusted selected-path accounted native output. Rooted automation must use
//! AuthorizedWorkspace instead; source exclusions never create destination authority.
//! Protected-reader work remains its existing bounded local Interrupt boundary.
use std::path::Path;

/// Serialize the actual owned snapshot under its quota and real supplied control.
/// Initial/selected/resolved/atomic-retry source guards are unchanged; successful
/// rename has no subsequent fallible bookkeeping or cancellation poll.
pub fn save_accounted_native_selected_path(
    snapshot: &photocraft_engine::DocumentSnapshot,
    path: &str,
    protection: &photocraft_engine::WriteProtection,
    control: &photocraft_raster::backing::ReadControl<'_>,
) -> Result<(), String> {
    use photocraft_io::accounted_native::{AccountedNativeError, ServiceAuthorizedNativeWritePath};
    use photocraft_raster::backing::{BackingError, quota::QuotaKind};
    control.check(1).map_err(|e| e.to_string())?;
    let owner = snapshot.accounted().ok_or("accounted native save requires the actual owned snapshot")?;
    if path.is_empty() || path.len() > 4096 {
        return Err("accounted native save destination exceeds path limits".into());
    }
    let quota = owner.quota();
    let held = protection.resident_bytes_with_interrupt(control.interrupt).map_err(|e| e.to_string())?;
    let _protection = quota.reserve(QuotaKind::Resident, held as u64).map_err(|e| e.to_string())?;
    // Prepay the service's derived PathBuf and the alias guard's bounded
    // native path/handle bookkeeping before either is constructed.
    let _paths = quota.reserve(QuotaKind::Resident, 65536).map_err(|e| e.to_string())?;
    let selected = Path::new(path);
    let validate = |candidate: &Path, ctl: &photocraft_raster::backing::ReadControl<'_>| -> Result<(), AccountedNativeError> {
        ctl.check(1)?;
        let fixed = photocraft_io::source_protection::SOURCE_PROTECTION_FIXED_BYTES;
        let _inspection = quota.reserve(QuotaKind::Resident, fixed).map_err(|_| BackingError::Quota)?;
        // No encoded-wrapper collection headroom: this accounted writer
        // already refuses wrappers. Plain destination fingerprints stream
        // through the existing fixed reader/hash scratch under this lease.
        let result = super::validate_destination_protected_with_resident_limit(candidate, protection, Some("pcraft"), ctl.interrupt, fixed);
        ctl.check(1)?;
        result.map_err(|e| AccountedNativeError::Io(std::io::Error::other(e.to_string())))
    };
    validate(selected, control).map_err(|e| e.to_string())?;
    let capability = ServiceAuthorizedNativeWritePath::from_platform_authorized(selected.to_path_buf(), quota, control).map_err(|e| e.to_string())?;
    photocraft_io::accounted_native::save_accounted_current_native_path(owner, &capability, Default::default(), quota, control, |resolved, ctl| {
        validate(selected, ctl)?;
        if resolved != selected {
            validate(resolved, ctl)?;
        }
        Ok(())
    })
    .map_err(|e| e.to_string())
    // No fallible response/bookkeeping or cancellation poll after rename.
    // The engine's prepared completion owns final publication separately.
}

#[cfg(test)]
#[path = "accounted_native_save_tests.rs"]
mod tests;
