//! Scoped cache leases. The audited helpers receive a live stage ceiling;
//! returned owned capacities remain charged until their render owner dies.
use super::preview_budget::{Lease, PreviewPreparationBudget};
use photocraft_raster::{Interrupt, RasterInputError};
use std::sync::Mutex;
pub(super) struct PreparationBridge<'a> {
    pub(super) budget: &'a PreviewPreparationBudget<'a>,
    retained: Mutex<Vec<Lease<'a, 'a>>>,
    transient: Mutex<Vec<Lease<'a, 'a>>>,
    _metadata: Lease<'a, 'a>,
}
impl<'a> PreparationBridge<'a> {
    pub(super) fn new(budget: &'a PreviewPreparationBudget<'a>, token: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        let bytes = 3072usize.checked_mul(std::mem::size_of::<Lease<'a, 'a>>()).and_then(|n| n.checked_add(128)).ok_or(RasterInputError::LimitExceeded)?;
        let metadata = budget.reserve(bytes, token)?;
        let mut retained = Vec::new();
        retained.try_reserve_exact(2048).map_err(|_| RasterInputError::AllocationFailed)?;
        let mut transient = Vec::new();
        transient.try_reserve_exact(1024).map_err(|_| RasterInputError::AllocationFailed)?;
        if retained.capacity() != 2048 || transient.capacity() != 1024 {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(Self { budget, retained: Mutex::new(retained), transient: Mutex::new(transient), _metadata: metadata })
    }
    pub(super) fn stage(&self, ceiling: usize, token: &Interrupt<'_>) -> Result<Lease<'a, 'a>, RasterInputError> {
        self.budget.reserve(ceiling.min(self.budget.available()), token)
    }
    pub(super) fn retained(&self, mut lease: Lease<'a, 'a>, bytes: usize) -> Result<(), RasterInputError> {
        lease.shrink(bytes)?;
        let mut entries = self.retained.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= 2048 {
            return Err(RasterInputError::LimitExceeded);
        }
        entries.push(lease);
        Ok(())
    }
    pub(super) fn transient(&self, mut lease: Lease<'a, 'a>, bytes: usize) -> Result<(), RasterInputError> {
        lease.shrink(bytes)?;
        let mut entries = self.transient.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= 1024 {
            return Err(RasterInputError::LimitExceeded);
        }
        entries.push(lease);
        Ok(())
    }
    // Called only after every temporary owner in the serial block was dropped.
    pub(super) fn release_transients(&self) {
        self.transient.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
