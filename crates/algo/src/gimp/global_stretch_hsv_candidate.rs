//! Once-prepared, explicitly declared HSV range-stretch mathematics.
//! The sealed discriminator13381344 remains strict-HOLD for effective ICC
//! identity. Binary64 HSV/final Float fits its four literal cases within2.98e-7;
//! intermediate precision is ambiguous, not bit-exact GIMP qualification.
//! Complete immutable caller-established nonlinear RGB Float source extent is
//! reduced once, including alpha-zero pixels. Independent degenerate S/V ranges
//! retain that component. Alpha/default/off-ROI words remain raw. No CMS, mask,
//! outer composition, source admission, or default renderer authority is granted.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleAccessError, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, Tile, checked_region_byte_len};
use std::sync::atomic::{AtomicU64, Ordering};
pub const PREPARATION_WORK: u64 = 512;
pub const SAMPLE_WORK: u64 = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericPolicy {
    DeclaredBinary64HsvFinalFloat,
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
#[derive(Clone, Copy, Debug)]
pub struct PreparationRequirements {
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
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
    Access(SampleAccessError),
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
        write!(f, "declared HSV range stretch: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn limit() -> Error {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn validate_limits(v: Limits) -> Result<()> {
    let d = Limits::default();
    if v.resident_bytes == 0 || v.resident_bytes > d.resident_bytes || v.work_units > d.work_units {
        return Err(limit());
    }
    Ok(())
}
fn validate(format: PixelFormat, context: WorkingContext) -> Result<()> {
    if format.mode != ColorMode::Rgb || format.sample != SampleType::F32 {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    if context.encoding != SampleEncoding::NonlinearProfile || context.profile_identity == [0; 32] {
        return Err(Error::ContextMismatch);
    }
    Ok(())
}
fn tiles(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Err(RasterInputError::InvalidBounds.into());
    }
    let first = TileCoord::containing(r.x0, r.y0);
    let last = TileCoord::containing(r.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, r.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for c in [first, last] {
        c.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        c.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
fn source_bound(source: &Surface) -> Result<u64> {
    let bpp = source.format().bytes_per_pixel() as u64;
    if source.default_bytes().len() as u64 != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    // Known native payload plus conservative sparse-map/Arc/header reservation;
    // not measured RSS, provider/whole enclosing ownership remains caller-held.
    add(
        add(std::mem::size_of::<Surface>() as u64, bpp)?,
        mul(source.tile_count() as u64, add(mul(65536, bpp)?, add(1024, std::mem::size_of::<Tile>() as u64)?)?)?,
    )
}
fn rgb(pixel: &[u8]) -> Result<[f64; 3]> {
    let mut values = [0.; 3];
    for (i, v) in values.iter_mut().enumerate() {
        *v = try_read_sample_f64(pixel, SampleType::F32, i)?;
        if !v.is_finite() {
            return Err(Error::NonFinite);
        }
        if !(0.0..=1.0).contains(v) {
            return Err(Error::OutsideUnit);
        }
    }
    Ok(values)
}
fn hsv([r, g, b]: [f64; 3]) -> [f64; 3] {
    let high = r.max(g).max(b);
    let low = r.min(g).min(b);
    if high == low {
        return [0., 0., high];
    }
    let delta = high - low;
    let s = delta / high;
    let rc = (high - r) / delta;
    let gc = (high - g) / delta;
    let bc = (high - b) / delta;
    let h = if r == high {
        bc - gc
    } else if g == high {
        2. + rc - bc
    } else {
        4. + gc - rc
    };
    [(h / 6.).rem_euclid(1.), s, high]
}
fn from_hsv([h, s, v]: [f64; 3]) -> [f64; 3] {
    if s == 0. {
        return [v; 3];
    }
    let phase = h * 6.;
    let sector = phase.floor();
    let fraction = phase - sector;
    let p = v * (1. - s);
    let q = v * (1. - s * fraction);
    let t = v * (1. - s * (1. - fraction));
    match (sector as u32) % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}
pub struct PreparedStretchHsv<'source> {
    source: &'source Surface,
    extent: Rect,
    context: WorkingContext,
    policy: NumericPolicy,
    minima: [f64; 2],
    spans: [f64; 2],
    limits: Limits,
    preparation: PreparationRequirements,
    remaining_work: AtomicU64,
}
/// O(1) prospective admission before source validation or temporary allocation.
pub fn preparation_requirements(source: &Surface, extent: Rect, context: WorkingContext, _policy: NumericPolicy) -> Result<PreparationRequirements> {
    validate(source.format(), context)?;
    checked_region_byte_len(extent, source.format())?;
    tiles(extent)?;
    let retained = add(source_bound(source)?, std::mem::size_of::<PreparedStretchHsv<'_>>() as u64)?;
    Ok(PreparationRequirements {
        retained_bytes: retained,
        peak_resident_bytes: retained,
        index_bytes: mul(source.tile_count() as u64, 1024)?,
        work_units: add(PREPARATION_WORK, add(mul(extent.size().area(), SAMPLE_WORK)?, mul(source.tile_count() as u64, 256)?)?)?,
    })
}
impl<'source> PreparedStretchHsv<'source> {
    pub fn prepare(
        source: &'source Surface,
        extent: Rect,
        context: WorkingContext,
        policy: NumericPolicy,
        budget: Limits,
        ctl: &Interrupt<'_>,
    ) -> Result<Self> {
        poll(ctl)?;
        validate_limits(budget)?;
        let preparation = preparation_requirements(source, extent, context, policy)?;
        if preparation.peak_resident_bytes > budget.resident_bytes || preparation.work_units > budget.work_units {
            return Err(limit());
        }
        // Prospective native bound precedes every retained tile visit and numeric pass.
        let payload = mul(65536, source.format().bytes_per_pixel() as u64)?;
        for (coord, tile) in source.tiles() {
            poll(ctl)?;
            coord.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
            coord.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
            if tile.bytes().len() as u64 != payload {
                return Err(RasterInputError::LengthMismatch.into());
            }
        }
        let bpp = source.format().bytes_per_pixel();
        let mut pixel = [0u8; 16];
        let bytes = pixel.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        let mut minima = [f64::INFINITY; 2];
        let mut maxima = [f64::NEG_INFINITY; 2];
        for y in extent.y0..extent.y1 {
            poll(ctl)?;
            for (i, x) in (extent.x0..extent.x1).enumerate() {
                if i.is_multiple_of(256) {
                    poll(ctl)?;
                }
                source.try_read_pixel_bytes(x, y, bytes)?;
                let [_, s, v] = hsv(rgb(bytes)?);
                for ((low, high), value) in minima.iter_mut().zip(&mut maxima).zip([s, v]) {
                    *low = low.min(value);
                    *high = high.max(value);
                }
            }
        }
        let mut spans = [0.; 2];
        for ((span, high), low) in spans.iter_mut().zip(maxima).zip(minima) {
            *span = high - low;
            if !span.is_finite() || *span < 0. {
                return Err(Error::NonFinite);
            }
        }
        poll(ctl)?;
        Ok(Self {
            source,
            extent,
            context,
            policy,
            minima,
            spans,
            limits: budget,
            preparation,
            remaining_work: AtomicU64::new(budget.work_units - preparation.work_units),
        })
    }
    pub fn source(&self) -> &'source Surface {
        self.source
    }
    pub fn extent(&self) -> Rect {
        self.extent
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    pub fn policy(&self) -> NumericPolicy {
        self.policy
    }
    pub fn extrema(&self) -> ([f64; 2], [f64; 2]) {
        (self.minima, self.spans)
    }
    pub fn requirements(&self) -> PreparationRequirements {
        self.preparation
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    pub fn sample_work(&self) -> u64 {
        SAMPLE_WORK
    }
    /// Pure declared tuple math. This does not debit a quota or prove a tuple came
    /// from this source; graph callers bind the original source and prepay samples.
    pub fn try_eval_float_color(&self, input: [f32; 3]) -> Result<[f32; 3]> {
        let mut native = [0.; 3];
        for (v, x) in native.iter_mut().zip(input) {
            if !x.is_finite() {
                return Err(Error::NonFinite);
            }
            if !(0.0..=1.0).contains(&x) {
                return Err(Error::OutsideUnit);
            }
            *v = f64::from(x);
        }
        let [h, s, v] = hsv(native);
        let mut components = [s, v];
        for ((component, low), span) in components.iter_mut().zip(self.minima).zip(self.spans) {
            if span != 0. {
                *component = (*component - low) / span;
            }
        }
        let values = from_hsv([h, components[0], components[1]]);
        let mut output = [0.; 3];
        for (v, x) in output.iter_mut().zip(values) {
            if !x.is_finite() || !(0.0..=1.0).contains(&x) {
                return Err(Error::OutsideUnit);
            }
            *v = x as f32;
        }
        Ok(output)
    }
    pub fn application_requirements(&self, roi: Rect, current: WorkingContext, ctl: &Interrupt<'_>) -> Result<Requirements> {
        poll(ctl)?;
        if current != self.context {
            return Err(Error::ContextMismatch);
        }
        if !self.extent.contains_rect(&roi) {
            return Err(RasterInputError::InvalidBounds.into());
        }
        let count = tiles(roi)?;
        let bpp = self.source.format().bytes_per_pixel() as u64;
        let length = checked_region_byte_len(roi, self.source.format())? as u64;
        let payload = mul(65536, bpp)?;
        let source_tiles = self.source.tile_count() as u64;
        let source_retained_bytes = source_bound(self.source)?;
        let parameter_retained_bytes = std::mem::size_of::<Self>() as u64;
        let output_retained_bytes = add(
            add(mul(add(source_tiles, count)?, add(payload, add(1024, std::mem::size_of::<Tile>() as u64)?)?)?, bpp)?,
            std::mem::size_of::<Surface>() as u64,
        )?;
        let output_index_bytes = mul(add(source_tiles, count)?, 1024)?;
        let scratch_bytes = add(mul(length, 2)?, add(mul(payload, 4)?, 4096)?)?;
        let peak_resident_bytes =
            add(add(source_retained_bytes, parameter_retained_bytes)?, add(add(output_retained_bytes, output_index_bytes)?, scratch_bytes)?)?;
        let work_units = add(
            add(mul(length, 8)?, mul(roi.size().area(), SAMPLE_WORK)?)?,
            add(mul(mul(count, add(source_tiles, count)?)?, 512)?, mul(mul(count, payload)?, 2)?)?,
        )?;
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
    pub fn apply_with_limits(&self, roi: Rect, current: WorkingContext, budget: Limits, ctl: &Interrupt<'_>) -> Result<Surface> {
        poll(ctl)?;
        validate_limits(budget)?;
        if budget.resident_bytes > self.limits.resident_bytes || budget.work_units > self.limits.work_units {
            return Err(limit());
        }
        let needs = self.application_requirements(roi, current, ctl)?;
        if needs.peak_resident_bytes > budget.resident_bytes || needs.work_units > budget.work_units {
            return Err(limit());
        }
        self.remaining_work.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(needs.work_units)).map_err(|_| limit())?;
        // One immutable first-pass admission establishes all selected source words.
        // No rescan of full extent, no quota reset/refund on failure or cancellation.
        poll(ctl)?;
        let length = checked_region_byte_len(roi, self.source.format())?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if raw.capacity() as u64 > mul(length as u64, 2)? {
            return Err(limit());
        }
        raw.resize(length, 0);
        self.source.try_read_interleaved_into_with(roi, &mut raw, ctl)?;
        for (i, pixel) in raw.chunks_exact_mut(self.source.format().bytes_per_pixel()).enumerate() {
            if i.is_multiple_of(256) {
                poll(ctl)?;
            }
            let values = rgb(pixel)?;
            let output = self.try_eval_float_color(values.map(|v| v as f32))?;
            for (channel, value) in output.into_iter().enumerate() {
                try_write_sample_f64(pixel, SampleType::F32, channel, f64::from(value))?;
            }
        }
        poll(ctl)?;
        let mut output = self.source.clone();
        // Existing native writer polls at tile boundaries; full tile default fill/copy
        // is priced, not claimed to poll once per byte or reflect physical RSS.
        output.try_write_interleaved_with(roi, &raw, ctl)?;
        poll(ctl)?;
        Ok(output)
    }
}
#[cfg(test)]
#[path = "global_stretch_hsv_candidate_tests.rs"]
mod tests;
