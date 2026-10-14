//! Opt-in persistent ledger. No source bytes, callback, timer or permissions are
//! retained here. Mutex transactions publish component/aggregate/peak changes
//! together. Standard Arc/Mutex allocation uses a bounded prepaid header envelope,
//! not a process-RSS guarantee. The legacy Cell budget never allocates this state.
use super::{BUDGET_COUNT, Budget, BudgetKind, ResourceLimits};
use crate::{ErrorKind, XcfError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

struct State {
    limits: ResourceLimits,
    used: [u64; BUDGET_COUNT],
    peak: [u64; BUDGET_COUNT],
    failed: bool,
}
#[derive(Clone)]
pub(super) struct SharedLedger {
    state: Arc<Mutex<State>>,
}
fn failure(message: &str, phase: &str) -> XcfError {
    XcfError::new(ErrorKind::Limit, 0, format!("persistent XCF budget {message}; {phase}"))
}
fn denial(limits: &ResourceLimits, kind: BudgetKind, observed: u64, phase: &str) -> XcfError {
    XcfError::new(ErrorKind::Limit, 0, format!("{} {}: allowed {}, observed {observed}; {phase}", limits.policy().name(), kind.name(), limits.get(kind)))
}
fn set(state: &mut State, kind: BudgetKind, n: u64) -> Result<(), XcfError> {
    let used = state.used.get_mut(kind.index()).ok_or_else(|| failure("kind invariant", "transition"))?;
    let peak = state.peak.get_mut(kind.index()).ok_or_else(|| failure("kind invariant", "transition"))?;
    *used = n;
    *peak = (*peak).max(n);
    Ok(())
}
impl SharedLedger {
    pub(super) fn new(limits: &ResourceLimits) -> Result<Self, XcfError> {
        // Mutex<State> includes actual inline counters/limits.128 bytes covers
        // fixed Arc/header allocator overhead; there is no self-referential lease.
        // This baseline remains until the last actual ledger Arc is destroyed.
        let header = (std::mem::size_of::<Mutex<State>>() as u64).checked_add(128).ok_or_else(|| failure("header overflow", "construction"))?;
        for kind in [BudgetKind::Index, BudgetKind::Resident] {
            if header > limits.get(kind) {
                return Err(denial(limits, kind, header, "ledger header admission"));
            }
        }
        let mut state = State { limits: limits.clone(), used: [0; BUDGET_COUNT], peak: [0; BUDGET_COUNT], failed: false };
        set(&mut state, BudgetKind::Index, header)?;
        set(&mut state, BudgetKind::Resident, header)?;
        Ok(Self { state: Arc::new(Mutex::new(state)) })
    }
    fn lock(&self, phase: &str) -> Result<MutexGuard<'_, State>, XcfError> {
        let state = self.state.lock().map_err(|_| failure("ledger poisoned", phase))?;
        if state.failed {
            return Err(failure("ledger invariant failed", phase));
        }
        Ok(state)
    }
    pub(super) fn valid(&self, phase: &str) -> Result<(), XcfError> {
        drop(self.lock(phase)?);
        Ok(())
    }
    pub(super) fn used(&self, kind: BudgetKind) -> u64 {
        self.lock("usage").ok().and_then(|s| s.used.get(kind.index()).copied()).unwrap_or(u64::MAX)
    }
    pub(super) fn peak(&self, kind: BudgetKind) -> u64 {
        self.lock("peak").ok().and_then(|s| s.peak.get(kind.index()).copied()).unwrap_or(u64::MAX)
    }
    pub(super) fn reserve(&self, kind: BudgetKind, n: u64, resident: bool, phase: &str) -> Result<(), XcfError> {
        let mut state = self.lock(phase)?;
        let old = *state.used.get(kind.index()).ok_or_else(|| failure("kind invariant", phase))?;
        let next = old.checked_add(n).ok_or_else(|| denial(&state.limits, kind, u64::MAX, phase))?;
        if next > state.limits.get(kind) {
            return Err(denial(&state.limits, kind, next, phase));
        }
        let aggregate = if resident {
            let value = state
                .used
                .get(BudgetKind::Resident.index())
                .copied()
                .ok_or_else(|| failure("resident invariant", phase))?
                .checked_add(n)
                .ok_or_else(|| denial(&state.limits, BudgetKind::Resident, u64::MAX, phase))?;
            if value > state.limits.get(BudgetKind::Resident) {
                return Err(denial(&state.limits, BudgetKind::Resident, value, phase));
            }
            Some(value)
        } else {
            None
        };
        set(&mut state, kind, next)?;
        if let Some(n) = aggregate {
            set(&mut state, BudgetKind::Resident, n)?;
        }
        Ok(())
    }
    pub(super) fn resize(&self, kind: BudgetKind, old: u64, new: u64, resident: bool, phase: &str) -> Result<(), XcfError> {
        if new >= old {
            return self.reserve(kind, new - old, resident, phase);
        }
        let mut state = self.lock(phase)?;
        if !Self::subtract(&mut state, kind, old - new, resident) {
            state.failed = true;
            return Err(failure("refund invariant", phase));
        }
        Ok(())
    }
    fn subtract(state: &mut State, kind: BudgetKind, n: u64, resident: bool) -> bool {
        let Some(next) = state.used.get(kind.index()).and_then(|v| v.checked_sub(n)) else {
            return false;
        };
        let aggregate = if resident {
            let Some(next) = state.used.get(BudgetKind::Resident.index()).and_then(|v| v.checked_sub(n)) else {
                return false;
            };
            Some(next)
        } else {
            None
        };
        let Some(slot) = state.used.get_mut(kind.index()) else {
            return false;
        };
        *slot = next;
        if let Some(next) = aggregate {
            let Some(slot) = state.used.get_mut(BudgetKind::Resident.index()) else {
                return false;
            };
            *slot = next;
        }
        true
    }
    pub(super) fn refund(&self, kind: BudgetKind, n: u64, resident: bool) {
        // Recovery is cleanup only. Poison remains visible to every later poll,
        // admission and statistic; no user callback executes while this lock is held.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !Self::subtract(&mut state, kind, n, resident) {
            state.failed = true;
        }
    }
    pub(super) fn owned(&self, kind: BudgetKind, bytes: u64, resident: bool) -> OwnedReservation {
        OwnedReservation { ledger: self.clone(), kind, bytes, resident }
    }
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}
/// A noncloneable receipt for an actual existing persistent-ledger charge. Only
/// Reservation::detach_owned constructs this; no numeric assertion is accepted.
/// Keep it behind the payload in owner field order. It retains no Budget or
/// cancellation callback and can outlive their scopes. Work is never refundable.
pub struct OwnedReservation {
    ledger: SharedLedger,
    kind: BudgetKind,
    bytes: u64,
    resident: bool,
}
impl OwnedReservation {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn kind(&self) -> BudgetKind {
        self.kind
    }
    pub fn shares_ledger(&self, other: &Self) -> bool {
        self.ledger.same(&other.ledger)
    }
    pub fn belongs_to(&self, budget: &Budget<'_>) -> bool {
        budget.shared.as_ref().is_some_and(|ledger| self.ledger.same(ledger))
    }
}
impl std::fmt::Debug for OwnedReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedReservation").field("kind", &self.kind).field("bytes", &self.bytes).finish_non_exhaustive()
    }
}
impl Drop for OwnedReservation {
    fn drop(&mut self) {
        self.ledger.refund(self.kind, self.bytes, self.resident);
    }
}
#[cfg(test)]
#[path = "persistent_tests.rs"]
mod tests;
