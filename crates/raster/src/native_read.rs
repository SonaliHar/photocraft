//! Exact native reads into caller-owned storage. No numerical conversion or heap scratch.
use crate::{Interrupt, RasterInputError, Surface, checked_region_len};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};

impl Surface {
    /// Copy native interleaved bytes into an exactly sized caller buffer.
    ///
    /// Sparse pixels retain the exact default bytes, including floating payloads and
    /// signed zero. Bounds use the same limits as the existing allocating read API.
    /// No allocation or conversion occurs. Cancellation is polled before each tile-row
    /// segment (at most 64KiB) and at completion. On an error/cancellation the caller
    /// buffer may contain a copied prefix; this surface is never modified.
    pub fn try_read_interleaved_into_with(&self, bounds: Rect, output: &mut [u8], interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let bpp = self.format.bytes_per_pixel();
        let length = checked_region_len(bounds, bpp)?;
        if output.len() != length || self.default_pixel.len() != bpp || bpp == 0 {
            return Err(RasterInputError::LengthMismatch);
        }
        let tile_width = usize::try_from(TILE_SIZE).map_err(|_| RasterInputError::InvalidBounds)?;
        let max_pixels = (64 * 1024usize).checked_div(bpp).filter(|n| *n > 0).ok_or(RasterInputError::LimitExceeded)?;
        let mut cursor = 0usize;
        for y in bounds.y0..bounds.y1 {
            let local_y = usize::try_from(y.rem_euclid(TILE_SIZE)).map_err(|_| RasterInputError::InvalidBounds)?;
            let mut x = bounds.x0;
            while x < bounds.x1 {
                if interrupt.cancelled() {
                    return Err(RasterInputError::Cancelled);
                }
                let local_x = usize::try_from(x.rem_euclid(TILE_SIZE)).map_err(|_| RasterInputError::InvalidBounds)?;
                let remaining = usize::try_from(i64::from(bounds.x1) - i64::from(x)).map_err(|_| RasterInputError::InvalidBounds)?;
                let count = tile_width.checked_sub(local_x).ok_or(RasterInputError::InvalidBounds)?.min(remaining).min(max_pixels);
                let bytes = count.checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                let end = cursor.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
                let destination = output.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                if let Some(tile) = self.tiles.get(&TileCoord::containing(x, y)) {
                    let start = local_y
                        .checked_mul(tile_width)
                        .and_then(|n| n.checked_add(local_x))
                        .and_then(|n| n.checked_mul(bpp))
                        .ok_or(RasterInputError::LimitExceeded)?;
                    let source_end = start.checked_add(bytes).ok_or(RasterInputError::LimitExceeded)?;
                    destination.copy_from_slice(tile.bytes().get(start..source_end).ok_or(RasterInputError::LengthMismatch)?);
                } else {
                    for pixel in destination.chunks_exact_mut(bpp) {
                        pixel.copy_from_slice(&self.default_pixel);
                    }
                }
                cursor = end;
                x = x.checked_add(i32::try_from(count).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
            }
        }
        if cursor != length {
            return Err(RasterInputError::LengthMismatch);
        }
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        Ok(())
    }
}
