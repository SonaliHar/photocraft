//! Caller-defined straight-channel bilinear geometry, not a GEGL source mapping.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct Affine { pub xx: f64, pub xy: f64, pub x_offset: f64, pub yx: f64, pub yy: f64, pub y_offset: f64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha { Channelwise, PreserveNearest }
#[derive(Clone, Copy, Debug)]
pub enum Abyss<'a> { Clamp, ConstantNative(&'a [u8]) }
pub struct Sampler<'a> {
    pub map: Affine,
    pub source_extent: Rect,
    pub output_extent: Rect,
    pub output_format: PixelFormat,
    pub alpha: Alpha,
    pub abyss: Abyss<'a>,
    pub output_default: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits { pub resident_bytes: u64, pub work_units: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { Raster(RasterInputError), ContextMismatch, InvalidConfiguration, NonFinite, InvalidAlpha }
impl From<RasterInputError> for Error { fn from(value: RasterInputError) -> Self { Self::Raster(value) } }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "straight-channel bilinear geometry: {self:?}") }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> { control.check().map_err(|_| RasterInputError::Cancelled.into()) }
fn limit() -> Error { RasterInputError::LimitExceeded.into() }
fn tile_count(rect: Rect) -> Result<u64> {
    if rect.is_empty() { return Err(RasterInputError::InvalidBounds.into()); }
    let first = TileCoord::containing(rect.x0, rect.y0);
    let last = TileCoord::containing(rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or_else(limit)
}
/// Includes four source lookups, native sample stages, source metadata validation
/// and repeated growing output-map publications before allocation/traversal.
pub fn required_work(source: &Surface, roi: Rect) -> Result<u64> {
    let bytes = checked_region_byte_len(roi, source.format())? as u64;
    let stored = source.tile_count() as u64;
    let tiles = tile_count(roi)?;
    bytes.checked_mul(stored.checked_add(1).and_then(|n| n.checked_mul(8)).and_then(|n| n.checked_add(128)).ok_or_else(limit)?)
        .and_then(|n| n.checked_add(stored.checked_mul(8)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?)).ok_or_else(limit)
}
fn validate_pixel(pixel: &[u8], format: PixelFormat) -> Result<()> {
    if pixel.len() != format.bytes_per_pixel() { return Err(RasterInputError::LengthMismatch.into()); }
    for channel in 0..format.channels() {
        let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() { return Err(Error::NonFinite); }
        if channel >= format.mode.color_channels() && !(0.0..=1.0).contains(&value) { return Err(Error::InvalidAlpha); }
    }
    Ok(())
}
fn transform(map: Affine, x: i32, y: i32) -> Result<(f64, f64)> {
    let x = f64::from(x) + 0.5;
    let y = f64::from(y) + 0.5;
    let xx = map.xx * x;
    let xy = map.xy * y;
    let yx = map.yx * x;
    let yy = map.yy * y;
    let horizontal = xx + xy;
    let vertical = yx + yy;
    let source_x = horizontal + map.x_offset;
    let source_y = vertical + map.y_offset;
    if [xx, xy, yx, yy, horizontal, vertical, source_x, source_y].iter().any(|value| !value.is_finite()) { return Err(Error::NonFinite); }
    let position = (source_x - 0.5, source_y - 0.5);
    if !position.0.is_finite() || !position.1.is_finite() { return Err(Error::NonFinite); }
    Ok(position)
}
fn tap(source: &Surface, configuration: &Sampler<'_>, x: i64, y: i64, output: &mut [u8], control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let extent = configuration.source_extent;
    let inside = x >= i64::from(extent.x0) && x < i64::from(extent.x1) && y >= i64::from(extent.y0) && y < i64::from(extent.y1);
    if !inside && let Abyss::ConstantNative(pixel) = configuration.abyss {
        output.copy_from_slice(pixel);
    } else {
        let x = x.clamp(i64::from(extent.x0), i64::from(extent.x1) - 1);
        let y = y.clamp(i64::from(extent.y0), i64::from(extent.y1) - 1);
        source.try_read_pixel_bytes(i32::try_from(x).map_err(|_| RasterInputError::InvalidBounds)?, i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?, output)?;
    }
    validate_pixel(output, source.format())
}
fn lerp(first: f64, second: f64, weight: f64) -> Result<f64> {
    if weight == 0.0 { return Ok(first); }
    let first_part = first * (1.0 - weight);
    let second_part = second * weight;
    let value = first_part + second_part;
    if !first_part.is_finite() || !second_part.is_finite() || !value.is_finite() { return Err(Error::NonFinite); }
    Ok(value)
}
fn sample(source: &Surface, configuration: &Sampler<'_>, mut x: f64, mut y: f64, destination: &mut [u8], control: &Interrupt<'_>) -> Result<()> {
    let extent = configuration.source_extent;
    match configuration.abyss {
        Abyss::Clamp => {
            x = x.clamp(f64::from(extent.x0), f64::from(extent.x1) - 1.0);
            y = y.clamp(f64::from(extent.y0), f64::from(extent.y1) - 1.0);
        }
        Abyss::ConstantNative(pixel) if x <= f64::from(extent.x0) - 1.0 || x >= f64::from(extent.x1) || y <= f64::from(extent.y0) - 1.0 || y >= f64::from(extent.y1) => {
            // No tap can contribute an in-extent sample. Constant pixel was
            // already validated; convert depth explicitly if requested.
            return encode_pixel(pixel, source.format(), destination, configuration.output_format);
        }
        Abyss::ConstantNative(_) => {}
    }
    // Positions here are bounded by finite i32 extents plus one neighbor.
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let wx = x - x0 as f64;
    let wy = y - y0 as f64;
    let x1 = if wx == 0.0 { x0 } else { x0.checked_add(1).ok_or(RasterInputError::InvalidBounds)? };
    let y1 = if wy == 0.0 { y0 } else { y0.checked_add(1).ok_or(RasterInputError::InvalidBounds)? };
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let mut corners = [[0u8; 32]; 4];
    for (index, (tx, ty)) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].into_iter().enumerate() {
        let pixel = corners.get_mut(index).and_then(|pixel| pixel.get_mut(..bpp)).ok_or(RasterInputError::LengthMismatch)?;
        tap(source, configuration, tx, ty, pixel, control)?;
    }
    let first = corners.first().and_then(|pixel| pixel.get(..bpp)).ok_or(RasterInputError::LengthMismatch)?;
    if configuration.output_format == format && corners.iter().all(|pixel| pixel.get(..bpp) == Some(first)) {
        destination.copy_from_slice(first);
        return Ok(());
    }
    for channel in 0..format.channels() {
        let mut values = [0.0; 4];
        for (slot, pixel) in corners.iter().enumerate() {
            *values.get_mut(slot).ok_or(RasterInputError::LengthMismatch)? = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        }
        let value = if channel >= format.mode.color_channels() && configuration.alpha == Alpha::PreserveNearest {
            // Exact half ties select the greater coordinate in each axis.
            let nearest = usize::from(wx >= 0.5) + 2 * usize::from(wy >= 0.5);
            *values.get(nearest).ok_or(RasterInputError::LengthMismatch)?
        } else {
            let a = lerp(*values.first().ok_or(RasterInputError::LengthMismatch)?, *values.get(1).ok_or(RasterInputError::LengthMismatch)?, wx)?;
            let b = lerp(*values.get(2).ok_or(RasterInputError::LengthMismatch)?, *values.get(3).ok_or(RasterInputError::LengthMismatch)?, wx)?;
            lerp(a, b, wy)?
        };
        if channel >= format.mode.color_channels() && !(0.0..=1.0).contains(&value) { return Err(Error::InvalidAlpha); }
        try_write_sample_f64(destination, configuration.output_format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
    }
    validate_pixel(destination, configuration.output_format)
}
fn encode_pixel(source: &[u8], source_format: PixelFormat, destination: &mut [u8], output_format: PixelFormat) -> Result<()> {
    if source_format == output_format { destination.copy_from_slice(source); return Ok(()); }
    for channel in 0..source_format.channels() {
        let value = try_read_sample_f64(source, source_format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        try_write_sample_f64(destination, output_format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
    }
    validate_pixel(destination, output_format)
}

/// Lattice centers are (integer x+0.5, integer y+0.5). The map converts an
/// output center to a source center; subtract 0.5 before neighbor selection.
/// Two unfused f64 horizontal weighted sums precede one vertical weighted sum;
/// final F32/F64 encoding happens once. Colors are straight finite HDR, alpha
/// is unit-valued; no premultiplication, TRC or CMS conversion occurs. Contexts
/// must match the caller's established interpretation. Same-format identity or
/// identical taps retain exact native words (including signed zero). Outside ROI
/// uses the exact finite caller default. All private output is discarded on error.
pub fn try_apply(source: &Surface, configuration: &Sampler<'_>, roi: Rect, source_context: WorkingContext, output_context: WorkingContext,
    limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    if source_context != output_context { return Err(Error::ContextMismatch); }
    let format = source.format();
    let output_format = configuration.output_format;
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64)
        || !matches!(output_format.sample, SampleType::F32 | SampleType::F64) || output_format.mode != format.mode || output_format.alpha != format.alpha {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    let map = configuration.map;
    if [map.xx, map.xy, map.x_offset, map.yx, map.yy, map.y_offset].iter().any(|value| !value.is_finite()) { return Err(Error::InvalidConfiguration); }
    checked_region_byte_len(configuration.source_extent, format)?;
    checked_region_byte_len(configuration.output_extent, output_format)?;
    tile_count(configuration.source_extent)?;
    tile_count(configuration.output_extent)?;
    if roi.intersect(&configuration.output_extent) != roi { return Err(RasterInputError::InvalidBounds.into()); }
    let count = tile_count(roi)?;
    let work = required_work(source, roi)?;
    if work > limits.work_units { return Err(limit()); }
    validate_pixel(configuration.output_default, output_format)?;
    if let Abyss::ConstantNative(pixel) = configuration.abyss { validate_pixel(pixel, format)?; }
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(output_format.bytes_per_pixel() as u64)).ok_or_else(limit)?;
    let scratch_envelope = tile_bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    let extra = count.checked_mul(tile_bytes.checked_add(1024).ok_or_else(limit)?)
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(6)?)).and_then(|n| n.checked_add(scratch_envelope)).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if extra > cap { return Err(limit()); }
    let source_tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(limit)?;
    let mut retained = source.default_bytes().len() as u64;
    if retained as usize != format.bytes_per_pixel() { return Err(RasterInputError::LengthMismatch.into()); }
    for (index, (coordinate, tile)) in source.tiles().enumerate() {
        if index.is_multiple_of(256) { poll(control)?; }
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != source_tile_bytes { return Err(RasterInputError::LengthMismatch.into()); }
        retained = retained.checked_add(source_tile_bytes.checked_add(256).ok_or_else(limit)?).ok_or_else(limit)?;
        if retained.checked_add(extra).ok_or_else(limit)? > cap { return Err(limit()); }
    }
    if retained.checked_add(extra).ok_or_else(limit)? > cap { return Err(limit()); }
    poll(control)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(usize::try_from(tile_bytes).map_err(|_| limit())?).map_err(|_| RasterInputError::AllocationFailed)?;
    if pixels.capacity() as u64 > scratch_envelope { return Err(limit()); }
    let mut output = Surface::try_with_default_bytes(output_format, configuration.output_default)?;
    let bpp = output_format.bytes_per_pixel();
    for coordinate in roi.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&roi);
        let length = checked_region_byte_len(part, output_format)?;
        pixels.truncate(length);
        while pixels.len() < length {
            poll(control)?;
            pixels.resize(pixels.len().checked_add((length - pixels.len()).min(64 << 10)).ok_or_else(limit)?, 0);
        }
        let width = usize::try_from(i64::from(part.x1) - i64::from(part.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let row_bytes = width.checked_mul(bpp).filter(|n| *n > 0).ok_or(RasterInputError::InvalidBounds)?;
        for (row_index, row) in pixels.chunks_exact_mut(row_bytes).enumerate() {
            poll(control)?;
            let y = part.y0.checked_add(i32::try_from(row_index).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
            for (column, pixel) in row.chunks_exact_mut(bpp).enumerate() {
                let x = part.x0.checked_add(i32::try_from(column).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                let (sx, sy) = transform(map, x, y)?;
                sample(source, configuration, sx, sy, pixel, control)?;
            }
        }
        output.try_write_interleaved_with(part, &pixels, control)?;
    }
    poll(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext { WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [3; 32] } }
    fn limits() -> Limits { Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 } }
    fn identity() -> Affine { Affine { xx: 1.0, xy: 0.0, x_offset: 0.0, yx: 0.0, yy: 1.0, y_offset: 0.0 } }
    fn rect(width: i32, height: i32) -> Rect { Rect { x0: 0, y0: 0, x1: width, y1: height } }
    fn sampler<'a>(format: PixelFormat, extent: Rect, default: &'a [u8]) -> Sampler<'a> {
        Sampler { map: identity(), source_extent: extent, output_extent: extent, output_format: format, alpha: Alpha::Channelwise, abyss: Abyss::Clamp, output_default: default }
    }

    #[test]
    fn four_corners_use_straight_hdr_colors_and_explicit_alpha_policy() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let extent = rect(2, 2);
        let mut source = Surface::new(format);
        source.try_write_region_f64(extent, &[-2.0, 0.0, 2.0, 1.0, 4.0, 0.5, 8.0, 0.25]).unwrap();
        let default = [0u8; 16];
        let mut config = sampler(format, extent, &default);
        config.map.x_offset = 0.5;
        config.map.y_offset = 0.5;
        let region = rect(1, 1);
        let output = try_apply(&source, &config, region, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(region).unwrap(), vec![3.0, 0.4375]);
        config.alpha = Alpha::PreserveNearest;
        let output = try_apply(&source, &config, region, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(region).unwrap(), vec![3.0, 0.25]);
        assert_eq!(source.try_read_region_f64(extent).unwrap(), vec![-2.0, 0.0, 2.0, 1.0, 4.0, 0.5, 8.0, 0.25]);
    }

    #[test]
    fn negative_identity_retains_f64_low_bits_signed_zero_and_raw_outside_default() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = Rect { x0: -1, y0: -1, x1: 1, y1: 0 };
        let mut source = Surface::new(format);
        let mut raw = Vec::new();
        for word in [(-0.0f64).to_bits(), 1.0f64.to_bits() + 1] { raw.extend_from_slice(&word.to_ne_bytes()); }
        source.try_write_interleaved_with(extent, &raw, &Interrupt::NONE).unwrap();
        let default = (-0.0f64).to_ne_bytes();
        let config = sampler(format, extent, &default);
        let output = try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(extent).unwrap(), raw);
        assert_eq!(output.default_bytes(), default);
        assert_eq!(output.try_read_interleaved(rect(1, 1)).unwrap(), default);
        assert_eq!(source.try_read_interleaved(extent).unwrap(), raw);
    }

    #[test]
    fn constant_native_edge_and_clamp_are_distinct_contracts() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = rect(1, 1);
        let mut source = Surface::new(format);
        source.try_write_region_f64(extent, &[-2.0]).unwrap();
        let default = 0.0f64.to_ne_bytes();
        let constant = 10.0f64.to_ne_bytes();
        let mut config = sampler(format, extent, &default);
        config.map.x_offset = -0.5;
        config.abyss = Abyss::ConstantNative(&constant);
        let output = try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(extent).unwrap(), vec![4.0]);
        config.abyss = Abyss::Clamp;
        let output = try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(extent).unwrap(), vec![-2.0]);
        config.abyss = Abyss::ConstantNative(&constant);
        config.map.x_offset = -f64::MAX;
        let output = try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(extent).unwrap(), vec![10.0]);
    }

    #[test]
    fn nonfinite_samples_quantization_overflow_context_limits_and_cancel_refuse() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = rect(1, 1);
        let mut source = Surface::new(format);
        let default = 0.0f64.to_ne_bytes();
        let mut config = sampler(format, extent, &default);
        let raw = 0x7ff8_0000_0000_1234u64.to_ne_bytes();
        source.try_write_interleaved_with(extent, &raw, &Interrupt::NONE).unwrap();
        assert_eq!(try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).unwrap_err(), Error::NonFinite);
        assert!(matches!(try_apply(&source, &config, extent, context(), context(), Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(Error::Raster(RasterInputError::LimitExceeded))));
        assert_eq!(try_apply(&source, &config, extent, context(), WorkingContext { profile_identity: [4; 32], ..context() }, limits(), &Interrupt::NONE).unwrap_err(), Error::ContextMismatch);
        source.try_write_region_f64(extent, &[f64::MAX]).unwrap();
        let narrow_default = 0.0f32.to_ne_bytes();
        config.output_format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        config.output_default = &narrow_default;
        assert!(try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::NONE).is_err());
        config.output_format = format;
        config.output_default = &default;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 8;
        assert!(matches!(try_apply(&source, &config, extent, context(), context(), limits(), &Interrupt::cancel_only(&cancel)), Err(Error::Raster(RasterInputError::Cancelled))));
        assert_eq!(source.try_read_region_f64(extent).unwrap(), vec![f64::MAX]);
    }
}
