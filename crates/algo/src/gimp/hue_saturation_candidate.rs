//! DISTINCT declared GIMP Hue/Saturation numerical candidate, not GEGL HueChroma.
//! Storage49c09d47e4626b98b707955a9990e0e6dae8f790487a532f6480a410ed74cb89;
//! partial17e3d4e8c2ddbadc4a2469e8083c2f180480a4539744838b0e22c392a7652329.
//! Native-nonlinear HSL/f64/finalFloat model is a tested hypothesis:42/96 opaque
//! RGB words exact, maxresidual1.6391277313232422e-7, not hiddenTRC/precision proof.
//! Only exclusive ALL+RED hue, negative ALL saturation, positive ALL lightness,
//! or neutral recipes are represented. Finite coefficients in these directions
//! extend a declared formula, not authored-source/default evaluator admission.
//! All23 raw configuration words remain immutable; unsupported controls refuse.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::atomic::{AtomicU64, Ordering};
pub const PREPARATION_WORK: u64 = 1024;
/// Declared operation work, not a measured libm latency bound.
pub const SAMPLE_WORK: u64 = 768;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub time_bits: Option<u64>,
    /// all/red/yellow/green/cyan/blue/magenta, each hue/saturation/lightness.
    pub ranges: [[u64; 3]; 7],
    pub overlap_bits: u64,
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
    OutsideUnit,
    UnsupportedConfiguration,
}
impl From<RasterInputError> for Error {
    fn from(v: RasterInputError) -> Self {
        Self::Raster(v)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GIMP Hue/Saturation numerical candidate: {self:?}")
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
#[derive(Clone, Copy)]
enum Lane {
    Neutral,
    Hue,
    Saturation,
    Lightness,
}
pub struct PreparedHueSaturation {
    format: PixelFormat,
    context: WorkingContext,
    parameters: Parameters,
    lane: Lane,
    all_hue: f64,
    red_hue: f64,
    all_saturation: f64,
    all_lightness: f64,
    overlap: f64,
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
impl PreparedHueSaturation {
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
        if format.mode != ColorMode::Rgb
            || !matches!(format.sample, SampleType::F32 | SampleType::F64)
            || context.encoding != SampleEncoding::NonlinearProfile
            || context.profile_identity == [0; 32]
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
        let all = parameters.ranges.first().ok_or(Error::Parameters)?;
        let red = parameters.ranges.get(1).ok_or(Error::Parameters)?;
        let all_hue = f64::from_bits(*all.first().ok_or(Error::Parameters)?);
        let all_saturation = f64::from_bits(*all.get(1).ok_or(Error::Parameters)?);
        let all_lightness = f64::from_bits(*all.get(2).ok_or(Error::Parameters)?);
        let red_hue = f64::from_bits(*red.first().ok_or(Error::Parameters)?);
        if parameters.ranges.iter().skip(2).flatten().any(|bits| f64::from_bits(*bits) != 0.0)
            || red.iter().skip(1).any(|bits| f64::from_bits(*bits) != 0.0)
            || all_saturation > 0.0
            || all_lightness < 0.0
        {
            return Err(Error::UnsupportedConfiguration);
        }
        let hue = all_hue != 0.0 || red_hue != 0.0;
        let saturation = all_saturation != 0.0;
        let lightness = all_lightness != 0.0;
        if u8::from(hue) + u8::from(saturation) + u8::from(lightness) > 1 {
            return Err(Error::UnsupportedConfiguration);
        }
        let lane = if hue {
            Lane::Hue
        } else if saturation {
            Lane::Saturation
        } else if lightness {
            Lane::Lightness
        } else {
            Lane::Neutral
        };
        if hue && overlap > 0.0 && overlap / 6.0 == 0.0 {
            return Err(Error::Parameters);
        }
        poll(ctl)?;
        Ok(Self {
            format,
            context,
            parameters,
            lane,
            all_hue,
            red_hue,
            all_saturation,
            all_lightness,
            overlap,
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
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    /// Allocation-free declared native-nonlinear unit RGB HSL math. Enclosing
    /// consumers prepay SAMPLE_WORK per RGB tuple and poll their actual job token.
    /// Output is not clamped; only the explicit hue weight uses a unit clamp.
    /// Neutral still follows conversion equations, not a raw-source alias.
    pub fn try_eval_float_rgb(&self, input: [f32; 3]) -> Result<[f32; 3]> {
        if input.iter().any(|value| !value.is_finite()) {
            return Err(Error::NonFinite);
        }
        if input.iter().any(|value| !(0.0..=1.0).contains(value)) {
            return Err(Error::OutsideUnit);
        }
        let [r, g, b] = input.map(f64::from);
        let high = r.max(g).max(b);
        let low = r.min(g).min(b);
        let chroma = high - low;
        let mut light = (high + low) / 2.0;
        let mut hue = 0.0;
        let mut saturation = 0.0;
        if chroma != 0.0 {
            // Public HSL piecewise denominator avoids cancellation for dark
            // subnormal Float colors, retaining finite-unit input without clamp.
            saturation = chroma / if light <= 0.5 { high + low } else { 2.0 - high - low };
            let sector = if high == r {
                (g - b) / chroma
            } else if high == g {
                (b - r) / chroma + 2.0
            } else {
                (r - g) / chroma + 4.0
            };
            hue = (sector / 6.0).rem_euclid(1.0);
        }
        match self.lane {
            Lane::Neutral => {}
            Lane::Hue => {
                let distance = hue.min(1.0 - hue);
                let weight = if self.overlap == 0.0 {
                    if hue < 1.0 / 12.0 || hue >= 11.0 / 12.0 { 1.0 } else { 0.0 }
                } else {
                    (0.5 + (1.0 / 12.0 - distance) / (self.overlap / 6.0)).clamp(0.0, 1.0)
                };
                hue = (hue + 0.5 * (self.all_hue + weight * self.red_hue)).rem_euclid(1.0);
            }
            Lane::Saturation => saturation *= 1.0 + self.all_saturation,
            Lane::Lightness => light = light + (1.0 - light) * self.all_lightness,
        }
        // Stable inverse of the same public HSL relation: subtracting from1
        // would erase the chroma of valid subnormal dark Float colors.
        let c = (if light <= 0.5 { 2.0 * light } else { 2.0 - 2.0 * light }) * saturation;
        let h = hue * 6.0;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let m = light - c / 2.0;
        let rgb = if h < 1.0 {
            [c, x, 0.0]
        } else if h < 2.0 {
            [x, c, 0.0]
        } else if h < 3.0 {
            [0.0, c, x]
        } else if h < 4.0 {
            [0.0, x, c]
        } else if h < 5.0 {
            [x, 0.0, c]
        } else {
            [c, 0.0, x]
        };
        let output = rgb.map(|value| (value + m) as f32);
        if !hue.is_finite() || !saturation.is_finite() || !light.is_finite() || output.iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite);
        }
        Ok(output)
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
            add(mul(length, 8)?, mul(length / bpp, SAMPLE_WORK)?)?,
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
    /// F64 selected colors explicitly enter Float32 before HSL math; final
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
            let mut rgb = [0f32; 3];
            for (channel, target) in rgb.iter_mut().enumerate() {
                let sample = try_read_sample_f64(pixel, self.format.sample, channel).map_err(|_| Error::NonFinite)?;
                if !sample.is_finite() {
                    return Err(Error::NonFinite);
                }
                if !(0.0..=1.0).contains(&sample) {
                    return Err(Error::OutsideUnit);
                }
                *target = sample as f32;
            }
            let rgb = self.try_eval_float_rgb(rgb)?;
            for (channel, value) in rgb.into_iter().enumerate() {
                try_write_sample_f64(pixel, self.format.sample, channel, f64::from(value)).map_err(|_| Error::NonFinite)?;
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
#[path = "hue_saturation_candidate_tests.rs"]
mod tests;
