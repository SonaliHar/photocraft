//! Atomic owning conversion of actual persistent reservation receipts. No source,
//! raw bytes or caller-asserted sizes enter this ownership boundary.
use super::super::Reservation;
use super::{Budget, BudgetKind, OwnedReservation, SharedLedger, failure};
use crate::{ErrorKind, XcfError};
const PHASE: &str = "persistent batch handoff";
const MAX_RECEIPTS: usize = 16384;
/// Accounting information derived from retained receipts, not an ownership proof.
/// Index is a resident subset; resident already includes index. Ledger baseline
/// ownership is reported separately and must be deduplicated by actual identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalUsage {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
}
impl PhysicalUsage {
    fn add(self, other: Self) -> Result<Self, XcfError> {
        Ok(Self {
            resident_bytes: self.resident_bytes.checked_add(other.resident_bytes).ok_or_else(|| failure("resident total overflow", PHASE))?,
            index_bytes: self.index_bytes.checked_add(other.index_bytes).ok_or_else(|| failure("index total overflow", PHASE))?,
            scratch_bytes: self.scratch_bytes.checked_add(other.scratch_bytes).ok_or_else(|| failure("scratch total overflow", PHASE))?,
        })
    }
}
pub(super) fn physical(kind: BudgetKind, bytes: u64, resident: bool) -> PhysicalUsage {
    PhysicalUsage {
        resident_bytes: if kind == BudgetKind::Resident || resident { bytes } else { 0 },
        index_bytes: if kind == BudgetKind::Index { bytes } else { 0 },
        scratch_bytes: if kind == BudgetKind::Scratch { bytes } else { 0 },
    }
}
/// Opaque retention of an actual ledger Arc and its already-admitted baseline.
/// This token charges nothing again. Copies obtained from different owned
/// receipts of one ledger refer to the SAME header and must not be added twice.
pub struct OwnedLedgerHeader {
    ledger: SharedLedger,
}
impl OwnedLedgerHeader {
    pub(super) fn new(ledger: &SharedLedger) -> Self {
        Self { ledger: ledger.clone() }
    }
    pub fn physical_usage(&self) -> PhysicalUsage {
        PhysicalUsage { resident_bytes: self.ledger.header_bytes, index_bytes: self.ledger.header_bytes, scratch_bytes: 0 }
    }
    pub fn shares_ledger(&self, other: &Self) -> bool {
        self.ledger.same(&other.ledger)
    }
    pub fn belongs_to(&self, budget: &Budget<'_>) -> bool {
        budget.shared.as_ref().is_some_and(|ledger| self.ledger.same(ledger))
    }
}
impl std::fmt::Debug for OwnedLedgerHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedLedgerHeader").field("physical_usage", &self.physical_usage()).finish_non_exhaustive()
    }
}
/// Every receipt remains physically owned until this batch drops. Receipt Vec
/// storage is destroyed BEFORE its genuine metadata fee. There is no extraction,
/// cloning, callback, Budget lifetime or external lease assertion.
#[derive(Debug)]
pub struct OwnedReservationBatch {
    values: Vec<OwnedReservation>,
    header: OwnedLedgerHeader,
    physical: PhysicalUsage,
    _fee: OwnedReservation,
}
impl OwnedReservationBatch {
    pub fn reservations(&self) -> &[OwnedReservation] {
        &self.values
    }
    /// Includes the actual Vec/inline metadata fee; excludes the separate shared
    /// ledger baseline, whose header token must be deduplicated across batches.
    pub fn physical_usage(&self) -> PhysicalUsage {
        self.physical
    }
    pub fn ledger_header(&self) -> &OwnedLedgerHeader {
        &self.header
    }
}
struct Staging<'b, 'a> {
    values: Vec<OwnedReservation>,
    fee: Reservation<'b, 'a>,
}
impl<'b, 'a> Reservation<'b, 'a> {
    /// One all-or-none ownership transfer, bounded to16384 real same-ledger
    /// receipts. All validation, actual capacity admission and the final poll
    /// finish BEFORE any original receipt is emptied. Empty batches lack a ledger.
    pub fn detach_owned_batch(receipts: &mut [&mut Reservation<'b, 'a>]) -> Result<OwnedReservationBatch, XcfError> {
        let count = receipts.len();
        if count == 0 || count > MAX_RECEIPTS {
            return Err(failure("batch count limit", PHASE));
        }
        let first = receipts.first().ok_or_else(|| failure("empty batch", PHASE))?;
        let budget = first.budget;
        let ledger = budget.shared.as_ref().ok_or_else(|| XcfError::new(ErrorKind::Unsupported, 0, "local XCF reservations cannot detach as a batch"))?;
        let mut physical_total = PhysicalUsage::default();
        for receipt in receipts.iter() {
            receipt.budget.poll(PHASE)?;
            if !receipt.budget.shared.as_ref().is_some_and(|other| ledger.same(other)) {
                return Err(failure("batch ledger mismatch", PHASE));
            }
            // Prepay bounded receipt inspection/copy and count visits in the
            // same nonrefundable ledger, before any batch allocation or move.
            budget.consume_work((std::mem::size_of::<OwnedReservation>() + 64) as u64, PHASE)?;
            budget.consume_visits(1, PHASE)?;
            physical_total = physical_total.add(physical(receipt.kind, receipt.bytes, receipt.resident))?;
        }
        let slots = count.checked_mul(2).ok_or_else(|| failure("batch capacity overflow", PHASE))?;
        let header = std::mem::size_of::<OwnedReservationBatch>();
        let envelope = slots
            .checked_mul(std::mem::size_of::<OwnedReservation>())
            .and_then(|n| n.checked_add(header))
            .ok_or_else(|| failure("batch metadata overflow", PHASE))?;
        // The fee is a REAL reservation in the same actual ledger, before Vec
        // allocation. The staging field order also protects every error path.
        let fee = budget.reserve(BudgetKind::Index, u64::try_from(envelope).map_err(|_| failure("batch metadata address space", PHASE))?, PHASE)?;
        let mut staged = Staging { values: Vec::new(), fee };
        staged.values.try_reserve_exact(count).map_err(|_| failure("batch allocation", PHASE))?;
        if staged.values.capacity() < count || staged.values.capacity() > slots {
            return Err(failure("batch capacity envelope", PHASE));
        }
        let actual = staged
            .values
            .capacity()
            .checked_mul(std::mem::size_of::<OwnedReservation>())
            .and_then(|n| n.checked_add(header))
            .ok_or_else(|| failure("batch actual metadata overflow", PHASE))?;
        staged.fee.resize(u64::try_from(actual).map_err(|_| failure("batch actual address space", PHASE))?, PHASE)?;
        physical_total = physical_total.add(physical(staged.fee.kind, staged.fee.bytes, staged.fee.resident))?;
        // Public persistent budgets currently have one borrowed control context.
        // All receipts were polled above; no callback is retained in the result.
        budget.poll(PHASE)?;
        // NO fallible operation or callback below this line. The prepaid Vec has
        // enough capacity for every push. Actual ledger charges never change.
        let Staging { values, mut fee } = staged;
        let owned_fee = ledger.owned(fee.kind, fee.bytes, fee.resident);
        fee.bytes = 0;
        let mut output = OwnedReservationBatch { values, header: OwnedLedgerHeader::new(ledger), physical: physical_total, _fee: owned_fee };
        for receipt in receipts.iter_mut() {
            let owned = ledger.owned(receipt.kind, receipt.bytes, receipt.resident);
            receipt.bytes = 0;
            output.values.push(owned);
        }
        Ok(output)
    }
}
#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
