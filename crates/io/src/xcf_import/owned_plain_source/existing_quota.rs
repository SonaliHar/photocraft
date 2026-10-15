//! Opt-in existing-catalog admission. These real leases are accounting, not
//! input/source authority. Original decoded receipts remain the retained proof.
use super::*;
use photocraft_raster::backing::{
    BackingLimits,
    quota::{QuotaKind, QuotaLease},
};

pub(in crate::xcf_import) struct Context<'c, 'i> {
    pub quota: BackingQuota,
    pub control: &'c ReadControl<'i>,
}
/// Reserve a complete caller-lowered preparation envelope before constructing
/// even the private persistent decoder Budget. No usage/peak totals become proof.
pub(in crate::xcf_import) struct PreparationHold {
    index: QuotaLease,
    resident: QuotaLease,
    scratch: QuotaLease,
    resident_limit: u64,
    index_limit: u64,
    scratch_limit: u64,
}
impl PreparationHold {
    pub fn reserve(quota: &BackingQuota, limits: &photocraft_xcf::ResourceLimits, c: &ReadControl<'_>) -> Result<Self, IoError> {
        c.check(1).map_err(backing)?;
        if limits.get(BudgetKind::Scratch) != 0 {
            return Err(invalid("existing-quota plain memory import requires explicit scratchBytes zero"));
        }
        let resident_limit = limits.get(BudgetKind::Resident);
        let index_limit = limits.get(BudgetKind::Index).min(resident_limit);
        let scratch_limit = limits.get(BudgetKind::Scratch);
        let index = quota.reserve(QuotaKind::Index, index_limit).map_err(|_| backing(BackingError::Quota))?;
        let resident = quota
            .reserve(QuotaKind::Resident, resident_limit.checked_sub(index_limit).ok_or_else(|| invalid("existing quota preparation partition"))?)
            .map_err(|_| backing(BackingError::Quota))?;
        let scratch = quota.reserve(QuotaKind::Scratch, scratch_limit).map_err(|_| backing(BackingError::Quota))?;
        Ok(Self { index, resident, scratch, resident_limit, index_limit, scratch_limit })
    }
    /// All borrowed transient decoder/sink registries are gone; only the actual
    /// opaque owned batches/header and result remain. Original constructor-owned
    /// metadata envelopes are admitted before releasing preparation slack.
    pub fn retain_physical(&mut self, physical: PhysicalUsage, layers: usize, c: &ReadControl<'_>) -> Result<(), IoError> {
        c.check(1).map_err(backing)?;
        // Initial metadata depends on cache capacity only; override capacity is
        // a future edit limit and allocates no entries in this constructor.
        let per_raster = BackedRaster::metadata_reservation_bytes(BackingLimits { cache_tiles: 1, override_tiles: 1 }).map_err(backing)?;
        let promotion = (layers as u64)
            .checked_mul(per_raster)
            .and_then(|n| n.checked_add(AccountedDocument::owned_model_reservation_bytes()))
            .ok_or_else(|| invalid("existing quota promotion requirements overflow"))?;
        let resident = physical
            .resident_bytes
            .checked_add(physical.scratch_bytes)
            .and_then(|n| n.checked_add(promotion))
            .ok_or_else(|| invalid("existing quota promotion aggregate overflow"))?;
        if resident > self.resident_limit
            || physical.index_bytes.checked_add(promotion).is_none_or(|n| n > self.index_limit)
            || physical.scratch_bytes > self.scratch_limit
        {
            return Err(backing(BackingError::Quota));
        }
        let nonindex = physical.resident_bytes.checked_sub(physical.index_bytes).ok_or_else(|| invalid("existing quota physical index partition"))?;
        // This single-lock conversion has no refund/recharge gap even when the
        // actual non-Index resident component exceeds the initial R-I partition.
        QuotaLease::repartition_resident_index(&mut self.index, &mut self.resident, physical.index_bytes, nonindex)
            .map_err(|_| backing(BackingError::Quota))?;
        self.scratch.reconcile_capacity(physical.scratch_bytes).map_err(|_| backing(BackingError::Quota))?;
        c.check(1).map_err(backing)?;
        Ok(())
    }
    #[allow(clippy::result_large_err)]
    pub fn into_baseline(self) -> Result<QuotaBaseline, (IoError, Self)> {
        let Self { index, resident, scratch, resident_limit, index_limit, scratch_limit } = self;
        match QuotaBaseline::from_component_leases(index, resident, scratch) {
            Ok(baseline) => Ok(baseline),
            Err((_, index, resident, scratch)) => {
                Err((backing(BackingError::Integrity), Self { index, resident, scratch, resident_limit, index_limit, scratch_limit }))
            }
        }
    }
}
/// Decode the existing closed staged plain-XCF class using the caller's actual
/// catalog quota. The input Arc is retained, never copied. Its INITIAL allocation
/// is the caller's responsibility; a reader/capture owner must reserve before
/// allocating and retain that overlap until this returned import owns the bytes.
/// `limits` must explicitly lower scratchBytes to zero for this memory route.
/// A complete lowered R/I preparation envelope is held before decoder creation;
/// defaults may honestly refuse an already-populated catalog unless lowered.
/// All decoder work forwards to the same caller control; native handoff's prepaid
/// allowance forwards exactly once, with only local Atomic spending afterward.
/// This adds no source/render/profile qualification or serialized authority.
pub fn import_owned_staged_plain_bytes_in_quota(
    name: &str,
    bytes: Arc<Vec<u8>>,
    options: &DecodeOptions<'_>,
    limits: &photocraft_xcf::ResourceLimits,
    quota: BackingQuota,
    control: &ReadControl<'_>,
) -> Result<OwnedPlainXcfImport, IoError> {
    super::import_impl(name, bytes, options, limits, control.interrupt, Some(Context { quota, control }))
}
#[cfg(test)]
#[path = "existing_quota_tests.rs"]
mod tests;
