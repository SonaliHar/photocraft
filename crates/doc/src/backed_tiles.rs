//! Descriptor-only finite tile enumeration, without pixel reads or allocation.
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::backing::{BackingError, NativePlaneDescriptor, ReadControl};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileEnumerationLimits {
    pub max_tiles: u64,
    pub max_index_bytes: u64,
    pub index_bytes_per_tile: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileEnumerationError {
    Bounds,
    TileLimit,
    IndexLimit,
    Control(BackingError),
}
impl std::fmt::Display for TileEnumerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "finite tile enumeration: {self:?}")
    }
}
impl std::error::Error for TileEnumerationError {}
/// Limits are prospective metadata admission estimates, not allocated-index
/// reservations. A writer must separately account its actual index capacities.
#[derive(Clone, Copy, Debug)]
pub struct BackedTilePlan {
    descriptor: NativePlaneDescriptor,
    min_tx: i32,
    min_ty: i32,
    columns: u64,
    count: u64,
    index_bytes: u64,
}
impl BackedTilePlan {
    pub fn new(descriptor: NativePlaneDescriptor, limits: TileEnumerationLimits, control: &ReadControl<'_>) -> Result<Self, TileEnumerationError> {
        control.check(1).map_err(TileEnumerationError::Control)?;
        let bounds = descriptor.bounds();
        if bounds.is_empty() {
            return Err(TileEnumerationError::Bounds);
        }
        let min_tx = bounds.x0.div_euclid(TILE_SIZE);
        let min_ty = bounds.y0.div_euclid(TILE_SIZE);
        let max_tx = bounds.x1.checked_sub(1).ok_or(TileEnumerationError::Bounds)?.div_euclid(TILE_SIZE);
        let max_ty = bounds.y1.checked_sub(1).ok_or(TileEnumerationError::Bounds)?.div_euclid(TILE_SIZE);
        for coordinate in [min_tx, min_ty, max_tx, max_ty] {
            validate_axis(coordinate)?;
        }
        let columns = u64::try_from(i64::from(max_tx).checked_sub(i64::from(min_tx)).and_then(|n| n.checked_add(1)).ok_or(TileEnumerationError::Bounds)?)
            .map_err(|_| TileEnumerationError::Bounds)?;
        let rows = u64::try_from(i64::from(max_ty).checked_sub(i64::from(min_ty)).and_then(|n| n.checked_add(1)).ok_or(TileEnumerationError::Bounds)?)
            .map_err(|_| TileEnumerationError::Bounds)?;
        let count = columns.checked_mul(rows).ok_or(TileEnumerationError::TileLimit)?;
        if count > limits.max_tiles {
            return Err(TileEnumerationError::TileLimit);
        }
        let index_bytes = count.checked_mul(limits.index_bytes_per_tile).ok_or(TileEnumerationError::IndexLimit)?;
        if index_bytes > limits.max_index_bytes {
            return Err(TileEnumerationError::IndexLimit);
        }
        control.check(1).map_err(TileEnumerationError::Control)?;
        Ok(Self { descriptor, min_tx, min_ty, columns, count, index_bytes })
    }
    pub fn descriptor(&self) -> NativePlaneDescriptor {
        self.descriptor
    }
    pub fn tile_count(&self) -> u64 {
        self.count
    }
    pub fn index_bytes(&self) -> u64 {
        self.index_bytes
    }
    /// Row-major order, x advancing first, including default-only tiles.
    /// No stored cancellation callback, pixel access, or heap allocation.
    pub fn iter<'c, 'a>(&self, control: &'c ReadControl<'a>) -> BackedTileIter<'c, 'a> {
        BackedTileIter { plan: *self, control, position: 0, error: None }
    }
}
pub struct BackedTileIter<'c, 'a> {
    plan: BackedTilePlan,
    control: &'c ReadControl<'a>,
    position: u64,
    error: Option<TileEnumerationError>,
}
impl BackedTileIter<'_, '_> {
    /// Errors remain errors on subsequent calls; failure is never converted to
    /// successful EOF. Even exhausted calls poll cancellation and charge work.
    pub fn next_tile(&mut self) -> Result<Option<TileCoord>, TileEnumerationError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let result = self.advance();
        if let Err(error) = result {
            self.error = Some(error);
        }
        result
    }
    fn advance(&mut self) -> Result<Option<TileCoord>, TileEnumerationError> {
        self.control.check(1).map_err(TileEnumerationError::Control)?;
        if self.position == self.plan.count {
            return Ok(None);
        }
        let x = self.position.checked_rem(self.plan.columns).ok_or(TileEnumerationError::Bounds)?;
        let y = self.position.checked_div(self.plan.columns).ok_or(TileEnumerationError::Bounds)?;
        let tx = axis_at(self.plan.min_tx, x)?;
        let ty = axis_at(self.plan.min_ty, y)?;
        validate_axis(tx)?;
        validate_axis(ty)?;
        self.position = self.position.checked_add(1).ok_or(TileEnumerationError::Bounds)?;
        Ok(Some(TileCoord::new(tx, ty)))
    }
}
fn axis_at(start: i32, offset: u64) -> Result<i32, TileEnumerationError> {
    let offset = i64::try_from(offset).map_err(|_| TileEnumerationError::Bounds)?;
    i32::try_from(i64::from(start).checked_add(offset).ok_or(TileEnumerationError::Bounds)?).map_err(|_| TileEnumerationError::Bounds)
}
fn validate_axis(coordinate: i32) -> Result<(), TileEnumerationError> {
    coordinate.checked_mul(TILE_SIZE).ok_or(TileEnumerationError::Bounds)?;
    coordinate.checked_add(1).and_then(|n| n.checked_mul(TILE_SIZE)).ok_or(TileEnumerationError::Bounds)?;
    Ok(())
}
