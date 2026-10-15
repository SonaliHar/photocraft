//! Independent grid-aligned rectangular means; caller supplies linear working
//! samples. Only full-size square blocks are represented by this named API.
use super::{GaussianAbyss, GaussianLimits, NeighborhoodRequirements, allocate, clone_requirements, poll, read_premultiplied};
use photocraft_color::try_write_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug)]
pub struct PixelizeParams {
    pub size_x: u32,
    pub size_y: u32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// Allocation-free, cancellation-polled requirements for the existing kernel.
/// These are prospective kernel costs, not retained memory or quota authority.
/// Empty output reports a conservative source-clone lifetime envelope and scan
/// work. The legacy constructor still bypasses numeric limits on that branch.
pub fn pixelize_requirements(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: PixelizeParams,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    validate(source, output, input, params, interrupt)?;
    if output.is_empty() {
        return clone_requirements(source, interrupt);
    }
    cost(source, output, input, interrupt)
}
fn validate(source: &Surface, output: Rect, input: Rect, params: PixelizeParams, interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    checked_region_byte_len(input, source.format())?;
    checked_region_byte_len(output, source.format())?;
    source.try_validate_tile_origins_with(interrupt)?;
    if input.is_empty() || output.intersect(&input) != output {
        return Err(RasterInputError::InvalidBounds);
    }
    if params.size_x == 0 || params.size_y == 0 || params.size_x > i32::MAX as u32 || params.size_y > i32::MAX as u32 {
        return Err(RasterInputError::InvalidBounds);
    }
    let channels = source.channels();
    if channels == 0 || channels > 5 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(())
}
fn cost(source: &Surface, output: Rect, input: Rect, interrupt: &Interrupt<'_>) -> Result<NeighborhoodRequirements, RasterInputError> {
    let channels = source.channels();
    let tile_bytes = (256 * 256 * source.format().bytes_per_pixel()) as u64;
    let mut growth = 0u64;
    for (i, tile) in output.tiles().enumerate() {
        if i.is_multiple_of(256) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if source.tile(tile).is_none() {
            growth = growth.checked_add(tile_bytes + 128).ok_or(RasterInputError::LimitExceeded)?;
        }
    }
    let input_pixels = input.width() as u64 * input.height() as u64;
    let output_pixels = output.width() as u64 * output.height() as u64;
    let work = input_pixels
        .checked_mul(channels as u64 * 32)
        .and_then(|v| v.checked_add(output_pixels * channels as u64 * 16))
        .ok_or(RasterInputError::LimitExceeded)?;
    let peak = source
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(growth))
        .and_then(|v| v.checked_add(tile_bytes * 3 + source.tile_count() as u64 * 128 + 4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(NeighborhoodRequirements { peak_resident_bytes: peak, work_units: work })
}

/// Square/full-ratio Pixelize. Grid coordinates use Euclidean division, including
/// negative offsets. Each block averages only its actual intersection with the
/// finite input extent (no padded/clamped edge denominator). Associated-alpha
/// f64 arithmetic retains finite HDR color; caller owns CMS/filter envelope.
pub fn try_pixelize_square(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: PixelizeParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    validate(source, output, input, params, interrupt)?;
    if output.is_empty() {
        return Ok(source.clone());
    }
    let requirements = cost(source, output, input, interrupt)?;
    if requirements.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || requirements.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let channels = source.channels();
    let sx = i64::from(params.size_x);
    let sy = i64::from(params.size_y);
    let ox = i64::from(params.offset_x);
    let oy = i64::from(params.offset_y);
    let mut result = source.clone();
    let mut units = 0;
    let mut by = (i64::from(output.y0) - oy).div_euclid(sy) * sy + oy;
    while by < i64::from(output.y1) {
        let mut bx = (i64::from(output.x0) - ox).div_euclid(sx) * sx + ox;
        while bx < i64::from(output.x1) {
            let block = Rect::new(
                i64::from(input.x0).max(bx) as i32,
                i64::from(input.y0).max(by) as i32,
                i64::from(input.x1).min(bx + sx) as i32,
                i64::from(input.y1).min(by + sy) as i32,
            );
            let target = block.intersect(&output);
            if !target.is_empty() {
                let mut sum = [0.; 5];
                let count = block.width() as u64 * block.height() as u64;
                for y in block.y0..block.y1 {
                    for x in block.x0..block.x1 {
                        poll(&mut units, channels * 32, interrupt)?;
                        let mut p = [0.; 5];
                        read_premultiplied(
                            source,
                            input,
                            GaussianAbyss::None,
                            i64::from(x),
                            i64::from(y),
                            p.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?,
                        )?;
                        for (s, v) in sum.iter_mut().zip(p.iter()).take(channels) {
                            *s += v;
                        }
                    }
                }
                let alpha = if source.format().alpha { *sum.get(channels - 1).ok_or(RasterInputError::LengthMismatch)? / count as f64 } else { 1. };
                let mut pixel = allocate::<u8>(source.format().bytes_per_pixel())?;
                for (c, v) in sum.iter().take(channels).enumerate() {
                    let value =
                        if source.format().alpha && c + 1 < channels { if alpha > 0. { *v / count as f64 / alpha } else { 0. } } else { *v / count as f64 };
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(
                        &mut pixel,
                        source.format().sample,
                        c,
                        if source.format().alpha && c + 1 == channels { value.clamp(0., 1.) } else { value },
                    )
                    .map_err(|_| RasterInputError::InvalidSample)?;
                }
                for tile in target.tiles() {
                    let part = tile.rect().intersect(&target);
                    let mut raw = allocate::<u8>(checked_region_byte_len(part, source.format())?)?;
                    for dest in raw.chunks_exact_mut(pixel.len()) {
                        poll(&mut units, channels * 16, interrupt)?;
                        dest.copy_from_slice(&pixel);
                    }
                    result.try_write_interleaved_with(part, &raw, interrupt)?;
                }
            }
            bx += sx;
        }
        by += sy;
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(result)
}
