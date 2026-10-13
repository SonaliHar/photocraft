//! PhotoCraft coordinate-counter uniform primitive, not a serialized GEGL mapping.
//! No ambient RNG, default profile, seed interpretation or dither is selected.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain { Unit, FiniteHdr }
#[derive(Clone, Copy, Debug)]
pub struct Range { pub low: f64, pub high: f64 }
#[derive(Clone, Copy, Debug)]
pub struct Noise<'a> {
    pub seed: u64,
    pub format: PixelFormat,
    /// Exact caller-established interpretation; equality is not ICC validation.
    pub context: WorkingContext,
    pub domain: Domain,
    /// One explicit range per RGB/Gray color channel. Equal endpoints are constant.
    pub ranges: &'a [Range],
    /// Required exactly when the output format has alpha; finite unit constant.
    pub alpha: Option<f64>,
    /// Exact native pixel copied outside ROI, including signed-zero bits.
    /// All default samples must be finite, with unit alpha and chosen color domain.
    pub output_default: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits { pub resident_bytes: u64, pub work_units: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error { Raster(RasterInputError), ContextMismatch, InvalidConfiguration, NonFinite, OutsideRange }
impl From<RasterInputError> for Error { fn from(value: RasterInputError) -> Self { Self::Raster(value) } }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "coordinate counter noise: {self:?}") }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> { control.check().map_err(|_| RasterInputError::Cancelled.into()) }
fn limit() -> Error { RasterInputError::LimitExceeded.into() }

