//! Platform-independent standard ledger for explicit owned document intake.
//! Existing catalog owners reuse their actual ledger rather than this factory.
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaLimits, QuotaUsage},
};
use photocraft_xcf::{BudgetKind, DecodeOptions, ResourceLimits, ResourcePolicy};

#[derive(Debug, thiserror::Error)]
pub enum AccountedOpenQuotaError {
    #[error("owned open backing: {0}")]
    Backing(#[from] BackingError),
    #[error("standard owned open resource policy is unavailable")]
    PolicyUnavailable,
}

/// A real fresh ledger under the unchanged standard decoder/bridge ceiling.
/// This supplies no path, source eligibility, profile decision or permission.
pub fn new_accounted_open_quota(control: &ReadControl<'_>) -> Result<BackingQuota, AccountedOpenQuotaError> {
    control.check(u64::try_from(BudgetKind::ALL.len()).map_err(|_| BackingError::Bounds)?)?;
    let policy = ResourceLimits::for_policy(ResourcePolicy::default()).map_err(|_| AccountedOpenQuotaError::PolicyUnavailable)?;
    let aggregate = policy.get(BudgetKind::Resident).min(u64::try_from(DecodeOptions::default().limits.resident_bytes).map_err(|_| BackingError::Bounds)?);
    let limits = QuotaLimits { resident: aggregate, scratch: policy.get(BudgetKind::Scratch), index: policy.get(BudgetKind::Index) };
    control.check(1)?;
    let (quota, zero_baseline) = BackingQuota::new_with_aggregate_initial(limits, aggregate, QuotaUsage::default()).map_err(|_| BackingError::Quota)?;
    // The permanent header stays paid by the ledger. The empty baseline owns no payload.
    drop(zero_baseline);
    control.check(0)?;
    Ok(quota)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::{Interrupt, backing::quota::QuotaKind};

    #[test]
    fn portable_owned_intake_uses_one_real_header_and_the_existing_aggregate() {
        // UNRUN: exercised on native and wasm when validation is resumed.
        let control = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) };
        let quota = new_accounted_open_quota(&control).unwrap();
        let other = new_accounted_open_quota(&control).unwrap();
        assert!(!quota.shares_ledger(&other));
        let header = BackingQuota::aggregate_header_bytes();
        assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
        let limits = ResourceLimits::for_policy(ResourcePolicy::default()).unwrap();
        let aggregate = limits.get(BudgetKind::Resident).min(DecodeOptions::default().limits.resident_bytes as u64);
        let hold = quota.reserve(QuotaKind::Resident, aggregate - header).unwrap();
        assert!(quota.reserve(QuotaKind::Resident, 1).is_err());
        assert!(quota.reserve(QuotaKind::Scratch, 1).is_err());
        drop(hold);
        assert_eq!(quota.usage().unwrap(), QuotaUsage { resident: header, index: header, scratch: 0 });
    }

    #[test]
    fn caller_failure_is_preserved_before_constructing_a_fresh_ledger() {
        // UNRUN: arbitrary callback failures must retain their backing category.
        for failure in [BackingError::Cancelled, BackingError::Work, BackingError::Unsupported] {
            let charge = |_| Err(failure);
            let control = ReadControl { interrupt: &Interrupt::NONE, work: &charge };
            assert!(matches!(new_accounted_open_quota(&control), Err(AccountedOpenQuotaError::Backing(error)) if error == failure));
        }
    }
}
