//! Checked authoritative wide edits. Unsupported kernels never enter the f32 Image path.
use photocraft_color::{try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

/// Invert color channels in their native encoding; alpha and unselected bytes stay exact.
pub fn try_invert(source: &Surface, area: Rect, selection: Option<&Surface>, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    validate_extent(source, area)?;
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let mut out = source.clone();
    for tile in area.tiles() {
        let rect = tile.rect().intersect(&area);
        let mut raw = source.try_read_interleaved_with(rect, control)?;
        let width = usize::try_from(rect.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            let x = rect.x0 + (index % width) as i32;
            let y = rect.y0 + (index / width) as i32;
            let weight = selection_weight(selection, x, y)?;
            if weight == 0.0 {
                continue;
            }
            for channel in 0..format.mode.color_channels() {
                let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
                if !value.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                // Stored Lab chroma mirrors about encoded neutral at every
                // authoritative wide depth, matching the existing stored-channel adjustment.
                let mirror = if format.mode == photocraft_color::ColorMode::Lab
                    && matches!(format.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64)
                    && channel > 0
                {
                    256.0 / 255.0
                } else {
                    1.0
                };
                let inverted = mirror - value;
                let result = if weight == 1.0 { inverted } else { value * (1.0 - weight) + inverted * weight };
                if !result.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                try_write_sample_f64(pixel, format.sample, channel, result).map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
        out.try_write_interleaved_with(rect, &raw, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(out)
}

pub(crate) fn selection_weight(selection: Option<&Surface>, x: i32, y: i32) -> Result<f64, RasterInputError> {
    let Some(selection) = selection else {
        return Ok(1.0);
    };
    let mut values = [0.0; 5];
    let values = values.get_mut(..selection.channels()).ok_or(RasterInputError::LengthMismatch)?;
    selection.try_read_pixel_f64(x, y, values)?;
    let value = values.first().copied().ok_or(RasterInputError::LengthMismatch)?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(value)
}

/// Exact integer translation of explicit source bounds, including raw defaults.
pub fn try_translate(source: &Surface, bounds: Rect, dx: i32, dy: i32, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    let destination = Rect::new(
        bounds.x0.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?,
        bounds.y0.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?,
        bounds.x1.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?,
        bounds.y1.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?,
    );
    // Validate both extents without allocating their complete image buffers.
    validate_extent(source, bounds)?;
    validate_extent(source, destination)?;
    let mut out = Surface::try_with_default_bytes(source.format(), source.default_bytes())?;
    for tile in bounds.tiles() {
        let rect = tile.rect().intersect(&bounds);
        let data = source.try_read_interleaved_with(rect, control)?;
        let translated = Rect::new(rect.x0 + dx, rect.y0 + dy, rect.x1 + dx, rect.y1 + dy);
        out.try_write_interleaved_with(translated, &data, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(out)
}

/// Keep explicit bounds, byte for byte. Outside reads retain the original exact default.
pub fn try_crop(source: &Surface, bounds: Rect, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    try_translate(source, bounds, 0, 0, control)
}

fn validate_extent(source: &Surface, bounds: Rect) -> Result<(), RasterInputError> {
    photocraft_raster::checked_region_byte_len(bounds, source.format())?;
    Ok(())
}
