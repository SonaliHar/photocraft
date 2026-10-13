//! Standalone standard, unnormalized Sobel derivative candidate.
//! Public parameter names/sign semantics: https://gegl.org/operations/gegl-edge-sobel.html
//! Standard 3x3 gradient: https://homepages.inf.ed.ac.uk/rbf/HIPR2/sobel.htm
//! This is NOT a qualified GEGL evaluator. Its initial mathematical domain is one
//! direction, no alpha, explicit caller CMS working transfer, Float/Double RGB/Gray.
//! Combined-direction scaling, GEGL precision/transfer, alpha and native source
//! envelopes require independent qualification and deliberately refuse here.
use photocraft_color::{ColorMode, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SobelTransfer {
    Linear,
    Nonlinear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SobelAbyss {
    None,
    Clamp,
}
#[derive(Clone, Copy, Debug)]
pub struct SobelParams {
    pub horizontal: bool,
    pub vertical: bool,
    pub keep_sign: bool,
}
impl SobelParams {
    /// Version matching only; does not confer serialized evaluator admission.
    pub fn for_version(version: &str, horizontal: bool, vertical: bool, keep_sign: bool) -> Result<Self, RasterInputError> {
        if version != "0:0" || horizontal == vertical {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(Self { horizontal, vertical, keep_sign })
    }
}
/// The caller has already converted source samples through its matched CMS.
/// Transfer is an explicit description, never an inferred ICC/default alias.
pub struct SobelWorkingInput<'a> {
    pub source: &'a Surface,
    pub extent: Rect,
    pub transfer: SobelTransfer,
}
#[derive(Clone, Copy, Debug)]
pub struct SobelLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}

/// One-pixel support required by the standard kernel; the selected abyss policy
/// resolves support beyond the declared input extent without ambient reads.
pub fn sobel_required_input(output: Rect) -> Result<Rect, RasterInputError> {
    Ok(Rect::new(
        output.x0.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        output.y0.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        output.x1.checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
        output.y1.checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
    ))
}

/// A single direction uses the public unnormalized standard kernel. Positive
/// horizontal response increases from left to right; vertical increases downward.
/// The other public combinations refuse until their scaling/rounding is witnessed.
pub fn try_sobel_standard(
    input: SobelWorkingInput<'_>,
    output: Rect,
    params: SobelParams,
    abyss: SobelAbyss,
    limits: SobelLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let source = input.source;
    let format = source.format();
    if params.horizontal == params.vertical
        || format.alpha
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    // Both declarations evaluate the supplied samples directly; no hidden transfer.
    match input.transfer {
        SobelTransfer::Linear | SobelTransfer::Nonlinear => {}
    }
    checked_region_byte_len(input.extent, format)?;
    checked_region_byte_len(output, format)?;
    if input.extent.is_empty() || output.intersect(&input.extent) != output {
        return Err(RasterInputError::InvalidBounds);
    }
    sobel_required_input(output)?;
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let channels = source.channels();
    let tiles = output_tile_count(output)?;
    let work = required_work(source, output)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let tile_payload =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let replacements = tiles.checked_mul(tile_payload.checked_add(512).ok_or(RasterInputError::LimitExceeded)?)
        .ok_or(RasterInputError::LimitExceeded)?;
    // Reject the constant-time replacement lower bound before scanning resident
    // source storage to calculate the complete peak.
    if limits.resident_bytes == 0 || replacements > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let copied_map = (source.tile_count() as u64)
        .checked_mul(256)
        .and_then(|n| n.checked_add(source.default_bytes().len() as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    let peak = source
        .resident_accounted_bytes()?
        .checked_add(copied_map.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(replacements))
        .and_then(|n| n.checked_add(tile_payload.checked_mul(6)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if limits.resident_bytes == 0 || peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    source.try_validate_tile_origins_with(interrupt)?;
    let mut result = source.clone();
    for tile in output.tiles() {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let part = tile.rect().intersect(&output);
        let len = checked_region_byte_len(part, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(len, 0);
        let mut offset = 0usize;
        for y in part.y0..part.y1 {
            interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
            for x in part.x0..part.x1 {
                let mut sum = [0.0f64; 3];
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let weight = if params.horizontal { dx * if dy == 0 { 2 } else { 1 } } else { dy * if dx == 0 { 2 } else { 1 } };
                        let mut pixel = [0.0f64; 3];
                        sample(
                            source,
                            input.extent,
                            i64::from(x) + dx,
                            i64::from(y) + dy,
                            abyss,
                            pixel.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?,
                        )?;
                        for (value, sample) in sum.iter_mut().zip(pixel).take(channels) {
                            *value += sample * weight as f64;
                        }
                    }
                }
                let end = offset.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let encoded = bytes.get_mut(offset..end).ok_or(RasterInputError::LengthMismatch)?;
                for (channel, value) in sum.iter().take(channels).enumerate() {
                    let value = if params.keep_sign { *value } else { value.abs() };
                    if !value.is_finite() || (format.sample == SampleType::F32 && !(value as f32).is_finite()) {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(encoded, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                }
                offset = end;
            }
        }
        result.try_write_interleaved_with(part, &bytes, interrupt)?;
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(result)
}

/// Scalar admission estimate shared by the kernel and graph scheduler. No map
/// validation or tile iteration occurs here. Each raw tile publication can
/// traverse/clone the growing maps: 256 work units per potential map entry.
pub(crate) fn required_work(source: &Surface, output: Rect) -> Result<u64, RasterInputError> {
    checked_region_byte_len(output, source.format())?;
    let area = u64::from(output.width()).checked_mul(u64::from(output.height())).ok_or(RasterInputError::LimitExceeded)?;
    let tiles = output_tile_count(output)?;
    let source_tiles = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    let source_work = source_tiles.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?;
    let scheduler_work = tiles.checked_mul(source_tiles.checked_add(tiles).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_mul(256)).ok_or(RasterInputError::LimitExceeded)?;
    area.checked_mul(source.channels() as u64).and_then(|n| n.checked_mul(40))
        .and_then(|n| n.checked_add(source_work)).and_then(|n| n.checked_add(scheduler_work))
        .ok_or(RasterInputError::LimitExceeded)
}

/// Checked O(1) half-open tile span, including negative coordinates. Empty
/// rectangles touch no tile; the last included coordinate is end - 1.
fn output_tile_count(rect: Rect) -> Result<u64, RasterInputError> {
    if rect.is_empty() { return Ok(0); }
    let side = i64::from(TILE_SIZE);
    let span = |start: i32, end: i32| -> Result<u64, RasterInputError> {
        let first = i64::from(start).div_euclid(side);
        let last = i64::from(end).checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.div_euclid(side);
        u64::try_from(last.checked_sub(first).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?)
            .map_err(|_| RasterInputError::InvalidBounds)
    };
    span(rect.x0, rect.x1)?.checked_mul(span(rect.y0, rect.y1)?).ok_or(RasterInputError::LimitExceeded)
}

fn sample(source: &Surface, extent: Rect, mut x: i64, mut y: i64, abyss: SobelAbyss, pixel: &mut [f64]) -> Result<(), RasterInputError> {
    let inside = x >= i64::from(extent.x0) && x < i64::from(extent.x1) && y >= i64::from(extent.y0) && y < i64::from(extent.y1);
    if !inside {
        match abyss {
            SobelAbyss::None => {
                pixel.fill(0.0);
                return Ok(());
            }
            SobelAbyss::Clamp => {
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
    if pixel.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    #[test]
    fn thin_roi_tiny_budgets_refuse_without_tile_enumeration() {
        let source = Surface::new(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false });
        let rect = Rect { x0: 0, y0: 0, x1: 1, y1: 1_000_000 };
        for limits in [SobelLimits { resident_bytes: 1, work_units: u64::MAX }, SobelLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 }] {
            assert!(matches!(try_sobel_standard(
                SobelWorkingInput { source: &source, extent: rect, transfer: SobelTransfer::Linear }, rect,
                SobelParams { horizontal: true, vertical: false, keep_sign: true }, SobelAbyss::None,
                limits, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        }
        assert_eq!(source.tile_count(), 0);
        assert_eq!(output_tile_count(Rect { x0: -1, y0: -1, x1: 1, y1: 1 }).unwrap(), 4);
    }
    #[test]
    fn unsupported_combined_or_alpha_domains_refuse() {
        assert!(SobelParams::for_version("0:0", true, true, true).is_err());
        assert!(SobelParams::for_version("1:0", false, true, true).is_err());
        let source = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F64, alpha: true });
        assert!(matches!(
            try_sobel_standard(
                SobelWorkingInput { source: &source, extent: Rect::new(0, 0, 3, 3), transfer: SobelTransfer::Linear },
                Rect::new(1, 1, 2, 2),
                SobelParams { horizontal: true, vertical: false, keep_sign: true },
                SobelAbyss::Clamp,
                SobelLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1000 },
                &Interrupt::NONE
            ),
            Err(RasterInputError::UnsupportedConversion)
        ));
    }
    #[test]
    fn standard_horizontal_ramp_and_roi_preservation() {
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let mut source = Surface::new(format);
        source.try_write_region_f64(Rect::new(0, 0, 3, 3), &[0., 1., 2., 0., 1., 2., 0., 1., 2.]).unwrap();
        let result = try_sobel_standard(
            SobelWorkingInput { source: &source, extent: Rect::new(0, 0, 3, 3), transfer: SobelTransfer::Linear },
            Rect::new(1, 1, 2, 2),
            SobelParams { horizontal: true, vertical: false, keep_sign: true },
            SobelAbyss::Clamp,
            SobelLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1000 },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(result.try_read_region_f64(Rect::new(1, 1, 2, 2)).unwrap(), vec![8.]);
        assert_eq!(result.try_read_region_f64(Rect::new(0, 0, 1, 3)).unwrap(), source.try_read_region_f64(Rect::new(0, 0, 1, 3)).unwrap());
    }
    #[test]
    fn cancellation_and_limits_refuse_without_source_mutation() {
        let source = Surface::new(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false });
        let rect = Rect::new(0, 0, 3, 3);
        let call = |interrupt: &Interrupt<'_>, work| {
            try_sobel_standard(
                SobelWorkingInput { source: &source, extent: rect, transfer: SobelTransfer::Nonlinear },
                rect,
                SobelParams { horizontal: false, vertical: true, keep_sign: false },
                SobelAbyss::None,
                SobelLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: work },
                interrupt,
            )
        };
        assert!(matches!(call(&Interrupt::cancel_only(&|| true), 1000), Err(RasterInputError::Cancelled)));
        assert!(matches!(call(&Interrupt::NONE, 0), Err(RasterInputError::LimitExceeded)));
        assert_eq!(source.tile_count(), 0);
    }
}
