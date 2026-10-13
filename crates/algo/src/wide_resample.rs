//! Checked f64 resampling using the existing public filter equations, without f32 taps.
use crate::{
    resample::Resample,
    wide_geometry::{GeometryLimits, check_peak},
};
use photocraft_color::try_read_sample_f64;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct ResizeOptions {
    pub scale_x: f64,
    pub scale_y: f64,
    pub filter: Resample,
    pub clamp_edges: bool,
}

struct Taps {
    start: i32,
    weights: Vec<f64>,
}

fn tap_count(scale: f64, filter: Resample) -> Result<u64, RasterInputError> {
    let support = filter.support() * if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let count = (2.0 * support).ceil() + 2.0;
    if !count.is_finite() || count > 4096.0 {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(count as u64)
}

fn taps(coordinate: i32, scale: f64, filter: Resample, control: &Interrupt<'_>) -> Result<Taps, RasterInputError> {
    let center = (f64::from(coordinate) + 0.5) / scale - 0.5;
    let stretch = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    let support = filter.support() * stretch;
    let low = (center - support).ceil();
    let high = (center + support).floor();
    if !center.is_finite() || low < f64::from(i32::MIN) || high > f64::from(i32::MAX) || high < low {
        return Err(RasterInputError::InvalidBounds);
    }
    let low = low as i32;
    let high = high as i32;
    let count = usize::try_from(i64::from(high) - i64::from(low) + 1).map_err(|_| RasterInputError::LimitExceeded)?;
    if count > 4096 {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut weights = Vec::new();
    weights.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
    for i in low..=high {
        if (i64::from(i) - i64::from(low)) % 512 == 0 && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        weights.push(filter.weight((f64::from(i) - center) / stretch));
    }
    let total: f64 = weights.iter().sum();
    if !total.is_finite() || total.abs() <= 1e-12 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    for weight in &mut weights {
        *weight /= total;
    }
    Ok(Taps { start: low, weights })
}

fn scaled(bounds: Rect, sx: f64, sy: f64) -> Result<Rect, RasterInputError> {
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return Err(RasterInputError::InvalidBounds);
    }
    let values =
        [(f64::from(bounds.x0) * sx).floor(), (f64::from(bounds.y0) * sy).floor(), (f64::from(bounds.x1) * sx).ceil(), (f64::from(bounds.y1) * sy).ceil()];
    if values.iter().any(|v| !v.is_finite() || *v < f64::from(i32::MIN) || *v > f64::from(i32::MAX)) {
        return Err(RasterInputError::InvalidBounds);
    }
    Ok(Rect::new(values[0] as i32, values[1] as i32, values[2] as i32, values[3] as i32))
}

/// Scale explicit source bounds about the document origin. Missing/outside samples use
/// the exact source default, or repeat `bounds` edges when requested. Nearest and unit
/// scale retain raw bytes; other methods premultiply/unpremultiply entirely in f64.
/// Caller chooses the working color encoding. PreserveDetails sharpening is unsupported.
pub fn try_resize(
    source: &Surface,
    bounds: Rect,
    options: ResizeOptions,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<(Surface, Rect), RasterInputError> {
    let format = source.format();
    checked_region_byte_len(bounds, format)?;
    if options.filter == Resample::PreserveDetails {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let scaled_bounds = scaled(bounds, options.scale_x, options.scale_y)?;
    let destination = if bounds.is_empty() { Rect::EMPTY } else { scaled_bounds };
    checked_region_byte_len(destination, format)?;
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    let mut out = Surface::try_with_default_bytes(format, source.default_bytes())?;
    if bounds.is_empty() {
        return Ok((out, destination));
    }
    let raw = options.filter == Resample::Nearest || (options.scale_x == 1.0 && options.scale_y == 1.0);
    let nx = if raw { 1 } else { tap_count(options.scale_x, options.filter)? };
    let ny = if raw { 1 } else { tap_count(options.scale_y, options.filter)? };
    let pixels = u64::from(destination.width()).checked_mul(u64::from(destination.height())).ok_or(RasterInputError::LimitExceeded)?;
    let work =
        pixels.checked_mul(nx).and_then(|v| v.checked_mul(ny)).and_then(|v| v.checked_mul(format.channels() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut work_done = 0u64;
    for tile in destination.tiles() {
        let area = tile.rect().intersect(&destination);
        let bytes = checked_region_byte_len(area, format)?;
        let taps_bytes = u64::from(area.width())
            .checked_mul(nx)
            .and_then(|v| v.checked_add(u64::from(area.height()).checked_mul(ny)?))
            .and_then(|v| v.checked_mul(32))
            .ok_or(RasterInputError::LimitExceeded)?;
        check_peak(source, &out, (bytes as u64).checked_add(taps_bytes).ok_or(RasterInputError::LimitExceeded)?, limits)?;
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(bytes).map_err(|_| RasterInputError::AllocationFailed)?;
        encoded.resize(bytes, 0);
        let mut horizontal = Vec::new();
        let mut vertical = Vec::new();
        if !raw {
            horizontal.try_reserve_exact(area.width() as usize).map_err(|_| RasterInputError::AllocationFailed)?;
            vertical.try_reserve_exact(area.height() as usize).map_err(|_| RasterInputError::AllocationFailed)?;
            for x in area.x0..area.x1 {
                horizontal.push(taps(x, options.scale_x, options.filter, control)?);
            }
            for y in area.y0..area.y1 {
                vertical.push(taps(y, options.scale_y, options.filter, control)?);
            }
        }
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if work_done.is_multiple_of(4096) && control.cancelled() {
                    return Err(RasterInputError::Cancelled);
                }
                let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let target = encoded.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                if raw {
                    let sx = nearest(x, options.scale_x)?;
                    let sy = nearest(y, options.scale_y)?;
                    read_raw(source, bounds, sx, sy, options.clamp_edges, target)?;
                    work_done = work_done.saturating_add(1);
                } else {
                    let tx = horizontal.get((x - area.x0) as usize).ok_or(RasterInputError::LengthMismatch)?;
                    let ty = vertical.get((y - area.y0) as usize).ok_or(RasterInputError::LengthMismatch)?;
                    let mut sum = [0.0; 5];
                    for (iy, wy) in ty.weights.iter().enumerate() {
                        for (ix, wx) in tx.weights.iter().enumerate() {
                            work_done = work_done.saturating_add(1);
                            if work_done.is_multiple_of(4096) && control.cancelled() {
                                return Err(RasterInputError::Cancelled);
                            }
                            let weight = wx * wy;
                            if weight == 0.0 {
                                continue;
                            }
                            let sx =
                                tx.start.checked_add(i32::try_from(ix).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                            let sy =
                                ty.start.checked_add(i32::try_from(iy).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                            let mut pixel = [0.0; 5];
                            let mut sample = [0u8; 40];
                            let sample = sample.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                            read_raw(source, bounds, sx, sy, options.clamp_edges, sample)?;
                            for (c, value) in pixel.iter_mut().take(format.channels()).enumerate() {
                                *value = try_read_sample_f64(sample, format.sample, c).map_err(|_| RasterInputError::InvalidSample)?;
                                if !value.is_finite() {
                                    return Err(RasterInputError::InvalidSample);
                                }
                            }
                            let alpha =
                                if format.alpha { pixel.get(format.mode.color_channels()).copied().ok_or(RasterInputError::LengthMismatch)? } else { 1.0 };
                            for (c, acc) in sum.iter_mut().take(format.channels()).enumerate() {
                                let value = pixel.get(c).copied().ok_or(RasterInputError::LengthMismatch)?;
                                *acc += value * if c < format.mode.color_channels() { alpha * weight } else { weight };
                            }
                        }
                    }
                    if format.alpha {
                        let alpha = sum.get(format.mode.color_channels()).copied().ok_or(RasterInputError::LengthMismatch)?;
                        for value in sum.iter_mut().take(format.mode.color_channels()) {
                            *value = if alpha != 0.0 { *value / alpha } else { 0.0 };
                        }
                    }
                    for (c, value) in sum.iter().take(format.channels()).enumerate() {
                        if !value.is_finite() {
                            return Err(RasterInputError::InvalidSample);
                        }
                        photocraft_color::try_write_sample_f64(target, format.sample, c, *value).map_err(|_| RasterInputError::InvalidSample)?;
                    }
                }
                cursor = end;
            }
        }
        out.try_write_interleaved_with(area, &encoded, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok((out, destination))
}

fn nearest(coordinate: i32, scale: f64) -> Result<i32, RasterInputError> {
    let value = ((f64::from(coordinate) + 0.5) / scale).floor();
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(RasterInputError::InvalidBounds);
    }
    Ok(value as i32)
}
fn read_raw(source: &Surface, bounds: Rect, mut x: i32, mut y: i32, clamp: bool, target: &mut [u8]) -> Result<(), RasterInputError> {
    if clamp {
        x = x.clamp(bounds.x0, bounds.x1 - 1);
        y = y.clamp(bounds.y0, bounds.y1 - 1);
    }
    if x < bounds.x0 || x >= bounds.x1 || y < bounds.y0 || y >= bounds.y1 {
        target.copy_from_slice(source.default_bytes());
        Ok(())
    } else {
        source.try_read_pixel_bytes(x, y, target)
    }
}
