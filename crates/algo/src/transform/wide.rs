//! Finite-ROI RGB/Gray exact nearest, f64 bilinear, or explicit BC(1/2,1/4) sampling.
use super::{Homography, Interp};
use crate::wide_geometry::{GeometryLimits, check_peak};
use photocraft_color::{ColorMode, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};

pub(super) fn warp(
    source: &Surface,
    bounds: Rect,
    h: &Homography,
    interp: Interp,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    warp_kernel(source, bounds, h, Some(interp), false, limits, control)
}

pub(super) fn bilinear_hidden(
    source: &Surface,
    bounds: Rect,
    h: &Homography,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    warp_kernel(source, bounds, h, Some(Interp::Bilinear), true, limits, control)
}

pub(super) fn cubic(source: &Surface, bounds: Rect, h: &Homography, limits: GeometryLimits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    warp_kernel(source, bounds, h, None, false, limits, control)
}

// Mitchell/Netravali (SIGGRAPH 1988), equation 8, B=1/2 and C=1/4.
// These explicit mathematical parameters are separate from legacy Catmull-Rom.
fn cubic_weight(distance: f64) -> f64 {
    let x = distance.abs();
    if x < 1. {
        (6. * x * x * x - 10.5 * x * x + 5.) / 6.
    } else if x < 2. {
        (-2. * x * x * x + 10.5 * x * x - 18. * x + 10.) / 6.
    } else {
        0.
    }
}

