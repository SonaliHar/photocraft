//! Standalone explicit native floating convolution, not a GEGL/source evaluator.
//! Caller CMS owns the working context. No gamma, premultiplication, automatic
//! kernel-sum normalization or profile conversion is performed here.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
const MAX_WORK: u64 = 1 << 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    PreserveRaw,
    Filter,
}
#[derive(Clone, Copy, Debug)]
pub enum Abyss<'a> {
    Clamp,
    ConstantNative(&'a [u8]),
}
#[derive(Clone, Copy, Debug)]
pub struct Kernel<'a> {
    pub width: u8,
    pub height: u8,
    /// Row-major borrowed coefficients. Capacity remains charged as retained input.
    pub coefficients: &'a Vec<f64>,
    /// Output is (row-major sum * factor) + bias, with separate binary64 operations.
    pub factor: f64,
    pub bias: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Params<'a> {
    pub kernel: Kernel<'a>,
    pub alpha: Alpha,
    pub abyss: Abyss<'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
/// Borrowed-input/private-output preflight, not allocator/RSS accounting.
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
    Kernel,
    ContextMismatch,
    NonFinite,
    AlphaRange,
}
impl From<RasterInputError> for Error {
    fn from(e: RasterInputError) -> Self {
        Self::Raster(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native convolution: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn finite(v: f64) -> Result<f64> {
    if v.is_finite() { Ok(v) } else { Err(Error::NonFinite) }
}
fn kernel_shape(kernel: Kernel<'_>) -> Result<(usize, usize, usize)> {
    let w = usize::from(kernel.width);
    let h = usize::from(kernel.height);
    if w == 0
        || h == 0
        || w > 65
        || h > 65
        || w % 2 == 0
        || h % 2 == 0
        || kernel.coefficients.len() != w * h
        || !kernel.factor.is_finite()
        || !kernel.bias.is_finite()
    {
        return Err(Error::Kernel);
    }
    Ok((w, h, w * h))
}
fn tile_count(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Ok(0);
    }
    let span = |a: i32, b: i32| {
        u64::try_from(i64::from(b.div_euclid(TILE_SIZE)) - i64::from(a.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)
    };
    mul(span(r.x0, r.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?)?, span(r.y0, r.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?)?)
}
fn support(r: Rect, w: usize, h: usize) -> Result<Rect> {
    let x = i32::try_from(w / 2).map_err(|_| RasterInputError::InvalidBounds)?;
    let y = i32::try_from(h / 2).map_err(|_| RasterInputError::InvalidBounds)?;
    Ok(Rect::new(
        r.x0.checked_sub(x).ok_or(RasterInputError::InvalidBounds)?,
        r.y0.checked_sub(y).ok_or(RasterInputError::InvalidBounds)?,
        r.x1.checked_add(x).ok_or(RasterInputError::InvalidBounds)?,
        r.y1.checked_add(y).ok_or(RasterInputError::InvalidBounds)?,
    ))
}
/// Conservative encoded-copy, sample-decode, tap multiply/add and staging work.
/// Includes every tap even for zero coefficients, and all retained map visits.
/// No coefficient-value shortcut changes the accumulation schedule.
pub fn required_work(roi: Rect, format: PixelFormat, params: Params<'_>, source_tiles: usize) -> Result<u64> {
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    checked_region_byte_len(roi, format)?;
    let (w, h, taps) = kernel_shape(params.kernel)?;
    let tiles = tile_count(roi)?;
    let channels = if params.alpha == Alpha::Filter { format.channels() } else { format.mode.color_channels() };
    let pixels = mul(u64::from(roi.width()), u64::from(roi.height()))?;
    let halo = mul((256 + w - 1) as u64, (256 + h - 1) as u64)?;
    let staging = mul(tiles, mul(65536, format.bytes_per_pixel() as u64)?.checked_mul(8).ok_or(RasterInputError::LimitExceeded)?)?;
    add(
        add(
            add(staging, mul(mul(tiles, halo)?, mul(format.channels() as u64, 32)?)?)?,
            mul(pixels, add(mul(mul(taps as u64, channels as u64)?, 3)?, mul(channels as u64, 16)?)?)?,
        )?,
        add(mul(source_tiles as u64, 4)?, taps as u64)?,
    )
}
/// Allocation-free controlled preflight matching the kernel's conservative
/// source/parameter/output/index/scratch admission. Reads metadata/coefficient
/// descriptors and the at-most-one native constant, never raster samples.
#[allow(clippy::too_many_arguments)]
pub fn requirements_with_interrupt(
    source: &Surface,
    full_extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    params: Params<'_>,
    ctl: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(ctl)?;
    if source_context != expected_context {
        return Err(Error::ContextMismatch);
    }
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    checked_region_byte_len(full_extent, format)?;
    checked_region_byte_len(roi, format)?;
    if full_extent.is_empty() || !full_extent.contains_rect(&roi) {
        return Err(RasterInputError::InvalidBounds.into());
    }
    let (w, h, _) = kernel_shape(params.kernel)?;
    let bpp = format.bytes_per_pixel();
    let source_tiles = source.tile_count() as u64;
    let outputs = tile_count(roi)?;
    let payload = mul(65536, bpp as u64)?;
    let source_retained_bytes = add(mul(source_tiles, add(payload, 256)?)?, bpp as u64)?;
    if source_retained_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded.into());
    }
    if source.default_bytes().len() != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    for (_, tile) in source.tiles() {
        poll(ctl)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch.into());
        }
    }
    source.try_validate_tile_origins_with(ctl)?;
    for &coefficient in params.kernel.coefficients {
        poll(ctl)?;
        finite(coefficient)?;
    }
    if let Abyss::ConstantNative(bytes) = params.abyss {
        if bytes.len() != bpp {
            return Err(RasterInputError::LengthMismatch.into());
        }
        let mut values = [0f64; 4];
        for (channel, target) in values.get_mut(..format.channels()).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {
            poll(ctl)?;
            *target = try_read_sample_f64(bytes, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        }
        validated_pixel(values.get(..format.channels()).ok_or(RasterInputError::LengthMismatch)?, format.alpha)?;
    }
    let parameter_retained_bytes = mul(params.kernel.coefficients.capacity() as u64, 8)?;
    let output_retained_bytes = add(mul(outputs, payload)?, bpp as u64)?;
    let output_index_bytes = add(mul(source_tiles, 256)?, mul(outputs, 1024)?)?;
    let halo_values = mul(mul((256 + w - 1) as u64, (256 + h - 1) as u64)?, format.channels() as u64)?;
    // The existing apply preflight deliberately includes scratch even for empty ROI.
    let scratch_bytes = add(add(payload, payload / 16)?, mul(halo_values, 8)?)?;
    let peak_resident_bytes = add(add(add(add(source_retained_bytes, parameter_retained_bytes)?, output_retained_bytes)?, output_index_bytes)?, scratch_bytes)?;
    let work_units = required_work(roi, format, params, source.tile_count())?;
    poll(ctl)?;
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
struct Meter {
    staging: u64,
    work: u64,
}
impl StagingBudget for Meter {
    type Lease = ();
    fn reserve(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.staging = self.staging.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn allocate<T: Default + Clone>(n: usize, ctl: &Interrupt<'_>) -> Result<Vec<T>> {
    poll(ctl)?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
    poll(ctl)?;
    if v.capacity() != n {
        return Err(RasterInputError::AllocationFailed.into());
    }
    let chunk = (32768 / std::mem::size_of::<T>().max(1)).max(1);
    while v.len() < n {
        poll(ctl)?;
        v.resize(v.len().checked_add(chunk).ok_or(RasterInputError::LimitExceeded)?.min(n), T::default());
    }
    poll(ctl)?;
    Ok(v)
}
fn validated_pixel(pixel: &[f64], alpha: bool) -> Result<()> {
    for &value in pixel {
        finite(value)?;
    }
    if alpha && pixel.last().is_none_or(|v| !(0.0..=1.0).contains(v)) {
        return Err(Error::AlphaRange);
    }
    Ok(())
}
/// Only ROI is changed. The full source-grid boundary tile is staged so all
/// other raw samples/defaults remain exact, including unselected NaN payloads.
/// Used halo colors must be finite (HDR/negative permitted); used alpha is unit.
/// PreserveRaw keeps center alpha bits; Filter applies the same straight-channel
/// convolution and refuses non-unit alpha. F32 output rounds once via native
/// conversion after binary64 accumulation; overflow is an error, never zero.
/// Known payload/capacity/map envelopes are charged, not total allocator/RSS.
#[allow(clippy::too_many_arguments)]
pub fn try_apply(
    source: &Surface,
    full_extent: Rect,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    params: Params<'_>,
    limits: Limits,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    poll(ctl)?;
    if source_context != expected_context {
        return Err(Error::ContextMismatch);
    }
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    checked_region_byte_len(full_extent, format)?;
    checked_region_byte_len(roi, format)?;
    if full_extent.is_empty() || !full_extent.contains_rect(&roi) {
        return Err(RasterInputError::InvalidBounds.into());
    }
    let (w, h, taps) = kernel_shape(params.kernel)?;
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let work = required_work(roi, format, params, source.tile_count())?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let bpp = format.bytes_per_pixel();
    let channels = format.channels();
    let count = if params.alpha == Alpha::Filter { channels } else { format.mode.color_channels() };
    let tile_bytes = mul(65536, bpp as u64)?;
    let tiles = tile_count(roi)?;
    let staging = mul(tiles, add(tile_bytes, 1024)?)?;
    let source_lower = mul(source.tile_count() as u64, add(tile_bytes, 256)?)?;
    let table_bytes = mul(params.kernel.coefficients.capacity() as u64, 8)?;
    let halo_values = (256 + w - 1).checked_mul(256 + h - 1).and_then(|v| v.checked_mul(channels)).ok_or(RasterInputError::LimitExceeded)?;
    let scratch = add(add(tile_bytes, tile_bytes / 16)?, mul(halo_values as u64, 8)?)?;
    let base = add(add(add(add(source_lower, mul(source.tile_count() as u64, 256)?)?, staging)?, table_bytes)?, (bpp * 2) as u64)?;
    if add(base, scratch)? > limits.resident_bytes {
        return Err(RasterInputError::LimitExceeded.into());
    }
    if source.default_bytes().len() != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    for (_, tile) in source.tiles() {
        poll(ctl)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(RasterInputError::LengthMismatch.into());
        }
    }
    source.try_validate_tile_origins_with(ctl)?;
    for &coefficient in params.kernel.coefficients {
        poll(ctl)?;
        finite(coefficient)?;
    }
    let mut constant = [0f64; 4];
    if let Abyss::ConstantNative(bytes) = params.abyss {
        if bytes.len() != bpp {
            return Err(RasterInputError::LengthMismatch.into());
        }
        for (index, target) in constant.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {
            *target = try_read_sample_f64(bytes, format.sample, index).map_err(|_| RasterInputError::InvalidSample)?;
        }
        validated_pixel(constant.get(..channels).ok_or(RasterInputError::LengthMismatch)?, format.alpha)?;
    }
    if roi.is_empty() {
        poll(ctl)?;
        let out = source.clone();
        poll(ctl)?;
        return Ok(out);
    }
    let mut raw = allocate::<u8>(usize::try_from(tile_bytes).map_err(|_| RasterInputError::LimitExceeded)?, ctl)?;
    let mut cell = allocate::<u8>(usize::try_from(tile_bytes / 16).map_err(|_| RasterInputError::LimitExceeded)?, ctl)?;
    let mut halo = allocate::<f64>(halo_values, ctl)?;
    let mut meter = Meter { staging, work: limits.work_units };
    meter.work(add(mul(source.tile_count() as u64, 4)?, taps as u64)?)?;
    poll(ctl)?;
    let mut output = source.clone();
    for coordinate in roi.tiles() {
        poll(ctl)?;
        let x0 = coordinate.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let y0 = coordinate.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let tile = Rect::new(
            x0,
            y0,
            x0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
            y0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
        );
        checked_region_byte_len(tile, format)?;
        let part = tile.intersect(&roi);
        let area = support(part, w, h)?;
        checked_region_byte_len(area, format)?;
        let halo_width = area.width() as usize;
        meter.work(mul(tile_bytes, 4)?)?;
        source.try_read_interleaved_into_with(tile, &mut raw, ctl)?;
        for y in area.y0..area.y1 {
            poll(ctl)?;
            for x in area.x0..area.x1 {
                meter.work(mul(channels as u64, 32)?)?;
                let row = usize::try_from(i64::from(y) - i64::from(area.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
                let col = usize::try_from(i64::from(x) - i64::from(area.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                let start = row
                    .checked_mul(halo_width)
                    .and_then(|v| v.checked_add(col))
                    .and_then(|v| v.checked_mul(channels))
                    .ok_or(RasterInputError::LimitExceeded)?;
                let pixel = halo.get_mut(start..start.checked_add(channels).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
                if full_extent.contains(x, y) {
                    source.try_read_pixel_f64(x, y, pixel)?;
                } else {
                    match params.abyss {
                        Abyss::Clamp => {
                            source.try_read_pixel_f64(x.clamp(full_extent.x0, full_extent.x1 - 1), y.clamp(full_extent.y0, full_extent.y1 - 1), pixel)?
                        }
                        Abyss::ConstantNative(_) => pixel.copy_from_slice(constant.get(..channels).ok_or(RasterInputError::LengthMismatch)?),
                    }
                }
                validated_pixel(pixel, format.alpha)?;
                poll(ctl)?;
            }
        }
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                meter.work(add(mul(mul(taps as u64, count as u64)?, 3)?, mul(count as u64, 16)?)?)?;
                for channel in 0..count {
                    let mut sum = 0f64;
                    for ky in 0..h {
                        poll(ctl)?;
                        for kx in 0..w {
                            let row = usize::try_from(i64::from(y) - i64::from(part.y0))
                                .map_err(|_| RasterInputError::InvalidBounds)?
                                .checked_add(ky)
                                .ok_or(RasterInputError::LimitExceeded)?;
                            let col = usize::try_from(i64::from(x) - i64::from(part.x0))
                                .map_err(|_| RasterInputError::InvalidBounds)?
                                .checked_add(kx)
                                .ok_or(RasterInputError::LimitExceeded)?;
                            let pos = row
                                .checked_mul(halo_width)
                                .and_then(|v| v.checked_add(col))
                                .and_then(|v| v.checked_mul(channels))
                                .and_then(|v| v.checked_add(channel))
                                .ok_or(RasterInputError::LimitExceeded)?;
                            let coefficient = *params.kernel.coefficients.get(ky * w + kx).ok_or(RasterInputError::LengthMismatch)?;
                            let product = finite(*halo.get(pos).ok_or(RasterInputError::LengthMismatch)? * coefficient)?;
                            sum = finite(sum + product)?;
                        }
                    }
                    let scaled = finite(sum * params.kernel.factor)?;
                    let value = finite(scaled + params.kernel.bias)?;
                    if format.alpha && channel == channels - 1 && !(0.0..=1.0).contains(&value) {
                        return Err(Error::AlphaRange);
                    }
                    let row = usize::try_from(i64::from(y) - i64::from(y0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let col = usize::try_from(i64::from(x) - i64::from(x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let sample = row
                        .checked_mul(256)
                        .and_then(|v| v.checked_add(col))
                        .and_then(|v| v.checked_mul(channels))
                        .and_then(|v| v.checked_add(channel))
                        .ok_or(RasterInputError::LimitExceeded)?;
                    try_write_sample_f64(&mut raw, format.sample, sample, value).map_err(|_| RasterInputError::InvalidSample)?;
                    finite(try_read_sample_f64(&raw, format.sample, sample).map_err(|_| RasterInputError::InvalidSample)?)?;
                }
            }
        }
        let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), tile, &mut meter, ctl)?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                poll(ctl)?;
                let width = 64usize.checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                for row in 0..64usize {
                    poll(ctl)?;
                    let src = (cy as usize)
                        .checked_add(row)
                        .and_then(|v| v.checked_mul(256))
                        .and_then(|v| v.checked_add(cx as usize))
                        .and_then(|v| v.checked_mul(bpp))
                        .ok_or(RasterInputError::LimitExceeded)?;
                    let dst = row.checked_mul(width).ok_or(RasterInputError::LimitExceeded)?;
                    cell.get_mut(dst..dst.checked_add(width).ok_or(RasterInputError::LimitExceeded)?)
                        .ok_or(RasterInputError::LengthMismatch)?
                        .copy_from_slice(raw.get(src..src.checked_add(width).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?);
                }
                builder.push_region(Rect::new(x0 + cx, y0 + cy, x0 + cx + 64, y0 + cy + 64), &cell, &mut meter, ctl)?;
            }
        }
        let (replacement, ()) = builder.finalize(&mut meter, ctl)?.into_parts();
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(coord, tile)| (*coord, Arc::clone(tile))));
    }
    poll(ctl)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [9; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 128 << 20, work_units: MAX_WORK }
    }
    fn apply(source: &Surface, extent: Rect, roi: Rect, coefficients: &Vec<f64>, width: u8, alpha: Alpha, abyss: Abyss<'_>) -> Result<Surface> {
        try_apply(
            source,
            extent,
            roi,
            context(),
            context(),
            Params { kernel: Kernel { width, height: width, coefficients, factor: 1.0, bias: 0.0 }, alpha, abyss },
            limits(),
            &Interrupt::NONE,
        )
    }
    #[test]
    fn identity_hdr_raw_signed_alpha_and_boundary_preservation() {
        for sample in [SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut original = vec![0; format.bytes_per_pixel()];
            for (channel, value) in [4.25, -2.5, 0.125, -0.0].into_iter().enumerate() {
                try_write_sample_f64(&mut original, sample, channel, value).unwrap();
            }
            let source = Surface::try_with_default_bytes(format, &original).unwrap();
            let table = vec![1.0];
            let output = apply(&source, Rect::new(-2, -2, 2, 2), Rect::new(-1, -1, 1, 1), &table, 1, Alpha::PreserveRaw, Abyss::Clamp).unwrap();
            let mut actual = vec![0; original.len()];
            output.try_read_pixel_bytes(-1, -1, &mut actual).unwrap();
            assert_eq!(actual, original);
            output.try_read_pixel_bytes(-2, -1, &mut actual).unwrap();
            assert_eq!(actual, original);
            assert_eq!(source.tile_count(), 0);
            assert_eq!(output.default_bytes(), source.default_bytes());
        }
    }
    #[test]
    fn laplacian_and_sharpen_negative_constant_abyss_are_explicit() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::try_with_default_bytes(format, &10.0f64.to_ne_bytes()).unwrap();
        let zero = 0f64.to_ne_bytes();
        let extent = Rect::new(-1, -1, 0, 0);
        for (kernel, expected) in
            [(vec![0.0, -1.0, 0.0, -1.0, 4.0, -1.0, 0.0, -1.0, 0.0], 40.0f64), (vec![0.0, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0], 50.0)]
        {
            let output = apply(&source, extent, extent, &kernel, 3, Alpha::PreserveRaw, Abyss::ConstantNative(&zero)).unwrap();
            let mut raw = [0u8; 8];
            output.try_read_pixel_bytes(-1, -1, &mut raw).unwrap();
            assert_eq!(raw, expected.to_ne_bytes());
            let clamped = apply(&source, extent, extent, &kernel, 3, Alpha::PreserveRaw, Abyss::Clamp).unwrap();
            clamped.try_read_pixel_bytes(-1, -1, &mut raw).unwrap();
            assert_eq!(f64::from_ne_bytes(raw), if expected == 40.0 { 0.0 } else { 10.0 });
        }
    }
    #[test]
    fn alpha_overflow_work_and_late_cancel_refuse_without_changing_source() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let original: Vec<u8> = [4.0f64, 0.75].into_iter().flat_map(f64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(format, &original).unwrap();
        let coefficients = vec![2.0];
        let roi = Rect::new(0, 0, 1, 1);
        assert!(matches!(apply(&source, roi, roi, &coefficients, 1, Alpha::Filter, Abyss::Clamp), Err(Error::AlphaRange)));
        let params = Params {
            kernel: Kernel { width: 1, height: 1, coefficients: &coefficients, factor: f64::MAX, bias: 0.0 },
            alpha: Alpha::PreserveRaw,
            abyss: Abyss::Clamp,
        };
        assert!(matches!(try_apply(&source, roi, roi, context(), context(), params, limits(), &Interrupt::NONE), Err(Error::NonFinite)));
        let thin = Rect::new(0, 0, 1, 100000);
        assert!(try_apply(&source, thin, thin, context(), context(), params, Limits { resident_bytes: 128 << 20, work_units: 1 }, &Interrupt::NONE).is_err());
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) > 50;
        assert!(matches!(
            try_apply(&source, roi, roi, context(), context(), params, limits(), &Interrupt::cancel_only(&cancel)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.default_bytes(), original);
        assert_eq!(source.tile_count(), 0);
    }
}

#[cfg(test)]
mod requirements_tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    #[test]
    fn controlled_requirements_match_apply_peak_and_fail_one_byte_lower() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::try_with_default_bytes(format, &1.0f64.to_ne_bytes()).unwrap();
        let extent = Rect::new(-1, -1, 2, 2);
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let coefficients = vec![1.0];
        let params = Params {
            kernel: Kernel { width: 1, height: 1, coefficients: &coefficients, factor: 1.0, bias: 0.0 },
            alpha: Alpha::PreserveRaw,
            abyss: Abyss::Clamp,
        };
        let req = requirements_with_interrupt(&source, extent, extent, context, context, params, &Interrupt::NONE).unwrap();
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
                params,
                Limits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units },
                &Interrupt::NONE
            )
            .is_err()
        );
        let cancel = || true;
        assert!(matches!(
            requirements_with_interrupt(&source, extent, extent, context, context, params, &Interrupt::cancel_only(&cancel)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.tile_count(), 0);
    }
}
