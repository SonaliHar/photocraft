//! Explicit mathematical Gray Hue/Saturation transfer-chain candidate.
//! Complete collector41f5fa31/analysis30252ac5 retains all source/stored native
//! planes and six acted replay differences of1ULP. Declared binary64-chain/final
//! Float hypotheses fit within4.77e-7; this is not a qualified GIMP epoch, unique
//! hidden TRC/intermediate precision, default renderer or source admission.
//!
//! The independently caller-established input role/profile identity is required:
//! Linear Gray passes to linear; exact Gamma2 Gray squares it. Both then use
//! analytic IEC encoding, signed ALL lightness, IEC decoding, and the explicitly
//! selected reverse transfer before final Float (widened for F64 output). Neither
//! parent ICC nor a Yfloat/nonlinear alias establishes that role. No CMS lookup or
//! profile-byte validation occurs here; caller preparation binds those authorities.
//! Neutral/H/S settings still traverse the chain, never a raw identity shortcut.
pub use super::sector_hue_saturation_candidate::{Parameters, PreparationRequirements, Requirements};
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleAccessError, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::atomic::{AtomicU64, Ordering};
pub const PREPARATION_WORK: u64 = 512;
/// Declared arithmetic work, not a measured libm/CPU-time promise.
pub const SAMPLE_WORK: u64 = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericPolicy {
    DeclaredPerceptualIecSignedLightnessFinalFloat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRole {
    CallerEstablishedLinearGray,
    CallerEstablishedExactGamma2Gray,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputProfileWitness {
    pub role: InputRole,
    pub profile_identity: [u8; 32],
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
    Access(SampleAccessError),
    Parameters,
    ContextMismatch,
    NonFinite,
    OutsideUnit,
    UnsupportedConfiguration,
}
impl From<RasterInputError> for Error {
    fn from(v: RasterInputError) -> Self {
        Self::Raster(v)
    }
}
impl From<SampleAccessError> for Error {
    fn from(v: SampleAccessError) -> Self {
        Self::Access(v)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "declared Gray Hue/Saturation transfer chain: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
pub struct PreparedGrayHueSaturation {
    format: PixelFormat,
    context: WorkingContext,
    parameters: Parameters,
    policy: NumericPolicy,
    input_profile: InputProfileWitness,
    lightness: f64,
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
impl PreparedGrayHueSaturation {
    pub fn preparation_requirements() -> PreparationRequirements {
        PreparationRequirements { retained_bytes: std::mem::size_of::<Self>() as u64, work_units: PREPARATION_WORK }
    }
    pub fn prepare(
        format: PixelFormat,
        context: WorkingContext,
        parameters: Parameters,
        policy: NumericPolicy,
        input_profile: InputProfileWitness,
        budget: Limits,
        ctl: &Interrupt<'_>,
    ) -> Result<Self> {
        poll(ctl)?;
        limits(budget)?;
        let needs = Self::preparation_requirements();
        if needs.retained_bytes > budget.resident_bytes || needs.work_units > budget.work_units {
            return Err(limit());
        }
        let encoding = match input_profile.role {
            InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
            InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
        };
        if format.mode != ColorMode::Grayscale
            || !matches!(format.sample, SampleType::F32 | SampleType::F64)
            || context.encoding != encoding
            || input_profile.profile_identity == [0; 32]
            || context.profile_identity != input_profile.profile_identity
        {
            return Err(Error::ContextMismatch);
        }
        if parameters.time_bits != Some(0) {
            return Err(Error::Parameters);
        }
        for range in parameters.ranges {
            for bits in range {
                let value = f64::from_bits(bits);
                if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
                    return Err(Error::Parameters);
                }
            }
        }
        let overlap = f64::from_bits(parameters.overlap_bits);
        if !overlap.is_finite() || !(0.0..=1.0).contains(&overlap) {
            return Err(Error::Parameters);
        }
        if parameters.ranges.iter().skip(1).flatten().any(|bits| f64::from_bits(*bits) != 0.0) {
            return Err(Error::UnsupportedConfiguration);
        }
        let lightness = f64::from_bits(*parameters.ranges.first().and_then(|range| range.get(2)).ok_or(Error::Parameters)?);
        poll(ctl)?;
        Ok(Self {
            format,
            context,
            parameters,
            policy,
            input_profile,
            lightness,
            limits: budget,
            remaining_work: AtomicU64::new(budget.work_units - PREPARATION_WORK),
        })
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
    pub fn policy(&self) -> NumericPolicy {
        self.policy
    }
    pub fn input_profile(&self) -> InputProfileWitness {
        self.input_profile
    }
    pub fn sample_work(&self) -> u64 {
        SAMPLE_WORK
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    /// Enclosing consumers prepay SAMPLE_WORK and poll their actual job control.
    /// This scalar API does not independently debit their preparation/graph ledger.
    /// All coefficients are retained; achromatic H/S does not bypass the transfer.
    pub fn try_eval_float_gray(&self, input: f32) -> Result<f32> {
        if !input.is_finite() {
            return Err(Error::NonFinite);
        }
        if !(0.0..=1.0).contains(&input) {
            return Err(Error::OutsideUnit);
        }
        let source = f64::from(input);
        let linear = match self.input_profile.role {
            InputRole::CallerEstablishedLinearGray => source,
            InputRole::CallerEstablishedExactGamma2Gray => source * source,
        };
        let perceptual = if linear <= 0.0031308 { 12.92 * linear } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 };
        let adjusted = if self.lightness < 0.0 { perceptual * (1.0 + self.lightness) } else { perceptual + (1.0 - perceptual) * self.lightness };
        let decoded = if adjusted <= 0.04045 { adjusted / 12.92 } else { ((adjusted + 0.055) / 1.055).powf(2.4) };
        let value = match self.input_profile.role {
            InputRole::CallerEstablishedLinearGray => decoded,
            InputRole::CallerEstablishedExactGamma2Gray => decoded.sqrt(),
        };
        let output = value as f32;
        if !linear.is_finite() || !perceptual.is_finite() || !adjusted.is_finite() || !decoded.is_finite() || !value.is_finite() || !output.is_finite() {
            return Err(Error::NonFinite);
        }
        Ok(output)
    }
    pub fn requirements_with_interrupt(&self, source: &Surface, roi: Rect, current: WorkingContext, ctl: &Interrupt<'_>) -> Result<Requirements> {
        poll(ctl)?;
        if source.format() != self.format || current != self.context {
            return Err(Error::ContextMismatch);
        }
        const COORD_LIMIT: i32 = 1 << 29;
        if roi.x1 < roi.x0 || roi.y1 < roi.y0 || [roi.x0, roi.x1, roi.y0, roi.y1].iter().any(|v| !(-COORD_LIMIT..=COORD_LIMIT).contains(v)) {
            return Err(RasterInputError::InvalidBounds.into());
        }
        if roi.width() > 262144 || roi.height() > 262144 {
            return Err(limit());
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
            add(mul(length, 8)?, mul(length / bpp, mul(SAMPLE_WORK, 2)?)?)?,
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
    /// Original selected F64 values are admitted as unit before Float ingress.
    /// A numeric first pass allocates nothing; raw alpha/default/outside remain exact.
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
        let bpp = self.format.bytes_per_pixel();
        let payload = mul(65536, bpp as u64)?;
        for (coordinate, tile) in source.tiles() {
            poll(ctl)?;
            if tile.bytes().len() as u64 != payload {
                return Err(RasterInputError::LengthMismatch.into());
            }
            coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
            coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        }
        let mut pixel = [0u8; 16];
        let selected = pixel.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        for y in roi.y0..roi.y1 {
            poll(ctl)?;
            for (i, x) in (roi.x0..roi.x1).enumerate() {
                if i.is_multiple_of(256) {
                    poll(ctl)?;
                }
                source.try_read_pixel_bytes(x, y, selected)?;
                let value = try_read_sample_f64(selected, self.format.sample, 0)?;
                if !value.is_finite() {
                    return Err(Error::NonFinite);
                }
                if !(0.0..=1.0).contains(&value) {
                    return Err(Error::OutsideUnit);
                }
                self.try_eval_float_gray(value as f32)?;
            }
        }
        poll(ctl)?;
        if roi.is_empty() {
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
        for (i, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if i.is_multiple_of(256) {
                poll(ctl)?;
            }
            let value = try_read_sample_f64(pixel, self.format.sample, 0)?;
            let output = self.try_eval_float_gray(value as f32)?;
            try_write_sample_f64(pixel, self.format.sample, 0, f64::from(output))?;
        }
        poll(ctl)?;
        let mut output = source.clone();
        // Existing writer map clone/default fill/native copy is bounded and priced;
        // its control polls at tile boundaries, not every byte of one256x256 tile.
        output.try_write_interleaved_with(roi, &raw, ctl)?;
        poll(ctl)?;
        Ok(output)
    }
}
#[cfg(test)]
#[path = "gray_hue_saturation_candidate_tests.rs"]
mod tests;
