//! Explicit per-channel exponent in a caller-established floating working context.
//! No ICC TRC, gamma convention, reciprocal exponent, premultiplication, GEGL
//! operation/version or serialized-source default is inferred.
use super::wide_point::WorkingContext;
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
const MAX_WORK: u64 = 1 << 30;
// Declared operation-work units, not a measured libm latency/RSS bound.
const SAMPLE_WORK: u64 = 80;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NegativePolicy {
    Reject,
    SignedMagnitude,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Unit,
    FiniteHdr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowPolicy {
    Reject,
}
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub negative: NegativePolicy,
    pub domain: Domain,
    pub overflow: OverflowPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct ChannelExponent {
    pub channel: usize,
    pub exponent: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Selector,
    Exponent,
    ContextMismatch,
    NonFinite,
    Negative,
    Domain,
    Overflow,
}
impl From<RasterInputError> for Error {
    fn from(value: RasterInputError) -> Self {
        Self::Raster(value)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native exponent: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
/// Conservative borrowed-input and private-output admission; not allocator/RSS.
/// Index bytes include the one cloned sparse map and temporary per-tile builder
/// envelopes. Existing upstream input charges are never refunded by this estimate.
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
pub struct PreparedGamma<'a> {
    format: PixelFormat,
    context: WorkingContext,
    channels: &'a Vec<ChannelExponent>,
    policy: Policy,
    retained: u64,
    sample_work: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn limits(value: Limits) -> Result<()> {
    if value.resident_bytes == 0 || value.resident_bytes > MAX_CHECKED_RASTER_BYTES || value.work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded.into());
    }
    Ok(())
}
fn validate_value(value: f64, policy: Policy) -> Result<()> {
    if !value.is_finite() {
        return Err(Error::NonFinite);
    }
    if policy.negative == NegativePolicy::Reject && value < 0.0 {
        return Err(Error::Negative);
    }
    // Signed Unit means magnitude<=1; Reject Unit means ordinary [0,1].
    if policy.domain == Domain::Unit && value.abs() > 1.0 {
        return Err(Error::Domain);
    }
    Ok(())
}
impl<'a> PreparedGamma<'a> {
    /// Select only color channels; alpha is never selected or numerically inspected.
    /// Positive finite exponents are used directly (not implicitly reciprocated).
    pub fn prepare(
        format: PixelFormat,
        context: WorkingContext,
        channels: &'a Vec<ChannelExponent>,
        policy: Policy,
        budget: Limits,
        control: &Interrupt<'_>,
    ) -> Result<Self> {
        poll(control)?;
        limits(budget)?;
        if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::F32 | SampleType::F64) {
            return Err(RasterInputError::UnsupportedConversion.into());
        }
        if channels.is_empty() || channels.len() > format.mode.color_channels() {
            return Err(Error::Selector);
        }
        let retained = (channels.capacity() as u64).checked_mul(std::mem::size_of::<ChannelExponent>() as u64).ok_or(RasterInputError::LimitExceeded)?;
        let sample_work = (channels.len() as u64).checked_mul(SAMPLE_WORK).ok_or(RasterInputError::LimitExceeded)?;
        if retained > budget.resident_bytes || sample_work > budget.work_units {
            return Err(RasterInputError::LimitExceeded.into());
        }
        let mut seen = [false; 3];
        for selected in channels {
            poll(control)?;
            if selected.channel >= format.mode.color_channels() {
                return Err(Error::Selector);
            }
            let flag = seen.get_mut(selected.channel).ok_or(Error::Selector)?;
            if *flag {
                return Err(Error::Selector);
            }
            *flag = true;
            if !selected.exponent.is_finite() || selected.exponent <= 0.0 {
                return Err(Error::Exponent);
            }
        }
        match policy.overflow {
            OverflowPolicy::Reject => {}
        }
        poll(control)?;
        Ok(Self { format, context, channels, policy, retained, sample_work })
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    /// Metadata-only checked requirements. No raster sample reads, table copies,
    /// rendering or hashing. Invalid retained tile layouts refuse before execution.
    pub fn requirements(&self, source: &Surface, roi: Rect, current: WorkingContext) -> Result<Requirements> {
        self.requirements_with_interrupt(source, roi, current, &Interrupt::NONE)
    }
    pub fn requirements_with_interrupt(&self, source: &Surface, roi: Rect, current: WorkingContext, control: &Interrupt<'_>) -> Result<Requirements> {
        poll(control)?;
        if current != self.context || source.format() != self.format {
            return Err(Error::ContextMismatch);
        }
        let format = source.format();
        let bpp = format.bytes_per_pixel();
        let length = checked_region_byte_len(roi, format)?;
        let output_tiles = tile_count(roi)?;
        let source_tiles = source.tile_count() as u64;
        let tile_bytes = 65536u64.checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
        let source_retained_bytes = source_tiles
            .checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|n| n.checked_add(bpp as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        // The standard source envelope bounds the metadata traversal even if the
        // eventual graph supplies a smaller aggregate cap. It never raises a cap.
        if source_retained_bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded.into());
        }
        if source.default_bytes().len() != bpp {
            return Err(RasterInputError::LengthMismatch.into());
        }
        for (_, tile) in source.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch.into());
            }
        }
        source.try_validate_tile_origins_with(control)?;
        let output_retained_bytes = output_tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
        let output_index_bytes =
            source_tiles.checked_mul(256).and_then(|n| n.checked_add(output_tiles.checked_mul(1024)?)).ok_or(RasterInputError::LimitExceeded)?;
        let scratch_bytes = if roi.is_empty() { 0 } else { tile_bytes.checked_add(tile_bytes / 16).ok_or(RasterInputError::LimitExceeded)? };
        let peak_resident_bytes = source_retained_bytes
            .checked_add(self.retained)
            .and_then(|n| n.checked_add(output_retained_bytes))
            .and_then(|n| n.checked_add(output_index_bytes))
            .and_then(|n| n.checked_add(scratch_bytes))
            .ok_or(RasterInputError::LimitExceeded)?;
        let work_units = (length as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(((length / bpp) as u64).checked_mul(self.sample_work)?))
            .and_then(|n| n.checked_add(output_tiles.checked_mul(tile_bytes)?.checked_mul(8)?))
            .and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        poll(control)?;
        Ok(Requirements {
            source_retained_bytes,
            parameter_retained_bytes: self.retained,
            output_retained_bytes,
            output_index_bytes,
            scratch_bytes,
            peak_resident_bytes,
            work_units,
        })
    }
    pub fn retained_parameter_bytes(&self) -> u64 {
        self.retained
    }
    /// Selected colors use f64 powf(abs(value)), then explicit sign and one native
    /// rounding. SignedMagnitude preserves negative zero; Reject permits input
    /// negative zero but nonidentity output is positive zero. Exponent1 preserves
    /// every selected finite raw bit after domain validation. Native F32 underflow
    /// follows its ordinary final conversion; finite overflow refuses.
    /// Unselected colors, raw alpha, defaults and all outside-ROI pixels are exact.
    /// Retained source/parameters/maps/output and bounded scratch are admitted;
    /// existing std map/Arc allocations are prepaid, not universal OOM protection.
    pub fn apply(&self, source: &Surface, roi: Rect, current: WorkingContext, budget: Limits, control: &Interrupt<'_>) -> Result<Surface> {
        poll(control)?;
        limits(budget)?;
        if current != self.context || source.format() != self.format {
            return Err(Error::ContextMismatch);
        }
        let format = source.format();
        let bpp = format.bytes_per_pixel();
        let length = checked_region_byte_len(roi, format)?;
        let tiles = tile_count(roi)?;
        let source_tiles = source.tile_count() as u64;
        let tile_bytes = 65536u64.checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
        let pixels = (length / bpp) as u64;
        let work = (length as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(pixels.checked_mul(self.sample_work)?))
            .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(8)?))
            .and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        let source_lower =
            source_tiles.checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        if work > budget.work_units || source_lower > budget.resident_bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        if source.default_bytes().len() != bpp {
            return Err(RasterInputError::LengthMismatch.into());
        }
        let mut retained = source.default_bytes().len() as u64;
        for (_, tile) in source.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch.into());
            }
            retained = retained.checked_add(tile.bytes().len() as u64).and_then(|n| n.checked_add(256)).ok_or(RasterInputError::LimitExceeded)?;
        }
        source.try_validate_tile_origins_with(control)?;
        let maps = source_tiles.checked_mul(256).and_then(|n| n.checked_add(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
        // Every touched source-grid tile is staged once, including its unchanged border.
        // Reserve all retained replacements/map nodes up front; scratch is tile bounded.
        let staging = tiles.checked_mul(tile_bytes.checked_add(1024).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
        let base = retained
            .checked_add(self.retained)
            .and_then(|n| n.checked_add(maps))
            .and_then(|n| n.checked_add(staging))
            .ok_or(RasterInputError::LimitExceeded)?;
        if base > budget.resident_bytes {
            return Err(RasterInputError::LimitExceeded.into());
        }
        if roi.is_empty() {
            poll(control)?;
            let result = source.clone();
            poll(control)?;
            return Ok(result);
        }
        let cell_bytes = tile_bytes / 16;
        if base.checked_add(tile_bytes).and_then(|n| n.checked_add(cell_bytes)).is_none_or(|n| n > budget.resident_bytes) {
            return Err(RasterInputError::LimitExceeded.into());
        }
        let mut raw = scratch(tile_bytes, control)?;
        let mut cell = scratch(cell_bytes, control)?;
        if base.checked_add(raw.capacity() as u64).and_then(|n| n.checked_add(cell.capacity() as u64)).is_none_or(|n| n > budget.resident_bytes) {
            return Err(RasterInputError::LimitExceeded.into());
        }
        let mut prepaid = Prepaid { bytes: staging, work: budget.work_units };
        prepaid.work(source_tiles.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)?;
        poll(control)?;
        // One sparse-map COW clone; per-tile insertion never clones the whole map.
        // Existing std map/Arc allocations are bounded/prepaid, not fallible allocator proof.
        let mut result = source.clone();
        for coordinate in roi.tiles() {
            poll(control)?;
            let x0 = coordinate.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
            let y0 = coordinate.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
            let bounds = Rect::new(
                x0,
                y0,
                x0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
                y0.checked_add(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?,
            );
            checked_region_byte_len(bounds, format)?;
            prepaid.work(tile_bytes.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)?;
            source.try_read_interleaved_into_with(bounds, &mut raw, control)?;
            let part = bounds.intersect(&roi);
            for y in part.y0..part.y1 {
                poll(control)?;
                prepaid.work((part.width() as u64).checked_mul(self.sample_work).ok_or(RasterInputError::LimitExceeded)?)?;
                for x in part.x0..part.x1 {
                    let row = usize::try_from(i64::from(y) - i64::from(y0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let col = usize::try_from(i64::from(x) - i64::from(x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                    let start = row
                        .checked_mul(TILE_SIZE as usize)
                        .and_then(|n| n.checked_add(col))
                        .and_then(|n| n.checked_mul(bpp))
                        .ok_or(RasterInputError::LengthMismatch)?;
                    let native = raw.get_mut(start..start.checked_add(bpp).ok_or(RasterInputError::LengthMismatch)?).ok_or(RasterInputError::LengthMismatch)?;
                    poll(control)?;
                    for selected in self.channels {
                        poll(control)?;
                        let value = try_read_sample_f64(native, format.sample, selected.channel).map_err(|_| RasterInputError::InvalidSample)?;
                        validate_value(value, self.policy)?;
                        // Exact exponent identity deliberately avoids powf/native rewrite.
                        if selected.exponent == 1.0 {
                            continue;
                        }
                        let magnitude = value.abs().powf(selected.exponent);
                        poll(control)?;
                        if !magnitude.is_finite() {
                            return Err(Error::Overflow);
                        }
                        let mapped = if self.policy.negative == NegativePolicy::SignedMagnitude && value.is_sign_negative() { -magnitude } else { magnitude };
                        validate_value(mapped, self.policy)?;
                        try_write_sample_f64(native, format.sample, selected.channel, mapped).map_err(|_| Error::Overflow)?;
                        let encoded = try_read_sample_f64(native, format.sample, selected.channel).map_err(|_| RasterInputError::InvalidSample)?;
                        validate_value(encoded, self.policy)?;
                        poll(control)?;
                    }
                }
            }
            let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), bounds, &mut prepaid, control)?;
            for cy in (0..TILE_SIZE).step_by(64) {
                for cx in (0..TILE_SIZE).step_by(64) {
                    poll(control)?;
                    let row_bytes = 64usize.checked_mul(bpp).ok_or(RasterInputError::LimitExceeded)?;
                    for dy in 0..64usize {
                        poll(control)?;
                        let src = (cy as usize)
                            .checked_add(dy)
                            .and_then(|n| n.checked_mul(TILE_SIZE as usize))
                            .and_then(|n| n.checked_add(cx as usize))
                            .and_then(|n| n.checked_mul(bpp))
                            .ok_or(RasterInputError::LimitExceeded)?;
                        let dst = dy.checked_mul(row_bytes).ok_or(RasterInputError::LimitExceeded)?;
                        cell.get_mut(dst..dst.checked_add(row_bytes).ok_or(RasterInputError::LimitExceeded)?)
                            .ok_or(RasterInputError::LengthMismatch)?
                            .copy_from_slice(
                                raw.get(src..src.checked_add(row_bytes).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?,
                            );
                    }
                    builder.push_region(Rect::new(x0 + cx, y0 + cy, x0 + cx + 64, y0 + cy + 64), &cell, &mut prepaid, control)?;
                }
            }
            let (replacement, ()) = builder.finalize(&mut prepaid, control)?.into_parts();
            poll(control)?;
            result.put_tiles(replacement.tiles().map(|(coordinate, tile)| (*coordinate, Arc::clone(tile))));
        }
        poll(control)?;
        Ok(result)
    }
}
fn tile_count(rect: Rect) -> std::result::Result<u64, RasterInputError> {
    if rect.is_empty() {
        return Ok(0);
    }
    let last_x = rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let last_y = rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let span = |first: i32, last: i32| {
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)
    };
    span(rect.x0, last_x)?.checked_mul(span(rect.y0, last_y)?).ok_or(RasterInputError::LimitExceeded)
}

// Unit leases do not refund: aggregate retained output admission is owned by this call.
struct Prepaid {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Prepaid {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn scratch(bytes: u64, ctl: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(ctl)?;
    let len = usize::try_from(bytes).map_err(|_| RasterInputError::LimitExceeded)?;
    let mut output = Vec::new();
    output.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
    poll(ctl)?;
    if output.capacity() != len {
        return Err(RasterInputError::AllocationFailed.into());
    }
    while output.len() < len {
        poll(ctl)?;
        let next = output.len().checked_add(32768).ok_or(RasterInputError::LimitExceeded)?.min(len);
        output.resize(next, 0);
    }
    poll(ctl)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [4; 32] }
    }
    fn budget() -> Limits {
        Limits { resident_bytes: 128 << 20, work_units: MAX_WORK }
    }
    fn policy(negative: NegativePolicy, domain: Domain) -> Policy {
        Policy { negative, domain, overflow: OverflowPolicy::Reject }
    }
    #[test]
    fn exponent_identity_preserves_f64_low_bits_raw_alpha_and_boundary() {
        let format = PixelFormat::RGBA64F;
        let original: Vec<u8> =
            [1.0f64.to_bits() + 1, (-0.0f64).to_bits(), 4.0f64.to_bits(), 0x7ff8000000000012].into_iter().flat_map(u64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(format, &original).unwrap();
        let channels = vec![ChannelExponent { channel: 0, exponent: 1.0 }, ChannelExponent { channel: 1, exponent: 1.0 }];
        let gamma =
            PreparedGamma::prepare(format, context(), &channels, policy(NegativePolicy::Reject, Domain::FiniteHdr), budget(), &Interrupt::NONE).unwrap();
        let output = gamma.apply(&source, Rect::new(-1, -1, 1, 1), context(), budget(), &Interrupt::NONE).unwrap();
        let mut actual = [0u8; 32];
        output.try_read_pixel_bytes(-1, -1, &mut actual).unwrap();
        assert_eq!(actual.as_slice(), original);
        output.try_read_pixel_bytes(-2, -1, &mut actual).unwrap();
        assert_eq!(actual.as_slice(), original);
        assert_eq!(source.tile_count(), 0);
        assert_eq!(source.default_bytes(), output.default_bytes());
    }
    #[test]
    fn explicit_signed_hdr_and_unit_magnitude_policies() {
        for sample in [SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Rgb, sample, true);
            let mut original = vec![0; format.bytes_per_pixel()];
            for (channel, value) in [-4.0f64, -0.0, 0.25, 0.375].into_iter().enumerate() {
                try_write_sample_f64(&mut original, sample, channel, value).unwrap();
            }
            let source = Surface::try_with_default_bytes(format, &original).unwrap();
            let channels = vec![ChannelExponent { channel: 0, exponent: 0.5 }, ChannelExponent { channel: 1, exponent: 2.0 }];
            let gamma =
                PreparedGamma::prepare(format, context(), &channels, policy(NegativePolicy::SignedMagnitude, Domain::FiniteHdr), budget(), &Interrupt::NONE)
                    .unwrap();
            let output = gamma.apply(&source, Rect::new(0, 0, 1, 1), context(), budget(), &Interrupt::NONE).unwrap();
            let mut actual = vec![0; original.len()];
            output.try_read_pixel_bytes(0, 0, &mut actual).unwrap();
            let mut expected = original.clone();
            try_write_sample_f64(&mut expected, sample, 0, -2.0).unwrap();
            assert_eq!(actual, expected);
            for p in [policy(NegativePolicy::Reject, Domain::FiniteHdr), policy(NegativePolicy::SignedMagnitude, Domain::Unit)] {
                let candidate = PreparedGamma::prepare(format, context(), &channels, p, budget(), &Interrupt::NONE).unwrap();
                assert!(candidate.apply(&source, Rect::new(0, 0, 1, 1), context(), budget(), &Interrupt::NONE).is_err());
            }
        }
    }
    #[test]
    fn nonfinite_overflow_bad_exponent_and_context_refuse() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let channels = vec![ChannelExponent { channel: 0, exponent: 2.0 }];
        let p = policy(NegativePolicy::Reject, Domain::FiniteHdr);
        let gamma = PreparedGamma::prepare(format, context(), &channels, p, budget(), &Interrupt::NONE).unwrap();
        for value in [f64::NAN, f64::INFINITY, f64::MAX] {
            let source = Surface::try_with_default_bytes(format, &value.to_ne_bytes()).unwrap();
            assert!(gamma.apply(&source, Rect::new(0, 0, 1, 1), context(), budget(), &Interrupt::NONE).is_err());
            assert_eq!(source.default_bytes(), &value.to_ne_bytes());
            assert_eq!(source.tile_count(), 0);
        }
        for exponent in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let channels = vec![ChannelExponent { channel: 0, exponent }];
            assert!(PreparedGamma::prepare(format, context(), &channels, p, budget(), &Interrupt::NONE).is_err());
        }
        let source = Surface::try_with_default_bytes(format, &0.5f64.to_ne_bytes()).unwrap();
        let mut different = context();
        different.profile_identity[0] = 5;
        assert!(matches!(gamma.apply(&source, Rect::new(0, 0, 1, 1), different, budget(), &Interrupt::NONE), Err(Error::ContextMismatch)));
    }
    #[test]
    fn work_rejection_and_late_private_cancel_preserve_source() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::try_with_default_bytes(format, &0.5f64.to_ne_bytes()).unwrap();
        let channels = vec![ChannelExponent { channel: 0, exponent: 2.0 }];
        let gamma = PreparedGamma::prepare(format, context(), &channels, policy(NegativePolicy::Reject, Domain::Unit), budget(), &Interrupt::NONE).unwrap();
        assert!(gamma.apply(&source, Rect::new(0, 0, 1, 100000), context(), Limits { resident_bytes: 128 << 20, work_units: 1 }, &Interrupt::NONE).is_err());
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) > 400;
        assert!(matches!(
            gamma.apply(&source, Rect::new(0, 0, 64, 64), context(), budget(), &Interrupt::cancel_only(&cancelled)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.tile_count(), 0);
        assert_eq!(source.default_bytes(), &0.5f64.to_ne_bytes());
    }
}

#[cfg(test)]
mod requirements_tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    #[test]
    fn requirements_keep_inputs_charged_and_lower_peak_refuses() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let channels = vec![ChannelExponent { channel: 0, exponent: 2.0 }];
        let limits = Limits { resident_bytes: 64 << 20, work_units: MAX_WORK };
        let gamma = PreparedGamma::prepare(
            format,
            context,
            &channels,
            Policy { negative: NegativePolicy::Reject, domain: Domain::Unit, overflow: OverflowPolicy::Reject },
            limits,
            &Interrupt::NONE,
        )
        .unwrap();
        let source = Surface::try_with_default_bytes(format, &0.5f64.to_ne_bytes()).unwrap();
        let roi = Rect::new(0, 0, 1, 1);
        let req = gamma.requirements(&source, roi, context).unwrap();
        assert_eq!(req.parameter_retained_bytes, (channels.capacity() * std::mem::size_of::<ChannelExponent>()) as u64);
        assert_eq!(
            req.peak_resident_bytes,
            req.source_retained_bytes + req.parameter_retained_bytes + req.output_retained_bytes + req.output_index_bytes + req.scratch_bytes
        );
        assert_eq!(gamma.format(), format);
        assert_eq!(gamma.context(), context);
        assert!(
            gamma.apply(&source, roi, context, Limits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units }, &Interrupt::NONE).is_err()
        );
        let cancelled = || true;
        assert!(matches!(
            gamma.requirements_with_interrupt(&source, roi, context, &Interrupt::cancel_only(&cancelled)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.tile_count(), 0);
        assert_eq!(source.default_bytes(), &0.5f64.to_ne_bytes());
    }
}