fn mix(mut word: u64) -> u64 {
    word = word.wrapping_add(0x9e37_79b9_7f4a_7c15);
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}
/// Exact modular U64 contract. Signed coordinates use their two's-complement
/// U32 words; channel is an independent U32 counter, not a mutable stream.
/// Wraparound is intentional hash arithmetic, never length/coordinate arithmetic.
pub fn counter_word(seed: u64, x: i32, y: i32, channel: u32) -> u64 {
    let coordinates = u64::from(x as u32) | (u64::from(y as u32) << 32);
    mix(mix(seed ^ coordinates) ^ u64::from(channel).wrapping_mul(0xd6e8_feb8_6659_fd93))
}
/// The high 53 bits times exact 2^-53, a defined discrete uniform grid [0,1).
/// This mathematical contract makes no cryptographic/distribution equivalence claim.
pub fn uniform(seed: u64, x: i32, y: i32, channel: u32) -> f64 {
    (counter_word(seed, x, y, channel) >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
}
fn tiles(roi: Rect) -> Result<u64> {
    if roi.is_empty() { return Err(RasterInputError::InvalidBounds.into()); }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let columns = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    columns.checked_mul(rows).ok_or_else(limit)
}
/// Hash/sample work, native initialization/copies and conservative repeated map
/// publications. This admission is O(1), before output or tile scratch allocation.
pub fn required_work(format: PixelFormat, roi: Rect) -> Result<u64> {
    let bytes = checked_region_byte_len(roi, format)? as u64;
    let count = tiles(roi)?;
    bytes.checked_mul(128).and_then(|n| n.checked_add(count.checked_mul(count)?.checked_mul(256)?)).ok_or_else(limit)
}
fn finite(value: f64, format: PixelFormat, unit: bool) -> Result<()> {
    if !value.is_finite() { return Err(Error::NonFinite); }
    if unit && !(0.0..=1.0).contains(&value) { return Err(Error::OutsideRange); }
    if format.sample == SampleType::F32 && value.abs() > f64::from(f32::MAX) { return Err(Error::OutsideRange); }
    Ok(())
}
fn validate(configuration: &Noise<'_>, expected: WorkingContext) -> Result<()> {
    let format = configuration.format;
    if configuration.context != expected { return Err(Error::ContextMismatch); }
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    let colors = format.mode.color_channels();
    if configuration.ranges.len() != colors || configuration.output_default.len() != format.bytes_per_pixel() { return Err(Error::InvalidConfiguration); }
    for range in configuration.ranges {
        finite(range.low, format, configuration.domain == Domain::Unit)?;
        finite(range.high, format, configuration.domain == Domain::Unit)?;
        if range.low > range.high || !(range.high - range.low).is_finite() { return Err(Error::InvalidConfiguration); }
    }
    match (format.alpha, configuration.alpha) {
        (true, Some(alpha)) => finite(alpha, format, true)?,
        (false, None) => {}
        _ => return Err(Error::InvalidConfiguration),
    }
    for channel in 0..format.channels() {
        let value = try_read_sample_f64(configuration.output_default, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        finite(value, format, channel >= colors || configuration.domain == Domain::Unit)?;
    }
    Ok(())
}
/// Allocation-free exact configuration/resource preflight, with no source owner.
pub fn requirements(configuration: &Noise<'_>, roi: Rect, expected: WorkingContext) -> Result<super::generator_linear_gradient::Requirements> {
    validate(configuration, expected)?;
    let parameters = configuration.ranges.len().checked_mul(std::mem::size_of::<Range>())
        .and_then(|n| n.checked_add(configuration.output_default.len())).and_then(|n| n.checked_add(std::mem::size_of::<Noise<'_>>())).ok_or_else(limit)?;
    Ok(super::generator_linear_gradient::generation_requirements(configuration.format, roi, parameters as u64, required_work(configuration.format, roi)?)?)
}

/// Generate the finite ROI at absolute coordinates (negative coordinates included).
/// Per color: unfused f64 `low + (high-low)*uniform`, then a single native F32/F64
/// encode. Stored F32 rounding may include the high endpoint. Overflow, nonfinite
/// or range violations refuse rather than clamp. Alpha is the explicit constant.
/// Outside ROI reads exact caller default; this is not an infinite generator or
/// a modification of an existing surface. Private partial output never escapes.
pub fn try_generate(configuration: &Noise<'_>, roi: Rect, expected: WorkingContext, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    validate(configuration, expected)?;
    let format = configuration.format;
    let admitted = requirements(configuration, roi, expected)?;
    if admitted.work_units > limits.work_units { return Err(limit()); }
    let bpp = format.bytes_per_pixel();
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let scratch_envelope = tile_bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    if admitted.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) { return Err(limit()); }
    poll(control)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(usize::try_from(tile_bytes).map_err(|_| limit())?).map_err(|_| RasterInputError::AllocationFailed)?;
    if pixels.capacity() as u64 > scratch_envelope { return Err(limit()); }
    let mut output = Surface::try_with_default_bytes(format, configuration.output_default)?;
    let colors = format.mode.color_channels();
    for coordinate in roi.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&roi);
        let length = checked_region_byte_len(part, format)?;
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
                for (channel, range) in configuration.ranges.iter().enumerate() {
                    let u = uniform(configuration.seed, x, y, u32::try_from(channel).map_err(|_| Error::InvalidConfiguration)?);
                    let product = (range.high - range.low) * u;
                    let value = if range.low == range.high { range.low } else { range.low + product };
                    finite(value, format, configuration.domain == Domain::Unit)?;
                    if value < range.low || value > range.high { return Err(Error::OutsideRange); }
                    try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                    let stored = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
                    finite(stored, format, configuration.domain == Domain::Unit)?;
                    if stored < range.low || stored > range.high { return Err(Error::OutsideRange); }
                }
                if let Some(alpha) = configuration.alpha {
                    try_write_sample_f64(pixel, format.sample, colors, alpha).map_err(|_| RasterInputError::InvalidSample)?;
                }
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
    fn context() -> WorkingContext { WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [7; 32] } }
    fn limits() -> Limits { Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 } }
    fn configuration<'a>(sample: SampleType, ranges: &'a [Range], default: &'a [u8]) -> Noise<'a> {
        Noise { seed: 123, format: PixelFormat::new(ColorMode::Grayscale, sample, false), context: context(), domain: Domain::Unit, ranges, alpha: None, output_default: default }
    }

    #[test]
    fn tile_and_roi_partitioning_negative_coordinates_and_seed_are_deterministic() {
        let ranges = [Range { low: 0.0, high: 1.0 }];
        let default = (-0.0f64).to_ne_bytes();
        let mut config = configuration(SampleType::F64, &ranges, &default);
        let whole = Rect { x0: -65, y0: -1, x1: 65, y1: 2 };
        let part = Rect { x0: -2, y0: -1, x1: 2, y1: 1 };
        let full = try_generate(&config, whole, context(), limits(), &Interrupt::NONE).unwrap();
        let partial = try_generate(&config, part, context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(full.try_read_interleaved(part).unwrap(), partial.try_read_interleaved(part).unwrap());
        assert_eq!(partial.default_bytes(), default);
        assert_eq!(partial.try_read_interleaved(Rect { x0: 80, y0: 0, x1: 81, y1: 1 }).unwrap(), default);
        let same = try_generate(&config, whole, context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(full.try_read_interleaved(whole).unwrap(), same.try_read_interleaved(whole).unwrap());
        config.seed = 124;
        let changed = try_generate(&config, part, context(), limits(), &Interrupt::NONE).unwrap();
        assert_ne!(partial.try_read_interleaved(part).unwrap(), changed.try_read_interleaved(part).unwrap());
        assert_ne!(counter_word(123, -1, 0, 0), counter_word(123, 1, 0, 0));
        assert_ne!(counter_word(123, -1, 0, 0), counter_word(123, -1, 0, 1));
    }

    #[test]
    fn float_depths_share_binary64_noise_before_single_final_rounding() {
        let ranges = [Range { low: 0.0, high: 1.0 }];
        let zero32 = 0.0f32.to_ne_bytes();
        let zero64 = 0.0f64.to_ne_bytes();
        let region = Rect { x0: -3, y0: 2, x1: 3, y1: 3 };
        let f32_output = try_generate(&configuration(SampleType::F32, &ranges, &zero32), region, context(), limits(), &Interrupt::NONE).unwrap();
        let f64_output = try_generate(&configuration(SampleType::F64, &ranges, &zero64), region, context(), limits(), &Interrupt::NONE).unwrap();
        for (narrow, wide) in f32_output.try_read_region_f64(region).unwrap().iter().zip(f64_output.try_read_region_f64(region).unwrap()) {
            assert_eq!(*narrow, f64::from(wide as f32));
            assert!((0.0..1.0).contains(&wide));
        }
    }

    #[test]
    fn constant_hdr_alpha_and_raw_default_are_explicit() {
        let ranges = [Range { low: -2.0, high: -2.0 }, Range { low: 4.0, high: 4.0 }, Range { low: 0.5, high: 0.5 }];
        let mut default = Vec::new();
        for value in [1.0f64, 2.0, 3.0, -0.0] { default.extend_from_slice(&value.to_ne_bytes()); }
        let config = Noise { seed: u64::MAX, format: PixelFormat::new(ColorMode::Rgb, SampleType::F64, true), context: context(), domain: Domain::FiniteHdr, ranges: &ranges, alpha: Some(0.25), output_default: &default };
        let region = Rect { x0: 0, y0: 0, x1: 2, y1: 1 };
        let output = try_generate(&config, region, context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(region).unwrap(), vec![-2.0, 4.0, 0.5, 0.25, -2.0, 4.0, 0.5, 0.25]);
        assert_eq!(output.default_bytes(), default);
    }

    #[test]
    fn hostile_range_alpha_context_limits_and_cancellation_refuse() {
        let default = 0.0f64.to_ne_bytes();
        let region = Rect { x0: 0, y0: 0, x1: 3, y1: 2 };
        for range in [Range { low: f64::NAN, high: 1.0 }, Range { low: 1.0, high: 0.0 }, Range { low: -f64::MAX, high: f64::MAX }] {
            let ranges = [range];
            assert!(try_generate(&configuration(SampleType::F64, &ranges, &default), region, context(), limits(), &Interrupt::NONE).is_err());
        }
        let ranges = [Range { low: 0.0, high: 1.0 }];
        let mut config = configuration(SampleType::F64, &ranges, &default);
        config.alpha = Some(0.5);
        assert!(try_generate(&config, region, context(), limits(), &Interrupt::NONE).is_err());
        config.alpha = None;
        assert_eq!(try_generate(&config, region, WorkingContext { profile_identity: [8; 32], ..context() }, limits(), &Interrupt::NONE).unwrap_err(), Error::ContextMismatch);
        assert!(matches!(try_generate(&config, region, context(), Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(Error::Raster(RasterInputError::LimitExceeded))));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 6;
        assert!(matches!(try_generate(&config, region, context(), limits(), &Interrupt::cancel_only(&cancel)), Err(Error::Raster(RasterInputError::Cancelled))));
        let success = try_generate(&config, region, context(), limits(), &Interrupt::NONE).unwrap();
        let repeat = try_generate(&config, region, context(), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(success.try_read_interleaved(region).unwrap(), repeat.try_read_interleaved(region).unwrap());
    }
}
