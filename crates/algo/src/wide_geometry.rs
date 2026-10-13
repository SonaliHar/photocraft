//! Checked geometry in authoritative raw storage. No f32 display conversion.
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct GeometryLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for GeometryLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrthogonalTransform {
    FlipHorizontal,
    FlipVertical,
    RotateClockwise,
    RotateCounterclockwise,
    Rotate180,
}

/// Transform only `bounds` into a new Surface at an explicit destination origin.
/// Outside the returned extent reads the source's exact default; the source is immutable.
pub fn try_transform_orthogonal(
    source: &Surface,
    bounds: Rect,
    origin: (i32, i32),
    transform: OrthogonalTransform,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<(Surface, Rect), RasterInputError> {
    let format = source.format();
    checked_region_byte_len(bounds, format)?;
    let width = i32::try_from(bounds.width()).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = i32::try_from(bounds.height()).map_err(|_| RasterInputError::InvalidBounds)?;
    let swap = matches!(transform, OrthogonalTransform::RotateClockwise | OrthogonalTransform::RotateCounterclockwise);
    let destination = Rect::new(
        origin.0,
        origin.1,
        origin.0.checked_add(if swap { height } else { width }).ok_or(RasterInputError::InvalidBounds)?,
        origin.1.checked_add(if swap { width } else { height }).ok_or(RasterInputError::InvalidBounds)?,
    );
    let bytes = checked_region_byte_len(destination, format)? as u64;
    if bytes > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    let mut result = Surface::try_with_default_bytes(format, source.default_bytes())?;
    for tile in destination.tiles() {
        let area = tile.rect().intersect(&destination);
        let len = checked_region_byte_len(area, format)?;
        check_peak(source, &result, len as u64, limits)?;
        let mut data = Vec::new();
        data.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
        data.resize(len, 0);
        let bpp = format.bytes_per_pixel();
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if cursor.is_multiple_of(4096) && control.cancelled() {
                    return Err(RasterInputError::Cancelled);
                }
                let dx = x - origin.0;
                let dy = y - origin.1;
                let (sx, sy) = match transform {
                    OrthogonalTransform::FlipHorizontal => (width - 1 - dx, dy),
                    OrthogonalTransform::FlipVertical => (dx, height - 1 - dy),
                    OrthogonalTransform::RotateClockwise => (dy, height - 1 - dx),
                    OrthogonalTransform::RotateCounterclockwise => (width - 1 - dy, dx),
                    OrthogonalTransform::Rotate180 => (width - 1 - dx, height - 1 - dy),
                };
                let end = cursor.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?;
                source.try_read_pixel_bytes(bounds.x0 + sx, bounds.y0 + sy, data.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?)?;
                cursor = end;
            }
        }
        result.try_write_interleaved_with(area, &data, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok((result, destination))
}

pub(crate) fn check_peak(source: &Surface, result: &Surface, scratch: u64, limits: GeometryLimits) -> Result<(), RasterInputError> {
    let tile_bytes = (256 * 256 * source.format().bytes_per_pixel()) as u64;
    let peak = source
        .resident_payload_bytes()?
        .checked_add(result.resident_payload_bytes()?)
        .and_then(|v| v.checked_add(scratch.checked_mul(3)?))
        .and_then(|v| v.checked_add(tile_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add((source.tile_count() as u64).checked_add(result.tile_count() as u64)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
