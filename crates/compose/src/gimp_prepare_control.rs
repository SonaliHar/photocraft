//! Bounded per-level prepared-layer storage and cooperative preparation fences.
//! Existing evaluator/source qualification and tree limits remain authoritative.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
pub(super) fn checkpoint(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| CompositionError::Raster(RasterInputError::Cancelled))
}
/// This is a local known Vec-capacity guard, not an aggregate renderer/doc RSS
/// measurement. It preserves the existing per-level count and recursion ceilings.
pub(super) fn reserve<T>(count: usize, depth: usize, control: &Interrupt<'_>) -> Result<Vec<T>> {
    checkpoint(control)?;
    if depth > photocraft_doc::MAX_GROUP_DEPTH || count > 10000 {
        return Err(CompositionError::Invalid("layer tree limit"));
    }
    let bytes = count.checked_mul(std::mem::size_of::<T>()).ok_or(CompositionError::Invalid("composition preparation capacity overflow"))?;
    if bytes as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(CompositionError::Invalid("composition preparation capacity limit"));
    }
    let mut prepared = Vec::new();
    prepared.try_reserve_exact(count).map_err(|_| CompositionError::Invalid("composition preparation allocation denied"))?;
    let actual = prepared.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(CompositionError::Invalid("composition preparation capacity overflow"))?;
    if actual as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(CompositionError::Invalid("composition preparation capacity limit"));
    }
    checkpoint(control)?;
    Ok(prepared)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_preparation_and_existing_tree_limits_refuse() {
        let cancel = || true;
        assert!(matches!(reserve::<u8>(1, 0, &Interrupt::cancel_only(&cancel)), Err(CompositionError::Raster(RasterInputError::Cancelled))));
        assert!(reserve::<u8>(10001, 0, &Interrupt::NONE).is_err());
        assert!(reserve::<u8>(1, photocraft_doc::MAX_GROUP_DEPTH + 1, &Interrupt::NONE).is_err());
    }
}
