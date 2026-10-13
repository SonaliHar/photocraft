//! Per-render checked vector geometry and staging. Legacy feather/source-shape
//! preparation is separate; this accounts known payloads, not allocator overhead.
use photocraft_doc::VectorMask;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};
use photocraft_vector::{CompiledVectorMask, VectorPreparationLimits};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
pub(super) const VECTOR_BYTES: usize = 128 << 20;
#[derive(Default)]
struct State {
    masks: HashMap<usize, Arc<CompiledVectorMask>>,
    retained: usize,
}
#[derive(Default)]
pub(super) struct CheckedVectorMasks {
    state: Mutex<State>,
    failure: Mutex<Option<RasterInputError>>,
}
impl CheckedVectorMasks {
    pub(super) fn error(&self) -> Result<(), RasterInputError> {
        self.failure.lock().unwrap_or_else(|e| e.into_inner()).map_or(Ok(()), Err)
    }
    pub(super) fn record(&self, error: RasterInputError) {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(error);
        }
    }
    pub(super) fn values_budgeted(
        &self,
        mask: &VectorMask,
        identity: &VectorMask,
        rect: Rect,
        control: &Interrupt<'_>,
        bridge: Option<&super::preparation_bridge::PreparationBridge<'_>>,
    ) -> Result<Vec<f32>, RasterInputError> {
        self.error()?;
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let key = std::ptr::from_ref(identity) as usize;
        let compiled = {
            // Compilation does not enter Rayon; serializing it limits peak scratch
            // while all other workers may retain their previously compiled masks.
            let mut state = loop {
                match self.state.try_lock() {
                    Ok(state) => break state,
                    Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        control.check().map_err(|_| RasterInputError::Cancelled)?;
                        std::thread::yield_now();
                    }
                }
            };
            if let Some(value) = state.masks.get(&key) {
                value.clone()
            } else {
                let budget = (64_usize << 20).checked_sub(state.retained).ok_or(RasterInputError::LimitExceeded)?;
                if state.masks.len() >= 1024 || budget < 1024 {
                    return Err(RasterInputError::LimitExceeded);
                }
                let stage = bridge.map(|b| b.stage(budget, control)).transpose()?;
                let budget = stage.as_ref().map_or(budget, |s| s.bytes());
                if budget < 1024 {
                    return Err(RasterInputError::LimitExceeded);
                }
                let value =
                    CompiledVectorMask::try_new_with_interrupt(mask, VectorPreparationLimits { max_bytes: budget - 1024, ..Default::default() }, control)?;
                // Grow the publication table only after compilation succeeds;
                // a failed helper must not leave newly allocated slots behind
                // after its temporary reservation has been refunded.
                state.masks.try_reserve(1).map_err(|_| RasterInputError::AllocationFailed)?;
                state.retained =
                    state.retained.checked_add(value.preparation_bytes()).and_then(|n| n.checked_add(1024)).ok_or(RasterInputError::LimitExceeded)?;
                if let (Some(bridge), Some(stage)) = (bridge, stage) {
                    bridge.retained(stage, value.preparation_bytes().checked_add(1024).ok_or(RasterInputError::LimitExceeded)?)?;
                }
                let value = Arc::new(value);
                state.masks.insert(key, value.clone());
                value
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        let workers = rayon::current_num_threads().max(1);
        #[cfg(target_arch = "wasm32")]
        let workers = 1_usize;
        // Geometry is immutable/shared. Per-worker raster scratch is partitioned
        // from a separate aggregate 64 MiB payload budget.
        let bytes = compiled.preparation_bytes().checked_add((64 << 20) / workers).ok_or(RasterInputError::LimitExceeded)?;
        let stage = bridge.map(|b| b.stage(bytes, control)).transpose()?;
        let bytes = stage.as_ref().map_or(bytes, |s| s.bytes());
        let values = compiled.try_render_with_interrupt(rect, VectorPreparationLimits { max_bytes: bytes, ..Default::default() }, control)?;
        if let (Some(bridge), Some(stage)) = (bridge, stage) {
            bridge.transient(stage, values.capacity().checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)?;
        }
        Ok(values)
    }
}
