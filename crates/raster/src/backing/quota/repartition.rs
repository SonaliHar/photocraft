//! Atomic reclassification of two genuine leases on the same persistent ledger.
use super::*;
impl QuotaLease {
    /// Repartition existing Index and non-Index Resident ownership atomically.
    /// Index remains included in resident usage. All validation/cap checks use
    /// one mutex and a private candidate; failure leaves both leases and usage
    /// unchanged. Other leases and the permanent ledger header are untouched.
    /// This is accounting only, never a payload/source-validity assertion. The
    /// caller must retain actual storage throughout and supply its proven needs.
    pub fn repartition_resident_index(index: &mut Self, resident: &mut Self, new_index: u64, new_resident_nonindex: u64) -> Result<(), QuotaError> {
        if index.kind != QuotaKind::Index || resident.kind != QuotaKind::Resident || !index.quota.shares_ledger(&resident.quota) {
            return Err(QuotaError::Invariant);
        }
        let mut state = index.quota.state.lock().map_err(|_| QuotaError::Poisoned)?;
        if state.failed {
            return Err(QuotaError::Invariant);
        }
        let candidate = removed(state.used, QuotaKind::Index, index.bytes)?;
        let candidate = removed(candidate, QuotaKind::Resident, resident.bytes)?;
        let candidate = added(candidate, state.limits, state.aggregate_limit, QuotaKind::Index, new_index)?;
        let candidate = added(candidate, state.limits, state.aggregate_limit, QuotaKind::Resident, new_resident_nonindex)?;
        state.used = candidate;
        index.bytes = new_index;
        resident.bytes = new_resident_nonindex;
        Ok(())
    }
}
impl QuotaBaseline {
    /// Move three actual same-ledger leases into a fixed retention owner.
    /// No numeric totals, allocation, lock or new charge is accepted here.
    /// On rejection every original lease is returned so the caller can retain
    /// fees until associated payload is destroyed. This grants no source or
    /// payload validity/authority, and does not inspect the caller's storage.
    #[allow(clippy::result_large_err)]
    pub fn from_component_leases(
        index: QuotaLease,
        resident: QuotaLease,
        scratch: QuotaLease,
    ) -> Result<Self, (QuotaError, QuotaLease, QuotaLease, QuotaLease)> {
        if index.kind != QuotaKind::Index
            || resident.kind != QuotaKind::Resident
            || scratch.kind != QuotaKind::Scratch
            || !index.quota.shares_ledger(&resident.quota)
            || !index.quota.shares_ledger(&scratch.quota)
        {
            return Err((QuotaError::Invariant, index, resident, scratch));
        }
        Ok(Self { _index: index, _resident: resident, _scratch: scratch })
    }
}
#[cfg(test)]
#[path = "repartition_tests.rs"]
mod tests;
