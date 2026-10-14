//! DISTINCT signed GIMP Brightness/Contrast numerical hypothesis, not source admission.
//! Sealed research capsule SHA256230c574cd854eca93de730ae9b6f2f851406bf9d6333b510cfa5fb8397721bb6.
//! Explicit nonlinear Float ingress -> analytic IEC decode -> brightness b/2
//! THEN tangent contrast about .5 -> IEC encode -> final Float rounding.
//! This is not GEGL contrast's neutral1 convention or proof of GIMP's hidden TRC
//! or intermediate precision (29/144 observed opaque RGB words were exact).
//! Finite HDR colors remain unclipped; encoded alpha/default/outside words remain
//! raw. Original source correspondence and any matched-display claim are separate.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::atomic::{AtomicU64, Ordering};
pub const PREPARATION_WORK: u64 = 128;
/// Declared operation work, not a measured libm latency bound.
pub const SAMPLE_WORK: u64 = 256;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub brightness: f64,
    pub contrast: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 } }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Parameters,
    ContextMismatch,
    NonFinite,
}
impl From<RasterInputError> for Error {
    fn from(v: RasterInputError) -> Self {
        Self::Raster(v)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GIMP Brightness/Contrast numerical candidate: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparationRequirements {
    pub retained_bytes: u64,
    pub work_units: u64,
}
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
pub struct PreparedBrightnessContrast {
    format: PixelFormat,
    context: WorkingContext,
    parameters: Parameters,
    brightness_half: f64,
    slope: f64,
    limits: Limits,
    remaining_work: AtomicU64,
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn limit() -> Error {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn limits(value: Limits) -> Result<()> {
    let standard = Limits::default();
    if value.resident_bytes == 0 || value.resident_bytes > standard.resident_bytes || value.work_units > standard.work_units {
        return Err(limit());
    }
    Ok(())
}
fn tile_count(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(rect.x0, rect.y0);
    let last =
        TileCoord::containing(rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
impl PreparedBrightnessContrast {
    pub fn preparation_requirements() -> PreparationRequirements {
        PreparationRequirements { retained_bytes: std::mem::size_of::<Self>() as u64, work_units: PREPARATION_WORK }
    }
    pub fn prepare(format: PixelFormat, context: WorkingContext, parameters: Parameters, budget: Limits, ctl: &Interrupt<'_>) -> Result<Self> {
        poll(ctl)?;
        limits(budget)?;
        let needs = Self::preparation_requirements();
        if needs.retained_bytes > budget.resident_bytes || needs.work_units > budget.work_units {
            return Err(limit());
        }
        if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !matches!(format.sample, SampleType::F32 | SampleType::F64)
            || context.encoding != SampleEncoding::NonlinearProfile
            || context.profile_identity == [0; 32]
        {
            return Err(Error::ContextMismatch);
        }
        if !parameters.brightness.is_finite()
            || !(-1.0..=1.0).contains(&parameters.brightness)
            || !parameters.contrast.is_finite()
            || parameters.contrast <= -1.0
            || parameters.contrast >= 1.0
        {
            return Err(Error::Parameters);
        }
        let brightness_half = parameters.brightness * 0.5;
        let angle = (parameters.contrast + 1.0) * std::f64::consts::PI / 4.0;
        let slope = angle.tan();
        if !slope.is_finite() {
            return Err(Error::NonFinite);
        }
        poll(ctl)?;
        Ok(Self { format, context, parameters, brightness_half, slope, limits: budget, remaining_work: AtomicU64::new(budget.work_units - PREPARATION_WORK) })
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    pub fn parameters(&self) -> Parameters {
        self.parameters
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    /// Allocation-free scalar math on caller-established nonlinear Float samples.
    /// Enclosing consumers prepay SAMPLE_WORK and poll their actual job token.
    /// No raw-neutral shortcut: even0/0 follows the declared analytic schedule.
    pub fn try_eval_float_color(&self, input: f32) -> Result<f32> {
        if !input.is_finite() {
            return Err(Error::NonFinite);
        }
        let x = f64::from(input);
        let decoded = if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
        let bright = if self.brightness_half < 0.0 { decoded * (1.0 + self.brightness_half) } else { decoded + (1.0 - decoded) * self.brightness_half };
        let contrasted = (bright - 0.5) * self.slope + 0.5;
        let encoded = if contrasted <= 0.0031308 { 12.92 * contrasted } else { 1.055 * contrasted.powf(1.0 / 2.4) - 0.055 };
        let result = encoded as f32;
        if !decoded.is_finite() || !bright.is_finite() || !contrasted.is_finite() || !encoded.is_finite() || !result.is_finite() {
            return Err(Error::NonFinite);
        }
        Ok(result)
    }
    /// O(1) prospective native capacities; malformed source tiles are checked
    /// only in the admitted polled traversal. No sample scan/hash/profile lookup.
    pub fn requirements_with_interrupt(&self, source: &Surface, roi: Rect, current: WorkingContext, ctl: &Interrupt<'_>) -> Result<Requirements> {
        poll(ctl)?;
        if source.format() != self.format || current != self.context {
            return Err(Error::ContextMismatch);
        }
        let bpp = self.format.bytes_per_pixel() as u64;
        if source.default_bytes().len() as u64 != bpp {
            return Err(RasterInputError::LengthMismatch.into());
        }
        let length = checked_region_byte_len(roi, self.format)? as u64;
        let count = tile_count(roi)?;
        let source_tiles = source.tile_count() as u64;
        let payload = mul(65536, bpp)?;
        let source_retained_bytes = add(mul(source_tiles, add(payload, 256)?)?, bpp)?;
        let parameter_retained_bytes = Self::preparation_requirements().retained_bytes;
        let output_retained_bytes = add(add(mul(add(source_tiles, count)?, add(payload, 256)?)?, bpp)?, 256)?;
        let output_index_bytes = add(mul(add(source_tiles, count)?, 1024)?, 256)?;
        // One fallible full-ROI buffer with actual capacity<=2*length, native
        // writer/copy scratch and retained source COW map priced conservatively.
        let scratch_bytes = add(mul(length, 4)?, add(mul(payload, 6)?, 4096)?)?;
        let peak_resident_bytes =
            add(add(source_retained_bytes, parameter_retained_bytes)?, add(add(output_retained_bytes, output_index_bytes)?, scratch_bytes)?)?;
        let work_units = add(
            add(mul(length, 8)?, mul(mul(length / bpp, self.format.mode.color_channels() as u64)?, SAMPLE_WORK)?)?,
            add(add(mul(mul(count, add(source_tiles, count)?)?, 512)?, mul(source_tiles, 256)?)?, mul(mul(count, payload)?, 2)?)?,
        )?;
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
    /// F64 selected colors explicitly enter Float32 before analytic math; final
    /// Float is widened back to F64 storage. This is a declared candidate policy,
    /// not a native F64/source-exact GIMP assertion. Alpha is never interpreted.
    pub fn apply(&self, source: &Surface, roi: Rect, current: WorkingContext, budget: Limits, ctl: &Interrupt<'_>) -> Result<Surface> {
        poll(ctl)?;
        limits(budget)?;
        if budget.resident_bytes > self.limits.resident_bytes || budget.work_units > self.limits.work_units {
            return Err(limit());
        }
        let needs = self.requirements_with_interrupt(source, roi, current, ctl)?;
        if needs.peak_resident_bytes > budget.resident_bytes || needs.work_units > budget.work_units {
            return Err(limit());
        }
        self.remaining_work.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(needs.work_units)).map_err(|_| limit())?;
        for (_, tile) in source.tiles() {
            poll(ctl)?;
            if tile.bytes().len() as u64 != 65536 * self.format.bytes_per_pixel() as u64 {
                return Err(RasterInputError::LengthMismatch.into());
            }
        }
        source.try_validate_tile_origins_with(ctl)?;
        if roi.is_empty() {
            poll(ctl)?;
            return Ok(source.clone());
        }
        let length = checked_region_byte_len(roi, self.format)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if raw.capacity() as u64 > mul(length as u64, 2)? {
            return Err(limit());
        }
        raw.resize(length, 0);
        source.try_read_interleaved_into_with(roi, &mut raw, ctl)?;
        for (i, pixel) in raw.chunks_exact_mut(self.format.bytes_per_pixel()).enumerate() {
            if i.is_multiple_of(256) {
                poll(ctl)?;
            }
            for channel in 0..self.format.mode.color_channels() {
                let sample = try_read_sample_f64(pixel, self.format.sample, channel).map_err(|_| Error::NonFinite)?;
                let input = sample as f32;
                if !sample.is_finite() || !input.is_finite() {
                    return Err(Error::NonFinite);
                }
                let output = self.try_eval_float_color(input)?;
                try_write_sample_f64(pixel, self.format.sample, channel, f64::from(output)).map_err(|_| Error::NonFinite)?;
            }
        }
        poll(ctl)?;
        let mut output = source.clone();
        // The existing native writer polls tile boundaries; within one admitted
        // 256x256 tile, default fill/native copying remains its provider boundary.
        output.try_write_interleaved_with(roi, &raw, ctl)?;
        poll(ctl)?;
        Ok(output)
    }
}

#[cfg(test)]
#[path = "brightness_contrast_candidate_tests.rs"]
mod tests;
