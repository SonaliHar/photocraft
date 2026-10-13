//! Private construction from complete, disjoint 64-pixel source-grid cells.
//!
//! All requested Vec/default/tile capacities are reserved fallibly under a caller lease.
//! Publication into the existing Surface BTreeMap/Arc storage is bounded and prepaid,
//! but those standard-library node/Arc allocations have no stable fallible API. This
//! foundation does not claim complete allocator control or process RSS accounting.
use crate::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, Tile, checked_region_byte_len};
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use std::{collections::BTreeMap, mem::size_of, sync::Arc};

pub const SOURCE_CELL_SIZE: i32 = 64;
const MAP_ARC_BYTES_PER_TILE: u64 = 256;

/// Shared job callbacks. The lease must refund only when dropped; work denial must
/// propagate. A retained result transfers this lease with its complete Surface.
pub trait StagingBudget {
    type Lease;
    fn reserve(&mut self, bytes: u64) -> Result<Self::Lease, RasterInputError>;
    /// Encoded bytes initialized/copied, plus descriptor/bitmap/publication entries.
    /// This is additional staging work, not a second decoded-sample charge.
    fn work(&mut self, units: u64) -> Result<(), RasterInputError>;
}

/// A complete Surface and its still-live conservative allocation lease.
/// Field order ensures raster storage is destroyed before the lease refunds.
pub struct StagedSurface<L> {
    surface: Surface,
    lease: L,
}
impl<L> StagedSurface<L> {
    pub fn surface(&self) -> &Surface {
        &self.surface
    }
    /// Transfers both owners. The caller must retain the lease until all transferred
    /// raster storage is destroyed or charged to another owner; no implicit refund.
    pub fn into_parts(self) -> (Surface, L) {
        (self.surface, self.lease)
    }
}

