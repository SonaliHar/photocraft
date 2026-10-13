//! Preparation fences for the admitted byte-owned linked-image consumer.
//! CMS calls remain their existing bounded, indivisible operations; these fences
//! do not claim cancellation inside profile parsing or transform construction.
use crate::gimp_composite::Result;
use photocraft_raster::{Interrupt, RasterInputError};

pub(super) fn checkpoint(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}

pub(super) fn prepare<T>(control: &Interrupt<'_>, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    checkpoint(control)?;
    let result = operation();
    checkpoint(control)?;
    result
}

pub(super) fn copy_pixels(bytes: &[u8], control: &Interrupt<'_>) -> Result<Vec<u8>> {
    checkpoint(control)?;
    let length = u64::try_from(bytes.len()).map_err(|_| RasterInputError::LimitExceeded)?;
    if length > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut output = Vec::new();
    output.try_reserve_exact(bytes.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    let capacity = u64::try_from(output.capacity()).map_err(|_| RasterInputError::LimitExceeded)?;
    if capacity > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded.into());
    }
    checkpoint(control)?;
    for chunk in bytes.chunks(64 << 10) {
        checkpoint(control)?;
        let end = output.len().checked_add(chunk.len()).ok_or(RasterInputError::LimitExceeded)?;
        if end > bytes.len() || end > output.capacity() {
            return Err(RasterInputError::LimitExceeded.into());
        }
        output.extend_from_slice(chunk);
    }
    checkpoint(control)?;
    Ok(output)
}
