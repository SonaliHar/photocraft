//! Standalone native alpha association with explicit arithmetic policy.
//! No CMS/TRC conversion, premultiplied-source inference or GEGL admission.
use super::wide_point::WorkingContext;
use photocraft_color::checked_arithmetic::{self as arithmetic, ArithmeticError, Destination, Domain, Operand, Policy, Rounding, Storage, UnitOverflow};
use photocraft_color::{ColorMode, PixelFormat, SampleType, sample_byte_range};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Association {
    Premultiply,
    Unpremultiply,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaZeroPolicy {
    Reject,
    ZeroColors,
    PreserveRaw,
}
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub association: Association,
    pub policy: Policy,
    pub alpha_zero: AlphaZeroPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Arithmetic(ArithmeticError),
    ContextMismatch,
    AlphaRange,
    AlphaZero,
}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl From<ArithmeticError> for Error {
    fn from(error: ArithmeticError) -> Self {
        Self::Arithmetic(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native alpha association: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| RasterInputError::LimitExceeded.into())
}
fn integer_max(sample: SampleType) -> Option<u64> {
    match sample {
        SampleType::U8 => Some(u64::from(u8::MAX)),
        SampleType::U16 => Some(u64::from(u16::MAX)),
        SampleType::U32 => Some(u64::from(u32::MAX)),
        _ => None,
    }
}
fn validate(format: PixelFormat, source: WorkingContext, expected: WorkingContext, params: Params) -> Result<()> {
    if source != expected {
        return Err(Error::ContextMismatch);
    }
    if !format.alpha || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    if (integer_max(format.sample).is_some() && params.policy.domain != Domain::Unit)
        || (params.policy.domain == Domain::FloatHdr && params.policy.unit_overflow != UnitOverflow::Reject)
    {
        return Err(ArithmeticError::InvalidPolicy.into());
    }
    Ok(())
}
fn code(input: &[u8], sample: SampleType, index: usize) -> Result<u64> {
    let bytes = input.get(sample_byte_range(sample, index, input.len()).map_err(ArithmeticError::from)?).ok_or(RasterInputError::LengthMismatch)?;
    Ok(match sample {
        SampleType::U8 => u64::from(*bytes.first().ok_or(RasterInputError::LengthMismatch)?),
        SampleType::U16 => u64::from(u16::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        SampleType::U32 => u64::from(u32::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        _ => return Err(RasterInputError::UnsupportedConversion.into()),
    })
}
fn write_code(output: &mut [u8], sample: SampleType, index: usize, value: u64) -> Result<()> {
    let range = sample_byte_range(sample, index, output.len()).map_err(ArithmeticError::from)?;
    let bytes = output.get_mut(range).ok_or(RasterInputError::LengthMismatch)?;
    match sample {
        SampleType::U8 => {
            let value = u8::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?;
            *bytes.first_mut().ok_or(RasterInputError::LengthMismatch)? = value;
        }
        SampleType::U16 => bytes.copy_from_slice(&u16::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?.to_ne_bytes()),
        SampleType::U32 => bytes.copy_from_slice(&u32::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?.to_ne_bytes()),
        _ => return Err(RasterInputError::UnsupportedConversion.into()),
    }
    Ok(())
}
fn rational(color: u64, alpha: u64, max: u64, params: Params) -> Result<u64> {
    let (numerator, denominator) = match params.association {
        Association::Premultiply => (color.checked_mul(alpha), max),
        Association::Unpremultiply => (color.checked_mul(max), alpha),
    };
    let numerator = numerator.ok_or(RasterInputError::LimitExceeded)?;
    if denominator == 0 {
        return Err(Error::AlphaZero);
    }
    let max_numerator = max.checked_mul(denominator).ok_or(RasterInputError::LimitExceeded)?;
    if numerator > max_numerator {
        return match params.policy.unit_overflow {
            UnitOverflow::Reject => Err(ArithmeticError::OutsideDomain.into()),
            UnitOverflow::Clamp => Ok(max),
        };
    }
    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    match params.policy.rounding {
        Rounding::Exact => {
            if remainder != 0 {
                return Err(ArithmeticError::PrecisionLoss.into());
            }
            Ok(whole)
        }
        Rounding::Nearest => {
            let increment = u64::from(remainder.checked_mul(2).ok_or(RasterInputError::LimitExceeded)? >= denominator);
            whole.checked_add(increment).ok_or_else(|| RasterInputError::LimitExceeded.into())
        }
    }
}
/// Allocation-free one-pixel transaction. Integer colors use exact checked u64
/// rationals with positive-half-up Nearest or remainder-zero Exact. Floats use
/// one f64 multiply/divide, then the selected native quantization policy.
/// Alpha is validated as finite unit and always copied raw, including -0.
/// At alpha=1 or nonzero alpha with zero color, provable raw identity is kept.
/// Zero-alpha policy applies to both operations after color-domain validation.
pub fn evaluate_pixel_into(
    format: PixelFormat,
    input: &[u8],
    output: &mut [u8],
    source_context: WorkingContext,
    expected_context: WorkingContext,
    params: Params,
) -> Result<()> {
    validate(format, source_context, expected_context, params)?;
    let bpp = format.bytes_per_pixel();
    if input.len() != bpp || output.len() != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    let colors = format.mode.color_channels();
    let alpha_index = colors;
    let mut staging = [0u8; 32];
    let encoded = staging.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    encoded.copy_from_slice(input);
    if let Some(max) = integer_max(format.sample) {
        let alpha = code(input, format.sample, alpha_index)?;
        if alpha == 0 && params.alpha_zero == AlphaZeroPolicy::Reject {
            return Err(Error::AlphaZero);
        }
        for channel in 0..colors {
            let color = code(input, format.sample, channel)?;
            if alpha == 0 {
                match params.alpha_zero {
                    AlphaZeroPolicy::PreserveRaw => continue,
                    AlphaZeroPolicy::ZeroColors => {
                        write_code(encoded, format.sample, channel, 0)?;
                        continue;
                    }
                    AlphaZeroPolicy::Reject => return Err(Error::AlphaZero),
                }
            }
            if alpha == max || color == 0 {
                continue;
            }
            write_code(encoded, format.sample, channel, rational(color, alpha, max, params)?)?;
        }
    } else {
        let alpha = arithmetic::decode_finite(Operand { bytes: input, storage: Storage::Native(format.sample), index: alpha_index }, Domain::Unit).map_err(
            |error| match error {
                ArithmeticError::OutsideDomain => Error::AlphaRange,
                other => Error::Arithmetic(other),
            },
        )?;
        if alpha == 0.0 && params.alpha_zero == AlphaZeroPolicy::Reject {
            return Err(Error::AlphaZero);
        }
        for channel in 0..colors {
            let value = arithmetic::decode_finite(Operand { bytes: input, storage: Storage::Native(format.sample), index: channel }, params.policy.domain)?;
            if alpha == 0.0 {
                match params.alpha_zero {
                    AlphaZeroPolicy::PreserveRaw => continue,
                    AlphaZeroPolicy::ZeroColors => {
                        arithmetic::encode_finite(0.0, Destination { bytes: encoded, storage: Storage::Native(format.sample), index: channel }, params.policy)?;
                        continue;
                    }
                    AlphaZeroPolicy::Reject => return Err(Error::AlphaZero),
                }
            }
            if alpha == 1.0 || value == 0.0 {
                continue;
            }
            let associated = match params.association {
                Association::Premultiply => value * alpha,
                Association::Unpremultiply => value / alpha,
            };
            arithmetic::encode_finite(associated, Destination { bytes: encoded, storage: Storage::Native(format.sample), index: channel }, params.policy)?;
        }
    }
    output.copy_from_slice(encoded);
    Ok(())
}
fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
struct Meter {
    work: u64,
    used: u64,
    staging: u64,
}
impl Meter {
    fn debit(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.used = self.used.checked_add(units).filter(|v| *v <= self.work).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
impl StagingBudget for Meter {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        if bytes > self.staging { Err(RasterInputError::LimitExceeded) } else { Ok(()) }
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.debit(units)
    }
}
fn allocate(length: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    if bytes.capacity() != length {
        return Err(RasterInputError::AllocationFailed.into());
    }
    while bytes.len() < length {
        poll(control)?;
        let end = bytes.len().saturating_add(8192).min(length);
        bytes.resize(end, 0);
    }
    Ok(bytes)
}
/// Private tile transaction. Untouched pixels/defaults/raw alpha are preserved.
/// Known source/output payload, scratch, map/Arc envelopes and work are bounded;
/// stable standard-library map allocation is prepaid, not a process RSS promise.
#[allow(clippy::too_many_arguments)]
pub fn try_apply(
    source: &Surface,
    roi: Rect,
    source_context: WorkingContext,
    expected_context: WorkingContext,
    params: Params,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    poll(control)?;
    let format = source.format();
    validate(format, source_context, expected_context, params)?;
    checked_region_byte_len(roi, format)?;
    if roi.is_empty() {
        return Ok(source.clone());
    }
    let tiles = tile_count(roi)?;
    let retained = source.tile_count() as u64;
    let bpp = format.bytes_per_pixel();
    let side = usize::try_from(TILE_SIZE).map_err(|_| RasterInputError::LimitExceeded)?;
    let payload = mul(mul(side as u64, side as u64)?, bpp as u64)?;
    let staging = add(mul(payload, 4)?, 8192)?;
    let maps = mul(add(retained, tiles)?, 1024)?;
    let extra = add(add(mul(tiles, add(payload, 512)?)?, maps)?, add(staging, add(payload, mul(64 * 64, bpp as u64)?)?)?)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if cap == 0 || extra > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let work = add(mul(tiles, mul(payload, 128)?)?, add(mul(retained, 32)?, mul(tiles, mul(add(retained, tiles)?, 512)?)?)?)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut resident = source.default_bytes().len() as u64;
    for (coordinate, tile) in source.tiles() {
        poll(control)?;
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch.into());
        }
        resident = add(resident, tile.bytes().len() as u64)?;
    }
    if add(resident, extra)? > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    poll(control)?;
    let mut output = source.clone();
    let mut meter = Meter { work: limits.work_units, used: mul(retained, 32)?, staging };
    for coordinate in roi.tiles() {
        poll(control)?;
        let tile = coordinate.rect();
        let part = tile.intersect(&roi);
        let mut raw = allocate(usize::try_from(payload).map_err(|_| RasterInputError::LimitExceeded)?, control)?;
        source.try_read_interleaved_into_with(tile, &mut raw, control)?;
        let mut changed = false;
        for y in part.y0..part.y1 {
            poll(control)?;
            let row = usize::try_from(i64::from(y) - i64::from(tile.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
            for x in part.x0..part.x1 {
                poll(control)?;
                meter.debit((format.channels() * 64) as u64)?;
                let column = usize::try_from(i64::from(x) - i64::from(tile.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                let start =
                    row.checked_mul(side).and_then(|v| v.checked_add(column)).and_then(|v| v.checked_mul(bpp)).ok_or(RasterInputError::LimitExceeded)?;
                let pixel = raw.get_mut(start..start.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
                let mut input = [0u8; 32];
                let input = input.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                input.copy_from_slice(pixel);
                evaluate_pixel_into(format, input, pixel, source_context, expected_context, params)?;
                changed |= pixel != input;
            }
        }
        if !changed {
            continue;
        }
        let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), tile, &mut meter, control)?;
        let mut cell = allocate(64 * 64 * bpp, control)?;
        for cy in 0..4 {
            for cx in 0..4 {
                poll(control)?;
                for row in 0..64 {
                    let start = ((cy * 64 + row) * side + cx * 64) * bpp;
                    let target = row * 64 * bpp;
                    let span = 64 * bpp;
                    cell.get_mut(target..target + span)
                        .ok_or(RasterInputError::LengthMismatch)?
                        .copy_from_slice(raw.get(start..start + span).ok_or(RasterInputError::LengthMismatch)?);
                }
                let x = tile.x0.checked_add(i32::try_from(cx * 64).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                let y = tile.y0.checked_add(i32::try_from(cy * 64).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
                builder.push_region(
                    Rect::new(x, y, x.checked_add(64).ok_or(RasterInputError::InvalidBounds)?, y.checked_add(64).ok_or(RasterInputError::InvalidBounds)?),
                    &cell,
                    &mut meter,
                    control,
                )?;
            }
        }
        let (replacement, ()) = builder.finalize(&mut meter, control)?.into_parts();
        poll(control)?;
        meter.debit(mul(add(retained, tiles)?, 512)?)?;
        output.put_tiles(replacement.tiles().map(|(coordinate, tile)| (*coordinate, tile.clone())));
    }
    poll(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [9; 32] }
    }
    fn params(association: Association, rounding: Rounding, overflow: UnitOverflow, zero: AlphaZeroPolicy, domain: Domain) -> Params {
        Params { association, policy: Policy { domain, unit_overflow: overflow, rounding }, alpha_zero: zero }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    #[test]
    fn u32_halfway_uses_exact_rational_and_exact_refuses_without_publication() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, true);
        let mut input = Vec::new();
        input.extend_from_slice(&1u32.to_ne_bytes());
        input.extend_from_slice(&2u32.to_ne_bytes());
        let mut output = [0x5a; 8];
        let nearest = params(Association::Unpremultiply, Rounding::Nearest, UnitOverflow::Reject, AlphaZeroPolicy::Reject, Domain::Unit);
        evaluate_pixel_into(format, &input, &mut output, context(), context(), nearest).unwrap();
        assert_eq!(code(&output, SampleType::U32, 0).unwrap(), 2_147_483_648);
        assert_eq!(output.get(4..).unwrap(), input.get(4..).unwrap());
        output.fill(0x5a);
        assert!(matches!(
            evaluate_pixel_into(
                format,
                &input,
                &mut output,
                context(),
                context(),
                Params { policy: Policy { rounding: Rounding::Exact, ..nearest.policy }, ..nearest }
            ),
            Err(Error::Arithmetic(ArithmeticError::PrecisionLoss))
        ));
        assert_eq!(output, [0x5a; 8]);
        let mut high = Vec::new();
        high.extend_from_slice(&(u32::MAX - 1).to_ne_bytes());
        high.extend_from_slice(&u32::MAX.to_ne_bytes());
        evaluate_pixel_into(format, &high, &mut output, context(), context(), nearest).unwrap();
        assert_eq!(code(&output, SampleType::U32, 0).unwrap(), u64::from(u32::MAX - 1));
    }
    #[test]
    fn integer_native_codes_roundtrip_and_unit_overflow_is_explicit() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, true);
            let max = integer_max(sample).unwrap();
            let mut input = vec![0; format.bytes_per_pixel()];
            write_code(&mut input, sample, 0, max).unwrap();
            write_code(&mut input, sample, 1, 1).unwrap();
            let reject = params(Association::Unpremultiply, Rounding::Nearest, UnitOverflow::Reject, AlphaZeroPolicy::Reject, Domain::Unit);
            let mut output = vec![0x5a; input.len()];
            let untouched = output.clone();
            assert!(matches!(
                evaluate_pixel_into(format, &input, &mut output, context(), context(), reject),
                Err(Error::Arithmetic(ArithmeticError::OutsideDomain))
            ));
            assert_eq!(output, untouched);
            let clamp = Params { policy: Policy { unit_overflow: UnitOverflow::Clamp, ..reject.policy }, ..reject };
            evaluate_pixel_into(format, &input, &mut output, context(), context(), clamp).unwrap();
            assert_eq!(code(&output, sample, 0).unwrap(), max);
            assert_eq!(code(&output, sample, 1).unwrap(), 1);
            let multiply = Params { association: Association::Premultiply, ..reject };
            evaluate_pixel_into(format, &input, &mut output, context(), context(), multiply).unwrap();
            assert_eq!(code(&output, sample, 0).unwrap(), 1);
        }
    }
    #[test]
    fn hdr_double_signed_zero_and_alpha_zero_policies_keep_native_words() {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let words = [6.0f64.to_bits(), (-0.0f64).to_bits(), (-3.0f64).to_bits(), 0.5f64.to_bits()];
        let input: Vec<u8> = words.into_iter().flat_map(u64::to_ne_bytes).collect();
        let mut output = [0u8; 32];
        let p = params(Association::Unpremultiply, Rounding::Exact, UnitOverflow::Reject, AlphaZeroPolicy::Reject, Domain::FloatHdr);
        evaluate_pixel_into(format, &input, &mut output, context(), context(), p).unwrap();
        assert_eq!(photocraft_color::try_read_sample_f64(&output, SampleType::F64, 0).unwrap(), 12.0);
        assert_eq!(output.get(8..16).unwrap(), input.get(8..16).unwrap());
        assert_eq!(output.get(24..).unwrap(), input.get(24..).unwrap());
        let mut zero = input.clone();
        zero.get_mut(24..32).unwrap().copy_from_slice(&(-0.0f64).to_ne_bytes());
        output.fill(0x5a);
        assert!(matches!(evaluate_pixel_into(format, &zero, &mut output, context(), context(), p), Err(Error::AlphaZero)));
        assert_eq!(output, [0x5a; 32]);
        evaluate_pixel_into(format, &zero, &mut output, context(), context(), Params { alpha_zero: AlphaZeroPolicy::PreserveRaw, ..p }).unwrap();
        assert_eq!(&output[..], zero);
        evaluate_pixel_into(format, &zero, &mut output, context(), context(), Params { alpha_zero: AlphaZeroPolicy::ZeroColors, ..p }).unwrap();
        assert!(output.get(..24).unwrap().iter().all(|byte| *byte == 0));
        assert_eq!(output.get(24..).unwrap(), zero.get(24..).unwrap());
    }
    #[test]
    fn floating_narrowing_refuses_and_tile_identity_limits_cancel_are_private() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, true);
        let p = params(Association::Unpremultiply, Rounding::Nearest, UnitOverflow::Reject, AlphaZeroPolicy::Reject, Domain::FloatHdr);
        let mut input = Vec::new();
        input.extend_from_slice(&f32::MAX.to_ne_bytes());
        input.extend_from_slice(&0.5f32.to_ne_bytes());
        let mut output = [0x5a; 8];
        assert!(evaluate_pixel_into(format, &input, &mut output, context(), context(), p).is_err());
        assert_eq!(output, [0x5a; 8]);
        let source = Surface::with_default(format, &[-0.0, 1.0]);
        let roi = Rect::new(-1, 0, 257, 1);
        assert_eq!(try_apply(&source, roi, context(), context(), p, limits(), &Interrupt::NONE).unwrap(), source);
        assert!(matches!(
            try_apply(&source, roi, context(), context(), p, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
        let count = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 100;
        assert!(matches!(
            try_apply(&source, roi, context(), context(), p, limits(), &Interrupt::cancel_only(&cancel)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source.default_bytes(), [-0.0f32, 1.0].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>());
    }
    #[test]
    fn nonidentity_tile_changes_only_roi_color_and_keeps_alpha_default_outside() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true);
        let mut default = vec![0; 4];
        write_code(&mut default, SampleType::U16, 1, u64::from(u16::MAX)).unwrap();
        let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
        let mut input = vec![0; 8];
        write_code(&mut input, SampleType::U16, 0, 40000).unwrap();
        write_code(&mut input, SampleType::U16, 1, 32768).unwrap();
        write_code(&mut input, SampleType::U16, 2, 55000).unwrap();
        write_code(&mut input, SampleType::U16, 3, 65535).unwrap();
        let region = Rect::new(0, 0, 2, 1);
        source.try_write_interleaved(region, &input).unwrap();
        let original = source.clone();
        let p = params(Association::Premultiply, Rounding::Nearest, UnitOverflow::Reject, AlphaZeroPolicy::Reject, Domain::Unit);
        let output = try_apply(&source, Rect::new(0, 0, 1, 1), context(), context(), p, limits(), &Interrupt::NONE).unwrap();
        let actual = output.try_read_interleaved(region).unwrap();
        assert_eq!(code(&actual, SampleType::U16, 0).unwrap(), (40000u64 * 32768 + 32767) / 65535);
        assert_eq!(actual.get(2..).unwrap(), input.get(2..).unwrap());
        assert_eq!(output.default_bytes(), default);
        assert_eq!(source, original);
    }
}
