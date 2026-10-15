//! Fresh standard-profile ledger for an explicitly requested owned native open.
//! Existing catalog owners must reuse their actual ledger instead of this factory.
use super::AccountedNativeError;
use photocraft_raster::backing::{ReadControl, quota::BackingQuota};

/// Construct a genuine fresh ledger under the existing standard import ceiling.
/// No document, path, permission, usage receipt or source qualification is implied.
/// The permanent ledger header is charged by the quota constructor; the returned
/// zero baseline owns no payload and can be dropped immediately.
pub fn new_accounted_native_open_quota(control: &ReadControl<'_>) -> Result<BackingQuota, AccountedNativeError> {
    crate::accounted_open_quota::new_accounted_open_quota(control).map_err(|error| match error {
        crate::accounted_open_quota::AccountedOpenQuotaError::Backing(error) => AccountedNativeError::Backing(error),
        crate::accounted_open_quota::AccountedOpenQuotaError::PolicyUnavailable => {
            AccountedNativeError::Unsupported("standard native open resource policy is unavailable")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::backing::{BackingError, quota::QuotaUsage};
    use photocraft_raster::{Interrupt, backing::quota::QuotaKind};
    use photocraft_xcf::{BudgetKind, DecodeOptions, ResourceLimits, ResourcePolicy};

    /// UNRUN: real header remains charged and scratch plus resident shares the
    /// standard aggregate instead of obtaining an independent extra allowance.
    #[test]
    fn fresh_standard_ledger_retains_real_header_and_enforces_shared_aggregate() {
        let ctl = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let q = new_accounted_native_open_quota(&ctl).unwrap();
        let other = new_accounted_native_open_quota(&ctl).unwrap();
        assert!(!q.shares_ledger(&other));
        let header = BackingQuota::aggregate_header_bytes();
        assert_eq!(q.usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
        let p = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
        let aggregate = p.get(BudgetKind::Resident).min(DecodeOptions::default().limits.resident_bytes as u64);
        let held = q.reserve(QuotaKind::Resident, aggregate - header).unwrap();
        assert!(q.reserve(QuotaKind::Resident, 1).is_err());
        assert!(q.reserve(QuotaKind::Scratch, 1).is_err());
        drop(held);
        assert_eq!(q.usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
    }

    /// UNRUN: cancellation/work denial is observed before a fresh ledger is made.
    #[test]
    fn cancelled_or_denied_factory_does_not_manufacture_an_owner() {
        let stop = || true;
        let interrupt = Interrupt::cancel_only(&stop);
        let cancelled = ReadControl { interrupt: &interrupt, work: &|_| Ok(()) };
        assert!(matches!(new_accounted_native_open_quota(&cancelled), Err(AccountedNativeError::Backing(BackingError::Cancelled))));
        let denied = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert!(matches!(new_accounted_native_open_quota(&denied), Err(AccountedNativeError::Backing(BackingError::Work))));
    }
}
