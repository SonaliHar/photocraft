//! Constructor-owned prospective planning for the existing Young recurrence.
//! Logical allocation bounds, not quota receipts or process RSS measurements.
use super::*;
pub(super) fn validated_output(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    interrupt: &Interrupt<'_>,
) -> Result<Rect, RasterInputError> {
    checked_region_byte_len(output, source.format())?;
    checked_region_byte_len(input_extent, source.format())?;
    if input_extent.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    for s in [params.std_dev_x, params.std_dev_y] {
        if !s.is_finite() || !(0.0..=1500.0).contains(&s) {
            return Err(RasterInputError::InvalidSample);
        }
        if s > 0.0 && s < 0.5 {
            return Err(RasterInputError::UnsupportedConversion);
        }
    }
    Ok(if params.clip_extent { output.intersect(&input_extent) } else { output })
}
pub(super) fn work(output: Rect, gx: usize, gy: usize, channels: usize, interrupt: &Interrupt<'_>) -> Result<u64, RasterInputError> {
    let mut work = 0u64;
    for tile in output.tiles() {
        if interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let rect = tile.rect().intersect(&output);
        let width = (rect.width() as u64).checked_add((gx as u64) * 2).ok_or(RasterInputError::LimitExceeded)?;
        let height = (rect.height() as u64).checked_add((gy as u64) * 2).ok_or(RasterInputError::LimitExceeded)?;
        let units =
            width.checked_mul(height).and_then(|v| v.checked_mul(channels as u64)).and_then(|v| v.checked_mul(64)).ok_or(RasterInputError::LimitExceeded)?;
        work = work.checked_add(units).ok_or(RasterInputError::LimitExceeded)?;
    }
    Ok(work)
}
pub(super) struct TileRequirements {
    pub width: usize,
    pub height: usize,
    pub line_width: usize,
    pub rows: usize,
    pub samples: usize,
    pub line_samples: usize,
    pub raw_len: usize,
    pub scratch: u64,
}
pub(super) fn tile_requirements(source: &Surface, part: Rect, gx: usize, gy: usize) -> Result<TileRequirements, RasterInputError> {
    let width = part.width() as usize;
    let height = part.height() as usize;
    let channels = source.channels();
    let line_width = width.checked_add(gx.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let rows = height.checked_add(gy.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let samples = width.checked_mul(rows).and_then(|v| v.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?;
    let line_samples = line_width.max(rows).checked_mul(channels).ok_or(RasterInputError::LimitExceeded)?;
    let raw_len = checked_region_byte_len(part, source.format())?;
    let scratch = (samples as u64)
        .checked_add(line_samples as u64)
        .and_then(|v| v.checked_mul(8))
        .and_then(|v| v.checked_add(raw_len as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(TileRequirements { width, height, line_width, rows, samples, line_samples, raw_len, scratch })
}
pub(super) fn peak(source: u64, output: u64, scratch: u64, bpp: usize) -> Result<u64, RasterInputError> {
    source.checked_add(output).and_then(|v| v.checked_add(scratch)).and_then(|v| v.checked_add((256 * 256 * bpp) as u64)).ok_or(RasterInputError::LimitExceeded)
}
/// Allocation-free, interrupt-polled source/output/scratch/map envelope. The
/// constructor shares its exact validation, work and per-tile peak arithmetic;
/// its existing limit-check placement and numerical recurrence remain unchanged.
/// The caller must reserve actual simultaneous ownership before allocating.
pub fn gaussian_young_recursive_requirements(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    params: GaussianParams,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    let output = validated_output(source, output, input_extent, params, interrupt)?;
    if output.is_empty() || params.std_dev_x == 0.0 && params.std_dev_y == 0.0 {
        return clone_requirements(source, interrupt);
    }
    let gx = kernel_length(params.std_dev_x, KernelKind::Young)?.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
    let gy = kernel_length(params.std_dev_y, KernelKind::Young)?.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
    let kernel_work = work(output, gx, gy, source.channels(), interrupt)?;
    let mut source_payload = source.default_bytes().len() as u64;
    for (_, tile) in source.tiles() {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        source_payload = source_payload.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
    }
    let mut output_payload = source_payload;
    let tile_bytes = (256u64 * 256).checked_mul(source.format().bytes_per_pixel() as u64).ok_or(RasterInputError::LimitExceeded)?;
    let mut output_tiles = source.tile_count() as u64;
    let mut peak_bytes = 0u64;
    let mut map_work = (source.tile_count() as u64)
        .checked_mul(512)
        .and_then(|v| v.checked_add(source.default_bytes().len() as u64 + 128))
        .ok_or(RasterInputError::LimitExceeded)?;
    for tile in output.tiles() {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let part = tile.rect().intersect(&output);
        let layout = tile_requirements(source, part, gx, gy)?;
        peak_bytes = peak_bytes.max(peak(source_payload, output_payload, layout.scratch, source.format().bytes_per_pixel())?);
        // The existing Surface writer stages a complete replacement map and
        // encoded tile for every output tile, even for sparse tiny ROIs.
        map_work = map_work
            .checked_add(output_tiles.checked_add(1).and_then(|n| n.checked_mul(512)).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|v| v.checked_add(tile_bytes.checked_mul(3)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        if source.tile(tile).is_none() {
            output_payload = output_payload.checked_add(tile_bytes).ok_or(RasterInputError::LimitExceeded)?;
            output_tiles = output_tiles.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
        }
    }
    // Retained source and cloned/growing output maps/defaults, temporary Vec
    // headers and Surface owners coexist with the old numeric payload peak.
    let metadata = (source.tile_count() as u64)
        .checked_add(output_tiles.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|v| v.checked_mul(512))
        .and_then(|v| v.checked_add((source.default_bytes().len() as u64).checked_mul(3)?))
        .and_then(|v| v.checked_add((3 * std::mem::size_of::<Surface>() + 2 * std::mem::size_of::<Vec<f64>>() + std::mem::size_of::<Vec<u8>>() + 256) as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(NeighborhoodRequirements {
        peak_resident_bytes: peak_bytes.checked_add(metadata).ok_or(RasterInputError::LimitExceeded)?,
        work_units: kernel_work.checked_add(map_work).ok_or(RasterInputError::LimitExceeded)?,
    })
}
#[cfg(test)]
#[path = "recursive_requirements_tests.rs"]
mod tests;
