//! Private store accepts only the moved output of this staged importer.
use super::*;
use photocraft_geom::TileCoord;
use photocraft_raster::backing::{NativePlaneDescriptor, NativePlaneStore, NativeTileVisitor};
pub(super) struct SurfaceStore {
    surface: Surface,
    descriptor: NativePlaneDescriptor,
    retention: Arc<Retention>,
    quota: BackingQuota,
}
impl std::fmt::Debug for SurfaceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedXcfSurfaceStore").field("descriptor", &self.descriptor).finish_non_exhaustive()
    }
}
pub(super) fn validate(
    surface: &Surface,
    bounds: Rect,
    c: &ReadControl<'_>,
    poll: &dyn Fn() -> Result<(), BackingError>,
) -> Result<NativePlaneDescriptor, BackingError> {
    poll()?;
    c.check(1)?;
    let descriptor = NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes())?;
    let bpp = surface.format().bytes_per_pixel();
    let expected = 65536usize.checked_mul(bpp).ok_or(BackingError::Length)?;
    for (coord, tile) in surface.tiles() {
        poll()?;
        c.check(1)?;
        let x = coord.tx.checked_mul(256).ok_or(BackingError::Bounds)?;
        let y = coord.ty.checked_mul(256).ok_or(BackingError::Bounds)?;
        let tile_bounds = Rect::new(x, y, x.checked_add(256).ok_or(BackingError::Bounds)?, y.checked_add(256).ok_or(BackingError::Bounds)?);
        if tile.bytes().len() != expected || tile_bounds.intersect(&bounds).is_empty() {
            return Err(BackingError::Integrity);
        }
        for (row, bytes) in tile.bytes().chunks_exact(256 * bpp).enumerate() {
            poll()?;
            c.check(bytes.len() as u64)?;
            let py = y.checked_add(i32::try_from(row).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
            for (column, pixel) in bytes.chunks_exact(bpp).enumerate() {
                let px = x.checked_add(i32::try_from(column).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
                if !bounds.contains(px, py) && pixel != descriptor.default_bytes() {
                    return Err(BackingError::Integrity);
                }
            }
        }
    }
    poll()?;
    c.check(1)?;
    Ok(descriptor)
}
impl SurfaceStore {
    pub(super) fn new(surface: Surface, descriptor: NativePlaneDescriptor, retention: Arc<Retention>, quota: BackingQuota) -> Self {
        Self { surface, descriptor, retention, quota }
    }
}
impl NativePlaneStore for SurfaceStore {
    fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    fn validate_quota(&self, q: &BackingQuota) -> Result<(), BackingError> {
        if self.quota.shares_ledger(q) && self.retention.quota.shares_ledger(q) { Ok(()) } else { Err(BackingError::Quota) }
    }
    fn visit_native_tile(&self, coord: TileCoord, c: &ReadControl<'_>, stage: &mut NativeTileVisitor<'_>) -> Result<bool, BackingError> {
        c.check(1)?;
        let Some(tile) = self.surface.tile(coord) else { return Ok(true) };
        let mut offset = 0u64;
        for bytes in tile.bytes().chunks(65536) {
            c.check(bytes.len() as u64)?;
            stage(offset, bytes)?;
            offset = offset.checked_add(bytes.len() as u64).ok_or(BackingError::Length)?;
        }
        c.check(1)?;
        Ok(true)
    }
    fn read_exact_at(&self, offset: u64, out: &mut [u8], c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(1)?;
        if out.len() > 65536 || offset.checked_add(out.len() as u64).is_none_or(|n| n > self.descriptor.logical_bytes()) {
            return Err(BackingError::Bounds);
        }
        let bpp = self.descriptor.format().bytes_per_pixel();
        let width = u64::from(self.descriptor.bounds().width());
        let mut done = 0usize;
        let mut pixel = [0u8; 40];
        while done < out.len() {
            let at = offset.checked_add(done as u64).ok_or(BackingError::Bounds)?;
            let number = at / (bpp as u64);
            let first = (at % (bpp as u64)) as usize;
            let x = i64::from(self.descriptor.bounds().x0) + i64::try_from(number % width).map_err(|_| BackingError::Bounds)?;
            let y = i64::from(self.descriptor.bounds().y0) + i64::try_from(number / width).map_err(|_| BackingError::Bounds)?;
            let raw = pixel.get_mut(..bpp).ok_or(BackingError::Length)?;
            c.check(bpp as u64)?;
            self.surface
                .try_read_pixel_bytes(i32::try_from(x).map_err(|_| BackingError::Bounds)?, i32::try_from(y).map_err(|_| BackingError::Bounds)?, raw)
                .map_err(raster_error)?;
            let count = (bpp - first).min(out.len() - done);
            let end = done.checked_add(count).ok_or(BackingError::Length)?;
            out.get_mut(done..end).ok_or(BackingError::Length)?.copy_from_slice(raw.get(first..first + count).ok_or(BackingError::Length)?);
            done = end;
        }
        c.check(1)
    }
}
