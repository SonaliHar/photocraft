//! Reusable exact-native cell scratch for configured graph/render consumers.
//! The caller reserves its envelope before construction; no sample conversion.
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

pub(crate) struct NativeCells {
    bytes: Vec<u8>,
    envelope: usize,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
impl NativeCells {
    pub(crate) fn new(envelope: u64, control: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        poll(control)?;
        if envelope == 0 || envelope > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        let envelope = usize::try_from(envelope).map_err(|_| RasterInputError::LimitExceeded)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(envelope).map_err(|_| RasterInputError::AllocationFailed)?;
        // Reject allocator excess before zero-initializing or reading pixels.
        if bytes.capacity() > envelope {
            return Err(RasterInputError::LimitExceeded);
        }
        poll(control)?;
        Ok(Self { bytes, envelope })
    }
    pub(crate) fn read<'a>(&'a mut self, source: &Surface, cell: Rect, control: &Interrupt<'_>) -> Result<&'a mut [u8], RasterInputError> {
        poll(control)?;
        let length = checked_region_byte_len(cell, source.format())?;
        if length > self.envelope || length > self.bytes.capacity() {
            return Err(RasterInputError::LimitExceeded);
        }
        self.bytes.truncate(length);
        while self.bytes.len() < length {
            poll(control)?;
            let end = self.bytes.len().checked_add(64 << 10).ok_or(RasterInputError::LimitExceeded)?.min(length);
            self.bytes.resize(end, 0);
        }
        source.try_read_interleaved_into_with(cell, &mut self.bytes, control)?;
        poll(control)?;
        Ok(&mut self.bytes)
    }
}
