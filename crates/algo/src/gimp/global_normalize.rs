//! PhotoCraft mathematical per-channel extrema normalization, not GIMP mapping.
//! Extrema cover the entire caller-authoritative extent, including sparse defaults.
//! Only the explicit output ROI is changed; alpha and outside storage remain raw.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Degenerate {
    Reject,
    PreserveChannel,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    ContextMismatch,
    NonFinite,
    DegenerateChannel,
}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "per-channel extrema normalization: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}

fn tiles(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(rect.x0, rect.y0);
    let last =
        TileCoord::containing(rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let x = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let y = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    Ok(x.checked_mul(y).ok_or(RasterInputError::LimitExceeded)?)
}
/// Both finite reductions and output point math, native copying, source validation,
/// and repeated per-tile map publications are charged before any traversal.
pub fn required_work(source: &Surface, extent: Rect, roi: Rect) -> Result<u64> {
    let input = checked_region_byte_len(extent, source.format())? as u64;
    let output = checked_region_byte_len(roi, source.format())? as u64;
    let source_tiles = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    let output_tiles = tiles(roi)?;
    tiles(extent)?;
    Ok(input
        .checked_mul(32)
        .and_then(|n| n.checked_add(output.checked_mul(32)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(8)?))
        .and_then(|n| n.checked_add(output_tiles.checked_mul(source_tiles.checked_add(output_tiles)?)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)?)
}

/// Two-pass linear normalization: (value - minimum) / (maximum - minimum).
/// Inputs/intermediates are finite f64 HDR. An overflowing range is refused,
/// rather than rescaled with an unqualified alternative precision schedule.
/// Constant channels either refuse or retain each original encoded sample.
/// A channel already spanning numeric [0,1] is exact raw identity (including -0).
/// Other minimum/maximum values encode exact +0/+1; F32 rounds only once at the
/// final writer. No ICC conversion, premultiplication or alpha arithmetic occurs.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_apply(
    source: &Surface,
    extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    degenerate: Degenerate,
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
    if extent.is_empty() || roi.is_empty() || roi.intersect(&extent) != roi {
        return Err(RasterInputError::InvalidBounds.into());
    }
    let work = required_work(source, extent, roi)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let output_tiles = tiles(roi)?;
    let payload =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let maps = (source.tile_count() as u64).checked_mul(512).ok_or(RasterInputError::LimitExceeded)?;
    let extra = output_tiles
        .checked_mul(payload.checked_add(512).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(payload.checked_mul(6)?))
        .and_then(|n| n.checked_add(maps))
        .ok_or(RasterInputError::LimitExceeded)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if cap == 0 || extra > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    if source.resident_accounted_bytes()?.checked_add(extra).ok_or(RasterInputError::LimitExceeded)? > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    for (index, (coordinate, _)) in source.tiles().enumerate() {
        if index.is_multiple_of(256) {
            poll(control)?;
        }
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let colors = format.mode.color_channels();
    let bpp = format.bytes_per_pixel();
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    // A single reusable native tile buffer; no full-extent pixel allocation.
    let scratch_capacity = usize::try_from(payload).map_err(|_| RasterInputError::LimitExceeded)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(scratch_capacity).map_err(|_| RasterInputError::AllocationFailed)?;
    for tile in extent.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&extent);
        let length = checked_region_byte_len(part, format)?;
        bytes.resize(length, 0);
        source.try_read_interleaved_into_with(part, &mut bytes, control)?;
        for (index, pixel) in bytes.chunks_exact(bpp).enumerate() {
            if index.is_multiple_of(256) {
                poll(control)?;
            }
            for channel in 0..colors {
                let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| Error::NonFinite)?;
                if !value.is_finite() {
                    return Err(Error::NonFinite);
                }
                let min = minimum.get_mut(channel).ok_or(RasterInputError::LengthMismatch)?;
                let max = maximum.get_mut(channel).ok_or(RasterInputError::LengthMismatch)?;
                if value < *min {
                    *min = value;
                }
                if value > *max {
                    *max = value;
                }
            }
        }
    }
    let mut ranges = [0.0f64; 3];
    let mut identity = [false; 3];
    for channel in 0..colors {
        let low = *minimum.get(channel).ok_or(RasterInputError::LengthMismatch)?;
        let high = *maximum.get(channel).ok_or(RasterInputError::LengthMismatch)?;
        let range = high - low;
        if !low.is_finite() || !high.is_finite() || !range.is_finite() {
            return Err(Error::NonFinite);
        }
        if range == 0.0 && degenerate == Degenerate::Reject {
            return Err(Error::DegenerateChannel);
        }
        *ranges.get_mut(channel).ok_or(RasterInputError::LengthMismatch)? = range;
        *identity.get_mut(channel).ok_or(RasterInputError::LengthMismatch)? = range == 0.0 || (low == 0.0 && high == 1.0);
    }
    let mut output = source.clone();
    for tile in roi.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&roi);
        let length = checked_region_byte_len(part, format)?;
        bytes.resize(length, 0);
        source.try_read_interleaved_into_with(part, &mut bytes, control)?;
        for (index, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) {
                poll(control)?;
            }
            for channel in 0..colors {
                if *identity.get(channel).ok_or(RasterInputError::LengthMismatch)? {
                    continue;
                }
                let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| Error::NonFinite)?;
                let low = *minimum.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                let high = *maximum.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                let range = *ranges.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                let normalized = if value == low {
                    0.0
                } else if value == high {
                    1.0
                } else {
                    (value - low) / range
                };
                if !normalized.is_finite() || !(0.0..=1.0).contains(&normalized) {
                    return Err(Error::NonFinite);
                }
                try_write_sample_f64(pixel, format.sample, channel, normalized).map_err(|_| Error::NonFinite)?;
            }
        }
        output.try_write_interleaved_with(part, &bytes, control)?;
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
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
    #[test]
    fn independent_channel_extrema_and_hdr_endpoints() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F64, false));
        let extent = Rect { x0: 0, y0: 0, x1: 3, y1: 1 };
        source.try_write_region_f64(extent, &[-2., 4., 0., 0., 2., 0.5, 2., 0., 1.]).unwrap();
        let result = try_apply(&source, extent, extent, context(), context(), Degenerate::Reject, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_region_f64(extent).unwrap(), vec![0., 1., 0., 0.5, 0.5, 0.5, 1., 0., 1.]);
        assert_eq!(source.try_read_region_f64(extent).unwrap(), vec![-2., 4., 0., 0., 2., 0.5, 2., 0., 1.]);
    }
    #[test]
    fn alpha_payloads_and_outside_roi_stay_raw() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true));
        let extent = Rect { x0: -1, y0: 0, x1: 1, y1: 1 };
        let mut input = Vec::new();
        for word in [2.0f64.to_bits(), 0x7ff8_0000_0000_1234, 4.0f64.to_bits(), (-0.0f64).to_bits()] {
            input.extend_from_slice(&word.to_ne_bytes());
        }
        source.try_write_interleaved_with(extent, &input, &Interrupt::NONE).unwrap();
        let roi = Rect { x0: -1, y0: 0, x1: 0, y1: 1 };
        let result = try_apply(&source, extent, roi, context(), context(), Degenerate::Reject, limits(), &Interrupt::NONE).unwrap();
        let mut expected = input.clone();
        expected[..8].copy_from_slice(&0.0f64.to_ne_bytes());
        assert_eq!(result.try_read_interleaved(extent).unwrap(), expected);
        assert_eq!(source.try_read_interleaved(extent).unwrap(), input);
    }
    #[test]
    fn default_samples_constant_policy_and_refusals() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let pixel = Rect { x0: 1, y0: 0, x1: 2, y1: 1 };
        let extent = Rect { x0: 0, y0: 0, x1: 2, y1: 1 };
        source.try_write_region_f64(pixel, &[4.]).unwrap();
        let result = try_apply(&source, extent, pixel, context(), context(), Degenerate::Reject, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_region_f64(extent).unwrap(), vec![0., 1.]);
        assert!(matches!(
            try_apply(&source, pixel, pixel, context(), context(), Degenerate::Reject, limits(), &Interrupt::NONE),
            Err(Error::DegenerateChannel)
        ));
        let result = try_apply(&source, pixel, pixel, context(), context(), Degenerate::PreserveChannel, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_interleaved(pixel).unwrap(), source.try_read_interleaved(pixel).unwrap());
        assert!(matches!(
            try_apply(&source, extent, extent, context(), context(), Degenerate::Reject, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
        assert!(matches!(
            try_apply(&source, extent, extent, context(), context(), Degenerate::Reject, limits(), &Interrupt::cancel_only(&|| true)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
    }
}
