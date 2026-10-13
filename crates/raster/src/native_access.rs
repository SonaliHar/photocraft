//! Exact checked channel and copy access; no sample conversion or colour-space inference.
use crate::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, SnapshotSurface, Surface, checked_region_len, pixel_rect};
use photocraft_geom::Rect;

fn channel_range(format: photocraft_color::PixelFormat, channel: usize, output_len: usize) -> Result<std::ops::Range<usize>, RasterInputError> {
    if channel >= format.channels() || output_len != format.sample.bytes() {
        return Err(RasterInputError::LengthMismatch);
    }
    photocraft_color::pixel_channel_byte_range(format, 0, channel, format.bytes_per_pixel()).map_err(|_| RasterInputError::InvalidSample)
}

impl Surface {
    /// Atomic encoded channel edit. Other channels/defaults/history retain their bits.
    pub fn try_write_channel_bytes_with(&mut self, x: i32, y: i32, channel: usize, encoded: &[u8], interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let range = channel_range(self.format(), channel, encoded.len())?;
        let mut pixel = [0u8; 40];
        let pixel = pixel.get_mut(..self.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
        self.try_read_pixel_bytes(x, y, pixel)?;
        pixel.get_mut(range).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(encoded);
        self.try_write_interleaved_with(pixel_rect(x, y)?, pixel, interrupt)
    }
    /// Copy one encoded sample unchanged, including U32 low bits and F64 payloads.
    /// Caller output remains unchanged on invalid channel, coordinates or tile shape.
    pub fn try_read_channel_bytes(&self, x: i32, y: i32, channel: usize, output: &mut [u8]) -> Result<(), RasterInputError> {
        let range = channel_range(self.format(), channel, output.len())?;
        let mut pixel = [0u8; 40];
        let pixel = pixel.get_mut(..self.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
        self.try_read_pixel_bytes(x, y, pixel)?;
        output.copy_from_slice(pixel.get(range).ok_or(RasterInputError::LengthMismatch)?);
        Ok(())
    }

    /// Atomic exact same-format region copy. This is a raw byte operation, not a
    /// colour transform; source/destination encoding metadata must match exactly.
    pub fn try_copy_native_region_from(
        &mut self,
        source: &Surface,
        source_bounds: Rect,
        destination_bounds: Rect,
        interrupt: &Interrupt<'_>,
    ) -> Result<(), RasterInputError> {
        if self.format() != source.format() {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let source_len = checked_region_len(source_bounds, source.format().bytes_per_pixel())?;
        let destination_len = checked_region_len(destination_bounds, self.format().bytes_per_pixel())?;
        if source_bounds.width() != destination_bounds.width() || source_bounds.height() != destination_bounds.height() || source_len != destination_len {
            return Err(RasterInputError::LengthMismatch);
        }
        // Include source, destination, copied region, cloned sparse metadata and
        // all replacement tiles in one simultaneous peak, before any allocation.
        let tile_len = (photocraft_geom::TILE_SIZE as u64)
            .checked_mul(photocraft_geom::TILE_SIZE as u64)
            .and_then(|n| n.checked_mul(self.format().bytes_per_pixel() as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        let replacement = (destination_bounds.tiles().count() as u64)
            .checked_mul(tile_len.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
            .ok_or(RasterInputError::LimitExceeded)?;
        let peak = source
            .resident_accounted_bytes()?
            .checked_add(self.resident_accounted_bytes()?)
            .and_then(|n| n.checked_add(source_len as u64))
            .and_then(|n| n.checked_add(replacement))
            .and_then(|n| n.checked_add(self.default_bytes().len() as u64))
            .and_then(|n| n.checked_add((self.tile_count() as u64).checked_mul(256)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        if peak > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        let bytes = source.try_read_interleaved_with(source_bounds, interrupt)?;
        self.try_write_interleaved_with(destination_bounds, &bytes, interrupt)
    }
}

impl SnapshotSurface {
    /// Atomic lazy encoded channel edit; only the affected COW tile is materialized.
    pub fn try_write_channel_bytes_with(&mut self, x: i32, y: i32, channel: usize, encoded: &[u8], interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let format = self.snapshot().format();
        let range = channel_range(format, channel, encoded.len())?;
        let mut pixel = [0u8; 40];
        let pixel = pixel.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
        self.try_read_pixel_bytes(x, y, pixel)?;
        pixel.get_mut(range).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(encoded);
        self.try_write_interleaved(pixel_rect(x, y)?, pixel, interrupt)
    }
    /// Read owned lazy bytes without materializing tiles or allocating scratch.
    /// On cancellation/error the caller buffer may contain a prefix; storage is unchanged.
    pub fn try_read_interleaved_into_with(&self, bounds: Rect, output: &mut [u8], interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
        let bpp = self.snapshot().format().bytes_per_pixel();
        let length = checked_region_len(bounds, bpp)?;
        if output.len() != length {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut cursor = 0usize;
        let mut pixels = 0usize;
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                if pixels.is_multiple_of(256) && interrupt.cancelled() {
                    return Err(RasterInputError::Cancelled);
                }
                let end = cursor.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?;
                self.try_read_pixel_bytes(x, y, output.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?)?;
                cursor = end;
                pixels = pixels.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
            }
        }
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        Ok(())
    }

    /// Exact channel access without materializing the owned lazy plane.
    pub fn try_read_channel_bytes(&self, x: i32, y: i32, channel: usize, output: &mut [u8]) -> Result<(), RasterInputError> {
        let format = self.snapshot().format();
        let range = channel_range(format, channel, output.len())?;
        let mut pixel = [0u8; 40];
        let pixel = pixel.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
        self.try_read_pixel_bytes(x, y, pixel)?;
        output.copy_from_slice(pixel.get(range).ok_or(RasterInputError::LengthMismatch)?);
        Ok(())
    }
}
