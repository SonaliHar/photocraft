//! Immutable owned import snapshots and checked, lazily materialized COW tiles.
use super::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, Tile, checked_region_len, pixel_rect};
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use std::sync::Arc;

/// The bytes are owned by this snapshot; subsequent changes to the original file
/// cannot change reads. Clones retain the same backing allocation.
#[derive(Clone, Debug)]
pub struct OwnedRasterSnapshot {
    format: PixelFormat,
    bounds: Rect,
    bytes: Arc<Vec<u8>>,
    default: Arc<Vec<u8>>,
    resident_limit: u64,
    retained_bytes: u64,
}

impl OwnedRasterSnapshot {
    /// Adopt already-reserved native-endian interleaved bytes without copying them.
    /// The caller must charge source construction to its aggregate import budget.
    pub fn try_new(format: PixelFormat, bounds: Rect, bytes: Vec<u8>, default: Vec<u8>, resident_limit: u64) -> Result<Self, RasterInputError> {
        let length = checked_region_len(bounds, format.bytes_per_pixel())?;
        if bytes.len() != length || default.len() != format.bytes_per_pixel() {
            return Err(RasterInputError::LengthMismatch);
        }
        let resident_limit = resident_limit.min(MAX_CHECKED_RASTER_BYTES);
        let retained =
            (bytes.capacity() as u64).checked_add(default.capacity() as u64).and_then(|n| n.checked_add(128)).ok_or(RasterInputError::LimitExceeded)?;
        if retained > resident_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(Self { format, bounds, bytes: Arc::new(bytes), default: Arc::new(default), resident_limit, retained_bytes: retained })
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn default_bytes(&self) -> &[u8] {
        &self.default
    }
    pub fn resident_bytes(&self) -> u64 {
        self.retained_bytes
    }
    pub fn shares_backing_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes)
    }

    pub fn try_read_pixel_bytes(&self, x: i32, y: i32, output: &mut [u8]) -> Result<(), RasterInputError> {
        checked_region_len(pixel_rect(x, y)?, self.format.bytes_per_pixel())?;
        if output.len() != self.format.bytes_per_pixel() {
            return Err(RasterInputError::LengthMismatch);
        }
        if x < self.bounds.x0 || x >= self.bounds.x1 || y < self.bounds.y0 || y >= self.bounds.y1 {
            output.copy_from_slice(self.default_bytes());
            return Ok(());
        }
        let width = usize::try_from(i64::from(self.bounds.x1) - i64::from(self.bounds.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let row = usize::try_from(i64::from(y) - i64::from(self.bounds.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let col = usize::try_from(i64::from(x) - i64::from(self.bounds.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let start = row.checked_mul(width).and_then(|v| v.checked_add(col)).and_then(|v| v.checked_mul(output.len())).ok_or(RasterInputError::LimitExceeded)?;
        let end = start.checked_add(output.len()).ok_or(RasterInputError::LimitExceeded)?;
        output.copy_from_slice(self.bytes.get(start..end).ok_or(RasterInputError::LengthMismatch)?);
        Ok(())
    }

    /// Materialize only this tile, charging its buffer alongside retained backing.
    pub fn try_read_tile(&self, coord: TileCoord, interrupt: &Interrupt<'_>) -> Result<Tile, RasterInputError> {
        let rect = tile_rect(coord)?;
        let length = checked_region_len(rect, self.format.bytes_per_pixel())?;
        self.check_reservation(length as u64)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(length, 0);
        let bpp = self.format.bytes_per_pixel();
        let mut cursor = 0;
        for y in rect.y0..rect.y1 {
            if interrupt.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            for x in rect.x0..rect.x1 {
                let end = cursor + bpp;
                self.try_read_pixel_bytes(x, y, bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?)?;
                cursor = end;
            }
        }
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        Ok(Tile { data: bytes.into_boxed_slice() })
    }

    fn check_reservation(&self, extra: u64) -> Result<(), RasterInputError> {
        if self.resident_bytes().checked_add(extra).ok_or(RasterInputError::LimitExceeded)? > self.resident_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        Ok(())
    }
}

/// Editable lazy raster. Untouched pixels read directly from the immutable snapshot;
/// edits materialize complete affected tiles and share them in history clones.
/// Use `try_materialize` at a legacy Surface consumer or persistence boundary.
#[derive(Clone, Debug)]
pub struct SnapshotSurface {
    snapshot: OwnedRasterSnapshot,
    overrides: Surface,
}

impl SnapshotSurface {
    pub fn try_new(snapshot: OwnedRasterSnapshot) -> Result<Self, RasterInputError> {
        let overrides = Surface::try_with_default_bytes(snapshot.format(), snapshot.default_bytes())?;
        snapshot.check_reservation(overrides.default_bytes().len() as u64)?;
        Ok(Self { snapshot, overrides })
    }
    pub fn snapshot(&self) -> &OwnedRasterSnapshot {
        &self.snapshot
    }
    pub fn materialized_tile_count(&self) -> usize {
        self.overrides.tile_count()
    }
    /// Conservative retained payload accounting; shared history tiles may be counted
    /// again by an aggregate caller, but are never omitted from this local bound.
    pub fn resident_bytes(&self) -> u64 {
        self.try_resident_bytes().unwrap_or(u64::MAX)
    }
    pub fn try_resident_bytes(&self) -> Result<u64, RasterInputError> {
        self.snapshot.resident_bytes().checked_add(self.overrides.resident_accounted_bytes()?).ok_or(RasterInputError::LimitExceeded)
    }
    pub fn try_read_pixel_bytes(&self, x: i32, y: i32, output: &mut [u8]) -> Result<(), RasterInputError> {
        let (coord, _) = self.overrides.locate(x, y);
        if self.overrides.tile(coord).is_some() { self.overrides.try_read_pixel_bytes(x, y, output) } else { self.snapshot.try_read_pixel_bytes(x, y, output) }
    }
    pub fn try_write_interleaved(&mut self, bounds: Rect, bytes: &[u8], interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
        let expected = checked_region_len(bounds, self.snapshot.format().bytes_per_pixel())?;
        if bytes.len() != expected {
            return Err(RasterInputError::LengthMismatch);
        }
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let tile_len = (TILE_SIZE as u64)
            .checked_mul(TILE_SIZE as u64)
            .and_then(|v| v.checked_mul(self.snapshot.format().bytes_per_pixel() as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        let tiles = bounds.tiles().count() as u64;
        // Staging retains old COW tiles, prepared source tiles, and replacement tiles.
        let extra = tiles
            .checked_mul(tile_len.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|v| v.checked_mul(2))
            .ok_or(RasterInputError::LimitExceeded)?;
        let peak = self
            .try_resident_bytes()?
            .checked_add(extra)
            .and_then(|v| v.checked_add(bytes.len() as u64))
            .and_then(|v| v.checked_add((self.overrides.default_bytes().len() as u64).checked_mul(2)?))
            .and_then(|v| v.checked_add((self.overrides.tile_count() as u64).checked_mul(512)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        if peak > self.snapshot.resident_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut staged = self.overrides.clone();
        for coord in bounds.tiles() {
            if staged.tile(coord).is_none() {
                staged.tiles.insert(coord, Arc::new(self.snapshot.try_read_tile(coord, interrupt)?));
            }
        }
        staged.try_write_interleaved_with(bounds, bytes, interrupt)?;
        self.overrides = staged;
        Ok(())
    }
    pub fn try_materialize(&self, interrupt: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let tile_len = (TILE_SIZE as u64)
            .checked_mul(TILE_SIZE as u64)
            .and_then(|v| v.checked_mul(self.snapshot.format().bytes_per_pixel() as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        let count = self.snapshot.bounds().tiles().filter(|c| self.overrides.tile(*c).is_none()).count() as u64;
        let extra = count
            .checked_mul(tile_len.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|v| v.checked_add(self.overrides.default_bytes().len() as u64))
            .and_then(|v| v.checked_add((self.overrides.tile_count() as u64).checked_mul(256)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        if self.try_resident_bytes()?.checked_add(extra).ok_or(RasterInputError::LimitExceeded)? > self.snapshot.resident_limit {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut out = self.overrides.clone();
        for coord in self.snapshot.bounds().tiles() {
            if out.tile(coord).is_none() {
                out.tiles.insert(coord, Arc::new(self.snapshot.try_read_tile(coord, interrupt)?));
            }
        }
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        Ok(out)
    }
}

fn tile_rect(coord: TileCoord) -> Result<Rect, RasterInputError> {
    let x = coord.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
    let y = coord.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
    let x1 = x.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
    let y1 = y.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
    Ok(Rect::new(x, y, x1, y1))
}
