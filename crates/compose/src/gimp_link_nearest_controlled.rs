//! Admission for the existing raw nearest linked sampler; no new source policy.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_doc::{PixelFormat, Rect, xcf_items::TransformState};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
fn refused() -> CompositionError {
    CompositionError::Unsupported("unqualified nearest linked transform")
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
#[allow(clippy::too_many_arguments)]
pub(super) fn requirements(
    bytes: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    state: &TransformState,
    bounds: Rect,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    let bpp = format.bytes_per_pixel();
    let source = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(refused)?;
    let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or_else(refused)?;
    if state.interpolation != 0
        || !format.alpha
        || bpp == 0
        || bpp > 40
        || width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || source > 64 << 20
        || usize::try_from(source).ok() != Some(bytes.len())
        || bounds.is_empty()
        || bounds.width() > 8192
        || bounds.height() > 8192
        || pixels > 4_000_000
        || pixels.checked_mul(bpp as u64).is_none_or(|n| n > 64 << 20)
    {
        return Err(refused());
    }
    super::mapping_with_interpolation(width, height, state, bounds, 0)?;
    checked_region_byte_len(bounds, format)?;
    let first = TileCoord::containing(bounds.x0, bounds.y0);
    let last = TileCoord::containing(bounds.x1.checked_sub(1).ok_or_else(refused)?, bounds.y1.checked_sub(1).ok_or_else(refused)?);
    let cols = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| limit())?;
    let tiles = cols.checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| limit())?).ok_or_else(limit)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes).ok_or_else(limit)?;
    // Bound map/scheduler overlap conservatively. Standard map/Arc allocation
    // remains library-managed; this envelope does not measure process RSS.
    let output_index_bytes = tiles.checked_mul(1024).and_then(|n| n.checked_add(4096)).ok_or_else(limit)?;
    let patch_envelope = tile_bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    let prune_envelope = tiles.checked_mul(32).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    let scratch_bytes = patch_envelope.checked_add(tile_bytes).and_then(|n| n.checked_add(prune_envelope)).ok_or_else(limit)?;
    let peak_resident_bytes = source
        .checked_add(output_retained_bytes)
        .and_then(|n| n.checked_add(output_index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(limit)?;
    // One private patch/native replacement per destination tile, plus repeated
    // growing sparse-map census/clone and controlled pruning. No per-row cloning.
    let work_units = pixels
        .checked_mul(128)
        .and_then(|n| n.checked_add(source))
        .and_then(|n| n.checked_add(output_retained_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(512)?))
        .ok_or_else(limit)?;
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(Requirements { source_retained_bytes: source, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
pub(super) fn admit(requirements: Requirements, limits: Limits) -> Result<()> {
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || limits.work_units > 8_000_000_000
        || requirements.peak_resident_bytes > limits.resident_bytes
        || requirements.work_units > limits.work_units
    {
        return Err(limit());
    }
    Ok(())
}
