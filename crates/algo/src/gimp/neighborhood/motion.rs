//! Uniform centered line exposure with bilinear premultiplied sampling.
//! Public parameters and independent native holdouts qualify the named slice;
//! caller supplies linear working samples and owns CMS/outer filter semantics.
use super::{GaussianAbyss, GaussianLimits, NeighborhoodRequirements, allocate, poll, read_premultiplied};
use photocraft_color::try_write_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug)]
pub struct MotionBlurParams {
    pub length: f64,
    pub angle_degrees: f64,
}
fn line(params: MotionBlurParams) -> Result<(f64, f64, usize), RasterInputError> {
    if !params.length.is_finite()
        || !(0.0..=1000.0).contains(&params.length)
        || !params.angle_degrees.is_finite()
        || !(-180.0..=180.0).contains(&params.angle_degrees)
    {
        return Err(RasterInputError::InvalidSample);
    }
    let angle = params.angle_degrees.to_radians();
    Ok((params.length * angle.cos(), params.length * angle.sin(), params.length.ceil() as usize + 1))
}
/// Symmetric operation extent observed independently from native merged outputs.
pub fn motion_blur_extent(input: Rect, params: MotionBlurParams) -> Result<Rect, RasterInputError> {
    if input.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    let (dx, dy, _) = line(params)?;
    let rx = (dx.abs() * 0.5).ceil() as i32;
    let ry = (dy.abs() * 0.5).ceil() as i32;
    Ok(Rect::new(
        input.x0.checked_sub(rx).ok_or(RasterInputError::InvalidBounds)?,
        input.y0.checked_sub(ry).ok_or(RasterInputError::InvalidBounds)?,
        input.x1.checked_add(rx).ok_or(RasterInputError::InvalidBounds)?,
        input.y1.checked_add(ry).ok_or(RasterInputError::InvalidBounds)?,
    ))
}

/// Allocation-free, cancellation-polled requirements for the existing kernel.
/// These are prospective kernel costs, not retained memory or quota authority.
pub fn motion_blur_requirements(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MotionBlurParams,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    checked_region_byte_len(input, source.format())?;
    checked_region_byte_len(output, source.format())?;
    source.try_validate_tile_origins_with(interrupt)?;
    let extent = motion_blur_extent(input, params)?;
    checked_region_byte_len(extent, source.format())?;
    if output.intersect(&extent) != output {
        return Err(RasterInputError::InvalidBounds);
    }
    let (_, _, count) = line(params)?;
    let channels = source.channels();
    if channels == 0 || channels > 5 {
        return Err(RasterInputError::UnsupportedConversion);
    }
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
    let work = (output.width() as u64)
        .checked_mul(output.height() as u64)
        .and_then(|v| v.checked_mul(count as u64 * channels as u64 * 32))
        .ok_or(RasterInputError::LimitExceeded)?;
    let peak = source
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(growth))
        .and_then(|v| v.checked_add(tile_bytes * 3 + source.tile_count() as u64 * 128 + 4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(NeighborhoodRequirements { peak_resident_bytes: peak, work_units: work })
}

/// Transparent abyss, ceil(length)+1 equally spaced centered samples including
/// both line endpoints, each sampled bilinearly in associated alpha. Writes only
/// ROI; retained source/default/outside-ROI bits are exact. No hidden color epsilon
/// or transfer approximation is introduced; caller controls working color space.
pub fn try_motion_blur_linear(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MotionBlurParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let requirements = motion_blur_requirements(source, output, input, params, interrupt)?;
    if requirements.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || requirements.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    let (dx, dy, count) = line(params)?;
    let channels = source.channels();
    let mut result = source.clone();
    let mut units = 0;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let mut raw = allocate::<u8>(checked_region_byte_len(part, source.format())?)?;
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                let mut sum = [0.; 5];
                for j in 0..count {
                    let t = if count > 1 { j as f64 / (count - 1) as f64 - 0.5 } else { 0. };
                    let sx = f64::from(x) + t * dx;
                    let sy = f64::from(y) + t * dy;
                    let x0 = sx.floor() as i64;
                    let y0 = sy.floor() as i64;
                    let tx = sx - x0 as f64;
                    let ty = sy - y0 as f64;
                    for (xx, wx) in [(x0, 1. - tx), (x0 + 1, tx)] {
                        for (yy, wy) in [(y0, 1. - ty), (y0 + 1, ty)] {
                            let weight = wx * wy / count as f64;
                            if weight == 0. {
                                continue;
                            }
                            poll(&mut units, channels * 32, interrupt)?;
                            let mut p = [0.; 5];
                            read_premultiplied(source, input, GaussianAbyss::None, xx, yy, p.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?)?;
                            for (s, v) in sum.iter_mut().zip(p.iter()).take(channels) {
                                *s += v * weight;
                            }
                        }
                    }
                }
                let alpha = if source.format().alpha { *sum.get(channels - 1).ok_or(RasterInputError::LengthMismatch)? } else { 1. };
                let offset = (((y - part.y0) as usize * part.width() as usize) + (x - part.x0) as usize) * source.format().bytes_per_pixel();
                let dest = raw.get_mut(offset..offset + source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                for (c, v) in sum.iter().take(channels).enumerate() {
                    let value = if source.format().alpha && c + 1 < channels { if alpha > 0. { *v / alpha } else { 0. } } else { *v };
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(dest, source.format().sample, c, if source.format().alpha && c + 1 == channels { value.clamp(0., 1.) } else { value })
                        .map_err(|_| RasterInputError::InvalidSample)?;
                }
            }
        }
        result.try_write_interleaved_with(part, &raw, interrupt)?;
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "motion_pixelize_finite_requirements_tests.rs"]
mod requirements_tests;
