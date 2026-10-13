//! Exact-code integer and explicit-range float histograms of an authoritative extent.
//! No profile conversion, source configuration or GIMP operation mapping is inferred.
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_read_sample_raw};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::mem::size_of;

pub const MAX_BINS: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel { Color(usize), Alpha }
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Domain {
    /// Inclusive native code interval. Bin = floor((code-low)*bins/(high-low+1)).
    IntegerCodes { low: u32, high: u32 },
    /// Finite native values, low-inclusive/high-exclusive bins; the exact high
    /// endpoint belongs to the final bin. Values outside the range refuse.
    FloatRange { low: f64, high: f64 },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Specification { pub channel: Channel, pub domain: Domain, pub bins: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transparency { IncludeAll, ExcludeZeroAlpha }
#[derive(Clone, Copy, Debug)]
pub struct Limits { pub resident_bytes: u64, pub work_units: u64 }
#[derive(Debug)]
pub struct ChannelHistogram { pub specification: Specification, pub counts: Vec<u64> }
#[derive(Debug)]
pub struct Histogram {
    pub extent: Rect,
    pub sample: SampleType,
    pub transparency: Transparency,
    pub examined_pixels: u64,
    pub included_pixels: u64,
    pub channels: Vec<ChannelHistogram>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { Raster(RasterInputError), InvalidSpecification, NonFinite, OutsideRange }
impl From<RasterInputError> for Error { fn from(value: RasterInputError) -> Self { Self::Raster(value) } }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "native histogram: {self:?}") }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> { control.check().map_err(|_| RasterInputError::Cancelled.into()) }
fn limit() -> Error { RasterInputError::LimitExceeded.into() }
fn tile_count(extent: Rect) -> Result<u64> {
    if extent.is_empty() { return Err(RasterInputError::InvalidBounds.into()); }
    let first = TileCoord::containing(extent.x0, extent.y0);
    let last = TileCoord::containing(extent.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, extent.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or_else(limit)
}
/// Native reads, sample arithmetic and conservative sparse-map lookup work.
/// Computed without visiting source tiles or allocating an output.
pub fn required_work(source: &Surface, extent: Rect) -> Result<u64> {
    let bytes = checked_region_byte_len(extent, source.format())? as u64;
    let tiles = tile_count(extent)?;
    let stored = u64::try_from(source.tile_count()).map_err(|_| limit())?;
    bytes.checked_mul(64).and_then(|n| n.checked_add(stored.checked_mul(8)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(stored.checked_add(1)?)?.checked_mul(256)?)).ok_or_else(limit)
}
fn channel_index(channel: Channel, colors: usize, alpha: bool) -> Result<usize> {
    match channel {
        Channel::Color(index) if index < colors => Ok(index),
        Channel::Alpha if alpha => Ok(colors),
        _ => Err(Error::InvalidSpecification),
    }
}
fn integer(pixel: &[u8], sample: SampleType, channel: usize) -> Result<u32> {
    let raw = try_read_sample_raw(pixel, sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
    Ok(match sample {
        SampleType::U8 => u32::from(*raw.first().ok_or(RasterInputError::InvalidSample)?),
        SampleType::U16 => u32::from(u16::from_ne_bytes(raw.try_into().map_err(|_| RasterInputError::InvalidSample)?)),
        SampleType::U32 => u32::from_ne_bytes(raw.try_into().map_err(|_| RasterInputError::InvalidSample)?),
        _ => return Err(Error::InvalidSpecification),
    })
}
fn float(pixel: &[u8], sample: SampleType, channel: usize) -> Result<f64> {
    let value = try_read_sample_f64(pixel, sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
    if !value.is_finite() { return Err(Error::NonFinite); }
    Ok(value)
}
fn bin(pixel: &[u8], sample: SampleType, channel: usize, spec: Specification) -> Result<usize> {
    match spec.domain {
        Domain::IntegerCodes { low, high } => {
            let value = integer(pixel, sample, channel)?;
            if value < low || value > high { return Err(Error::OutsideRange); }
            let span = u64::from(high).checked_sub(u64::from(low)).and_then(|n| n.checked_add(1)).ok_or_else(limit)?;
            let scaled = u64::from(value - low).checked_mul(spec.bins as u64).ok_or_else(limit)?;
            usize::try_from(scaled / span).map_err(|_| limit())
        }
        Domain::FloatRange { low, high } => {
            let value = float(pixel, sample, channel)?;
            if value < low || value > high { return Err(Error::OutsideRange); }
            if value == high { return spec.bins.checked_sub(1).ok_or(Error::InvalidSpecification); }
            // Explicit f64 schedule, no f32 narrowing. A rounded interior ratio
            // equal to one is assigned to the final bin, never outside the range.
            let scaled = ((value - low) / (high - low)) * spec.bins as f64;
            if !scaled.is_finite() || scaled < 0.0 { return Err(Error::NonFinite); }
            Ok((scaled.floor() as usize).min(spec.bins - 1))
        }
    }
}

/// Compute private results over every pixel of `extent`, including sparse
/// defaults. Alpha is never a color channel: it requires `Channel::Alpha`.
/// IncludeAll does not inspect unselected alpha; ExcludeZeroAlpha accepts only
/// finite unit float alpha (both zero signs exclude), or native integer alpha.
/// A skipped pixel's color is not evaluated. Cancellation/refusal exposes no
/// partial histogram and never modifies source storage.
pub fn try_histogram(source: &Surface, extent: Rect, specifications: &[Specification], transparency: Transparency,
    limits: Limits, control: &Interrupt<'_>) -> Result<Histogram> {
    poll(control)?;
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) { return Err(RasterInputError::UnsupportedConversion.into()); }
    if specifications.is_empty() || specifications.len() > 4 { return Err(Error::InvalidSpecification); }
    let colors = format.mode.color_channels();
    let mut indices = [0usize; 4];
    let mut total_bins = 0usize;
    for (index, spec) in specifications.iter().enumerate() {
        if spec.bins == 0 || spec.bins > MAX_BINS { return Err(Error::InvalidSpecification); }
        let channel = channel_index(spec.channel, colors, format.alpha)?;
        if indices.get(..index).ok_or(Error::InvalidSpecification)?.contains(&channel) { return Err(Error::InvalidSpecification); }
        *indices.get_mut(index).ok_or(Error::InvalidSpecification)? = channel;
        match (format.sample, spec.domain) {
            (SampleType::U8 | SampleType::U16 | SampleType::U32, Domain::IntegerCodes { low, high }) => {
                let maximum = match format.sample { SampleType::U8 => 255, SampleType::U16 => 65535, _ => u32::MAX };
                if low > high || high > maximum { return Err(Error::InvalidSpecification); }
            }
            (SampleType::F32 | SampleType::F64, Domain::FloatRange { low, high }) => {
                if !low.is_finite() || !high.is_finite() || low >= high || !(high - low).is_finite() { return Err(Error::InvalidSpecification); }
            }
            _ => return Err(Error::InvalidSpecification),
        }
        total_bins = total_bins.checked_add(spec.bins).ok_or_else(limit)?;
    }
    let work = required_work(source, extent)?.checked_add((total_bins as u64).checked_mul(4).ok_or_else(limit)?).ok_or_else(limit)?;
    if work > limits.work_units { return Err(limit()); }
    let bpp = format.bytes_per_pixel();
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let requested = (total_bins as u64).checked_mul(size_of::<u64>() as u64)
        .and_then(|n| n.checked_add((specifications.len() as u64).checked_mul(size_of::<ChannelHistogram>() as u64)?))
        .and_then(|n| n.checked_add(tile_bytes)).ok_or_else(limit)?;
    let envelope = requested.checked_mul(2).and_then(|n| n.checked_add(384)).ok_or_else(limit)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if envelope > cap { return Err(limit()); }
    let mut retained = source.default_bytes().len() as u64;
    if retained as usize != bpp { return Err(RasterInputError::LengthMismatch.into()); }
    for (index, (coordinate, tile)) in source.tiles().enumerate() {
        if index.is_multiple_of(256) { poll(control)?; }
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != tile_bytes { return Err(RasterInputError::LengthMismatch.into()); }
        retained = retained.checked_add(tile_bytes.checked_add(256).ok_or_else(limit)?).ok_or_else(limit)?;
        if retained.checked_add(envelope).ok_or_else(limit)? > cap { return Err(limit()); }
    }
    if retained.checked_add(envelope).ok_or_else(limit)? > cap { return Err(limit()); }
    poll(control)?;
    let mut channels = Vec::new();
    channels.try_reserve_exact(specifications.len()).map_err(|_| RasterInputError::AllocationFailed)?;
    let mut actual = channels.capacity().checked_mul(size_of::<ChannelHistogram>()).ok_or_else(limit)? as u64;
    for spec in specifications {
        let mut counts = Vec::new();
        counts.try_reserve_exact(spec.bins).map_err(|_| RasterInputError::AllocationFailed)?;
        actual = actual.checked_add((counts.capacity() as u64).checked_mul(8).ok_or_else(limit)?).ok_or_else(limit)?;
        if actual > envelope { return Err(limit()); }
        while counts.len() < spec.bins {
            poll(control)?;
            counts.resize(counts.len().checked_add((spec.bins - counts.len()).min(4096)).ok_or_else(limit)?, 0u64);
        }
        channels.push(ChannelHistogram { specification: *spec, counts });
    }
    let mut scratch = Vec::new();
    scratch.try_reserve_exact(usize::try_from(tile_bytes).map_err(|_| limit())?).map_err(|_| RasterInputError::AllocationFailed)?;
    actual = actual.checked_add(scratch.capacity() as u64).ok_or_else(limit)?;
    if actual > envelope || retained.checked_add(actual).ok_or_else(limit)? > cap { return Err(limit()); }
    let mut examined_pixels = 0u64;
    let mut included_pixels = 0u64;
    for coordinate in extent.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&extent);
        let length = checked_region_byte_len(part, format)?;
        scratch.resize(length, 0);
        source.try_read_interleaved_into_with(part, &mut scratch, control)?;
        for (index, pixel) in scratch.chunks_exact(bpp).enumerate() {
            if index.is_multiple_of(256) { poll(control)?; }
            examined_pixels = examined_pixels.checked_add(1).ok_or_else(limit)?;
            if transparency == Transparency::ExcludeZeroAlpha && format.alpha {
                let zero = match format.sample {
                    SampleType::U8 | SampleType::U16 | SampleType::U32 => integer(pixel, format.sample, colors)? == 0,
                    SampleType::F32 | SampleType::F64 => {
                        let alpha = float(pixel, format.sample, colors)?;
                        if !(0.0..=1.0).contains(&alpha) { return Err(Error::OutsideRange); }
                        alpha == 0.0
                    }
                };
                if zero { continue; }
            }
            included_pixels = included_pixels.checked_add(1).ok_or_else(limit)?;
            for (slot, channel) in channels.iter_mut().enumerate() {
                let sample_index = *indices.get(slot).ok_or(Error::InvalidSpecification)?;
                let bin = bin(pixel, format.sample, sample_index, channel.specification)?;
                let count = channel.counts.get_mut(bin).ok_or(Error::InvalidSpecification)?;
                *count = count.checked_add(1).ok_or_else(limit)?;
            }
        }
    }
    poll(control)?;
    Ok(Histogram { extent, sample: format.sample, transparency, examined_pixels, included_pixels, channels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    fn limits() -> Limits { Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 } }
    fn extent(width: i32) -> Rect { Rect { x0: 0, y0: 0, x1: width, y1: 1 } }
    fn spec(domain: Domain, bins: usize) -> Specification { Specification { channel: Channel::Color(0), domain, bins } }

    #[test]
    fn adjacent_u32_codes_and_f64_values_do_not_narrow_to_f32() {
        let region = extent(2);
        let mut integer = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false));
        let mut raw = Vec::new();
        for value in [16_777_216u32, 16_777_217] { raw.extend_from_slice(&value.to_ne_bytes()); }
        integer.try_write_interleaved_with(region, &raw, &Interrupt::NONE).unwrap();
        let result = try_histogram(&integer, region, &[spec(Domain::IntegerCodes { low: 16_777_216, high: 16_777_217 }, 2)], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.channels[0].counts, vec![1, 1]);
        assert_eq!(integer.try_read_interleaved(region).unwrap(), raw);
        let mut floating = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let adjacent = f64::from_bits(1.0f64.to_bits() + 1);
        let upper = f64::from_bits(1.0f64.to_bits() + 2);
        floating.try_write_region_f64(region, &[1.0, adjacent]).unwrap();
        let result = try_histogram(&floating, region, &[spec(Domain::FloatRange { low: 1.0, high: upper }, 2)], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.channels[0].counts, vec![1, 1]);
    }

    #[test]
    fn sparse_defaults_and_full_integer_upper_endpoint_are_counted() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false));
        source.try_write_interleaved_with(Rect { x0: 1, y0: 0, x1: 2, y1: 1 }, &u32::MAX.to_ne_bytes(), &Interrupt::NONE).unwrap();
        let result = try_histogram(&source, extent(2), &[spec(Domain::IntegerCodes { low: 0, high: u32::MAX }, 2)], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.channels[0].counts, vec![1, 1]);
        assert_eq!((result.examined_pixels, result.included_pixels), (2, 2));
    }

    #[test]
    fn transparency_and_alpha_channel_are_explicit_and_upper_float_edge_is_inclusive() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true));
        source.try_write_region_f64(extent(3), &[0.0, -0.0, 0.5, 1.0, 1.0, 1.0]).unwrap();
        let specifications = [spec(Domain::FloatRange { low: 0.0, high: 1.0 }, 2)];
        let all = try_histogram(&source, extent(3), &specifications, Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(all.channels[0].counts, vec![1, 2]);
        let included = try_histogram(&source, extent(3), &specifications, Transparency::ExcludeZeroAlpha, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(included.channels[0].counts, vec![0, 2]);
        assert_eq!((included.examined_pixels, included.included_pixels), (3, 2));
        let alpha = Specification { channel: Channel::Alpha, ..specifications[0] };
        let result = try_histogram(&source, extent(3), &[alpha], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.channels[0].counts, vec![1, 2]);
        let mut raw = Vec::new();
        for word in [0.5f64.to_bits(), 0x7ff8_0000_0000_1234u64] { raw.extend_from_slice(&word.to_ne_bytes()); }
        source.try_write_interleaved_with(extent(1), &raw, &Interrupt::NONE).unwrap();
        assert!(try_histogram(&source, extent(1), &specifications, Transparency::IncludeAll, limits(), &Interrupt::NONE).is_ok());
        assert_eq!(try_histogram(&source, extent(1), &specifications, Transparency::ExcludeZeroAlpha, limits(), &Interrupt::NONE).unwrap_err(), Error::NonFinite);
        assert_eq!(source.try_read_interleaved(extent(1)).unwrap(), raw);
    }

    #[test]
    fn invalid_ranges_nonfinite_color_budgets_and_cancellation_refuse() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let region = extent(1);
        let valid = spec(Domain::FloatRange { low: 0.0, high: 1.0 }, 2);
        assert_eq!(try_histogram(&source, region, &[spec(Domain::FloatRange { low: -f64::MAX, high: f64::MAX }, 2)], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap_err(), Error::InvalidSpecification);
        source.try_write_region_f64(region, &[2.0]).unwrap();
        assert_eq!(try_histogram(&source, region, &[valid], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap_err(), Error::OutsideRange);
        let raw = 0x7ff8_0000_0000_1234u64.to_ne_bytes();
        source.try_write_interleaved_with(region, &raw, &Interrupt::NONE).unwrap();
        assert_eq!(try_histogram(&source, region, &[valid], Transparency::IncludeAll, limits(), &Interrupt::NONE).unwrap_err(), Error::NonFinite);
        assert!(matches!(try_histogram(&source, region, &[valid], Transparency::IncludeAll, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(Error::Raster(RasterInputError::LimitExceeded))));
        assert!(matches!(try_histogram(&source, region, &[valid], Transparency::IncludeAll, limits(), &Interrupt::cancel_only(&|| true)), Err(Error::Raster(RasterInputError::Cancelled))));
        assert_eq!(source.try_read_interleaved(region).unwrap(), raw);
    }
}
