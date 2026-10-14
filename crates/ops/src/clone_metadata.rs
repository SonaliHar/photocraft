//! Bounded resident history metadata admission; shared document snapshots are external backing.
use super::{History, HistoryState, LayerTarget};
use photocraft_raster::{Interrupt, RasterInputError};
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloneMetadataRequirements {
    pub retained_bytes: u64,
    /// Conservative clone and one-entry deque growth envelope, excluding new target/label data.
    pub clone_bytes: u64,
    /// Census work prepaid by the enclosing operation before this helper runs.
    pub census_work_units: u64,
    /// Prospective cloning/copy work; the helper does not perform the clone.
    pub clone_work_units: u64,
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn bytes(count: usize, size: usize) -> Result<u64, RasterInputError> {
    mul(u64::try_from(count).map_err(|_| RasterInputError::LimitExceeded)?, size as u64)
}
fn target_bytes(target: &LayerTarget) -> Result<u64, RasterInputError> {
    bytes(target.selected.capacity(), size_of::<photocraft_doc::LayerId>())
}
impl History {
    /// O(1) prospective census bound. Byte-budgeted/cached history needs a separate tile consumer.
    pub fn plain_clone_census_work(&self) -> Result<u64, RasterInputError> {
        if self.max_bytes != 0 || self.tiles.is_some() {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let states = add(self.undo.len() as u64, self.redo.len() as u64)?;
        add(mul(states, 4)?, 8)
    }
    /// Allocation-free, polled census of actual retained capacities. This does not grant edit
    /// authority or count pixels/models inside shared historical Arc<Document> snapshots.
    pub fn plain_clone_metadata_requirements(
        &self,
        byte_limit: u64,
        work_limit: u64,
        control: &Interrupt<'_>,
    ) -> Result<CloneMetadataRequirements, RasterInputError> {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let census_work_units = self.plain_clone_census_work()?;
        if census_work_units > work_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut retained_bytes = size_of::<Self>() as u64;
        for capacity in [self.undo.capacity(), self.redo.capacity()] {
            retained_bytes = add(retained_bytes, bytes(capacity, size_of::<HistoryState>())?)?;
        }
        retained_bytes = add(retained_bytes, self.current_label.capacity() as u64)?;
        retained_bytes = add(retained_bytes, target_bytes(&self.current_layers)?)?;
        if retained_bytes > byte_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        for state in self.undo.iter().chain(&self.redo) {
            control.check().map_err(|_| RasterInputError::Cancelled)?;
            retained_bytes = add(retained_bytes, state.label.capacity() as u64)?;
            retained_bytes = add(retained_bytes, target_bytes(&state.layers)?)?;
            if retained_bytes > byte_limit {
                return Err(RasterInputError::LimitExceeded);
            }
        }
        // Vec/String/VecDeque clones may allocate rounded capacities; allow twice the original
        // ownership and the new record slot. Original allocations stay retained simultaneously.
        let clone_bytes = add(mul(retained_bytes, 2)?, mul(size_of::<HistoryState>() as u64, 2)?)?;
        let clone_work_units = mul(clone_bytes, 4)?;
        if add(retained_bytes, clone_bytes)? > byte_limit || add(census_work_units, clone_work_units)? > work_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(CloneMetadataRequirements { retained_bytes, clone_bytes, census_work_units, clone_work_units })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_capacity_and_lower_bounds_precede_clone() {
        let mut history = History::new(20);
        history.current_layers.selected.reserve(64);
        history.current_layers.selected.push(photocraft_doc::LayerId(7));
        history.current_label.reserve(4096);
        let needs = history.plain_clone_metadata_requirements(1 << 20, 1 << 24, &Interrupt::NONE).unwrap();
        assert!(needs.retained_bytes >= 4096 + 64 * size_of::<photocraft_doc::LayerId>() as u64);
        assert!(history.plain_clone_metadata_requirements(needs.retained_bytes + needs.clone_bytes - 1, 1 << 24, &Interrupt::NONE).is_err());
        assert!(history.plain_clone_metadata_requirements(1 << 20, needs.census_work_units + needs.clone_work_units - 1, &Interrupt::NONE).is_err());
        history.max_bytes = 1;
        assert!(history.plain_clone_census_work().is_err());
    }
}
