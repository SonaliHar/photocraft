//! Separate masked mixed SOURCE endpoint; original unit-source endpoint stays exact.
use crate::gimp_composite::CompositionError;
use crate::gimp_render::PreparationBudget;
/// Original Source allocator: proof indexes use the same caller's actual Index
/// pool while ordinary prepared payloads retain their existing Resident pool.
pub trait MaskedFilterPreparationBudget: PreparationBudget {
    fn reserve_filter_mask_proof_index(&mut self, bytes:u64)->Result<Self::Lease,CompositionError>;
}
pub use crate::backed_filtered_flat_point_context::LeasedMaskedFilteredFlatPointPipeline;
