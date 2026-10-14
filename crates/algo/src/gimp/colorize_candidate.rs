//! Explicit Colorize scalar-triplet mathematical hypothesis, not GIMP/source admission.
//! Two sealed analyses retain strict profile/format HOLDs and ambiguous precision
//! or redundant scalar-vs-HSL authority. We explicitly choose original scalar
//! Float words, retaining independent native HSL words without reconstruction.
//! Caller establishes nonlinear RGB Float working context, IEC or exact gamma2
//! transfer role and independently supplies finite nonnegative Y weights summing
//! to one. Current ICC-derived weights are NOT inferred: the owned profile probe
//! disproved that general recipe. No CMS, masks, outer/default eligibility grants.
//! Binary64 decode -> weighted Y -> signed L -> scalar HSL -> matching encode ->
//! final Float. Native alpha/default/off-ROI words stay raw; no neutral shortcut.
pub use super::brightness_contrast_candidate::{PreparationRequirements, Requirements};
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
    DeclaredScalarTripletBinary64FinalFloat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferRole {
    CallerEstablishedIecSrgb,
    CallerEstablishedExactGamma2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferWitness {
    pub role: TransferRole,
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
        write!(f, "declared Colorize scalar-triplet hypothesis: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
/// Independent retained scalar and native HSL words. There is deliberately no
/// equality invariant or conversion between these representations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    scalar_f32_bits: [u32; 3],
    stored_hsl_f32_bits: [u32; 3],
}
impl Parameters {
    pub fn from_stored_bits(scalar_f32_bits: [u32; 3], stored_hsl_f32_bits: [u32; 3]) -> Result<Self> {
        for (i, bits) in scalar_f32_bits.into_iter().enumerate() {
            let x = f32::from_bits(bits);
            if !x.is_finite() || if i == 2 { !(-1.0..=1.0).contains(&x) } else { !(0.0..=1.0).contains(&x) } {
                return Err(Error::Parameters);
            }
        }
        for bits in stored_hsl_f32_bits {
            let x = f32::from_bits(bits);
            if !x.is_finite() || !(0.0..=1.0).contains(&x) {
                return Err(Error::Parameters);
            }
        }
        Ok(Self { scalar_f32_bits, stored_hsl_f32_bits })
    }
    pub fn scalar_bits(&self) -> [u32; 3] {
        self.scalar_f32_bits
    }
    pub fn stored_hsl_bits(&self) -> [u32; 3] {
        self.stored_hsl_f32_bits
    }
    pub fn scalar_values(&self) -> [f64; 3] {
        self.scalar_f32_bits.map(|v| f64::from(f32::from_bits(v)))
    }
}
/// Independently caller-supplied values, not values inferred from an ICC hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YWeights {
    bits: [u64; 3],
}
impl YWeights {
    pub fn from_bits(bits: [u64; 3]) -> Result<Self> {
        let values = bits.map(f64::from_bits);
        if values.iter().any(|x| !x.is_finite() || *x < 0.) || values.into_iter().sum::<f64>() != 1. {
            return Err(Error::Parameters);
        }
        Ok(Self { bits })
    }
    pub fn bits(&self) -> [u64; 3] {
        self.bits
    }
    pub fn values(&self) -> [f64; 3] {
        self.bits.map(f64::from_bits)
    }
}
fn read_rgb(pixel: &[u8]) -> Result<[f32; 3]> {
    let mut values = [0.; 3];
    for (i, x) in values.iter_mut().enumerate() {
        let value = try_read_sample_f64(pixel, SampleType::F32, i)?;
        if !value.is_finite() {
            return Err(Error::NonFinite);
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(Error::OutsideUnit);
        }
        *x = value as f32;
    }
    Ok(values)
}
fn hsl_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0. {
        return [l; 3];
    }
    // Standard q/p HSL construction avoids tiny-lightness cancellation.
    let q = if l <= 0.5 { l * (1. + s) } else { l + s - l * s };
    let p = 2. * l - q;
    fn component(p: f64, q: f64, t: f64) -> f64 {
        let t = t.rem_euclid(1.);
        if t < 1. / 6. {
            p + (q - p) * 6. * t
        } else if t < 0.5 {
            q
        } else if t < 2. / 3. {
            p + (q - p) * (2. / 3. - t) * 6.
        } else {
            p
        }
    }
    [component(p, q, h + 1. / 3.), component(p, q, h), component(p, q, h - 1. / 3.)]
}
pub struct PreparedColorize {
    format: PixelFormat,
    context: WorkingContext,
    parameters: Parameters,
    policy: NumericPolicy,
    input_profile: TransferWitness,
    weights: YWeights,
    hue: f64,
    saturation: f64,
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
// Capacity has already been prospectively admitted and reserved. Chunked
// initialization does not allocate again and polls at most64KiB apart.
fn initialize_with_interrupt(raw: &mut Vec<u8>, length: usize, ctl: &Interrupt<'_>) -> Result<()> {
    if raw.capacity() < length || raw.len() > length {
        return Err(RasterInputError::LengthMismatch.into());
    }
    while raw.len() < length {
        poll(ctl)?;
        let next = raw.len().checked_add(64 * 1024).ok_or_else(limit)?.min(length);
        raw.resize(next, 0);
    }
    Ok(())
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
impl PreparedColorize {
    pub fn preparation_requirements() -> PreparationRequirements {
        PreparationRequirements { retained_bytes: std::mem::size_of::<Self>() as u64, work_units: PREPARATION_WORK }
    }
    pub fn prepare(
        format: PixelFormat,
        context: WorkingContext,
        parameters: Parameters,
        policy: NumericPolicy,
        input_profile: TransferWitness,
        weights: YWeights,
        budget: Limits,
        ctl: &Interrupt<'_>,
    ) -> Result<Self> {
        poll(ctl)?;
        limits(budget)?;
        let needs = Self::preparation_requirements();
        if needs.retained_bytes > budget.resident_bytes || needs.work_units > budget.work_units {
            return Err(limit());
        }
        if format.mode != ColorMode::Rgb
            || format.sample != SampleType::F32
            || context.encoding != SampleEncoding::NonlinearProfile
            || input_profile.profile_identity == [0; 32]
            || context.profile_identity != input_profile.profile_identity
        {
            return Err(Error::ContextMismatch);
        }
        let [hue, saturation, lightness] = parameters.scalar_values();
        let values = weights.values();
        if values.iter().any(|x| !x.is_finite() || *x < 0.) || values.into_iter().sum::<f64>() != 1. {
            return Err(Error::Parameters);
        }
        poll(ctl)?;
        Ok(Self {
            format,
            context,
            parameters,
            policy,
            input_profile,
            weights,
            hue,
            saturation,
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
    pub fn input_profile(&self) -> TransferWitness {
        self.input_profile
    }
    pub fn transfer_witness(&self) -> TransferWitness {
        self.input_profile
    }
    pub fn sample_work(&self) -> u64 {
        SAMPLE_WORK
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    pub fn weights(&self) -> YWeights {
        self.weights
    }
    /// Pure tuple math; caller graph stages prepay SAMPLE_WORK and bind the actual
    /// current source/context. This does not debit their ledger or prove a source.
    pub fn try_eval_float_color(&self, input: [f32; 3]) -> Result<[f32; 3]> {
        let mut linear = [0.; 3];
        for (value, x) in linear.iter_mut().zip(input) {
            if !x.is_finite() {
                return Err(Error::NonFinite);
            }
            if !(0.0..=1.0).contains(&x) {
                return Err(Error::OutsideUnit);
            }
            let x = f64::from(x);
            *value = match self.input_profile.role {
                TransferRole::CallerEstablishedIecSrgb => {
                    if x <= 0.04045 {
                        x / 12.92
                    } else {
                        ((x + 0.055) / 1.055).powf(2.4)
                    }
                }
                TransferRole::CallerEstablishedExactGamma2 => x * x,
            };
        }
        // Unfused ordered sum of independently supplied coefficients. The profile
        // identity never supplies weights; no ICC row lookup/normalization occurs.
        let mut y = 0.;
        for (value, weight) in linear.into_iter().zip(self.weights.values()) {
            y += value * weight;
        }
        let adjusted = if self.lightness < 0. { y * (1. + self.lightness) } else { y + (1. - y) * self.lightness };
        let rgb = hsl_rgb(self.hue, self.saturation, adjusted);
        let mut output = [0.; 3];
        for (destination, value) in output.iter_mut().zip(rgb) {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(Error::OutsideUnit);
            }
            let encoded = match self.input_profile.role {
                TransferRole::CallerEstablishedIecSrgb => {
                    if value <= 0.0031308 {
                        12.92 * value
                    } else {
                        1.055 * value.powf(1. / 2.4) - 0.055
                    }
                }
                TransferRole::CallerEstablishedExactGamma2 => value.sqrt(),
            };
            *destination = encoded as f32;
            if !destination.is_finite() {
                return Err(Error::NonFinite);
            }
        }
        Ok(output)
    }
    /// One immutable per-instance scalar quota; no reset/refund on failure.
    pub fn evaluate_color_with_interrupt(&self, input: [f32; 3], current: WorkingContext, witness: TransferWitness, ctl: &Interrupt<'_>) -> Result<[f32; 3]> {
        poll(ctl)?;
        if current != self.context || witness != self.input_profile {
            return Err(Error::ContextMismatch);
        }
        self.remaining_work.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(SAMPLE_WORK)).map_err(|_| limit())?;
        let result = self.try_eval_float_color(input);
        poll(ctl)?;
        result
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
        // Known native payload and conservative map/Arc reservation, not RSS.
        // Whole document, source/profile providers and opaque backing remain
        // held in the enclosing caller ledger; no source permission is gained.
        let source_retained_bytes = add(
            add(mul(source_tiles, add(payload, add(1024, std::mem::size_of::<photocraft_raster::Tile>() as u64)?)?)?, bpp)?,
            std::mem::size_of::<Surface>() as u64,
        )?;
        let parameter_retained_bytes = Self::preparation_requirements().retained_bytes;
        let output_retained_bytes = add(
            add(mul(add(source_tiles, count)?, add(payload, add(1024, std::mem::size_of::<photocraft_raster::Tile>() as u64)?)?)?, bpp)?,
            std::mem::size_of::<Surface>() as u64,
        )?;
        let output_index_bytes = add(mul(add(source_tiles, count)?, 1024)?, std::mem::size_of::<Surface>() as u64)?;
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
    /// Selected native Float colors are admitted as unit before allocation.
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
                self.try_eval_float_color(read_rgb(selected)?)?;
            }
        }
        poll(ctl)?;
        if roi.is_empty() {
            return Ok(source.clone());
        }
        poll(ctl)?;
        let length = checked_region_byte_len(roi, self.format)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if raw.capacity() as u64 > mul(length as u64, 2)? {
            return Err(limit());
        }
        initialize_with_interrupt(&mut raw, length, ctl)?;
        source.try_read_interleaved_into_with(roi, &mut raw, ctl)?;
        for (i, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if i.is_multiple_of(256) {
                poll(ctl)?;
            }
            let values = self.try_eval_float_color(read_rgb(pixel)?)?;
            for (channel, value) in values.into_iter().enumerate() {
                try_write_sample_f64(pixel, SampleType::F32, channel, f64::from(value))?;
            }
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
#[path = "colorize_candidate_tests.rs"]
mod tests;
