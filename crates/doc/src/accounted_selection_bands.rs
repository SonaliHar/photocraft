//! Charged, private GRAY8 construction without a whole logical-image buffer.
//! The standard map/Arc envelopes follow the parent module's explicit allocator
//! boundary; native tile/default sizes are actual. Borrowed input bands remain
//! caller-owned. No external Surface or asserted lease can become an owner.
use super::*;

/// A one-shot zero-default Gray8 canvas. Bands cover its complete width in
/// strictly increasing contiguous row order. Any failed push poisons this owner;
/// only a complete successful canvas can become an AccountedSelection.
pub struct AccountedSelectionBandBuilder {
    // Native raster storage must be destroyed before its physical reservations.
    surface: Surface,
    canvas: Rect,
    next_y: i32,
    failed: bool,
    quota: BackingQuota,
    payload: QuotaLease,
    index: QuotaLease,
    allocation: QuotaLease,
    scratch: QuotaLease,
}
impl std::fmt::Debug for AccountedSelectionBandBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountedSelectionBandBuilder")
            .field("canvas", &self.canvas)
            .field("next_y", &self.next_y)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}
impl AccountedSelectionBandBuilder {
    pub fn new(canvas: Rect, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        let count = bounds(canvas)?;
        let length = mul(u64::from(canvas.width()), u64::from(canvas.height()))?;
        if length > MAX_CHECKED_RASTER_BYTES {
            return Err(BackingError::Length);
        }
        let tile_bytes = 65536;
        let payload = quota.reserve(QuotaKind::Resident, add(mul(count, tile_bytes)?, 1)?).map_err(|_| BackingError::Quota)?;
        let index_bytes = add(mul(count, add(1024, std::mem::size_of::<Tile>() as u64 + 128)?)?, std::mem::size_of::<SelectionOwner>() as u64)?;
        let index = quota.reserve(QuotaKind::Index, index_bytes).map_err(|_| BackingError::Quota)?;
        // Builder is consumed, not duplicated. Keep the conservative actual inline
        // builder + final owner and both Arc headers paid across the transition.
        let header = (std::mem::size_of::<Self>() + std::mem::size_of::<SelectionOwner>() + std::mem::size_of::<QuotaLease>() + 256) as u64;
        let allocation = quota.reserve(QuotaKind::Index, header).map_err(|_| BackingError::Quota)?;
        // try_tile_mut uses the existing fallible native Vec->Box constructor.
        // Its private fill is bounded by one 64KiB tile. Reserve transient native
        // preparation headroom before the first allocation, not per-band maxima.
        let scratch = quota.reserve(QuotaKind::Scratch, add(mul(tile_bytes, 4)?, 4097)?).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let surface = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[0]).map_err(raster)?;
        Ok(Self { surface, canvas, next_y: canvas.y0, failed: false, quota: quota.clone(), payload, index, allocation, scratch })
    }
    pub fn push_band(&mut self, band: Rect, native_gray8: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.failed {
            return Err(BackingError::Integrity);
        }
        self.failed = true;
        let result = self.push_inner(band, native_gray8, control);
        if result.is_ok() {
            self.failed = false;
        }
        result
    }
    fn push_inner(&mut self, band: Rect, native_gray8: &[u8], control: &ReadControl<'_>) -> Result<(), BackingError> {
        control.check(1)?;
        if band.x0 != self.canvas.x0 || band.x1 != self.canvas.x1 || band.y0 != self.next_y || band.y1 <= band.y0 || band.y1 > self.canvas.y1 {
            return Err(BackingError::Bounds);
        }
        let width = usize::try_from(band.width()).map_err(|_| BackingError::Length)?;
        let length = width.checked_mul(usize::try_from(band.height()).map_err(|_| BackingError::Length)?).ok_or(BackingError::Length)?;
        if native_gray8.len() != length {
            return Err(BackingError::Length);
        }
        for coordinate in band.tiles() {
            control.check(1024)?;
            let tile_rect = coordinate.rect();
            let part = tile_rect.intersect(&band);
            let span = usize::try_from(part.width()).map_err(|_| BackingError::Length)?;
            for y in part.y0..part.y1 {
                // Each checked source/copy segment is at most256bytes. The map
                // lookup allowance and scan/copy work are debited before touching it.
                control.check(add(mul(span as u64, 2)?, 1024)?)?;
                let row = usize::try_from(i64::from(y) - i64::from(band.y0)).map_err(|_| BackingError::Bounds)?;
                let column = usize::try_from(i64::from(part.x0) - i64::from(band.x0)).map_err(|_| BackingError::Bounds)?;
                let start = row.checked_mul(width).and_then(|v| v.checked_add(column)).ok_or(BackingError::Length)?;
                let end = start.checked_add(span).ok_or(BackingError::Length)?;
                let source = native_gray8.get(start..end).ok_or(BackingError::Length)?;
                // Complete disjoint rows have never previously written these
                // pixels; zero input remains the exact default without a tile.
                if !source.iter().any(|v| *v != 0) {
                    continue;
                }
                if self.surface.tile(coordinate).is_none() {
                    control.check(2 * 65536)?;
                }
                let tile = self.surface.try_tile_mut(coordinate).map_err(|_| BackingError::Allocation)?;
                if tile.bytes().len() != 65536 {
                    return Err(BackingError::Integrity);
                }
                let row = usize::try_from(i64::from(y) - i64::from(tile_rect.y0)).map_err(|_| BackingError::Bounds)?;
                let column = usize::try_from(i64::from(part.x0) - i64::from(tile_rect.x0)).map_err(|_| BackingError::Bounds)?;
                let target = row.checked_mul(256).and_then(|v| v.checked_add(column)).ok_or(BackingError::Length)?;
                let stop = target.checked_add(span).ok_or(BackingError::Length)?;
                tile.bytes_mut().get_mut(target..stop).ok_or(BackingError::Length)?.copy_from_slice(source);
            }
        }
        control.check(1)?;
        self.next_y = band.y1;
        Ok(())
    }
    /// None is the upstream sparse empty-selection result. No raw Surface escapes.
    pub fn finish(mut self, control: &ReadControl<'_>) -> Result<Option<AccountedSelection>, BackingError> {
        control.check(1)?;
        if self.failed || self.next_y != self.canvas.y1 {
            return Err(BackingError::Integrity);
        }
        if self.surface.tile_count() == 0 {
            return Ok(None);
        }
        // All temporary constructor storage is gone after each push. Reconcile
        // sparse retained capacity while the ordered owner still guarantees its
        // raster drops before leases on every fallible path.
        let tiles = self.surface.tile_count() as u64;
        self.payload.reconcile_capacity(add(mul(tiles, 65536)?, 1)?).map_err(|_| BackingError::Quota)?;
        self.index
            .reconcile_capacity(add(mul(tiles, add(1024, std::mem::size_of::<Tile>() as u64 + 128)?)?, std::mem::size_of::<SelectionOwner>() as u64)?)
            .map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Self { surface, canvas, quota, payload, index, allocation, scratch, .. } = self;
        drop(scratch);
        Ok(Some(AccountedSelection {
            owner: Arc::new(SelectionOwner { surface, bounds: canvas, quota, _payload: payload, _index: index }),
            _allocation: Arc::new(allocation),
        }))
    }
}
#[cfg(test)]
#[path = "accounted_selection_bands_tests.rs"]
mod tests;