/// Never publishes partial pixels. Every input cell must lie on the declared local
/// 64-grid, including clipped edge cells. Raw native bytes/defaults are copied without
/// color, alpha, finite-sample, or coverage normalization; validation is the caller's job.
pub struct StagedSurfaceBuilder<L> {
    format: PixelFormat,
    bounds: Rect,
    origin: TileCoord,
    destination_columns: usize,
    source_columns: usize,
    source_cells: usize,
    completed: usize,
    allocated: usize,
    tiles: Vec<Option<Vec<u8>>>,
    default: Vec<u8>,
    coverage: Vec<u8>,
    // Last: all staging allocations drop before their refund.
    lease: Option<L>,
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if ctl.cancelled() { Err(RasterInputError::Cancelled) } else { Ok(()) }
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn reserve_exact<T>(v: &mut Vec<T>, count: usize) -> Result<(), RasterInputError> {
    v.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    if v.capacity() != count {
        return Err(RasterInputError::AllocationFailed);
    }
    Ok(())
}
impl<L> StagedSurfaceBuilder<L> {
    pub fn new<B: StagingBudget<Lease = L>>(
        format: PixelFormat,
        default: &[u8],
        bounds: Rect,
        budget: &mut B,
        ctl: &Interrupt<'_>,
    ) -> Result<Self, RasterInputError> {
        poll(ctl)?;
        if format.bytes_per_pixel() == 0 {
            return Err(RasterInputError::UnsupportedConversion);
        }
        checked_region_byte_len(bounds, format)?;
        if bounds.is_empty() {
            return Err(RasterInputError::InvalidBounds);
        }
        if default.len() != format.bytes_per_pixel() {
            return Err(RasterInputError::LengthMismatch);
        }
        let origin = TileCoord::containing(bounds.x0, bounds.y0);
        let last = TileCoord::containing(
            bounds.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
            bounds.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        );
        let dc = usize::try_from(i64::from(last.tx) - i64::from(origin.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
        let dr = usize::try_from(i64::from(last.ty) - i64::from(origin.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
        let count = dc.checked_mul(dr).ok_or(RasterInputError::LimitExceeded)?;
        let sc = usize::try_from(bounds.width().div_ceil(64)).map_err(|_| RasterInputError::InvalidBounds)?;
        let sr = usize::try_from(bounds.height().div_ceil(64)).map_err(|_| RasterInputError::InvalidBounds)?;
        let cells = sc.checked_mul(sr).ok_or(RasterInputError::LimitExceeded)?;
        let coverage_bytes = cells.checked_add(7).ok_or(RasterInputError::LimitExceeded)? / 8;
        let tile_bytes = mul((TILE_SIZE * TILE_SIZE) as u64, format.bytes_per_pixel() as u64)?;
        let retained = mul(count as u64, add(tile_bytes, MAP_ARC_BYTES_PER_TILE)?)?;
        let requested = add(add(retained, mul(count as u64, size_of::<Option<Vec<u8>>>() as u64)?)?, add(default.len() as u64, coverage_bytes as u64)?)?;
        if requested > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        let lease = budget.reserve(requested)?;
        let mut out = Self {
            format,
            bounds,
            origin,
            destination_columns: dc,
            source_columns: sc,
            source_cells: cells,
            completed: 0,
            allocated: 0,
            tiles: Vec::new(),
            default: Vec::new(),
            coverage: Vec::new(),
            lease: Some(lease),
        };
        reserve_exact(&mut out.tiles, count)?;
        // No initialization/scan runs for more than64 entries without a poll/work charge.
        for start in (0..count).step_by(64) {
            poll(ctl)?;
            let n = (count - start).min(64);
            budget.work(n as u64)?;
            out.tiles.extend((0..n).map(|_| None));
        }
        reserve_exact(&mut out.default, default.len())?;
        poll(ctl)?;
        budget.work(default.len() as u64)?;
        out.default.extend_from_slice(default);
        reserve_exact(&mut out.coverage, coverage_bytes)?;
        for start in (0..coverage_bytes).step_by(64) {
            poll(ctl)?;
            let n = (coverage_bytes - start).min(64);
            budget.work(n as u64)?;
            out.coverage.resize(out.coverage.len() + n, 0);
        }
        poll(ctl)?;
        Ok(out)
    }
    pub fn allocated_tile_count(&self) -> usize {
        self.allocated
    }
    fn abort(&mut self) {
        self.tiles = Vec::new();
        self.default = Vec::new();
        self.coverage = Vec::new();
        self.allocated = 0;
        self.lease = None;
    }
    pub fn push_region<B: StagingBudget<Lease = L>>(
        &mut self,
        region: Rect,
        bytes: &[u8],
        budget: &mut B,
        ctl: &Interrupt<'_>,
    ) -> Result<(), RasterInputError> {
        let result = self.copy_region(region, bytes, budget, ctl);
        if result.is_err() {
            self.abort();
        }
        result
    }
    fn copy_region<B: StagingBudget<Lease = L>>(&mut self, r: Rect, bytes: &[u8], budget: &mut B, ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
        poll(ctl)?;
        if self.lease.is_none() || !self.bounds.contains_rect(&r) || r.is_empty() {
            return Err(RasterInputError::InvalidBounds);
        }
        let lx = r.x0 - self.bounds.x0;
        let ly = r.y0 - self.bounds.y0;
        if lx % SOURCE_CELL_SIZE != 0
            || ly % SOURCE_CELL_SIZE != 0
            || r.x1 != (r.x0 + SOURCE_CELL_SIZE).min(self.bounds.x1)
            || r.y1 != (r.y0 + SOURCE_CELL_SIZE).min(self.bounds.y1)
        {
            return Err(RasterInputError::InvalidBounds);
        }
        if checked_region_byte_len(r, self.format)? != bytes.len() {
            return Err(RasterInputError::LengthMismatch);
        }
        let cell = (ly as usize / 64).checked_mul(self.source_columns).and_then(|v| v.checked_add(lx as usize / 64)).ok_or(RasterInputError::LimitExceeded)?;
        let mask = 1_u8 << (cell % 8);
        if self.coverage.get(cell / 8).ok_or(RasterInputError::InvalidBounds)? & mask != 0 {
            return Err(RasterInputError::InvalidBounds);
        }
        let bpp = self.format.bytes_per_pixel();
        for tc in r.tiles() {
            let row = usize::try_from(tc.ty - self.origin.ty).map_err(|_| RasterInputError::InvalidBounds)?;
            let col = usize::try_from(tc.tx - self.origin.tx).map_err(|_| RasterInputError::InvalidBounds)?;
            let index = row.checked_mul(self.destination_columns).and_then(|v| v.checked_add(col)).ok_or(RasterInputError::LimitExceeded)?;
            let slot = self.tiles.get_mut(index).ok_or(RasterInputError::InvalidBounds)?;
            if slot.is_none() {
                let tile_bytes =
                    (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|v| v.checked_mul(bpp)).ok_or(RasterInputError::LimitExceeded)?;
                let mut raw = Vec::new();
                reserve_exact(&mut raw, tile_bytes)?;
                for _ in 0..(TILE_SIZE * TILE_SIZE / 64) {
                    poll(ctl)?;
                    budget.work(64 * bpp as u64)?;
                    for _ in 0..64 {
                        raw.extend_from_slice(&self.default);
                    }
                }
                *slot = Some(raw);
                self.allocated = self.allocated.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
            }
            let raw = slot.as_mut().ok_or(RasterInputError::InvalidBounds)?;
            let part = tc.rect().intersect(&r);
            for y in part.y0..part.y1 {
                poll(ctl)?;
                let width = (part.x1 - part.x0) as usize;
                budget.work((width * bpp) as u64)?;
                let src = (((y - r.y0) as usize) * r.width() as usize + (part.x0 - r.x0) as usize).checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                let dst = (((y - tc.ty * TILE_SIZE) as usize) * TILE_SIZE as usize + (part.x0 - tc.tx * TILE_SIZE) as usize)
                    .checked_mul(bpp)
                    .ok_or(RasterInputError::LimitExceeded)?;
                let len = width.checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                raw.get_mut(dst..dst.checked_add(len).ok_or(RasterInputError::LimitExceeded)?)
                    .ok_or(RasterInputError::LengthMismatch)?
                    .copy_from_slice(bytes.get(src..src.checked_add(len).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?);
            }
        }
        *self.coverage.get_mut(cell / 8).ok_or(RasterInputError::InvalidBounds)? |= mask;
        self.completed = self.completed.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    pub fn finalize<B: StagingBudget<Lease = L>>(mut self, budget: &mut B, ctl: &Interrupt<'_>) -> Result<StagedSurface<L>, RasterInputError> {
        poll(ctl)?;
        if self.lease.is_none() || self.completed != self.source_cells {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut map = BTreeMap::new();
        for (index, raw) in std::mem::take(&mut self.tiles).into_iter().enumerate() {
            poll(ctl)?;
            budget.work(1)?;
            let raw = raw.ok_or(RasterInputError::LengthMismatch)?;
            let x = self
                .origin
                .tx
                .checked_add(i32::try_from(index % self.destination_columns).map_err(|_| RasterInputError::InvalidBounds)?)
                .ok_or(RasterInputError::InvalidBounds)?;
            let y = self
                .origin
                .ty
                .checked_add(i32::try_from(index / self.destination_columns).map_err(|_| RasterInputError::InvalidBounds)?)
                .ok_or(RasterInputError::InvalidBounds)?;
            // Bounded prepaid std allocations; these two operations are not fallible.
            map.insert(TileCoord::new(x, y), Arc::new(Tile { data: raw.into_boxed_slice() }));
        }
        poll(ctl)?;
        let surface = Surface { format: self.format, default_pixel: std::mem::take(&mut self.default).into_boxed_slice(), tiles: map };
        let lease = self.lease.take().ok_or(RasterInputError::InvalidBounds)?;
        Ok(StagedSurface { surface, lease })
    }
}
