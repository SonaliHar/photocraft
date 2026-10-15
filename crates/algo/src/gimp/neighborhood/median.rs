//! Straight-color alpha-weighted percentiles derived from public median-blur
//! documentation and independent native impulse/neighborhood observations.
//! Caller supplies the operation working space; quantized evaluation has a
//! separate encoded-space entry point so no implicit profile conversion occurs.
use super::{GaussianLimits, NeighborhoodRequirements, allocate, checked_allocation_capacity, poll};
use photocraft_color::try_write_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug)]
pub enum MedianNeighborhood {
    Circle,
    Square,
    Diamond,
}
#[derive(Clone, Copy, Debug)]
pub struct MedianParams {
    pub radius: i32,
    pub neighborhood: MedianNeighborhood,
    pub percentile: f64,
    pub alpha_percentile: f64,
    pub high_precision: bool,
}
/// Checked Clamp, unchanged operation extent, high-precision percentile kernel.
/// Radius magnitude <=16 bounds each noninterruptible sort to <=1089 entries.
/// Colors use alpha-weighted ranks; alpha uses an independent unweighted rank.
/// Negative radii invert both percentiles. Source/default/outside-ROI bits remain
/// exact; cancellation abandons the private staged result.
pub fn try_median_blur(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MedianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    if !params.high_precision {
        return Err(RasterInputError::UnsupportedConversion);
    }
    evaluate(source, output, input, params, limits, interrupt)
}
/// Quantized stock percentile kernel on explicitly supplied encoded working
/// samples. Caller owns conversion into/out of the operation's encoded profile.
/// Colors/alpha clip and round to 8-bit; color weights are integer alpha codes
/// (integer accumulation is necessary to preserve exact percentile ties).
pub fn try_median_blur_quantized_encoded(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MedianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    if params.high_precision {
        return Err(RasterInputError::UnsupportedConversion);
    }
    evaluate(source, output, input, params, limits, interrupt)
}
/// Allocation-free, polled admission formulas shared by both percentile modes.
/// Encoded/high-precision entry-point selection remains the caller's decision.
/// Empty ROI includes a conservative clone envelope, independently of legacy limits.
pub fn median_requirements(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MedianParams,
    interrupt: &Interrupt<'_>,
) -> Result<NeighborhoodRequirements, RasterInputError> {
    match plan(source, output, input, params, interrupt)? {
        Some(required) => Ok(required),
        None => super::clone_requirements(source, interrupt),
    }
}
fn plan(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MedianParams,
    interrupt: &Interrupt<'_>,
) -> Result<Option<NeighborhoodRequirements>, RasterInputError> {
    checked_region_byte_len(input, source.format())?;
    checked_region_byte_len(output, source.format())?;
    source.try_validate_tile_origins_with(interrupt)?;
    if input.is_empty() || output.intersect(&input) != output {
        return Err(RasterInputError::InvalidBounds);
    }
    if !(-16..=16).contains(&params.radius) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    for p in [params.percentile, params.alpha_percentile] {
        if !p.is_finite() || !(0.0..=100.0).contains(&p) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    if interrupt.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if output.is_empty() {
        return Ok(None);
    }
    let channels = source.channels();
    if channels == 0 || channels > 5 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let r = params.radius.abs();
    let count = ((2 * r + 1) * (2 * r + 1)) as usize;
    let tile_bytes = (256 * 256 * source.format().bytes_per_pixel()) as u64;
    let work = (output.width() as u64)
        .checked_mul(output.height() as u64)
        .and_then(|v| v.checked_mul(count as u64))
        .and_then(|v| v.checked_mul(channels as u64 * 16))
        .ok_or(RasterInputError::LimitExceeded)?;
    let mut growth = 0u64;
    for (i, tile) in output.tiles().enumerate() {
        if i.is_multiple_of(256) && interrupt.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if source.tile(tile).is_none() {
            growth = growth.checked_add(tile_bytes + 128).ok_or(RasterInputError::LimitExceeded)?;
        }
    }
    let peak = source
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(growth))
        .and_then(|v| v.checked_add(tile_bytes * 3 + count as u64 * 64 + source.tile_count() as u64 * 128 + ((256 + 2 * r) as u64).pow(2) * 40 + 4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(Some(NeighborhoodRequirements { peak_resident_bytes: peak, work_units: work }))
}

fn evaluate(
    source: &Surface,
    output: Rect,
    input: Rect,
    params: MedianParams,
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    // Keep the legacy empty/identity clone before limit admission. Public
    // requirements additionally price that clone for an aggregate owned caller.
    let Some(required) = plan(source, output, input, params, interrupt)? else { return Ok(source.clone()) };
    if required.work_units > limits.work_units || required.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let channels = source.channels();
    let r = params.radius.abs();
    let count = ((2 * r + 1) * (2 * r + 1)) as usize;
    let mut samples = Vec::<[f64; 5]>::new();
    samples.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    checked_allocation_capacity::<[f64; 5]>(samples.capacity(), count)?;
    let mut ranks = Vec::<(f64, f64)>::new();
    ranks.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    checked_allocation_capacity::<(f64, f64)>(ranks.capacity(), count)?;
    let mut result = source.clone();
    let mut units = 0;
    for tile in output.tiles() {
        let part = tile.rect().intersect(&output);
        let cw = part.width() as usize + 2 * r as usize;
        let ch = part.height() as usize + 2 * r as usize;
        let mut cache = Vec::<[f64; 5]>::new();
        let cache_length = cw.checked_mul(ch).ok_or(RasterInputError::LimitExceeded)?;
        cache.try_reserve_exact(cache_length).map_err(|_| RasterInputError::AllocationFailed)?;
        checked_allocation_capacity::<[f64; 5]>(cache.capacity(), cache_length)?;
        for cy in 0..ch as i32 {
            for cx in 0..cw as i32 {
                poll(&mut units, channels, interrupt)?;
                let mut pixel = [0.; 5];
                source.try_read_pixel_f64(
                    (part.x0 + cx - r).clamp(input.x0, input.x1 - 1),
                    (part.y0 + cy - r).clamp(input.y0, input.y1 - 1),
                    pixel.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?,
                )?;
                if pixel.iter().take(channels).any(|v| !v.is_finite()) {
                    return Err(RasterInputError::InvalidSample);
                }
                if source.format().alpha && !(0.0..=1.0).contains(pixel.get(channels - 1).ok_or(RasterInputError::LengthMismatch)?) {
                    return Err(RasterInputError::InvalidSample);
                }
                if !params.high_precision {
                    for value in pixel.iter_mut().take(channels) {
                        *value = ((*value).clamp(0., 1.) * 255. + 0.5).floor() / 255.;
                    }
                }
                cache.push(pixel);
            }
        }
        let mut raw = allocate::<u8>(checked_region_byte_len(part, source.format())?)?;
        for y in part.y0..part.y1 {
            for x in part.x0..part.x1 {
                samples.clear();
                for dy in -r..=r {
                    for dx in -r..=r {
                        let inside = match params.neighborhood {
                            MedianNeighborhood::Square => true,
                            MedianNeighborhood::Diamond => dx.abs() + dy.abs() <= r,
                            MedianNeighborhood::Circle => dx * dx + dy * dy <= r * r + r,
                        };
                        if !inside {
                            continue;
                        }
                        poll(&mut units, channels, interrupt)?;
                        let index = ((y - part.y0 + dy + r) as usize) * cw + (x - part.x0 + dx + r) as usize;
                        let pixel = *cache.get(index).ok_or(RasterInputError::LengthMismatch)?;
                        samples.push(pixel);
                    }
                }
                let offset = (((y - part.y0) as usize * part.width() as usize) + (x - part.x0) as usize) * source.format().bytes_per_pixel();
                let dest = raw.get_mut(offset..offset + source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                for c in 0..channels {
                    if interrupt.cancelled() {
                        return Err(RasterInputError::Cancelled);
                    }
                    ranks.clear();
                    let alpha_channel = source.format().alpha && c + 1 == channels;
                    for pixel in &samples {
                        ranks.push((
                            *pixel.get(c).ok_or(RasterInputError::LengthMismatch)?,
                            if source.format().alpha && !alpha_channel {
                                let alpha = *pixel.get(channels - 1).ok_or(RasterInputError::LengthMismatch)?;
                                if params.high_precision { alpha } else { (alpha * 255.).round() }
                            } else {
                                1.
                            },
                        ));
                    }
                    ranks.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                    let pct = if alpha_channel { params.alpha_percentile } else { params.percentile };
                    let pct = if params.radius < 0 { 100. - pct } else { pct };
                    let target = ranks.iter().map(|v| v.1).sum::<f64>() * pct * 0.01;
                    let mut accumulated = 0.;
                    let mut value = 0.;
                    for &(sample, weight) in &ranks {
                        accumulated += weight;
                        if weight > 0. && accumulated >= target {
                            value = sample;
                            break;
                        }
                    }
                    try_write_sample_f64(dest, source.format().sample, c, value).map_err(|_| RasterInputError::InvalidSample)?;
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
