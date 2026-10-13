//! Staged allocation primitives for the optional owned document codec.
//! No public native route uses this module. These account logical capacities,
//! not allocator/RSS overhead. All constructors reserve before allocation.
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};

/// Payload precedes its lease so errors/drop release storage before its charge.
#[derive(Debug)]
pub struct ChargedVec<T> {
    values: Vec<T>,
    lease: QuotaLease,
}
impl<T> ChargedVec<T> {
    pub fn with_capacity(count: usize, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(0)?;
        let size = std::mem::size_of::<T>();
        if size == 0 {
            return Err(BackingError::Unsupported);
        }
        let bytes = count.checked_mul(size).ok_or(BackingError::Length)?;
        let envelope = bytes.checked_add(size.saturating_mul(16)).ok_or(BackingError::Length)?;
        let mut lease = quota.reserve(QuotaKind::Resident, u64::try_from(envelope).map_err(|_| BackingError::Length)?).map_err(|_| BackingError::Quota)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| BackingError::Allocation)?;
        let actual = values.capacity().checked_mul(size).ok_or(BackingError::Length)?;
        if actual > envelope {
            drop(values);
            return Err(BackingError::Allocation);
        }
        lease.reconcile_capacity(u64::try_from(actual).map_err(|_| BackingError::Length)?).map_err(|_| BackingError::Quota)?;
        let candidate = Self { values, lease };
        control.check(0)?;
        Ok(candidate)
    }
    pub fn push(&mut self, value: T, control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if self.values.len() == self.values.capacity() {
            return Err(BackingError::Quota);
        }
        self.values.push(value);
        Ok(())
    }
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    pub(crate) fn into_parts(self) -> (Vec<T>, QuotaLease) {
        (self.values, self.lease)
    }
}
impl ChargedVec<u8> {
    pub fn extend(&mut self, bytes: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(u64::try_from(bytes.len()).map_err(|_| BackingError::Length)?)?;
        let next = self.values.len().checked_add(bytes.len()).ok_or(BackingError::Length)?;
        if next > self.values.capacity() {
            return Err(BackingError::Length);
        }
        self.values.extend_from_slice(bytes);
        Ok(())
    }
}

/// A bounded catalogue of prepaid owned allocations. The physical model must
/// precede this owner in the final wrapper. A deep clone requires a new owner.
#[derive(Debug)]
pub struct RetainedReservations {
    leases: ChargedVec<QuotaLease>,
    quota: BackingQuota,
    maximum: usize,
}
impl RetainedReservations {
    pub fn new(maximum: usize, quota: BackingQuota, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        if maximum == 0 || maximum > 4096 {
            return Err(BackingError::Length);
        }
        let leases = ChargedVec::with_capacity(maximum, &quota, control)?;
        Ok(Self { leases, quota, maximum })
    }
    /// Called before the associated allocation. No callback is invoked under
    /// quota locks. Denial leaves prior retained ownership intact.
    pub fn reserve(&mut self, bytes: u64, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.leases.as_slice().len() >= self.maximum {
            return Err(BackingError::Quota);
        }
        control.check(1)?;
        let lease = self.quota.reserve(QuotaKind::Resident, bytes).map_err(|_| BackingError::Quota)?;
        self.leases.push(lease, control)
    }
    pub fn quota(&self) -> &BackingQuota {
        &self.quota
    }
    /// Transfer a real buffer's token. The caller must keep that buffer in the
    /// model or destroy it before dropping this owner.
    pub fn retain(&mut self, lease: QuotaLease, control: &ReadControl<'_>) -> Result<(), (BackingError, QuotaLease)> {
        if self.leases.as_slice().len() >= self.maximum || lease.kind() != QuotaKind::Resident {
            return Err((BackingError::Quota, lease));
        }
        if let Err(error) = control.check(1) {
            return Err((error, lease));
        }
        if self.leases.values.len() == self.leases.values.capacity() {
            return Err((BackingError::Quota, lease));
        }
        self.leases.values.push(lease);
        Ok(())
    }
}
