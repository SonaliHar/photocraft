//! Standalone rectangular mean in an explicitly supplied working context.
//! This defines its own unfused binary64 rolling order, not a GEGL evaluator.
//! No ICC lookup, transfer conversion or premultiplication is performed.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abyss {
    Clamp,
    Zero,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    PreserveRaw,
    Filter,
}
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub radius_x: u16,
    pub radius_y: u16,
    pub abyss: Abyss,
    pub alpha: Alpha,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Radius,
    ContextMismatch,
    NonFinite,
    AlphaRange,
}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rectangular box mean: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn index(row: usize, width: usize, column: usize, count: usize) -> Result<usize> {
    row.checked_mul(width).and_then(|n| n.checked_add(column)).and_then(|n| n.checked_mul(count)).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn slice(values: &[f64], start: usize, count: usize) -> Result<&[f64]> {
    values.get(start..start.checked_add(count).ok_or(RasterInputError::LimitExceeded)?).ok_or_else(|| RasterInputError::LengthMismatch.into())
}
fn finite(value: f64) -> Result<f64> {
    if value.is_finite() { Ok(value) } else { Err(Error::NonFinite) }
}
/// Checked support rectangle; sampling beyond the full extent follows Abyss.
pub fn required_input(roi: Rect, params: Params) -> Result<Rect> {
    if params.radius_x > 128 || params.radius_y > 128 {
        return Err(Error::Radius);
    }
    let x = i32::from(params.radius_x);
    let y = i32::from(params.radius_y);
    Ok(Rect::new(
        roi.x0.checked_sub(x).ok_or(RasterInputError::InvalidBounds)?,
        roi.y0.checked_sub(y).ok_or(RasterInputError::InvalidBounds)?,
        roi.x1.checked_add(x).ok_or(RasterInputError::InvalidBounds)?,
        roi.y1.checked_add(y).ok_or(RasterInputError::InvalidBounds)?,
    ))
}
fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
struct Meter {
    limit: u64,
    used: u64,
    staging: u64,
}
impl Meter {
    fn debit(&mut self, units: u64) -> Result<()> {
        self.used = add(self.used, units)?;
        if self.used > self.limit {
            return Err(RasterInputError::LimitExceeded.into());
        }
        Ok(())
    }
}
impl StagingBudget for Meter {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        if bytes > self.staging { Err(RasterInputError::LimitExceeded) } else { Ok(()) }
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.debit(units).map_err(|_| RasterInputError::LimitExceeded)
    }
}
fn allocate<T: Default + Clone>(length: usize, control: &Interrupt<'_>) -> Result<Vec<T>> {
    poll(control)?;
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    // The observable Vec capacity must fit the prepaid requested owner exactly.
    if values.capacity() != length {
        return Err(RasterInputError::AllocationFailed.into());
    }
    let chunk = (8192 / std::mem::size_of::<T>().max(1)).max(1);
    while values.len() < length {
        poll(control)?;
        let end = values.len().saturating_add(chunk).min(length);
        values.resize(end, T::default());
    }
    Ok(values)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn read(source: &Surface, extent: Rect, mut x: i64, mut y: i64, abyss: Abyss, pixel: &mut [f64], meter: &mut Meter, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    meter.debit(mul(pixel.len() as u64, 32)?)?;
    let inside = x >= i64::from(extent.x0) && x < i64::from(extent.x1) && y >= i64::from(extent.y0) && y < i64::from(extent.y1);
    if !inside {
        match abyss {
            Abyss::Zero => {
                pixel.fill(0.0);
                return Ok(());
            }
            Abyss::Clamp => {
                x = x.clamp(i64::from(extent.x0), i64::from(extent.x1) - 1);
                y = y.clamp(i64::from(extent.y0), i64::from(extent.y1) - 1);
            }
        }
    }
    source.try_read_pixel_f64(
        i32::try_from(x).map_err(|_| RasterInputError::InvalidBounds)?,
        i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?,
        pixel,
    )?;
    if pixel.iter().any(|value| !value.is_finite()) {
        return Err(Error::NonFinite);
    }
    if source.format().alpha && pixel.last().is_none_or(|value| !(0.0..=1.0).contains(value)) {
        return Err(Error::AlphaRange);
    }
    Ok(())
}
/// Each tile starts horizontal sums left-to-right. Subsequent columns subtract
/// the outgoing sample, then add the incoming sample. Horizontal means divide
/// by 2rx+1 in f64; vertical sums use the same top-to-bottom/rolling order and
/// divide by 2ry+1. No fused multiply-add or intermediate native quantization.
/// Filter alpha is straight-channel averaging, not premultiplied-color averaging.
/// PreserveRaw validates sampled alpha but keeps the exact center alpha bytes.
/// Original default/other tiles and pixels outside ROI remain byte-exact.
/// Limits include known payload/scratch/map envelopes, not global allocator RSS.
#[allow(clippy::too_many_arguments)]
pub fn try_apply(
    source: &Surface,
    full_extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    params: Params,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    poll(control)?;
    if source_context != expected_context {
        return Err(Error::ContextMismatch);
    }
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    checked_region_byte_len(full_extent, format)?;
    checked_region_byte_len(roi, format)?;
    if full_extent.is_empty() || roi.intersect(&full_extent) != roi {
        return Err(RasterInputError::InvalidBounds.into());
    }
    required_input(roi, params)?;
    if roi.is_empty() {
        return Ok(source.clone());
    }
    let tiles = tile_count(roi)?;
    let retained = source.tile_count() as u64;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let bpp = format.bytes_per_pixel();
    let channels = source.channels();
    let count = if params.alpha == Alpha::Filter { channels } else { format.mode.color_channels() };
    let rx = usize::from(params.radius_x);
    let ry = usize::from(params.radius_y);
    let nx = rx * 2 + 1;
    let ny = ry * 2 + 1;
    let side = usize::try_from(TILE_SIZE).map_err(|_| RasterInputError::LimitExceeded)?;
    let payload = mul(mul(side as u64, side as u64)?, bpp as u64)?;
    let replacements = mul(tiles, add(payload, 512)?)?;
    let max_samples = add(
        mul(mul((side + 2 * ry) as u64, side as u64)?, count as u64)?,
        add(mul((side + 2 * rx) as u64, channels as u64)?, mul(side as u64, count as u64)?)?,
    )?;
    let scratch = add(mul(max_samples, 8)?, add(payload, mul(64 * 64, bpp as u64)?)?)?;
    let maps = mul(add(retained, tiles)?, 1024)?;
    let staging = add(mul(payload, 4)?, 8192)?;
    let extra = add(add(replacements, scratch)?, add(maps, staging)?)?;
    if cap == 0 || extra > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    // Conservative constant-time tile upper bound includes decoding, rolling,
    // validation, full-tile raw preservation and staged cell copy/publication.
    let per_tile = add(mul(mul((side + 256) as u64, (side + 256) as u64)?, mul(channels as u64, 256)?)?, mul(payload, 16)?)?;
    let work = add(mul(tiles, per_tile)?, add(mul(retained, 32)?, mul(tiles, mul(add(retained, tiles)?, 512)?)?)?)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut resident = source.default_bytes().len() as u64;
    for (coordinate, tile) in source.tiles() {
        poll(control)?;
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch.into());
        }
        resident = add(resident, tile.bytes().len() as u64)?;
    }
    if add(resident, extra)? > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut meter = Meter { limit: limits.work_units, used: mul(retained, 32)?, staging };
    poll(control)?;
    let mut output = source.clone();
    for coordinate in roi.tiles() {
        poll(control)?;
        let tile_rect = coordinate.rect();
        let part = tile_rect.intersect(&roi);
        let width = usize::try_from(part.width()).map_err(|_| RasterInputError::LimitExceeded)?;
        let height = usize::try_from(part.height()).map_err(|_| RasterInputError::LimitExceeded)?;
        let rows = height.checked_add(2 * ry).ok_or(RasterInputError::LimitExceeded)?;
        let row_pixels = width.checked_add(2 * rx).ok_or(RasterInputError::LimitExceeded)?;
        let mut row_values = allocate::<f64>(row_pixels.checked_mul(channels).ok_or(RasterInputError::LimitExceeded)?, control)?;
        let mut horizontal = allocate::<f64>(rows.checked_mul(width).and_then(|n| n.checked_mul(count)).ok_or(RasterInputError::LimitExceeded)?, control)?;
        for row in 0..rows {
            poll(control)?;
            let y = i64::from(part.y0) + row as i64 - ry as i64;
            for (column, pixel) in row_values.chunks_exact_mut(channels).enumerate() {
                read(source, full_extent, i64::from(part.x0) + column as i64 - rx as i64, y, params.abyss, pixel, &mut meter, control)?;
            }
            let mut sums = [0.0f64; 4];
            for tap in 0..nx {
                poll(control)?;
                meter.debit(count as u64)?;
                for (sum, value) in sums.iter_mut().zip(slice(&row_values, tap * channels, count)?) {
                    *sum = finite(*sum + *value)?;
                }
            }
            for column in 0..width {
                poll(control)?;
                meter.debit((count * 4) as u64)?;
                let start = index(row, width, column, count)?;
                for (destination, sum) in horizontal
                    .get_mut(start..start.checked_add(count).ok_or(RasterInputError::LimitExceeded)?)
                    .ok_or(RasterInputError::LengthMismatch)?
                    .iter_mut()
                    .zip(sums)
                {
                    *destination = finite(sum / nx as f64)?;
                }
                if column + 1 < width {
                    for ((sum, outgoing), incoming) in
                        sums.iter_mut().zip(slice(&row_values, column * channels, count)?).zip(slice(&row_values, (column + nx) * channels, count)?)
                    {
                        *sum = finite(*sum - *outgoing)?;
                        *sum = finite(*sum + *incoming)?;
                    }
                }
            }
        }
        let mut vertical = allocate::<f64>(width.checked_mul(count).ok_or(RasterInputError::LimitExceeded)?, control)?;
        for tap in 0..ny {
            poll(control)?;
            for column in 0..width {
                meter.debit(count as u64)?;
                let start = index(tap, width, column, count)?;
                for (sum, value) in vertical.get_mut(column * count..(column + 1) * count).ok_or(RasterInputError::LengthMismatch)?.iter_mut().zip(slice(
                    &horizontal,
                    start,
                    count,
                )?) {
                    *sum = finite(*sum + *value)?;
                }
            }
        }
        let mut raw = allocate::<u8>(usize::try_from(payload).map_err(|_| RasterInputError::LimitExceeded)?, control)?;
        source.try_read_interleaved_into_with(tile_rect, &mut raw, control)?;
        for row in 0..height {
            poll(control)?;
            for column in 0..width {
                poll(control)?;
                meter.debit((channels * 8) as u64)?;
                let local_y = usize::try_from(i64::from(part.y0) - i64::from(tile_rect.y0))
                    .map_err(|_| RasterInputError::InvalidBounds)?
                    .checked_add(row)
                    .ok_or(RasterInputError::LimitExceeded)?;
                let local_x = usize::try_from(i64::from(part.x0) - i64::from(tile_rect.x0))
                    .map_err(|_| RasterInputError::InvalidBounds)?
                    .checked_add(column)
                    .ok_or(RasterInputError::LimitExceeded)?;
                let start = index(local_y, side, local_x, bpp)?;
                let encoded = raw.get_mut(start..start.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
                for (channel, sum) in slice(&vertical, column * count, count)?.iter().enumerate() {
                    let value = finite(*sum / ny as f64)?;
                    if format.alpha && params.alpha == Alpha::Filter && channel + 1 == count && !(0.0..=1.0).contains(&value) {
                        return Err(Error::AlphaRange);
                    }
                    try_write_sample_f64(encoded, format.sample, channel, value).map_err(|_| Error::NonFinite)?;
                }
                if row + 1 < height {
                    let leaving = index(row, width, column, count)?;
                    let entering = index(row + ny, width, column, count)?;
                    for ((sum, outgoing), incoming) in vertical
                        .get_mut(column * count..(column + 1) * count)
                        .ok_or(RasterInputError::LengthMismatch)?
                        .iter_mut()
                        .zip(slice(&horizontal, leaving, count)?)
                        .zip(slice(&horizontal, entering, count)?)
                    {
                        *sum = finite(*sum - *outgoing)?;
                        *sum = finite(*sum + *incoming)?;
                    }
                }
            }
        }
        let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), tile_rect, &mut meter, control)?;
        let mut cell = allocate::<u8>(64 * 64 * bpp, control)?;
        for cy in 0..4 {
            for cx in 0..4 {
                poll(control)?;
                for row in 0..64 {
                    let start = index(cy * 64 + row, side, cx * 64, bpp)?;
                    let target = row * 64 * bpp;
                    let span = 64 * bpp;
                    cell.get_mut(target..target + span)
                        .ok_or(RasterInputError::LengthMismatch)?
                        .copy_from_slice(raw.get(start..start + span).ok_or(RasterInputError::LengthMismatch)?);
                }
                let x =
                    tile_rect.x0.checked_add(i32::try_from(cx * 64).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                let y =
                    tile_rect.y0.checked_add(i32::try_from(cy * 64).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                builder.push_region(
                    Rect::new(x, y, x.checked_add(64).ok_or(RasterInputError::InvalidBounds)?, y.checked_add(64).ok_or(RasterInputError::InvalidBounds)?),
                    &cell,
                    &mut meter,
                    control,
                )?;
            }
        }
        let (replacement, ()) = builder.finalize(&mut meter, control)?.into_parts();
        poll(control)?;
        meter.debit(mul(add(retained, tiles)?, 512)?)?;
        output.put_tiles(replacement.tiles().map(|(coordinate, tile)| (*coordinate, tile.clone())));
    }
    poll(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [7; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    fn params(abyss: Abyss, alpha: Alpha) -> Params {
        Params { radius_x: 1, radius_y: 1, abyss, alpha }
    }
    #[test]
    fn constant_hdr_and_native_f64_low_bits_survive_clamp() {
        for sample in [SampleType::F32, SampleType::F64] {
            let source = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, sample, false), &[20.0]);
            let extent = Rect::new(-2, -1, 3, 2);
            let roi = Rect::new(-1, 0, 1, 1);
            let out = try_apply(&source, extent, roi, context(), context(), params(Abyss::Clamp, Alpha::PreserveRaw), limits(), &Interrupt::NONE).unwrap();
            assert_eq!(out.try_read_region_f64(roi).unwrap(), vec![20.0, 20.0]);
            assert_eq!(out.default_bytes(), source.default_bytes());
        }
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let extent = Rect::new(0, 0, 3, 1);
        let value = f64::from_bits(1.0f64.to_bits() + 1);
        source.try_write_region_f64(extent, &[value, value, value]).unwrap();
        let p = Params { radius_x: 1, radius_y: 0, abyss: Abyss::Clamp, alpha: Alpha::PreserveRaw };
        let out = try_apply(&source, extent, extent, context(), context(), p, limits(), &Interrupt::NONE).unwrap();
        assert!(out.try_read_region_f64(extent).unwrap().iter().all(|sample| sample.to_bits() == value.to_bits()));
    }
    #[test]
    fn zero_abyss_alpha_and_roi_preserve_outside_native_bytes() {
        let extent = Rect::new(0, 0, 1, 1);
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true));
        source.try_write_region_f64(extent, &[6.0, 0.5]).unwrap();
        source.try_write_pixel_f64(2, 0, &[19.0, 0.75]).unwrap();
        let raw = source.try_read_interleaved(Rect::new(2, 0, 3, 1)).unwrap();
        let preserved = try_apply(&source, extent, extent, context(), context(), params(Abyss::Zero, Alpha::PreserveRaw), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(preserved.try_read_region_f64(extent).unwrap(), vec![2.0 / 3.0, 0.5]);
        assert_eq!(preserved.try_read_interleaved(Rect::new(2, 0, 3, 1)).unwrap(), raw);
        let filtered = try_apply(&source, extent, extent, context(), context(), params(Abyss::Zero, Alpha::Filter), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(filtered.try_read_region_f64(extent).unwrap(), vec![2.0 / 3.0, (0.5 / 3.0) / 3.0]);
        assert_eq!(filtered.default_bytes(), source.default_bytes());
    }
    #[test]
    fn malformed_samples_intermediate_overflow_and_invalid_alpha_refuse() {
        let extent = Rect::new(0, 0, 1, 1);
        for value in [f64::NAN, f64::INFINITY, f64::MAX] {
            let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
            let bytes = value.to_ne_bytes();
            source.try_write_interleaved(extent, &bytes).unwrap();
            let before = source.clone();
            assert!(matches!(
                try_apply(&source, extent, extent, context(), context(), params(Abyss::Clamp, Alpha::PreserveRaw), limits(), &Interrupt::NONE),
                Err(Error::NonFinite)
            ));
            assert_eq!(source, before);
        }
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, true));
        source.try_write_region_f64(extent, &[1.0, 2.0, 3.0, 1.5]).unwrap();
        assert!(matches!(
            try_apply(&source, extent, extent, context(), context(), params(Abyss::Clamp, Alpha::PreserveRaw), limits(), &Interrupt::NONE),
            Err(Error::AlphaRange)
        ));
    }
    #[test]
    fn empty_context_limits_and_chunk_cancellation_are_explicit() {
        let source = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.25]);
        let extent = Rect::new(0, 0, 8, 8);
        let empty = Rect::new(0, 0, 0, 0);
        let p = params(Abyss::Clamp, Alpha::PreserveRaw);
        assert_eq!(try_apply(&source, extent, empty, context(), context(), p, limits(), &Interrupt::NONE).unwrap(), source);
        let mut other = context();
        other.profile_identity = [8; 32];
        assert!(matches!(try_apply(&source, extent, extent, context(), other, p, limits(), &Interrupt::NONE), Err(Error::ContextMismatch)));
        assert!(matches!(
            try_apply(&source, extent, extent, context(), context(), p, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 100;
        assert!(matches!(
            try_apply(&source, extent, extent, context(), context(), p, limits(), &Interrupt::cancel_only(&cancel)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert!(required_input(Rect::new(0, 0, i32::MAX, 1), p).is_err());
        assert!(matches!(required_input(extent, Params { radius_x: 129, ..p }), Err(Error::Radius)));
    }
}
