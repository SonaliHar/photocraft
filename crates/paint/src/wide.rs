//! Checked native-encoding normal painting with authoritative f64 arithmetic.
use photocraft_color::{try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

/// One private atomic native pixel using the existing wide source-over schedule.
pub(crate) fn try_normal_native_pixel(out: &mut [u8], format: photocraft_color::PixelFormat, src: &[f64], weight: f64, lock_transparency: bool) -> Result<bool, RasterInputError> {
    let bpp = format.bytes_per_pixel();
    if out.len() != bpp || bpp > 40 || format.channels() > 5 || src.len() < format.channels() { return Err(RasterInputError::LengthMismatch); }
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) { return Err(RasterInputError::InvalidSample); }
    if weight == 0.0 { return Ok(false); }
    let mut staged = [0u8; 40];
    let pixel = staged.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    pixel.copy_from_slice(out);
    let mut dst = [0.0_f64; 5];
            if src.iter().take(format.channels()).any(|v| !v.is_finite()) {
                return Err(RasterInputError::InvalidSample);
            }
            if format.alpha && src.get(format.mode.color_channels()).copied() == Some(0.0) {
                return Ok(false);
            }
            if lock_transparency && format.alpha {
                let alpha = try_read_sample_f64(pixel, format.sample, format.mode.color_channels()).map_err(|_| RasterInputError::InvalidSample)?;
                if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                    return Err(RasterInputError::InvalidSample);
                }
                if alpha == 0.0 {
                    return Ok(false);
                }
            }
            for (channel, value) in dst.iter_mut().enumerate().take(format.channels()) {
                *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
                if !value.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
            }
            let alpha = format.mode.color_channels();
            if lock_transparency && format.alpha && dst.get(alpha).copied().ok_or(RasterInputError::LengthMismatch)? == 0.0 {
                return Ok(false);
            }
            if format.alpha {
                let da = dst.get(alpha).copied().ok_or(RasterInputError::LengthMismatch)?;
                let sa = src.get(alpha).copied().ok_or(RasterInputError::LengthMismatch)?;
                if !(0.0..=1.0).contains(&da) || !(0.0..=1.0).contains(&sa) {
                    return Err(RasterInputError::InvalidSample);
                }
                let sa = sa * weight;
                if sa == 0.0 {
                    return Ok(false);
                }
                let oa = sa + da * (1.0 - sa);
                for channel in 0..alpha {
                    let d = dst.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
                    let s = src.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
                    let value = if sa == 1.0 || da == 0.0 { s } else { (s * sa + d * da * (1.0 - sa)) / oa };
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                }
                if !lock_transparency {
                    try_write_sample_f64(pixel, format.sample, alpha, oa).map_err(|_| RasterInputError::InvalidSample)?;
                }
            } else {
                for channel in 0..alpha {
                    let d = dst.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
                    let s = src.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
                    let value = if weight == 1.0 { s } else { d * (1.0 - weight) + s * weight };
                    if !value.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                }
            }
    let changed = pixel != out;
    out.copy_from_slice(pixel);
    Ok(changed)
}

/// Source-over in the caller's native working encoding. Color/profile conversion
/// and non-normal blend modes must be resolved separately; mismatched color models or alpha layouts fail. Sample depths may differ;
/// the paint source can retain f64 precision until the final destination quantization.
/// Commits only after every tile, sample and cancellation check succeeds.
#[allow(clippy::too_many_arguments)]
pub fn try_apply_normal_coverage(
    target: &mut Surface,
    paint: &Surface,
    rect: Rect,
    coverage: &[f64],
    opacity: f64,
    selection: Option<&Surface>,
    lock_transparency: bool,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    if target.format().mode != paint.format().mode || target.format().alpha != paint.format().alpha {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(RasterInputError::InvalidSample);
    }
    // Checked raw access validates complete bounds/length before arithmetic.
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    target.try_validate_tile_origins_with(control)?;
    paint.try_validate_tile_origins_with(control)?;
    if let Some(selection) = selection {
        selection.try_validate_tile_origins_with(control)?;
    }
    let format = target.format();
    let bpp = format.bytes_per_pixel();
    let count = photocraft_raster::checked_region_byte_len(rect, format)? / bpp;
    if coverage.len() != count {
        return Err(RasterInputError::LengthMismatch);
    }
    let bytes = (count as u64).checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
    let coverage_bytes = (count as u64).checked_mul(8).ok_or(RasterInputError::LimitExceeded)?;
    let retained = target
        .resident_payload_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(bytes))
        .and_then(|v| v.checked_add(coverage_bytes))
        .and_then(|v| v.checked_add(256 * 256 * bpp as u64 * 3))
        .ok_or(RasterInputError::LimitExceeded)?;
    let selection_bytes = selection.map_or(Ok(0), Surface::resident_payload_bytes)?;
    let retained = retained.checked_add(paint.resident_payload_bytes()?).and_then(|v| v.checked_add(selection_bytes)).ok_or(RasterInputError::LimitExceeded)?;
    if retained > photocraft_raster::MAX_CHECKED_RASTER_BYTES || bytes.checked_add(coverage_bytes).ok_or(RasterInputError::LimitExceeded)? > 1_000_000_000 {
        return Err(RasterInputError::LimitExceeded);
    }
    for (index, value) in coverage.iter().enumerate() {
        if index.is_multiple_of(4096) && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let width = usize::try_from(rect.width()).map_err(|_| RasterInputError::InvalidBounds)?;
    let mut staged = target.clone();
    for tile in rect.tiles() {
        let part = tile.rect().intersect(&rect);
        let mut raw = target.try_read_interleaved_with(part, control)?;
        let mut changed = false;
        let part_width = usize::try_from(part.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            let x = part.x0 + (index % part_width) as i32;
            let y = part.y0 + (index / part_width) as i32;
            let coverage_index =
                ((y - rect.y0) as usize).checked_mul(width).and_then(|v| v.checked_add((x - rect.x0) as usize)).ok_or(RasterInputError::LimitExceeded)?;
            let mut weight = coverage.get(coverage_index).copied().ok_or(RasterInputError::LengthMismatch)? * opacity;
            if weight == 0.0 {
                continue;
            }
            let mut src = [0.0_f64; 5];
            if let Some(mask) = selection {
                let mut values = [0.0; 5];
                mask.try_read_pixel_f64(x, y, values.get_mut(..mask.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
                let mask_weight = values.first().copied().ok_or(RasterInputError::LengthMismatch)?;
                if !mask_weight.is_finite() || !(0.0..=1.0).contains(&mask_weight) {
                    return Err(RasterInputError::InvalidSample);
                }
                weight *= mask_weight;
            }
            if weight == 0.0 {
                continue;
            }
            paint.try_read_pixel_f64(x, y, src.get_mut(..format.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
            changed |= try_normal_native_pixel(pixel, format, &src, weight, lock_transparency)?;
        }
        if changed {
            staged.try_write_interleaved_with(part, &raw, control)?;
        }
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    *target = staged;
    Ok(())
}
