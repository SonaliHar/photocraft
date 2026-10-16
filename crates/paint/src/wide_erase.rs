//! Checked alpha erasing without decoding or rewriting unrelated color samples.
use photocraft_color::{try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct EraseLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for EraseLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EraseColorPolicy {
    /// Matches the stroke eraser: keep exact hidden color samples when alpha reaches zero.
    Preserve,
    /// Matches the region eraser's explicit empty-pixel behavior at complete erasure.
    ClearAtZero,
}

/// One atomic alpha-only pixel. Preserve never decodes color words; locked
/// erasing returns before any sample decode, after shape and weight validation.
pub(crate) fn try_erase_native_pixel(out: &mut [u8], format: photocraft_color::PixelFormat, weight: f64, lock_transparency: bool, policy: EraseColorPolicy) -> Result<bool, RasterInputError> {
    let bpp = format.bytes_per_pixel();
    if out.len() != bpp || bpp > 40 { return Err(RasterInputError::LengthMismatch); }
    if !format.alpha { return Err(RasterInputError::UnsupportedConversion); }
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) { return Err(RasterInputError::InvalidSample); }
    if lock_transparency || weight == 0.0 { return Ok(false); }
    let alpha = format.mode.color_channels();
    let value = try_read_sample_f64(out, format.sample, alpha).map_err(|_| RasterInputError::InvalidSample)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) { return Err(RasterInputError::InvalidSample); }
    let mut staged = [0u8; 40];
    let pixel = staged.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    pixel.copy_from_slice(out);
    if value == 0.0 {
        if policy == EraseColorPolicy::ClearAtZero { pixel.fill(0); }
    } else {
        let result = value * (1.0 - weight);
        if policy == EraseColorPolicy::ClearAtZero && result == 0.0 { pixel.fill(0); }
        else { try_write_sample_f64(pixel, format.sample, alpha, result).map_err(|_| RasterInputError::InvalidSample)?; }
    }
    let changed = pixel != out;
    out.copy_from_slice(pixel);
    Ok(changed)
}

/// Coverage/opacity/selection are bounded unit weights. Commits once after all checks.
/// No-alpha background erasing needs a separate background-color operation.
#[allow(clippy::too_many_arguments)]
pub fn try_erase_coverage(
    target: &mut Surface,
    area: Rect,
    coverage: &[f64],
    opacity: f64,
    selection: Option<&Surface>,
    lock_transparency: bool,
    policy: EraseColorPolicy,
    limits: EraseLimits,
    control: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    target.try_validate_tile_origins_with(control)?;
    if let Some(selection) = selection {
        selection.try_validate_tile_origins_with(control)?;
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(RasterInputError::InvalidSample);
    }
    let format = target.format();
    let bpp = format.bytes_per_pixel();
    let bytes = checked_region_byte_len(area, format)?;
    if coverage.len() != bytes / bpp {
        return Err(RasterInputError::LengthMismatch);
    }
    let coverage_bytes = (coverage.len() as u64).checked_mul(8).ok_or(RasterInputError::LimitExceeded)?;
    let work = (bytes as u64).checked_add(coverage_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let limit = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let retained = target
        .resident_payload_bytes()?
        .checked_add(selection.map_or(Ok(0), Surface::resident_payload_bytes)?)
        .and_then(|v| v.checked_add(coverage_bytes))
        .ok_or(RasterInputError::LimitExceeded)?;
    if retained > limit || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut any_coverage = false;
    for (i, value) in coverage.iter().enumerate() {
        if i.is_multiple_of(4096) && control.cancelled() {
            return Err(RasterInputError::Cancelled);
        }
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
        any_coverage |= *value != 0.0;
    }
    if !format.alpha {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if lock_transparency || opacity == 0.0 || !any_coverage {
        return Ok(());
    }
    let width = area.width() as usize;
    let mut staged = target.clone();
    for tile in area.tiles() {
        let part = tile.rect().intersect(&area);
        let scratch = checked_region_byte_len(part, format)? as u64;
        let peak = retained
            .checked_add(staged.resident_payload_bytes()?)
            .and_then(|v| v.checked_add(scratch.checked_mul(3)?))
            .and_then(|v| v.checked_add((256 * 256 * bpp * 2) as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        if peak > limit {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut raw = target.try_read_interleaved_with(part, control)?;
        let part_width = part.width() as usize;
        let mut changed = false;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            let x = part.x0 + (index % part_width) as i32;
            let y = part.y0 + (index / part_width) as i32;
            let offset =
                ((y - area.y0) as usize).checked_mul(width).and_then(|v| v.checked_add((x - area.x0) as usize)).ok_or(RasterInputError::LimitExceeded)?;
            let mut weight = coverage.get(offset).copied().ok_or(RasterInputError::LengthMismatch)? * opacity;
            if weight == 0.0 {
                continue;
            }
            if let Some(selection) = selection {
                let mut values = [0.0; 5];
                selection.try_read_pixel_f64(x, y, values.get_mut(..selection.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
                let value = values.first().copied().ok_or(RasterInputError::LengthMismatch)?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(RasterInputError::InvalidSample);
                }
                weight *= value;
            }
            if weight == 0.0 {
                continue;
            }
            changed |= try_erase_native_pixel(pixel, format, weight, false, policy)?;
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
