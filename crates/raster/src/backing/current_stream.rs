//! Transactional bounded row-major current-native byte access for future writers.
//! This reads exact COW storage, not immutable source bytes or display samples.
use super::{BackedRaster, BackingError, ReadControl, allocate, pixel_offset, tile_rect};
use photocraft_geom::Rect;

pub const MAX_CURRENT_READ_BYTES: usize = 64 * 1024;

impl BackedRaster {
    /// Exact native-endian interleaved bytes within the operative logical plane.
    /// Offset is relative to descriptor.bounds()'s top-left pixel. Partial samples
    /// are valid. Empty reads at EOF succeed; beyond EOF/overflow are errors.
    /// Caller output is unchanged on every failure, including final cancellation.
    /// No supplied alternative ledger: staging and pins share this raster's quota.
    /// Cache occupancy may change on failure; it is derived state and remains charged.
    pub fn read_current_exact_at(&self, offset: u64, output: &mut [u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if output.len() > MAX_CURRENT_READ_BYTES {
            return Err(BackingError::Length);
        }
        let end = offset.checked_add(output.len() as u64).ok_or(BackingError::Bounds)?;
        if end > self.descriptor().logical_bytes() {
            return Err(BackingError::Bounds);
        }
        if output.is_empty() {
            return control.check(0);
        }
        // allocate() prepays/reconciles actual Vec capacity and a fixed128B
        // working-owner envelope. Payload bytes always drop before its lease.
        // At most three inline rectangles; no tile-index/pin-all allocation.
        control.check(output.len() as u64 + 1)?;
        let mut staged = allocate(self.quota(), output.len())?;
        let descriptor = self.descriptor();
        let bounds = descriptor.bounds();
        let bpp = descriptor.format().bytes_per_pixel();
        let row_bytes = u64::from(bounds.width()).checked_mul(bpp as u64).ok_or(BackingError::Bounds)?;
        for part in request_rectangles(bounds, bpp, offset, end)? {
            control.check(16)?;
            for coord in part.tiles() {
                let rect = tile_rect(coord)?;
                let pin = self.pin_tile(coord, control)?;
                let overlap = rect.intersect(&part);
                for y in overlap.y0..overlap.y1 {
                    let row = u64::try_from(i64::from(y) - i64::from(bounds.y0)).map_err(|_| BackingError::Bounds)?;
                    let col = u64::try_from(i64::from(overlap.x0) - i64::from(bounds.x0)).map_err(|_| BackingError::Bounds)?;
                    let first =
                        row.checked_mul(row_bytes).and_then(|n| col.checked_mul(bpp as u64).and_then(|c| n.checked_add(c))).ok_or(BackingError::Bounds)?;
                    let span = u64::from(overlap.width()).checked_mul(bpp as u64).ok_or(BackingError::Bounds)?;
                    let lo = first.max(offset);
                    let hi = first.checked_add(span).ok_or(BackingError::Bounds)?.min(end);
                    if lo >= hi {
                        continue;
                    }
                    let count = usize::try_from(hi - lo).map_err(|_| BackingError::Length)?;
                    let source = pixel_offset(rect, overlap.x0, y, bpp)?
                        .checked_add(usize::try_from(lo - first).map_err(|_| BackingError::Length)?)
                        .ok_or(BackingError::Bounds)?;
                    let source_end = source.checked_add(count).ok_or(BackingError::Bounds)?;
                    let target = usize::try_from(lo - offset).map_err(|_| BackingError::Length)?;
                    let target_end = target.checked_add(count).ok_or(BackingError::Bounds)?;
                    control.check(count as u64)?;
                    staged
                        .bytes
                        .get_mut(target..target_end)
                        .ok_or(BackingError::Length)?
                        .copy_from_slice(pin.bytes().get(source..source_end).ok_or(BackingError::Integrity)?);
                }
            }
        }
        // Charge the final bounded copy and check cancellation before publishing.
        // This <=64KiB copy is the only caller-output mutation.
        control.check(output.len() as u64)?;
        output.copy_from_slice(&staged.bytes);
        Ok(())
    }
}

fn coordinate(origin: i32, index: u64) -> Result<i32, BackingError> {
    let value = i64::from(origin).checked_add(i64::try_from(index).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
    i32::try_from(value).map_err(|_| BackingError::Bounds)
}
/// Three disjoint pixel rectangles cover a contiguous byte interval exactly.
/// Splitting avoids scanning a giant bounding hole when a read crosses the last
/// pixels of one enormous row and the first pixels of the next. Each rectangle
/// visits tiles once; shared boundary tiles may be pinned at most three times.
fn request_rectangles(bounds: Rect, bpp: usize, start: u64, end: u64) -> Result<[Rect; 3], BackingError> {
    let width = u64::from(bounds.width());
    if width == 0 || bpp == 0 || start >= end {
        return Err(BackingError::Bounds);
    }
    let first = start / (bpp as u64);
    let last = end.checked_sub(1).ok_or(BackingError::Bounds)? / (bpp as u64);
    let row0 = first / width;
    let row1 = last / width;
    let x0 = coordinate(bounds.x0, first % width)?;
    let x1 = coordinate(bounds.x0, (last % width).checked_add(1).ok_or(BackingError::Bounds)?)?;
    let y0 = coordinate(bounds.y0, row0)?;
    let y1 = coordinate(bounds.y0, row1)?;
    let after0 = y0.checked_add(1).ok_or(BackingError::Bounds)?;
    let after1 = y1.checked_add(1).ok_or(BackingError::Bounds)?;
    let empty = Rect::new(0, 0, 0, 0);
    if row0 == row1 {
        return Ok([Rect::new(x0, y0, x1, after0), empty, empty]);
    }
    Ok([Rect::new(x0, y0, bounds.x1, after0), Rect::new(bounds.x0, after0, bounds.x1, y1), Rect::new(bounds.x0, y1, x1, after1)])
}
