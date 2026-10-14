//! Additive closed sector Hue/Saturation declared numerical candidate.
//! Earlier exclusive and mixed recipes remain unchanged. Original HSL with
//! explicitly rounded Float six-phase chooses the local range; achromatic source
//! skips local controls. Result saturation clamps to0..1 as an observed equivalent
//! output policy, not proof of the unique internal floor placement. Combined
//! coefficients, lightness and RGB remain unclipped; finite HDR output is allowed.
//!
//! Sealed sector/floor c123003a/b794f8c6 supports Yellow/Cyan/Magenta pure hue at
//! zero overlap and exclusive local selection. Existing ALL/RED controls retain
//! the mixed58e26/af3a and bounds916c/d430 declared schedule. Green/Blue, adjacent
//! local combinations and other named-sector S/L remain explicit refusals.
//! The additive AllPureHue policy separately admits Green/Blue exclusive hue
//! at overlap0 from sealed c16ce6d3/3188bf93. It does not resolve the adjacent
//! mixed-window/HSL-blend ambiguity or enroll general local S/L/overlap controls.
//! The original policy still rejects Green/Blue with identical behavior.
//! This Float-ingress/f64-HSL/final-Float recipe is a candidate, not hidden TRC,
//! unique intermediate precision, source/default qualification or native equality.
//! AdjacentMappedHsl separately consumes nonmid6cfe5b4c/08d5a2b6: only Y/G or
//! C/B pairs with neutral ALL/RED/others and overlap exactly.5. Original local
//! HSL maps blend before RGB once. Dynamic unsupported hue wrap/half-turn refuses;
//! no universal circular policy, all-sector symmetry or coefficient-sum identity
//! follows. Earlier ALL/RED coefficient-sum recipes remain declared candidates,
//! not retroactively asserted as unique source mathematics.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::atomic::{AtomicU64, Ordering};
pub const PREPARATION_WORK: u64 = 1024;
/// Declared operation work, not a measured libm latency bound.
pub const SAMPLE_WORK: u64 = 768;
/// The distinct adjacent policy prices two weighted local HSL maps.
pub const ADJACENT_SAMPLE_WORK: u64 = 1536;
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
    UnsupportedHueWrap,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericPolicy {
    OriginalHslF32SixPhaseOutputSaturationBound,
    /// All six independently observed exclusive pure-hue sectors, not all-sector
    /// mixed controls. Earlier ALL/RED lanes retain their existing declared math.
    OriginalHslF32SixPhaseAllPureHueOutputSaturationBound,
    /// Closed Y/G or C/B pair at overlap.5, original mapped HSL components.
    /// Unsupported weighted hue wrapping/half-turn ambiguity refuses explicitly.
    OriginalHslF32SixPhaseAdjacentMappedHsl,
}
pub struct PreparedSectorHueSaturation {
    format: PixelFormat,
    context: WorkingContext,
    parameters: Parameters,
    policy: NumericPolicy,
    ///0 selects the previously supported RED lane;1/3/5 are exclusive hue sectors.
    local_sector: usize,
    all_hue: f64,
    local_hue: f64,
    local_saturation: f64,
    local_lightness: f64,
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
impl PreparedSectorHueSaturation {
    pub fn preparation_requirements() -> PreparationRequirements {
        PreparationRequirements { retained_bytes: std::mem::size_of::<Self>() as u64, work_units: PREPARATION_WORK }
    }
    pub fn prepare(
        format: PixelFormat,
        context: WorkingContext,
        parameters: Parameters,
        policy: NumericPolicy,
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
        // Compile one explicitly admitted lane without dropping any raw words.
        // The named-sector controls were separately observed only as exclusive
        // pure hue with ALL neutral and overlap0; no symmetry/mix is inferred.
        let mut local_sector = 0usize;
        let mut local = *red;
        if policy == NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl {
            if overlap != 0.5 || all.iter().chain(red).any(|bits| f64::from_bits(*bits) != 0.0) {
                return Err(Error::UnsupportedConfiguration);
            }
            let active =
                |index: usize| -> Result<bool> { Ok(parameters.ranges.get(index).ok_or(Error::Parameters)?.iter().any(|bits| f64::from_bits(*bits) != 0.0)) };
            let pair = if active(2)? && active(3)? && !active(4)? && !active(5)? && !active(6)? {
                (2usize, 6usize)
            } else if !active(2)? && !active(3)? && active(4)? && active(5)? && !active(6)? {
                (4usize, 7usize)
            } else {
                return Err(Error::UnsupportedConfiguration);
            };
            local = *parameters.ranges.get(pair.0).ok_or(Error::Parameters)?;
            local_sector = pair.1;
        } else {
            let mut named = None;
            for (index, range) in parameters.ranges.iter().enumerate().skip(2) {
                if range.iter().all(|bits| f64::from_bits(*bits) == 0.0) {
                    continue;
                }
                let admitted = matches!(index, 2 | 4 | 6)
                    || matches!(policy, NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound) && matches!(index, 3 | 5);
                if !admitted
                    || named.is_some()
                    || overlap != 0.0
                    || all.iter().any(|bits| f64::from_bits(*bits) != 0.0)
                    || red.iter().any(|bits| f64::from_bits(*bits) != 0.0)
                    || range.iter().skip(1).any(|bits| f64::from_bits(*bits) != 0.0)
                {
                    return Err(Error::UnsupportedConfiguration);
                }
                named = Some(index);
                local_sector = index.checked_sub(1).ok_or(Error::Parameters)?;
                local = *range;
            }
        }
        let local_hue = f64::from_bits(*local.first().ok_or(Error::Parameters)?);
        let local_saturation = f64::from_bits(*local.get(1).ok_or(Error::Parameters)?);
        let local_lightness = f64::from_bits(*local.get(2).ok_or(Error::Parameters)?);
        if overlap > 0.0 && overlap / 6.0 == 0.0 {
            return Err(Error::Parameters);
        }
        poll(ctl)?;
        Ok(Self {
            format,
            context,
            parameters,
            policy,
            local_sector,
            all_hue,
            local_hue,
            local_saturation,
            local_lightness,
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
    pub fn policy(&self) -> NumericPolicy {
        self.policy
    }
    pub fn parameters(&self) -> Parameters {
        self.parameters
    }
    pub fn sample_work(&self) -> u64 {
        match self.policy {
            NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl => ADJACENT_SAMPLE_WORK,
            NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound | NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound => SAMPLE_WORK,
        }
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    /// Allocation-free declared native-nonlinear unit RGB HSL math. Enclosing
    /// consumers prepay sample_work() per RGB tuple and poll their actual job token.
    /// RGB/lightness/combined parameters remain unclipped. Only the explicit
    /// hue weight and resulting saturation0..1 output are bounded here.
    /// This output-equivalent floor does not identify the hidden internal stage.
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
        if self.policy == NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl {
            // This policy retains the original source HSL for both maps; opposing
            // signed lightness branches must be mapped before components blend.
            if saturation != 0.0 {
                let phase = f64::from((hue * 6.0) as f32);
                let first = match self.local_sector {
                    6 => 2usize,
                    7 => 4usize,
                    _ => return Err(Error::UnsupportedConfiguration),
                };
                let second = first.checked_add(1).ok_or(Error::Parameters)?;
                let mut weight_sum = 0.0;
                let mut hue_sum = 0.0;
                let mut sat_sum = 0.0;
                let mut light_sum = 0.0;
                for index in [first, second] {
                    let center = index.checked_sub(1).ok_or(Error::Parameters)? as f64;
                    let difference = (phase - center + 3.0).rem_euclid(6.0) - 3.0;
                    let weight = (0.5 + (0.5 - difference.abs()) / self.overlap).clamp(0.0, 1.0);
                    if weight == 0.0 {
                        continue;
                    }
                    let range = self.parameters.ranges.get(index).ok_or(Error::Parameters)?;
                    let mapped_hue = hue + f64::from_bits(*range.first().ok_or(Error::Parameters)?) / 2.0;
                    // No universal circular-anchor claim: this closed policy
                    // accepts only nonwrapping shifts strictly below half a turn.
                    if !(0.0..1.0).contains(&mapped_hue) || (mapped_hue - hue).abs() >= 0.5 {
                        return Err(Error::UnsupportedHueWrap);
                    }
                    let mapped_sat = (saturation * (1.0 + f64::from_bits(*range.get(1).ok_or(Error::Parameters)?))).clamp(0.0, 1.0);
                    let lightness = f64::from_bits(*range.get(2).ok_or(Error::Parameters)?);
                    let mapped_light = if lightness < 0.0 { light * (1.0 + lightness) } else { light + (1.0 - light) * lightness };
                    weight_sum += weight;
                    hue_sum += weight * mapped_hue;
                    sat_sum += weight * mapped_sat;
                    light_sum += weight * mapped_light;
                }
                // Unaffected local ranges are neutral mapped source HSL. This
                // remaining weight preserves the neutral conversion outside pair.
                let neutral = 1.0 - weight_sum;
                hue = hue_sum + neutral * hue;
                saturation = sat_sum + neutral * saturation;
                light = light_sum + neutral * light;
            }
        } else {
            let original_saturation = saturation;
            let original_hue = hue;
            let phase = match self.policy {
                NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound
                | NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound
                | NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl => f64::from((original_hue * 6.0) as f32) / 6.0,
            };
            let distance = phase.min(1.0 - phase);
            let weight = if original_saturation == 0.0 {
                0.0
            } else if self.local_sector != 0 {
                // Named pure hue lanes: original Float six-phase band, lower
                // inclusive/upper exclusive. These sectors never receive overlap.
                let phase_six = match self.policy {
                    NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound
                    | NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound
                    | NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl => f64::from((original_hue * 6.0) as f32),
                };
                let center = self.local_sector as f64;
                if phase_six >= center - 0.5 && phase_six < center + 0.5 { 1.0 } else { 0.0 }
            } else if self.overlap == 0.0 {
                if phase < 1.0 / 12.0 || phase >= 11.0 / 12.0 { 1.0 } else { 0.0 }
            } else {
                (0.5 + (1.0 / 12.0 - distance) / (self.overlap / 6.0)).clamp(0.0, 1.0)
            };
            let combined_hue = self.all_hue + weight * self.local_hue;
            let combined_saturation = self.all_saturation + weight * self.local_saturation;
            let combined_lightness = self.all_lightness + weight * self.local_lightness;
            hue = (hue + combined_hue / 2.0).rem_euclid(1.0);
            saturation = saturation * (1.0 + combined_saturation);
            // Unit-input observations support this output-equivalent bound;
            // the hidden location of the negative floor remains unidentified.
            saturation = saturation.clamp(0.0, 1.0);
            light = if combined_lightness < 0.0 { light * (1.0 + combined_lightness) } else { light + (1.0 - light) * combined_lightness };
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
            add(mul(length, 8)?, mul(length / bpp, self.sample_work())?)?,
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
#[path = "sector_hue_saturation_candidate_tests.rs"]
mod tests;
