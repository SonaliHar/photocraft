//! Fixed coordinate views. Native storage, cache and COW entries retain root keys.
//! Target tile padding is the exact descriptor default, never neighboring pixels.
use super::*;

fn shifted(rect: Rect, dx: i64, dy: i64) -> Result<Rect, BackingError> {
    let x = |v: i32| i64::from(v).checked_add(dx).and_then(|n| i32::try_from(n).ok()).ok_or(BackingError::Bounds);
    let y = |v: i32| i64::from(v).checked_add(dy).and_then(|n| i32::try_from(n).ok()).ok_or(BackingError::Bounds);
    Ok(Rect::new(x(rect.x0)?, y(rect.y0)?, x(rect.x1)?, y(rect.y1)?))
}

impl BackedRaster {
    /// An allocation-free current coordinate view, retaining the same immutable
    /// store and actual quota. Repeated translations accumulate a fixed offset;
    /// they do not construct provider chains or reindex/copy native payloads.
    /// No source, rendering, filesystem or serialized locator authority follows.
    pub fn try_translated(&self, dx: i32, dy: i32, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if dx == 0 && dy == 0 {
            return Ok(self.clone());
        }
        let offset = (
            self.view_offset.0.checked_add(i64::from(dx)).ok_or(BackingError::Bounds)?,
            self.view_offset.1.checked_add(i64::from(dy)).ok_or(BackingError::Bounds)?,
        );
        let bounds = shifted(self.root_descriptor.bounds(), offset.0, offset.1)?;
        // Validate complete public tile endpoints without iterating the logical
        // plane. Logical source planes have no override-index tile-count cap.
        tile_rect(TileCoord::containing(bounds.x0, bounds.y0))?;
        tile_rect(TileCoord::containing(bounds.x1.checked_sub(1).ok_or(BackingError::Bounds)?, bounds.y1.checked_sub(1).ok_or(BackingError::Bounds)?))?;
        let descriptor = NativePlaneDescriptor::new(self.descriptor.format(), bounds, self.descriptor.default_bytes())?;
        control.check(1)?;
        let mut result = self.clone();
        result.descriptor = descriptor;
        result.view_offset = offset;
        Ok(result)
    }

    fn root_view(&self) -> Self {
        let mut root = self.clone();
        root.descriptor = self.root_descriptor;
        root.view_offset = (0, 0);
        root
    }

    pub(super) fn pin_translated_tile(&self, coord: TileCoord, control: &ReadControl<'_>) -> Result<TilePin, BackingError> {
        control.check(1)?;
        let target = tile_rect(coord)?;
        let overlap = target.intersect(&self.descriptor.bounds());
        if !overlap.is_empty() && self.view_offset.0 % i64::from(TILE_SIZE) == 0 && self.view_offset.1 % i64::from(TILE_SIZE) == 0 {
            // Exact tile alignment preserves layout and the operative/default
            // partition. The public checked view supplies the TARGET label;
            // physical bytes and their actual lease can be shared unchanged.
            let source =
                shifted(overlap, self.view_offset.0.checked_neg().ok_or(BackingError::Bounds)?, self.view_offset.1.checked_neg().ok_or(BackingError::Bounds)?)?;
            if !self.root_descriptor.bounds().contains_rect(&source) {
                return Err(BackingError::Integrity);
            }
            return self.root_view().pin_tile(TileCoord::containing(source.x0, source.y0), control);
        }
        let length = tile_length(self.descriptor.format())?;
        // allocate initializes its Vec; prepay that initialization before allocation.
        control.check(length as u64)?;
        let mut payload = allocate(&self.quota, length)?;
        let bpp = self.descriptor.format().bytes_per_pixel();
        for block in payload.bytes.chunks_mut(bpp.checked_mul(64).ok_or(BackingError::Bounds)?) {
            control.check(block.len() as u64)?;
            for pixel in block.chunks_exact_mut(bpp) {
                pixel.copy_from_slice(self.descriptor.default_bytes());
            }
        }
        // Empty target tiles do not inverse-map arbitrary exterior coordinates.
        if !overlap.is_empty() {
            let dx = self.view_offset.0.checked_neg().ok_or(BackingError::Bounds)?;
            let dy = self.view_offset.1.checked_neg().ok_or(BackingError::Bounds)?;
            let source = shifted(overlap, dx, dy)?;
            if !self.root_descriptor.bounds().contains_rect(&source) {
                return Err(BackingError::Integrity);
            }
            let root = self.root_view();
            // An at-most-one-tile-wide rectangle intersects at most four root
            // tiles. Pins are retained during each raw copy, alongside output.
            for (visited, source_coord) in source.tiles().enumerate() {
                control.check(1)?;
                if visited >= 4 {
                    return Err(BackingError::Integrity);
                }
                let source_tile = tile_rect(source_coord)?;
                let part = source_tile.intersect(&source);
                let pin = root.pin_tile(source_coord, control)?;
                for y in part.y0..part.y1 {
                    let destination =
                        shifted(Rect::new(part.x0, y, part.x1, y.checked_add(1).ok_or(BackingError::Bounds)?), self.view_offset.0, self.view_offset.1)?;
                    let from = pixel_offset(source_tile, part.x0, y, bpp)?;
                    let to = pixel_offset(target, destination.x0, destination.y0, bpp)?;
                    let count = (part.width() as usize).checked_mul(bpp).ok_or(BackingError::Bounds)?;
                    control.check(count as u64)?;
                    payload
                        .bytes
                        .get_mut(to..to.checked_add(count).ok_or(BackingError::Bounds)?)
                        .ok_or(BackingError::Length)?
                        .copy_from_slice(pin.bytes().get(from..from.checked_add(count).ok_or(BackingError::Bounds)?).ok_or(BackingError::Integrity)?);
                }
            }
        }
        control.check(1)?;
        Ok(TilePin { payload: Arc::new(payload) })
    }

    pub(super) fn write_translated_region(&mut self, bounds: Rect, bytes: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if bounds.is_empty() || !self.descriptor.bounds().contains_rect(&bounds) {
            return Err(BackingError::Bounds);
        }
        let source =
            shifted(bounds, self.view_offset.0.checked_neg().ok_or(BackingError::Bounds)?, self.view_offset.1.checked_neg().ok_or(BackingError::Bounds)?)?;
        let mut root = self.root_view();
        root.write_region(source, bytes, control)?;
        // The root operation has completed every fallible stage and final poll.
        self.overrides = root.overrides;
        Ok(())
    }
}

#[cfg(test)]
#[path = "translated_tests.rs"]
mod tests;
