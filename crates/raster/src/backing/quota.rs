//! Persistent allocation ownership, separate from borrowed operation cancellation/work.
//!
//! Reserve before allocating. Index is a resident subquota; scratch is independent.
//! A lease is deliberately not Clone: share the physical payload and its lease in one
//! Arc owner instead. Drop the payload before its lease. Standard Arc/Mutex owner
//! construction is a bounded infallible allocator limitation, not an allocation proof.
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaKind {
    Resident,
    Scratch,
    Index,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuotaLimits {
    pub resident: u64,
    pub scratch: u64,
    pub index: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuotaUsage {
    pub resident: u64,
    pub scratch: u64,
    pub index: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaError {
    LimitExceeded(QuotaKind),
    Overflow,
    Poisoned,
    InvalidCapacity,
    Invariant,
}
impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backing quota: {self:?}")
    }
}
impl std::error::Error for QuotaError {}
#[derive(Debug)]
struct State {
    limits: QuotaLimits,
    used: QuotaUsage,
    failed: bool,
}
#[derive(Clone, Debug)]
pub struct BackingQuota {
    state: Arc<Mutex<State>>,
}
#[derive(Debug)]
pub struct QuotaLease {
    quota: BackingQuota,
    kind: QuotaKind,
    bytes: u64,
}

impl BackingQuota {
    pub fn new(limits: QuotaLimits) -> Self {
        Self { state: Arc::new(Mutex::new(State { limits, used: QuotaUsage::default(), failed: false })) }
    }
    /// Runtime owner identity only; equal limits do not imply the same ledger.
    pub fn shares_ledger(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
    pub fn usage(&self) -> Result<QuotaUsage, QuotaError> {
        let state = self.state.lock().map_err(|_| QuotaError::Poisoned)?;
        if state.failed {
            return Err(QuotaError::Invariant);
        }
        Ok(state.used)
    }
    pub fn reserve(&self, kind: QuotaKind, bytes: u64) -> Result<QuotaLease, QuotaError> {
        {
            let mut state = self.state.lock().map_err(|_| QuotaError::Poisoned)?;
            if state.failed {
                return Err(QuotaError::Invariant);
            }
            state.used = added(state.used, state.limits, kind, bytes)?;
        }
        Ok(QuotaLease { quota: self.clone(), kind, bytes })
    }
}
fn added(mut used: QuotaUsage, limits: QuotaLimits, kind: QuotaKind, bytes: u64) -> Result<QuotaUsage, QuotaError> {
    match kind {
        QuotaKind::Resident => used.resident = used.resident.checked_add(bytes).ok_or(QuotaError::Overflow)?,
        QuotaKind::Scratch => used.scratch = used.scratch.checked_add(bytes).ok_or(QuotaError::Overflow)?,
        QuotaKind::Index => {
            used.index = used.index.checked_add(bytes).ok_or(QuotaError::Overflow)?;
            used.resident = used.resident.checked_add(bytes).ok_or(QuotaError::Overflow)?;
        }
    }
    if used.resident > limits.resident {
        return Err(QuotaError::LimitExceeded(QuotaKind::Resident));
    }
    if used.scratch > limits.scratch {
        return Err(QuotaError::LimitExceeded(QuotaKind::Scratch));
    }
    if used.index > limits.index {
        return Err(QuotaError::LimitExceeded(QuotaKind::Index));
    }
    Ok(used)
}
fn removed(mut used: QuotaUsage, kind: QuotaKind, bytes: u64) -> Result<QuotaUsage, QuotaError> {
    match kind {
        QuotaKind::Resident => used.resident = used.resident.checked_sub(bytes).ok_or(QuotaError::Invariant)?,
        QuotaKind::Scratch => used.scratch = used.scratch.checked_sub(bytes).ok_or(QuotaError::Invariant)?,
        QuotaKind::Index => {
            used.index = used.index.checked_sub(bytes).ok_or(QuotaError::Invariant)?;
            used.resident = used.resident.checked_sub(bytes).ok_or(QuotaError::Invariant)?;
        }
    }
    Ok(used)
}
impl QuotaLease {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn kind(&self) -> QuotaKind {
        self.kind
    }
    /// Reserve growth before allocating; denial leaves both lease and ledger intact.
    /// Shrinking is legal only after the corresponding physical storage is released.
    pub fn try_resize(&mut self, new_bytes: u64) -> Result<(), QuotaError> {
        let mut state = self.quota.state.lock().map_err(|_| QuotaError::Poisoned)?;
        if state.failed {
            return Err(QuotaError::Invariant);
        }
        let next = if new_bytes >= self.bytes {
            added(state.used, state.limits, self.kind, new_bytes - self.bytes)?
        } else {
            removed(state.used, self.kind, self.bytes - new_bytes)?
        };
        state.used = next;
        self.bytes = new_bytes;
        Ok(())
    }
    /// Reconcile actual retained allocation capacity within its prepaid envelope.
    /// An oversized allocation must be destroyed while this lease is still live.
    pub fn reconcile_capacity(&mut self, actual_bytes: u64) -> Result<(), QuotaError> {
        if actual_bytes > self.bytes {
            return Err(QuotaError::InvalidCapacity);
        }
        self.try_resize(actual_bytes)
    }
}
impl Drop for QuotaLease {
    fn drop(&mut self) {
        // Poison recovery is restricted to refunding an already-owned reservation.
        let mut state = self.quota.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match removed(state.used, self.kind, self.bytes) {
            Ok(next) => state.used = next,
            Err(_) => state.failed = true,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poison_fails_closed_but_last_drop_refunds_known_ownership() {
        let quota = BackingQuota::new(QuotaLimits { resident: 8, scratch: 0, index: 0 });
        let lease = quota.reserve(QuotaKind::Resident, 8).unwrap();
        let other = quota.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = other.state.lock().unwrap();
                panic!("intentional mutex poisoning test");
            })
            .join()
            .is_err()
        );
        assert_eq!(quota.usage(), Err(QuotaError::Poisoned));
        assert!(matches!(quota.reserve(QuotaKind::Resident, 0), Err(QuotaError::Poisoned)));
        drop(lease);
        let state = quota.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(state.used, QuotaUsage::default());
    }
}