fn warp_kernel(
    source: &Surface,
    bounds: Rect,
    h: &Homography,
    interp: Option<Interp>,
    association_floor: bool,
    limits: GeometryLimits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    source.try_validate_tile_origins_with(control)?;
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if !matches!(source.format().mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(bounds, source.format())?;
    if h.0.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let inverse = h.inverse().ok_or(RasterInputError::InvalidBounds)?;
    if inverse.0.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let [a, b, dx, c, d, dy, g, i, j] = h.0;
    let raw = a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && g == 0.0 && i == 0.0 && j == 1.0 && dx == dx.round() && dy == dy.round();
    if !raw && interp.is_some_and(|value| !matches!(value, Interp::Bilinear | Interp::Nearest)) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let destination = extent(bounds, h)?;
    let mut format = source.format();
    format.alpha = true;
    let destination_bytes = checked_region_byte_len(destination, format)? as u64;
    let nearest = interp == Some(Interp::Nearest);
    let work = (destination_bytes / format.sample.bytes() as u64)
        .checked_mul(if raw || nearest {
            1
        } else if interp.is_none() {
            16
        } else if association_floor {
            8
        } else {
            4
        })
        .ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if raw {
        let mut default = [0u8; 32];
        let default = default.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
        default.get_mut(..source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(source.default_bytes());
        if !source.format().alpha {
            try_write_sample_f64(default, format.sample, format.channels() - 1, 1.0).map_err(|_| RasterInputError::InvalidSample)?;
        }
        let mut output = Surface::try_with_default_bytes(format, default)?;
        check_peak(source, &output, 0, limits)?;
        for tile in destination.tiles() {
            let area = tile.rect().intersect(&destination);
            let len = checked_region_byte_len(area, format)?;
            check_peak(source, &output, len as u64, limits)?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
            bytes.resize(len, 0);
            let mut cursor = 0usize;
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    if cursor.is_multiple_of(4096) && control.cancelled() {
                        return Err(RasterInputError::Cancelled);
                    }
                    let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                    let target = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                    source.try_read_pixel_bytes(
                        x - dx as i32,
                        y - dy as i32,
                        target.get_mut(..source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?,
                    )?;
                    if !source.format().alpha {
                        try_write_sample_f64(target, format.sample, format.channels() - 1, 1.0).map_err(|_| RasterInputError::InvalidSample)?;
                    }
                    cursor = end;
                }
            }
            output.try_write_interleaved_with(area, &bytes, control)?;
        }
        return Ok(output);
    }
    // The inverse denominator is affine; same nonzero sign at all corners
    // proves no projective horizon intersects the entire output rectangle.
    extent(destination, &inverse)?;
    let mut output = Surface::new(format);
    check_peak(source, &output, 0, limits)?;
    let channels = format.channels();
    let colors = channels.checked_sub(1).ok_or(RasterInputError::LengthMismatch)?;
    for tile in destination.tiles() {
        let area = tile.rect().intersect(&destination);
        let len = checked_region_byte_len(area, format)?;
        check_peak(source, &output, (len as u64).checked_add(if association_floor { 32 } else { 0 }).ok_or(RasterInputError::LimitExceeded)?, limits)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(len, 0);
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if cursor.is_multiple_of(4096) && control.cancelled() {
                    return Err(RasterInputError::Cancelled);
                }
                let (u, v) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                if !u.is_finite() || !v.is_finite() || u.abs() > 1_000_000.0 || v.abs() > 1_000_000.0 {
                    return Err(RasterInputError::InvalidBounds);
                }
                if nearest {
                    // Pixel cells are half-open. Inverse output centers select the
                    // containing input cell, retaining every authoritative sample bit.
                    let (px, py) = (u.floor() as i32, v.floor() as i32);
                    let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                    let target = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                    if px >= bounds.x0 && px < bounds.x1 && py >= bounds.y0 && py < bounds.y1 {
                        source.try_read_pixel_bytes(px, py, target.get_mut(..source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?)?;
                        if !source.format().alpha {
                            try_write_sample_f64(target, format.sample, colors, 1.0).map_err(|_| RasterInputError::InvalidSample)?;
                        }
                    }
                    cursor = end;
                    continue;
                }
                let (sx, sy) = (u - 0.5, v - 0.5);
                let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
                let (fx, fy) = (sx - f64::from(ix), sy - f64::from(iy));
                let mut sum = [0.0; 4];
                let taps = if interp.is_none() { -1..3 } else { 0..2 };
                for oy in taps.clone() {
                    for ox in taps.clone() {
                        let (px, py) = (ix + ox, iy + oy);
                        let weight = if interp.is_none() {
                            cubic_weight(fx - f64::from(ox)) * cubic_weight(fy - f64::from(oy))
                        } else {
                            (if ox == 0 { 1. - fx } else { fx }) * (if oy == 0 { 1. - fy } else { fy })
                        };
                        if weight == 0.0 || px < bounds.x0 || px >= bounds.x1 || py < bounds.y0 || py >= bounds.y1 {
                            continue;
                        }
                        let mut sample = [0.0; 4];
                        source.try_read_pixel_f64(px, py, sample.get_mut(..source.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
                        let alpha = if source.format().alpha { *sample.get(colors).ok_or(RasterInputError::LengthMismatch)? } else { 1.0 };
                        for value in sample.get(..source.channels()).ok_or(RasterInputError::LengthMismatch)? {
                            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                                return Err(RasterInputError::UnsupportedConversion);
                            }
                        }
                        // Independently observed Babl associated working formats retain
                        // actual alpha and use 2^-16 only as the color association floor.
                        let associated_alpha = if association_floor { alpha.max(1.0 / 65536.0) } else { alpha };
                        for channel in 0..colors {
                            *sum.get_mut(channel).ok_or(RasterInputError::LengthMismatch)? +=
                                *sample.get(channel).ok_or(RasterInputError::LengthMismatch)? * associated_alpha * weight;
                        }
                        *sum.get_mut(colors).ok_or(RasterInputError::LengthMismatch)? += alpha * weight;
                    }
                }
                let alpha = *sum.get(colors).ok_or(RasterInputError::LengthMismatch)?;
                for channel in 0..colors {
                    let value = sum.get_mut(channel).ok_or(RasterInputError::LengthMismatch)?;
                    // Cubic reconstruction has signed lobes: a negative nonzero
                    // alpha still has a mathematically defined straight color.
                    // Retain generated float overshoot rather than silently erase it.
                    *value = if association_floor {
                        *value / alpha.max(1.0 / 65536.0)
                    } else if alpha > 0.0 || (interp.is_none() && alpha < 0.0) {
                        *value / alpha
                    } else {
                        0.0
                    };
                }
                let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let target = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                for (channel, value) in sum.get(..channels).ok_or(RasterInputError::LengthMismatch)?.iter().enumerate() {
                    // A convex normalized result may exceed an endpoint by one f64
                    // rounding ulp; keep the admitted normalized domain closed.
                    try_write_sample_f64(target, format.sample, channel, if interp.is_none() || association_floor { *value } else { value.clamp(0.0, 1.0) })
                        .map_err(|_| RasterInputError::InvalidSample)?;
                }
                cursor = end;
            }
        }
        output.try_write_interleaved_with(area, &bytes, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(output)
}

pub(super) fn extent(bounds: Rect, h: &Homography) -> Result<Rect, RasterInputError> {
    if bounds.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let mut low = [f64::MAX; 2];
    let mut high = [f64::MIN; 2];
    let mut sign = 0.0;
    for (x, y) in [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x1, bounds.y1), (bounds.x0, bounds.y1)] {
        if i64::from(x).abs() > 1_000_000 || i64::from(y).abs() > 1_000_000 {
            return Err(RasterInputError::InvalidBounds);
        }
        let denom = h.0[6] * f64::from(x) + h.0[7] * f64::from(y) + h.0[8];
        if !denom.is_finite() || denom.abs() < 1e-12 || (sign != 0.0 && denom.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = denom.signum();
        let (u, v) = h.apply(f64::from(x), f64::from(y));
        if !u.is_finite() || !v.is_finite() || u.abs() > 1_000_000.0 || v.abs() > 1_000_000.0 {
            return Err(RasterInputError::InvalidBounds);
        }
        low[0] = low[0].min(u);
        low[1] = low[1].min(v);
        high[0] = high[0].max(u);
        high[1] = high[1].max(v);
    }
    Ok(Rect::new(low[0].floor() as i32, low[1].floor() as i32, high[0].ceil() as i32, high[1].ceil() as i32))
}
