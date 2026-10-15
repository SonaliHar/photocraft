//! Persistent allocation ownership, separate from borrowed operation cancellation/work.
//!
//! Reserve before allocating. Index is a resident subquota; scratch is independent.
//! A lease is deliberately not Clone: share the physical payload and its lease in one
//! Arc owner instead. Drop the payload before its lease. Standard Arc/Mutex owner
//! construction is a bounded infallible allocator limitation, not an allocation proof.
use std::sync::{Arc, Mutex};

#[path = "quota/repartition.rs"]
mod repartition;

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
    aggregate_limit: Option<u64>,
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

/// Fixed initial reservations, refunded on drop. This is allocation accounting,
/// not evidence of the origin, validity or authority of any associated payload.
/// Keep associated storage alive no longer than this non-cloneable owner.
#[derive(Debug)]
pub struct QuotaBaseline {
    _index: QuotaLease,
    _resident: QuotaLease,
    _scratch: QuotaLease,
}

impl BackingQuota {
    pub fn new(limits: QuotaLimits) -> Self {
        Self { state: Arc::new(Mutex::new(State { limits, aggregate_limit: None, used: QuotaUsage::default(), failed: false })) }
    }
    /// Known Mutex/State size plus a conservative 128-byte reservation for Arc
    /// counters, allocation alignment and allocator metadata. This is a logical
    /// envelope, not a measurement of private allocator storage or process RSS.
    pub const fn aggregate_header_bytes() -> u64 {
        std::mem::size_of::<Mutex<State>>() as u64 + 128
    }
    /// Opt-in aggregate admission. `initial.resident` already includes index;
    /// aggregate usage is resident + scratch, never resident + index + scratch.
    /// Header ownership remains charged until the last ledger owner is dropped.
    /// All checks precede the one new Arc allocation. Arc allocation itself uses
    /// the standard allocator and is not made fallible by this accounting API.
    /// Aggregate denial uses `LimitExceeded(Resident)`; limits are never raised.
    pub fn new_with_aggregate_initial(limits: QuotaLimits, aggregate_limit: u64, initial: QuotaUsage) -> Result<(Self, QuotaBaseline), QuotaError> {
        let resident = initial.resident.checked_sub(initial.index).ok_or(QuotaError::Invariant)?;
        let header = Self::aggregate_header_bytes();
        let used = added(QuotaUsage::default(), limits, Some(aggregate_limit), QuotaKind::Index, header)?;
        let used = added(used, limits, Some(aggregate_limit), QuotaKind::Index, initial.index)?;
        let used = added(used, limits, Some(aggregate_limit), QuotaKind::Resident, resident)?;
        let used = added(used, limits, Some(aggregate_limit), QuotaKind::Scratch, initial.scratch)?;
        let quota = Self { state: Arc::new(Mutex::new(State { limits, aggregate_limit: Some(aggregate_limit), used, failed: false })) };
        let baseline = QuotaBaseline {
            _index: QuotaLease { quota: quota.clone(), kind: QuotaKind::Index, bytes: initial.index },
            _resident: QuotaLease { quota: quota.clone(), kind: QuotaKind::Resident, bytes: resident },
            _scratch: QuotaLease { quota: quota.clone(), kind: QuotaKind::Scratch, bytes: initial.scratch },
        };
        Ok((quota, baseline))
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
            state.used = added(state.used, state.limits, state.aggregate_limit, kind, bytes)?;
        }
        Ok(QuotaLease { quota: self.clone(), kind, bytes })
    }
}
fn added(mut used: QuotaUsage, limits: QuotaLimits, aggregate_limit: Option<u64>, kind: QuotaKind, bytes: u64) -> Result<QuotaUsage, QuotaError> {
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
    if let Some(limit) = aggregate_limit {
        if used.resident.checked_add(used.scratch).ok_or(QuotaError::Overflow)? > limit {
            return Err(QuotaError::LimitExceeded(QuotaKind::Resident));
        }
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
            added(state.used, state.limits, state.aggregate_limit, self.kind, new_bytes - self.bytes)?
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
#[path = "quota_aggregate_tests.rs"]
mod aggregate_tests;
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
