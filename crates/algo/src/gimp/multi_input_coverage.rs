//! Dimensionless coverage combination with explicit native outside values.
//! No selection/mask, CMS/transfer, source version or GEGL semantics inferred.
use photocraft_color::checked_arithmetic::{self as arithmetic, ArithmeticError, Destination, Domain, Operand, Policy, Rounding, Storage, UnitOverflow};
use photocraft_color::{ColorMode, PixelFormat, SampleType, sample_byte_range};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    UnionMax,
    IntersectionMin,
    SubtractSaturating,
    Multiply,
}
#[derive(Clone, Copy, Debug)]
pub enum Outside<'a> {
    Reject,
    NativeValue(&'a [u8]),
}
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub surface: &'a Surface,
    pub extent: Rect,
    pub outside: Outside<'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub operation: Operation,
    pub policy: Policy,
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
    FormatMismatch,
    ExtentMismatch,
    Outside,
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
        write!(f, "native coverage combination: {self:?}")
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
fn validate(format: PixelFormat, params: Params) -> Result<()> {
    if format.mode != ColorMode::Grayscale || format.alpha {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    // Coverage is always finite unit data. Saturation exists only in the named
    // subtract operation; encoding cannot silently introduce another clamp.
    if params.policy.domain != Domain::Unit || params.policy.unit_overflow != UnitOverflow::Reject {
        return Err(ArithmeticError::InvalidPolicy.into());
    }
    Ok(())
}
fn code(bytes: &[u8], sample: SampleType) -> Result<u64> {
    let bytes = bytes.get(sample_byte_range(sample, 0, bytes.len()).map_err(ArithmeticError::from)?).ok_or(RasterInputError::LengthMismatch)?;
    Ok(match sample {
        SampleType::U8 => u64::from(*bytes.first().ok_or(RasterInputError::LengthMismatch)?),
        SampleType::U16 => u64::from(u16::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        SampleType::U32 => u64::from(u32::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        _ => return Err(RasterInputError::UnsupportedConversion.into()),
    })
}
fn write_code(bytes: &mut [u8], sample: SampleType, value: u64) -> Result<()> {
    match sample {
        SampleType::U8 => *bytes.first_mut().ok_or(RasterInputError::LengthMismatch)? = u8::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?,
        SampleType::U16 => bytes.copy_from_slice(&u16::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?.to_ne_bytes()),
        SampleType::U32 => bytes.copy_from_slice(&u32::try_from(value).map_err(|_| ArithmeticError::OutsideDomain)?.to_ne_bytes()),
        _ => return Err(RasterInputError::UnsupportedConversion.into()),
    }
    Ok(())
}
fn finite(bytes: &[u8], sample: SampleType) -> Result<f64> {
    Ok(arithmetic::decode_finite(Operand { bytes, storage: Storage::Native(sample), index: 0 }, Domain::Unit)?)
}
/// Atomic one-sample operation. Max/min retain the winning native word; equal
/// numeric values choose the first word, including signed floating zero.
/// Integer multiply is colorCode*operandCode/nativeMAX in checked u64, rounded
/// Exact or nearest positive-half-up. Floats use f64 arithmetic then the selected
/// native encoding; inputs/results are finite unit. Subtract alone saturates at0.
pub fn evaluate_sample_into(format: PixelFormat, first: &[u8], operand: &[u8], output: &mut [u8], params: Params) -> Result<()> {
    validate(format, params)?;
    let bpp = format.bytes_per_pixel();
    if first.len() != bpp || operand.len() != bpp || output.len() != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    let mut staged = [0u8; 8];
    let destination = staged.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    if let Some(max) = integer_max(format.sample) {
        let a = code(first, format.sample)?;
        let b = code(operand, format.sample)?;
        match params.operation {
            Operation::UnionMax => destination.copy_from_slice(if a >= b { first } else { operand }),
            Operation::IntersectionMin => destination.copy_from_slice(if a <= b { first } else { operand }),
            Operation::SubtractSaturating => {
                if b == 0 {
                    destination.copy_from_slice(first);
                } else {
                    write_code(destination, format.sample, a.saturating_sub(b))?;
                }
            }
            Operation::Multiply => {
                if b == max {
                    destination.copy_from_slice(first);
                } else if a == max {
                    destination.copy_from_slice(operand);
                } else {
                    let numerator = a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)?;
                    let whole = numerator / max;
                    let remainder = numerator % max;
                    let result = match params.policy.rounding {
                        Rounding::Exact => {
                            if remainder != 0 {
                                return Err(ArithmeticError::PrecisionLoss.into());
                            }
                            whole
                        }
                        Rounding::Nearest => whole
                            .checked_add(u64::from(remainder.checked_mul(2).ok_or(RasterInputError::LimitExceeded)? >= max))
                            .ok_or(RasterInputError::LimitExceeded)?,
                    };
                    write_code(destination, format.sample, result)?;
                }
            }
        }
    } else {
        let a = finite(first, format.sample)?;
        let b = finite(operand, format.sample)?;
        match params.operation {
            Operation::UnionMax => destination.copy_from_slice(if a >= b { first } else { operand }),
            Operation::IntersectionMin => destination.copy_from_slice(if a <= b { first } else { operand }),
            Operation::SubtractSaturating => {
                let value = if a < b { 0.0 } else { a - b };
                arithmetic::encode_finite(value, Destination { bytes: destination, storage: Storage::Native(format.sample), index: 0 }, params.policy)?;
            }
            Operation::Multiply => {
                if b == 1.0 {
                    destination.copy_from_slice(first);
                } else if a == 1.0 {
                    destination.copy_from_slice(operand);
                } else {
                    arithmetic::encode_finite(a * b, Destination { bytes: destination, storage: Storage::Native(format.sample), index: 0 }, params.policy)?;
                }
            }
        }
    }
    output.copy_from_slice(destination);
    Ok(())
}
fn outside(input: Input<'_>, format: PixelFormat, params: Params) -> Result<()> {
    if let Outside::NativeValue(bytes) = input.outside {
        if bytes.len() != format.bytes_per_pixel() {
            return Err(RasterInputError::LengthMismatch.into());
        }
        if integer_max(format.sample).is_some() {
            code(bytes, format.sample)?;
        } else {
            finite(bytes, format.sample)?;
        }
        validate(format, params)?;
    }
    Ok(())
}
fn read(input: Input<'_>, x: i32, y: i32, pixel: &mut [u8], control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if x >= input.extent.x0 && x < input.extent.x1 && y >= input.extent.y0 && y < input.extent.y1 {
        input.surface.try_read_pixel_bytes(x, y, pixel)?;
    } else {
        match input.outside {
            Outside::Reject => return Err(Error::Outside),
            Outside::NativeValue(value) => {
                if value.len() != pixel.len() {
                    return Err(RasterInputError::LengthMismatch.into());
                }
                pixel.copy_from_slice(value);
            }
        }
    }
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
        bytes.resize(bytes.len().saturating_add(8192).min(length), 0);
    }
    Ok(bytes)
}
/// Two matching native Gray/no-alpha surfaces on matching declared extents.
/// The ROI may extend outside only under explicitly supplied one-sample native
/// values. Defaults and pixels outside ROI are raw-retained from the first input.
/// Known payload/map/scratch capacities and work are prepaid; not global RSS.
pub fn try_apply(first: Input<'_>, operand: Input<'_>, roi: Rect, params: Params, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let format = first.surface.format();
    validate(format, params)?;
    if operand.surface.format() != format {
        return Err(Error::FormatMismatch);
    }
    if first.extent != operand.extent {
        return Err(Error::ExtentMismatch);
    }
    checked_region_byte_len(first.extent, format)?;
    checked_region_byte_len(roi, format)?;
    outside(first, format, params)?;
    outside(operand, format, params)?;
    if roi.is_empty() {
        return Ok(first.surface.clone());
    }
    if (matches!(first.outside, Outside::Reject) || matches!(operand.outside, Outside::Reject)) && roi.intersect(&first.extent) != roi {
        return Err(Error::Outside);
    }
    let tiles = tile_count(roi)?;
    let retained = add(first.surface.tile_count() as u64, operand.surface.tile_count() as u64)?;
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
    let work = add(mul(tiles, mul(payload, 192)?)?, add(mul(retained, 32)?, mul(tiles, mul(add(retained, tiles)?, 512)?)?)?)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let mut resident = 0u64;
    for source in [first.surface, operand.surface] {
        resident = add(resident, source.default_bytes().len() as u64)?;
        for (coordinate, tile) in source.tiles() {
            poll(control)?;
            coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
            coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
            if tile.bytes().len() as u64 != payload {
                return Err(RasterInputError::LengthMismatch.into());
            }
            resident = add(resident, tile.bytes().len() as u64)?;
        }
    }
    if add(resident, extra)? > cap {
        return Err(RasterInputError::LimitExceeded.into());
    }
    poll(control)?;
    let mut output = first.surface.clone();
    let mut meter = Meter { work: limits.work_units, used: mul(retained, 32)?, staging };
    for coordinate in roi.tiles() {
        poll(control)?;
        let tile = coordinate.rect();
        let part = tile.intersect(&roi);
        let mut raw = allocate(usize::try_from(payload).map_err(|_| RasterInputError::LimitExceeded)?, control)?;
        first.surface.try_read_interleaved_into_with(tile, &mut raw, control)?;
        let mut changed = false;
        for y in part.y0..part.y1 {
            poll(control)?;
            let row = usize::try_from(i64::from(y) - i64::from(tile.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
            for x in part.x0..part.x1 {
                poll(control)?;
                meter.debit(128)?;
                let column = usize::try_from(i64::from(x) - i64::from(tile.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
                let start =
                    row.checked_mul(side).and_then(|v| v.checked_add(column)).and_then(|v| v.checked_mul(bpp)).ok_or(RasterInputError::LimitExceeded)?;
                let destination = raw.get_mut(start..start.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
                let mut a = [0u8; 8];
                let mut b = [0u8; 8];
                let mut result = [0u8; 8];
                let a = a.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                let b = b.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                let result = result.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                read(first, x, y, a, control)?;
                read(operand, x, y, b, control)?;
                evaluate_sample_into(format, a, b, result, params)?;
                changed |= destination != result;
                destination.copy_from_slice(result);
            }
        }
        if !changed {
            continue;
        }
        let mut builder = StagedSurfaceBuilder::new(format, first.surface.default_bytes(), tile, &mut meter, control)?;
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
    fn params(operation: Operation) -> Params {
        Params { operation, policy: Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest } }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    fn input(surface: &Surface, extent: Rect) -> Input<'_> {
        Input { surface, extent, outside: Outside::Reject }
    }
    #[test]
    fn u32_low_codes_winning_words_subtraction_and_exact_multiply() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let a = 16_777_217u32.to_ne_bytes();
        let b = 16_777_216u32.to_ne_bytes();
        let mut output = [0x5a; 4];
        evaluate_sample_into(format, &a, &b, &mut output, params(Operation::UnionMax)).unwrap();
        assert_eq!(output, a);
        evaluate_sample_into(format, &a, &b, &mut output, params(Operation::IntersectionMin)).unwrap();
        assert_eq!(output, b);
        evaluate_sample_into(format, &a, &b, &mut output, params(Operation::SubtractSaturating)).unwrap();
        assert_eq!(output, 1u32.to_ne_bytes());
        let low = 1u32.to_ne_bytes();
        let max = u32::MAX.to_ne_bytes();
        evaluate_sample_into(format, &a, &max, &mut output, params(Operation::Multiply)).unwrap();
        assert_eq!(output, a);
        let mut exact = params(Operation::Multiply);
        exact.policy.rounding = Rounding::Exact;
        output.fill(0x5a);
        assert!(matches!(evaluate_sample_into(format, &a, &low, &mut output, exact), Err(Error::Arithmetic(ArithmeticError::PrecisionLoss))));
        assert_eq!(output, [0x5a; 4]);
        evaluate_sample_into(format, &a, &b, &mut output, params(Operation::Multiply)).unwrap();
        let product = u64::from(16_777_217u32) * u64::from(16_777_216u32);
        let expected = (product + u64::from(u32::MAX) / 2) / u64::from(u32::MAX);
        assert_eq!(code(&output, SampleType::U32).unwrap(), expected);
    }
    #[test]
    fn all_native_depths_support_explicit_coverage_operations() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut a = vec![0; format.bytes_per_pixel()];
            let mut b = a.clone();
            if let Some(max) = integer_max(sample) {
                write_code(&mut a, sample, max).unwrap();
                write_code(&mut b, sample, max / 3).unwrap();
            } else {
                arithmetic::encode_finite(1.0, Destination { bytes: &mut a, storage: Storage::Native(sample), index: 0 }, params(Operation::UnionMax).policy)
                    .unwrap();
                arithmetic::encode_finite(0.25, Destination { bytes: &mut b, storage: Storage::Native(sample), index: 0 }, params(Operation::UnionMax).policy)
                    .unwrap();
            }
            let mut output = a.clone();
            evaluate_sample_into(format, &a, &b, &mut output, params(Operation::UnionMax)).unwrap();
            assert_eq!(output, a);
            evaluate_sample_into(format, &a, &b, &mut output, params(Operation::IntersectionMin)).unwrap();
            assert_eq!(output, b);
            evaluate_sample_into(format, &a, &b, &mut output, params(Operation::Multiply)).unwrap();
            assert_eq!(output, b);
            evaluate_sample_into(format, &b, &a, &mut output, params(Operation::SubtractSaturating)).unwrap();
            assert!(output.iter().all(|byte| *byte == 0));
        }
    }
    #[test]
    fn floating_zero_ties_native_precision_and_invalid_inputs_are_atomic() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let negative = (-0.0f64).to_ne_bytes();
        let positive = 0.0f64.to_ne_bytes();
        let mut output = [0x5a; 8];
        for operation in [Operation::UnionMax, Operation::IntersectionMin] {
            evaluate_sample_into(format, &negative, &positive, &mut output, params(operation)).unwrap();
            assert_eq!(output, negative);
        }
        evaluate_sample_into(format, &negative, &negative, &mut output, params(Operation::Multiply)).unwrap();
        assert_eq!(output, positive);
        let near = f64::from_bits(0.5f64.to_bits() + 1).to_ne_bytes();
        evaluate_sample_into(format, &near, &0.5f64.to_ne_bytes(), &mut output, params(Operation::UnionMax)).unwrap();
        assert_eq!(output, near);
        for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
            output.fill(0x5a);
            assert!(evaluate_sample_into(format, &value.to_ne_bytes(), &positive, &mut output, params(Operation::UnionMax)).is_err());
            assert_eq!(output, [0x5a; 8]);
        }
    }
    #[test]
    fn tiled_union_preserves_first_outside_and_defaults_across_negative_grid() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let extent = Rect::new(-1, 0, 257, 1);
        let mut first = Surface::try_with_default_bytes(format, &123u32.to_ne_bytes()).unwrap();
        let mut second = Surface::try_with_default_bytes(format, &124u32.to_ne_bytes()).unwrap();
        first.try_write_interleaved(Rect::new(-1, 0, 0, 1), &16_777_217u32.to_ne_bytes()).unwrap();
        second.try_write_interleaved(Rect::new(256, 0, 257, 1), &u32::MAX.to_ne_bytes()).unwrap();
        let before = first.clone();
        let operand_before = second.clone();
        let output =
            try_apply(input(&first, extent), input(&second, extent), Rect::new(0, 0, 257, 1), params(Operation::UnionMax), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), 16_777_217u32.to_ne_bytes());
        assert_eq!(output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), 124u32.to_ne_bytes());
        assert_eq!(output.try_read_interleaved(Rect::new(256, 0, 257, 1)).unwrap(), u32::MAX.to_ne_bytes());
        assert_eq!(output.default_bytes(), 123u32.to_ne_bytes());
        assert_eq!(first, before);
        assert_eq!(second, operand_before);
    }
    #[test]
    fn outside_native_values_mismatches_limits_and_cancellation_are_explicit() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false);
        let first = Surface::try_with_default_bytes(format, &1u16.to_ne_bytes()).unwrap();
        let second = Surface::try_with_default_bytes(format, &2u16.to_ne_bytes()).unwrap();
        let extent = Rect::new(0, 0, 1, 1);
        let roi = Rect::new(-1, 0, 1, 1);
        let p = params(Operation::UnionMax);
        assert!(matches!(try_apply(input(&first, extent), input(&second, extent), roi, p, limits(), &Interrupt::NONE), Err(Error::Outside)));
        let outside_a = 4u16.to_ne_bytes();
        let outside_b = 5u16.to_ne_bytes();
        let a = Input { surface: &first, extent, outside: Outside::NativeValue(&outside_a) };
        let b = Input { surface: &second, extent, outside: Outside::NativeValue(&outside_b) };
        let output = try_apply(a, b, roi, p, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), outside_b);
        assert!(matches!(try_apply(a, Input { extent: Rect::new(0, 0, 2, 1), ..b }, roi, p, limits(), &Interrupt::NONE), Err(Error::ExtentMismatch)));
        let wrong = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        assert!(matches!(try_apply(a, input(&wrong, extent), roi, p, limits(), &Interrupt::NONE), Err(Error::FormatMismatch)));
        assert!(matches!(
            try_apply(a, b, roi, p, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(Error::Raster(RasterInputError::LimitExceeded))
        ));
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 30;
        assert!(matches!(try_apply(a, b, roi, p, limits(), &Interrupt::cancel_only(&cancel)), Err(Error::Raster(RasterInputError::Cancelled))));
    }
}
