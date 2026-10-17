//! Bounded mathematical separable convolution with caller-supplied weights.
//! No Gaussian parameter fitting, serialized operation mapping or GEGL precision
//! equivalence is implied. The caller establishes the working CMS interpretation.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

const MAX_TAPS: usize = 257;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abyss {
    None,
    Clamp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    PreserveRaw,
    Filter,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}

/// Conservative local source/weights/output/map/scratch accounting. Borrowed
/// weight slices charge their addressed bytes; any larger backing owner remains
/// the enclosing caller's obligation. No allocator/RSS or input refund claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub parameter_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Weights,
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
        write!(f, "separable neighborhood: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

/// Borrowed immutable weights: odd, symmetric, nonnegative, finite, <=257 taps.
/// Their left-to-right f64 sum must be exactly 1. No renormalization occurs.
pub struct Weights<'a> {
    horizontal: &'a [f64],
    vertical: &'a [f64],
}
impl<'a> Weights<'a> {
    pub fn new(horizontal: &'a [f64], vertical: &'a [f64]) -> Result<Self> {
        validate_weights(horizontal)?;
        validate_weights(vertical)?;
        Ok(Self { horizontal, vertical })
    }
    pub fn horizontal(&self) -> &'a [f64] {
        self.horizontal
    }
    pub fn vertical(&self) -> &'a [f64] {
        self.vertical
    }
}
fn validate_weights(weights: &[f64]) -> Result<()> {
    if weights.is_empty() || weights.len() > MAX_TAPS || weights.len().is_multiple_of(2) {
        return Err(Error::Weights);
    }
    let mut sum = 0.0f64;
    for (index, weight) in weights.iter().enumerate() {
        if !weight.is_finite() || *weight < 0.0 || weights.get(weights.len() - 1 - index).is_none_or(|other| other.to_bits() != weight.to_bits()) {
            return Err(Error::Weights);
        }
        sum += weight;
    }
    if sum != 1.0 {
        return Err(Error::Weights);
    }
    Ok(())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn halo(roi: Rect, weights: &Weights<'_>) -> Result<Rect> {
    let x = i32::try_from(weights.horizontal.len() / 2).map_err(|_| Error::Weights)?;
    let y = i32::try_from(weights.vertical.len() / 2).map_err(|_| Error::Weights)?;
    Ok(Rect {
        x0: roi.x0.checked_sub(x).ok_or(RasterInputError::InvalidBounds)?,
        y0: roi.y0.checked_sub(y).ok_or(RasterInputError::InvalidBounds)?,
        x1: roi.x1.checked_add(x).ok_or(RasterInputError::InvalidBounds)?,
        y1: roi.y1.checked_add(y).ok_or(RasterInputError::InvalidBounds)?,
    })
}
pub fn required_input(roi: Rect, weights: &Weights<'_>) -> Result<Rect> {
    halo(roi, weights)
}

fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for c in [first, last] {
        c.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        c.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let x = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let y = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    Ok(x.checked_mul(y).ok_or(RasterInputError::LimitExceeded)?)
}
fn channels(source: &Surface, alpha: Alpha) -> usize {
    if alpha == Alpha::Filter { source.channels() } else { source.format().mode.color_channels() }
}
/// Includes the widened horizontal halo plane, bounded tap/map lookup work and
/// repeated publication of tile maps. This function never iterates source tiles.
pub fn required_work(source: &Surface, roi: Rect, weights: &Weights<'_>, alpha: Alpha) -> Result<u64> {
    checked_region_byte_len(roi, source.format())?;
    let support = halo(roi, weights)?;
    let width = u64::from(roi.width());
    let height = u64::try_from(i64::from(support.y1) - i64::from(support.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
    let horizontal = width.checked_mul(height).and_then(|n| n.checked_mul(weights.horizontal.len() as u64));
    let vertical = width.checked_mul(u64::from(roi.height())).and_then(|n| n.checked_mul(weights.vertical.len() as u64));
    let tiles = tile_count(roi)?;
    let retained = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    Ok(horizontal
        .and_then(|n| n.checked_add(vertical?))
        .and_then(|n| n.checked_mul(channels(source, alpha) as u64))
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(retained.checked_mul(8)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(retained.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(65536)?.checked_mul(source.format().bytes_per_pixel() as u64)?.checked_mul(8)?))
        .and_then(|n| n.checked_add(width.checked_mul(height)?.checked_mul(channels(source, alpha) as u64)?.checked_mul(16)?))
        .and_then(|n| n.checked_add((weights.horizontal.len() as u64).checked_add(weights.vertical.len() as u64)?))
        .ok_or(RasterInputError::LimitExceeded)?)
}
/// Allocation-free, controlled preparation shared with try_apply. No raster
/// samples are read; metadata traversal is bounded before inspecting retained tiles.
#[allow(clippy::too_many_arguments)]
pub fn requirements_with_interrupt(
    source: &Surface,
    input_extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    weights: &Weights<'_>,
    _abyss: Abyss,
    alpha: Alpha,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(control)?;
    if source_context != expected_context {
        return Err(Error::ContextMismatch);
    }
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    checked_region_byte_len(input_extent, format)?;
    checked_region_byte_len(roi, format)?;
    if input_extent.is_empty() || roi.is_empty() || !input_extent.contains_rect(&roi) {
        return Err(RasterInputError::InvalidBounds.into());
    }
    let support = halo(roi, weights)?;
    checked_region_byte_len(support, format)?;
    let payload = 65536u64.checked_mul(format.bytes_per_pixel() as u64).ok_or(RasterInputError::LimitExceeded)?;
    let retained = source.tile_count() as u64;
    let outputs = tile_count(roi)?;
    let source_retained_bytes = retained
        .checked_mul(payload.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(format.bytes_per_pixel() as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    if source_retained_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded.into());
    }
    if source.default_bytes().len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch.into());
    }
    for (coordinate, tile) in source.tiles() {
        poll(control)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch.into());
        }
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let parameter_retained_bytes =
        (weights.horizontal.len() as u64).checked_add(weights.vertical.len() as u64).and_then(|n| n.checked_mul(8)).ok_or(RasterInputError::LimitExceeded)?;
    let output_retained_bytes =
        outputs.checked_mul(payload).and_then(|n| n.checked_add(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output_index_bytes = retained.checked_add(outputs).and_then(|n| n.checked_mul(512)).ok_or(RasterInputError::LimitExceeded)?;
    let height = u64::try_from(i64::from(support.y1) - i64::from(support.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
    let scratch_bytes = u64::from(roi.width())
        .checked_mul(height)
        .and_then(|n| n.checked_mul(channels(source, alpha) as u64))
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(payload.checked_mul(6)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let peak_resident_bytes = source_retained_bytes
        .checked_add(parameter_retained_bytes)
        .and_then(|n| n.checked_add(output_retained_bytes))
        .and_then(|n| n.checked_add(output_index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or(RasterInputError::LimitExceeded)?;
    let work_units = required_work(source, roi, weights, alpha)?;
    poll(control)?;
    Ok(Requirements {
        source_retained_bytes,
        parameter_retained_bytes,
        output_retained_bytes,
        output_index_bytes,
        scratch_bytes,
        peak_resident_bytes,
        work_units,
    })
}
fn initialized<T: Default + Clone>(length: usize, control: &Interrupt<'_>) -> Result<Vec<T>> {
    poll(control)?;
    let mut data = Vec::new();
    data.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    poll(control)?;
    if data.capacity() != length {
        return Err(RasterInputError::AllocationFailed.into());
    }
    let chunk = (32768 / std::mem::size_of::<T>().max(1)).max(1);
    while data.len() < length {
        poll(control)?;
        data.resize(data.len().checked_add(chunk).ok_or(RasterInputError::LimitExceeded)?.min(length), T::default());
    }
    poll(control)?;
    Ok(data)
}
fn sample(source: &Surface, extent: Rect, mut x: i64, mut y: i64, abyss: Abyss, alpha: Alpha, pixel: &mut [f64]) -> Result<()> {
    let inside = x >= i64::from(extent.x0) && x < i64::from(extent.x1) && y >= i64::from(extent.y0) && y < i64::from(extent.y1);
    if !inside {
        match abyss {
            Abyss::None => {
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
    if pixel.get(..channels(source, alpha)).ok_or(RasterInputError::LengthMismatch)?.iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFinite);
    }
    if source.format().alpha && alpha == Alpha::Filter && pixel.last().is_none_or(|v| !(0.0..=1.0).contains(v)) {
        return Err(Error::AlphaRange);
    }
    Ok(())
}
fn accumulate(sum: &mut f64, value: f64, weight: f64) -> Result<()> {
    let product = value * weight;
    if !product.is_finite() {
        return Err(Error::NonFinite);
    }
    *sum += product;
    if !sum.is_finite() {
        return Err(Error::NonFinite);
    }
    Ok(())
}

/// Two unfused f64 convolution passes, no intermediate F32 quantization.
/// Colors are finite HDR, without premultiplication/TRC conversion. Filter alpha
/// additionally requires unit finite alpha; PreserveRaw never evaluates alpha.
/// Output is private until success; original/default/outside bytes are retained.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_apply(
    source: &Surface,
    input_extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    weights: &Weights<'_>,
    abyss: Abyss,
    alpha: Alpha,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let required = requirements_with_interrupt(source, input_extent, roi, source_context, expected_context, weights, abyss, alpha, control)?;
    if limits.resident_bytes == 0
        || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || required.peak_resident_bytes > limits.resident_bytes
        || required.work_units > limits.work_units
    {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let format = source.format();
    let support = halo(roi, weights)?;
    let width = usize::try_from(roi.width()).map_err(|_| RasterInputError::LimitExceeded)?;
    let height = usize::try_from(i64::from(support.y1) - i64::from(support.y0)).map_err(|_| RasterInputError::LimitExceeded)?;
    let count = channels(source, alpha);
    let horizontal_len = width.checked_mul(height).and_then(|n| n.checked_mul(count)).ok_or(RasterInputError::LimitExceeded)?;
    let mut horizontal = initialized::<f64>(horizontal_len, control)?;
    let rx = i64::try_from(weights.horizontal.len() / 2).map_err(|_| Error::Weights)?;
    for row in 0..height {
        poll(control)?;
        let y = i64::from(support.y0) + i64::try_from(row).map_err(|_| RasterInputError::InvalidBounds)?;
        for column in 0..width {
            let x = i64::from(roi.x0) + i64::try_from(column).map_err(|_| RasterInputError::InvalidBounds)?;
            let mut sums = [0.0f64; 4];
            for (tap, weight) in weights.horizontal.iter().enumerate() {
                if tap.is_multiple_of(32) {
                    poll(control)?;
                }
                if *weight == 0.0 {
                    continue;
                }
                let mut pixel = [0.0f64; 4];
                sample(
                    source,
                    input_extent,
                    x + i64::try_from(tap).map_err(|_| Error::Weights)? - rx,
                    y,
                    abyss,
                    alpha,
                    pixel.get_mut(..source.channels()).ok_or(RasterInputError::LengthMismatch)?,
                )?;
                for (sum, value) in sums.iter_mut().zip(pixel).take(count) {
                    accumulate(sum, value, *weight)?;
                }
            }
            let start = row.checked_mul(width).and_then(|n| n.checked_add(column)).and_then(|n| n.checked_mul(count)).ok_or(RasterInputError::LimitExceeded)?;
            let end = start.checked_add(count).ok_or(RasterInputError::LimitExceeded)?;
            horizontal.get_mut(start..end).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(sums.get(..count).ok_or(RasterInputError::LengthMismatch)?);
        }
    }
    let mut output = source.clone();
    let bpp = format.bytes_per_pixel();
    for tile in roi.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&roi);
        let len = checked_region_byte_len(part, format)?;
        let mut pixels = initialized::<u8>(len, control)?;
        source.try_read_interleaved_into_with(part, &mut pixels, control)?;
        let part_width = usize::try_from(part.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        if part_width == 0 {
            return Err(RasterInputError::InvalidBounds.into());
        }
        for (index, encoded) in pixels.chunks_exact_mut(bpp).enumerate() {
            let x = i64::from(part.x0) + i64::try_from(index % part_width).map_err(|_| RasterInputError::InvalidBounds)?;
            let y = i64::from(part.y0) + i64::try_from(index / part_width).map_err(|_| RasterInputError::InvalidBounds)?;
            let column = usize::try_from(x - i64::from(roi.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let row = usize::try_from(y - i64::from(roi.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let mut sums = [0.0f64; 4];
            for (tap, weight) in weights.vertical.iter().enumerate() {
                if tap.is_multiple_of(32) {
                    poll(control)?;
                }
                if *weight == 0.0 {
                    continue;
                }
                let start = row
                    .checked_add(tap)
                    .and_then(|n| n.checked_mul(width))
                    .and_then(|n| n.checked_add(column))
                    .and_then(|n| n.checked_mul(count))
                    .ok_or(RasterInputError::LimitExceeded)?;
                let end = start.checked_add(count).ok_or(RasterInputError::LimitExceeded)?;
                for (sum, value) in sums.iter_mut().zip(horizontal.get(start..end).ok_or(RasterInputError::LengthMismatch)?) {
                    accumulate(sum, *value, *weight)?;
                }
            }
            for (channel, value) in sums.iter().take(count).enumerate() {
                if format.alpha && alpha == Alpha::Filter && channel + 1 == count && !(0.0..=1.0).contains(value) {
                    return Err(Error::AlphaRange);
                }
                try_write_sample_f64(encoded, format.sample, channel, *value).map_err(|_| Error::NonFinite)?;
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
    use photocraft_color::PixelFormat;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
    #[test]
    fn dyadic_impulse_has_exact_separable_result() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let rect = Rect { x0: 0, y0: 0, x1: 3, y1: 3 };
        source.try_write_region_f64(rect, &[0., 0., 0., 0., 1., 0., 0., 0., 0.]).unwrap();
        let taps = [0.25, 0.5, 0.25];
        let weights = Weights::new(&taps, &taps).unwrap();
        let result = try_apply(&source, rect, rect, context(), context(), &weights, Abyss::None, Alpha::PreserveRaw, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_region_f64(rect).unwrap(), vec![0.0625, 0.125, 0.0625, 0.125, 0.25, 0.125, 0.0625, 0.125, 0.0625]);
        assert_eq!(source.try_read_region_f64(rect).unwrap(), vec![0., 0., 0., 0., 1., 0., 0., 0., 0.]);
    }
    #[test]
    fn raw_alpha_nan_is_preserved_only_under_explicit_policy() {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true));
        let rect = Rect { x0: -1, y0: 0, x1: 0, y1: 1 };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2.0f32.to_ne_bytes());
        bytes.extend_from_slice(&0x7fc0_1234u32.to_ne_bytes());
        source.try_write_interleaved_with(rect, &bytes, &Interrupt::NONE).unwrap();
        let taps = [1.0];
        let weights = Weights::new(&taps, &taps).unwrap();
        let result = try_apply(&source, rect, rect, context(), context(), &weights, Abyss::Clamp, Alpha::PreserveRaw, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.try_read_interleaved(rect).unwrap(), bytes);
        assert!(matches!(
            try_apply(&source, rect, rect, context(), context(), &weights, Abyss::Clamp, Alpha::Filter, limits(), &Interrupt::NONE),
            Err(Error::NonFinite)
        ));
    }
    #[test]
    fn invalid_weights_context_and_budget_refuse() {
        for taps in [&[0.5, 0.5][..], &[0.25, 0.25, 0.25][..], &[0.2, 0.5, 0.3][..], &[f64::NAN][..]] {
            assert!(Weights::new(taps, &[1.0]).is_err());
        }
        assert!(Weights::new(&[1.0; 259], &[1.0]).is_err());
        let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let rect = Rect { x0: 0, y0: 0, x1: 1, y1: 1 };
        let weights = Weights::new(&[1.0], &[1.0]).unwrap();
        let mut other = context();
        other.profile_identity = [2; 32];
        assert!(matches!(
            try_apply(&source, rect, rect, context(), other, &weights, Abyss::None, Alpha::PreserveRaw, limits(), &Interrupt::NONE),
            Err(Error::ContextMismatch)
        ));
        assert!(matches!(
            try_apply(
                &source,
                rect,
                rect,
                context(),
                context(),
                &weights,
                Abyss::None,
                Alpha::PreserveRaw,
                Limits { resident_bytes: 1, work_units: 1 },
                &Interrupt::NONE
            ),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
    }
}

#[cfg(test)]
mod requirements_tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::SampleEncoding;
    #[test]
    fn shared_requirements_account_weight_output_and_scratch_before_allocation() {
        let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        let extent = Rect::new(0, 0, 3, 3);
        let taps = [0.25, 0.5, 0.25];
        let weights = Weights::new(&taps, &taps).unwrap();
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let req = requirements_with_interrupt(&source, extent, extent, context, context, &weights, Abyss::None, Alpha::PreserveRaw, &Interrupt::NONE).unwrap();
        assert_eq!(req.parameter_retained_bytes, 48);
        assert_eq!(
            req.peak_resident_bytes,
            req.source_retained_bytes + req.parameter_retained_bytes + req.output_retained_bytes + req.output_index_bytes + req.scratch_bytes
        );
        assert!(
            try_apply(
                &source,
                extent,
                extent,
                context,
                context,
                &weights,
                Abyss::None,
                Alpha::PreserveRaw,
                Limits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(
            try_apply(
                &source,
                extent,
                extent,
                context,
                context,
                &weights,
                Abyss::None,
                Alpha::PreserveRaw,
                Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units - 1 },
                &Interrupt::NONE
            )
            .is_err()
        );
        let cancelled = || true;
        assert!(matches!(
            requirements_with_interrupt(
                &source,
                extent,
                extent,
                context,
                context,
                &weights,
                Abyss::None,
                Alpha::PreserveRaw,
                &Interrupt::cancel_only(&cancelled)
            ),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.tile_count(), 0);
    }
}
