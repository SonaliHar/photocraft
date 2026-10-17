//! Checked f64 square-neighborhood mean, independently derived from public
//! GEGL box-blur documentation and original native edge/alpha observations.
//! Caller supplies linear working-space samples and owns CMS/envelope mapping.
use super::{GaussianLimits, NeighborhoodRequirements, allocate, poll};
use photocraft_color::{PixelFormat, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

/// Public stock radius domain and independently observed radius-inflated extent.
pub fn box_blur_extent(input: Rect, radius: u32) -> Result<Rect, RasterInputError> {
    if radius > 1000 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if input.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    let r = radius as i32;
    Ok(Rect::new(
        input.x0.checked_sub(r).ok_or(RasterInputError::InvalidBounds)?,
        input.y0.checked_sub(r).ok_or(RasterInputError::InvalidBounds)?,
        input.x1.checked_add(r).ok_or(RasterInputError::InvalidBounds)?,
        input.y1.checked_add(r).ok_or(RasterInputError::InvalidBounds)?,
    ))
}
/// Allocation-free, polled admission formulas used by the existing evaluator.
/// An identity/empty ROI includes a conservative shared-payload/map clone envelope.
/// This logical peak is not a quota reservation or allocator-capacity receipt.
pub fn box_blur_requirements(
    source: &Surface,
    output: Rect,
    input: Rect,
    radius: u32,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    match plan(source, output, input, radius, interrupt)? {
        Some(required) => Ok(required),
        None => super::clone_requirements(source, interrupt),
    }
}
fn plan(source: &Surface, output: Rect, input: Rect, radius: u32, interrupt: &Interrupt<'_>) -> Result<Option<NeighborhoodRequirements>, RasterInputError> {
    checked_region_byte_len(input, source.format())?;
    checked_region_byte_len(output, source.format())?;
    source.try_validate_tile_origins_with(interrupt)?;
    let extent = box_blur_extent(input, radius)?;
    checked_region_byte_len(extent, source.format())?;
    if output.intersect(&extent) != output {
        return Err(RasterInputError::InvalidBounds);
    }
    if output.is_empty() || radius == 0 {
        return Ok(None);
    }
    let channels = source.channels();
    let r = radius as usize;
    let window = r.checked_mul(2).and_then(|v| v.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
    let tile_bytes = (256 * 256 * source.format().bytes_per_pixel()) as u64;
    let mut new_tiles = 0u64;
    let mut work = 0u64;
    let mut max_scratch = 0u64;
    for (index, tile) in output.tiles().enumerate() {
        if index.is_multiple_of(256) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        let part = tile.rect().intersect(&output);
        if source.tile(tile).is_none() {
            new_tiles = new_tiles.checked_add(tile_bytes + 64).ok_or(RasterInputError::LimitExceeded)?;
        }
        let w = part.width() as u64;
        let h = (part.height() as u64).checked_add((r as u64) * 2).ok_or(RasterInputError::LimitExceeded)?;
        let samples = w.checked_mul(h).and_then(|v| v.checked_mul(channels as u64)).ok_or(RasterInputError::LimitExceeded)?;
        let scratch = samples
            .checked_mul(8)
            .and_then(|v| v.checked_add(checked_region_byte_len(part, source.format()).ok()? as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        max_scratch = max_scratch.max(scratch);
        let units = h
            .checked_mul(w.checked_mul(4).and_then(|v| v.checked_add(window as u64)).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|v| v.checked_mul(channels as u64))
            .and_then(|v| v.checked_mul(16))
            .ok_or(RasterInputError::LimitExceeded)?;
        work = work.checked_add(units).ok_or(RasterInputError::LimitExceeded)?;
    }
    let peak = source
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(new_tiles))
        .and_then(|v| v.checked_add(max_scratch))
        .and_then(|v| v.checked_add(tile_bytes * 2 + 4096))
        .and_then(|v| v.checked_add((source.tile_count() as u64) * 128))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(Some(NeighborhoodRequirements { peak_resident_bytes: peak, work_units: work }))
}

/// Square mean with repeated endpoints (stock Clamp) and premultiplied alpha.
/// Writes only the requested ROI within the operation's radius-inflated extent.
/// Scratch is tile-local, all authoritative reads/math remain f64, original/default
/// bytes stay unchanged, and cancellation abandons the private output.
pub fn try_box_blur(
    source: &Surface,
    output: Rect,
    input: Rect,
    radius: u32,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    // Keep the legacy empty/identity clone before limit admission. Public
    // requirements additionally price that clone for an aggregate owned caller.
    let Some(required) = plan(source, output, input, radius, interrupt)? else { return Ok(source.clone()) };
    if required.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || required.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let channels = source.channels();
    let r = radius as usize;
    let window = r.checked_mul(2).and_then(|v| v.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
    let mut result = source.clone();
    let mut units = 0;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let raw =
            tile_core(part, input, source.format(), channels, r, window, &mut units, interrupt, &mut |x, y, output| source.try_read_pixel_f64(x, y, output))?;
        result.try_write_interleaved_with(part, &raw, interrupt)?;
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(result)
}

// One shared numeric loop. Legacy callers retain their requested-ROI tilepart;
// bounded callers supply the canonical tilepart of the full operation extent.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn tile_core(
    part: Rect,
    input: Rect,
    format: PixelFormat,
    channels: usize,
    r: usize,
    window: usize,
    units: &mut usize,
    interrupt: &Interrupt<'_>,
    read: &mut impl FnMut(i32, i32, &mut [f64]) -> Result<(), RasterInputError>,
) -> Result<Vec<u8>, RasterInputError> {
    let w = part.width() as usize;
    let h = part.height() as usize;
    let rows = h + 2 * r;
    let mut horizontal = allocate::<f64>(w.checked_mul(rows).and_then(|v| v.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?)?;
    for (row, dest) in horizontal.chunks_exact_mut(w * channels).enumerate() {
        horizontal_row(dest, part, input, format, channels, r, window, row, units, interrupt, read)?;
    }
    let mut raw = allocate::<u8>(checked_region_byte_len(part, format)?)?;
    for x in 0..w {
        let mut sum = [0.; 5];
        for row in 0..window {
            poll(units, channels, interrupt)?;
            let sample = horizontal.get((row * w + x) * channels..(row * w + x + 1) * channels).ok_or(RasterInputError::LengthMismatch)?;
            for (s, v) in sum.iter_mut().zip(sample) {
                *s += v;
            }
        }
        for y in 0..h {
            poll(units, channels * 4, interrupt)?;
            let offset = (y * w + x) * format.bytes_per_pixel();
            let pixel = raw.get_mut(offset..offset + format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
            write_box_pixel(&sum, format, channels, window, pixel)?;
            if y + 1 < h {
                for (sign, row) in [(-1., y), (1., y + window)] {
                    let sample = horizontal.get((row * w + x) * channels..(row * w + x + 1) * channels).ok_or(RasterInputError::LengthMismatch)?;
                    for (s, v) in sum.iter_mut().zip(sample) {
                        *s += sign * v;
                    }
                }
            }
        }
    }
    Ok(raw)
}

// The additive row scheduler reuses these equations. The old full-horizontal
// evaluator keeps its exact row/column traversal, read order and poll counts.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn horizontal_row<E: From<RasterInputError>>(
    dest: &mut [f64],
    part: Rect,
    input: Rect,
    format: PixelFormat,
    channels: usize,
    r: usize,
    window: usize,
    row: usize,
    units: &mut usize,
    interrupt: &Interrupt<'_>,
    read: &mut impl FnMut(i32, i32, &mut [f64]) -> Result<(), E>,
) -> Result<(), E> {
    let w = part.width() as usize;
    let y = i64::from(part.y0) + row as i64 - r as i64;
    let mut sum = [0.; 5];
    let mut pixel = [0.; 5];
    for offset in 0..window {
        poll(units, channels, interrupt).map_err(E::from)?;
        read_box_premultiplied(
            read,
            format,
            input,
            i64::from(part.x0) + offset as i64 - r as i64,
            y,
            pixel.get_mut(..channels).ok_or(RasterInputError::LengthMismatch).map_err(E::from)?,
        )?;
        for (s, v) in sum.iter_mut().zip(pixel.iter()).take(channels) {
            *s += v;
        }
    }
    for (x, p) in dest.chunks_exact_mut(channels).enumerate() {
        poll(units, channels * 4, interrupt).map_err(E::from)?;
        for (v, s) in p.iter_mut().zip(&sum) {
            *v = *s / window as f64;
        }
        if x + 1 < w {
            for (sign, xx) in [(-1., i64::from(part.x0) + x as i64 - r as i64), (1., i64::from(part.x0) + x as i64 + r as i64 + 1)] {
                read_box_premultiplied(read, format, input, xx, y, pixel.get_mut(..channels).ok_or(RasterInputError::LengthMismatch).map_err(E::from)?)?;
                for (s, v) in sum.iter_mut().zip(pixel.iter()).take(channels) {
                    *s += sign * v;
                }
            }
        }
    }
    Ok(())
}

fn write_box_pixel(sum: &[f64], format: PixelFormat, channels: usize, window: usize, pixel: &mut [u8]) -> Result<(), RasterInputError> {
    let alpha = if format.alpha { sum.get(channels - 1).copied().ok_or(RasterInputError::LengthMismatch)? / window as f64 } else { 1. };
    for (c, s) in sum.iter().take(channels).enumerate() {
        let value = if format.alpha && c + 1 < channels { if alpha > 0. { *s / window as f64 / alpha } else { 0. } } else { *s / window as f64 };
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        try_write_sample_f64(pixel, format.sample, c, value).map_err(|_| RasterInputError::InvalidSample)?;
    }
    Ok(())
}
// Exact old Box Clamp/read/premultiply order. The original source rectangle
// remains authoritative; packed support/output rectangles never replace it.
fn read_box_premultiplied<E: From<RasterInputError>>(
    read: &mut impl FnMut(i32, i32, &mut [f64]) -> Result<(), E>,
    format: PixelFormat,
    extent: Rect,
    x: i64,
    y: i64,
    output: &mut [f64],
) -> Result<(), E> {
    let x = x.clamp(i64::from(extent.x0), i64::from(extent.x1) - 1);
    let y = y.clamp(i64::from(extent.y0), i64::from(extent.y1) - 1);
    read(
        i32::try_from(x).map_err(|_| E::from(RasterInputError::InvalidBounds))?,
        i32::try_from(y).map_err(|_| E::from(RasterInputError::InvalidBounds))?,
        output,
    )?;
    if output.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample.into());
    }
    if format.alpha {
        let alpha = output.last().copied().ok_or(RasterInputError::LengthMismatch).map_err(E::from)?;
        if !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::InvalidSample.into());
        }
        let colors = output.len() - 1;
        for value in output.iter_mut().take(colors) {
            *value *= alpha;
        }
    }
    Ok(())
}
#[path = "box_blur/packed.rs"]
mod packed;
pub use packed::{BoxBlurTilePlan, try_box_blur_packed_tile};
#[path = "box_blur/row_fed.rs"]
mod row_fed;
pub use row_fed::{BoxBlurRowError, BoxBlurRowPlan, BoxBlurRowRequirements, try_box_blur_row_tile};
