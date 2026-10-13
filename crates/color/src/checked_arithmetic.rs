//! Allocation-free finite f64 sample arithmetic, separate from raw bit identity.
//! No color/profile/alpha interpretation or operation precision schedule is inferred.
//! Native integers decode as code/MAX; arithmetic never passes U32/F64 through f32.
use crate::{SampleAccessError, SampleType, sample_byte_range, try_copy_sample_raw, try_read_sample_f64, try_write_sample_f64, widen_f16_to_f32_bits};

/// Binary16 is an explicit encoded source/target, not a new document SampleType.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage { Native(SampleType), Binary16 }
impl Storage {
    pub const fn bytes(self) -> usize { match self { Self::Native(sample) => sample.bytes(), Self::Binary16 => 2 } }
    fn integer(self) -> bool { matches!(self, Self::Native(SampleType::U8 | SampleType::U16 | SampleType::U32)) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Arithmetic operands must be finite normalized samples.
    Unit,
    /// Floating storage may contain finite negative/over-one samples. Integer
    /// targets still cannot represent HDR; out-of-unit results are refused.
    FloatHdr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitOverflow { Reject, Clamp }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Refuse unless decode after encoding retains the exact f64 word, including -0.
    Exact,
    /// Explicit quantization: integers nearest with positive halfway upward;
    /// F32/binary16 nearest ties-to-even. F64 retains the calculated word.
    Nearest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub domain: Domain,
    /// Encoding only; Unit operands are never silently clamped during admission.
    /// Clamp is meaningful only for Unit results and is explicitly requested.
    pub unit_overflow: UnitOverflow,
    pub rounding: Rounding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArithmeticError {
    Access(SampleAccessError), NonFinite, OutsideDomain, InvalidPolicy,
    PrecisionLoss, StorageMismatch, DivisionByZero,
}
impl std::fmt::Display for ArithmeticError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(formatter, "checked sample arithmetic: {self:?}") }
}
impl std::error::Error for ArithmeticError {}
impl From<SampleAccessError> for ArithmeticError { fn from(error: SampleAccessError) -> Self { Self::Access(error) } }
type Result<T> = std::result::Result<T, ArithmeticError>;

pub struct Operand<'a> { pub bytes: &'a [u8], pub storage: Storage, pub index: usize }
pub struct Destination<'a> { pub bytes: &'a mut [u8], pub storage: Storage, pub index: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOperation { Add, Subtract, Multiply, Divide }

fn range(storage: Storage, index: usize, length: usize) -> Result<std::ops::Range<usize>> {
    match storage {
        Storage::Native(sample) => Ok(sample_byte_range(sample, index, length)?),
        Storage::Binary16 => {
            let start = index.checked_mul(2).ok_or(SampleAccessError::OffsetOverflow)?;
            let end = start.checked_add(2).ok_or(SampleAccessError::OffsetOverflow)?;
            if end > length { return Err(SampleAccessError::OutOfBounds.into()); }
            Ok(start..end)
        }
    }
}

/// Same-storage bit identity only. Nonfinite values and signed zero are not
/// evaluated; both accesses/widths are checked before destination publication.
pub fn copy_identity(source: Operand<'_>, destination: Destination<'_>) -> Result<()> {
    if source.storage != destination.storage { return Err(ArithmeticError::StorageMismatch); }
    match source.storage {
        Storage::Native(sample) => Ok(try_copy_sample_raw(source.bytes, source.index, destination.bytes, destination.index, sample)?),
        Storage::Binary16 => {
            let source_range = range(source.storage, source.index, source.bytes.len())?;
            let destination_range = range(destination.storage, destination.index, destination.bytes.len())?;
            let source = source.bytes.get(source_range).ok_or(SampleAccessError::OutOfBounds)?;
            destination.bytes.get_mut(destination_range).ok_or(SampleAccessError::OutOfBounds)?.copy_from_slice(source);
            Ok(())
        }
    }
}

fn read_value(source: &Operand<'_>) -> Result<f64> {
    match source.storage {
        Storage::Native(sample) => Ok(try_read_sample_f64(source.bytes, sample, source.index)?),
        Storage::Binary16 => {
            let data = source.bytes.get(range(source.storage, source.index, source.bytes.len())?).ok_or(SampleAccessError::OutOfBounds)?;
            let bits = u16::from_ne_bytes(data.try_into().map_err(|_| SampleAccessError::OutOfBounds)?);
            // Binary16 -> binary32 -> binary64 are exact widenings, never narrowing.
            Ok(f64::from(f32::from_bits(widen_f16_to_f32_bits(bits))))
        }
    }
}

/// All arithmetic admission is finite-only. Raw retention uses copy_identity.
pub fn decode_finite(source: Operand<'_>, domain: Domain) -> Result<f64> {
    let value = read_value(&source)?;
    if !value.is_finite() { return Err(ArithmeticError::NonFinite); }
    if (domain == Domain::Unit || source.storage.integer()) && !(0.0..=1.0).contains(&value) { return Err(ArithmeticError::OutsideDomain); }
    Ok(value)
}

// Direct binary64 -> binary16 quantization. A binary32 intermediate can double
// round values immediately beside a binary16 midpoint, so it is not used here.
fn round_binary16(value: f64) -> Result<u16> {
    let bits = value.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exponent < -25 { return Ok(sign); }
    if exponent > 15 { return Err(SampleAccessError::NarrowingOverflow.into()); }
    let significand = (bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52);
    let shift = if exponent < -14 { u32::try_from(28 - exponent).map_err(|_| SampleAccessError::OffsetOverflow)? } else { 42 };
    let mut rounded = significand.checked_shr(shift).ok_or(SampleAccessError::OffsetOverflow)?;
    let unit = 1u64.checked_shl(shift).ok_or(SampleAccessError::OffsetOverflow)?;
    let remainder = significand & (unit - 1);
    let halfway = unit / 2;
    if remainder > halfway || (remainder == halfway && rounded & 1 != 0) { rounded = rounded.checked_add(1).ok_or(SampleAccessError::OffsetOverflow)?; }
    let magnitude = if exponent < -14 { rounded } else {
        u64::try_from(exponent + 14).map_err(|_| SampleAccessError::OffsetOverflow)?.checked_shl(10)
            .and_then(|base| base.checked_add(rounded)).ok_or(SampleAccessError::OffsetOverflow)?
    };
    if magnitude >= 0x7c00 { return Err(SampleAccessError::NarrowingOverflow.into()); }
    Ok(sign | u16::try_from(magnitude).map_err(|_| SampleAccessError::NarrowingOverflow)?)
}

/// Stage at most eight bytes before changing the caller's destination. Integer
/// clamping is never inherited from the compatibility writer: it is either an
/// explicit Unit/Clamp request here or a named refusal. Float HDR is not clamped.
pub fn encode_finite(value: f64, destination: Destination<'_>, policy: Policy) -> Result<()> {
    let destination_range = range(destination.storage, destination.index, destination.bytes.len())?;
    if !value.is_finite() { return Err(ArithmeticError::NonFinite); }
    if policy.domain == Domain::FloatHdr && policy.unit_overflow != UnitOverflow::Reject { return Err(ArithmeticError::InvalidPolicy); }
    let value = if policy.domain == Domain::Unit && !(0.0..=1.0).contains(&value) {
        match policy.unit_overflow { UnitOverflow::Reject => return Err(ArithmeticError::OutsideDomain), UnitOverflow::Clamp => value.clamp(0.0, 1.0) }
    } else { value };
    if destination.storage.integer() && !(0.0..=1.0).contains(&value) { return Err(ArithmeticError::OutsideDomain); }
    let mut scratch = [0u8; 8];
    let encoded = scratch.get_mut(..destination.storage.bytes()).ok_or(SampleAccessError::OutOfBounds)?;
    match destination.storage {
        Storage::Native(sample) => try_write_sample_f64(encoded, sample, 0, value)?,
        Storage::Binary16 => encoded.copy_from_slice(&round_binary16(value)?.to_ne_bytes()),
    }
    let rounded = read_value(&Operand { bytes: encoded, storage: destination.storage, index: 0 })?;
    if !rounded.is_finite() { return Err(ArithmeticError::NonFinite); }
    if policy.rounding == Rounding::Exact && rounded.to_bits() != value.to_bits() { return Err(ArithmeticError::PrecisionLoss); }
    destination.bytes.get_mut(destination_range).ok_or(SampleAccessError::OutOfBounds)?.copy_from_slice(encoded);
    Ok(())
}

/// One explicitly selected f64 stage, with no fused multiply-add or intermediate
/// storage quantization. The enclosing evaluator must qualify that precision
/// schedule independently. Any input/result/offset failure preserves output.
pub fn binary(operation: BinaryOperation, left: Operand<'_>, right: Operand<'_>, destination: Destination<'_>, policy: Policy) -> Result<()> {
    range(destination.storage, destination.index, destination.bytes.len())?;
    let left = decode_finite(left, policy.domain)?;
    let right = decode_finite(right, policy.domain)?;
    let result = match operation {
        BinaryOperation::Add => left + right,
        BinaryOperation::Subtract => left - right,
        BinaryOperation::Multiply => left * right,
        BinaryOperation::Divide => {
            if right == 0.0 { return Err(ArithmeticError::DivisionByZero); }
            left / right
        }
    };
    encode_finite(result, destination, policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(rounding: Rounding) -> Policy {
        Policy { domain: Domain::FloatHdr, unit_overflow: UnitOverflow::Reject, rounding }
    }

    #[test]
    fn refused_encoding_and_arithmetic_preserve_destination() {
        for (value, storage, policy, expected) in [
            (f64::INFINITY, Storage::Native(SampleType::F64), hdr(Rounding::Nearest), ArithmeticError::NonFinite),
            (f64::NAN, Storage::Native(SampleType::F64), hdr(Rounding::Exact), ArithmeticError::NonFinite),
            (f64::from(f32::MAX) * 2.0, Storage::Native(SampleType::F32), hdr(Rounding::Nearest), ArithmeticError::Access(SampleAccessError::NarrowingOverflow)),
            (1.0 + f64::EPSILON, Storage::Native(SampleType::F32), hdr(Rounding::Exact), ArithmeticError::PrecisionLoss),
        ] {
            let mut output = [0xa5; 8];
            let before = output;
            assert_eq!(encode_finite(value, Destination { bytes: &mut output, storage, index: 0 }, policy), Err(expected));
            assert_eq!(output, before);
        }
        let left = f64::MAX.to_ne_bytes();
        for (operation, right, expected) in [
            (BinaryOperation::Multiply, 2.0, ArithmeticError::NonFinite),
            (BinaryOperation::Divide, -0.0, ArithmeticError::DivisionByZero),
            (BinaryOperation::Add, f64::NAN, ArithmeticError::NonFinite),
        ] {
            let right = right.to_ne_bytes();
            let mut output = [0x5a; 8];
            assert_eq!(binary(operation,
                Operand { bytes: &left, storage: Storage::Native(SampleType::F64), index: 0 },
                Operand { bytes: &right, storage: Storage::Native(SampleType::F64), index: 0 },
                Destination { bytes: &mut output, storage: Storage::Native(SampleType::F64), index: 0 }, hdr(Rounding::Nearest)), Err(expected));
            assert_eq!(output, [0x5a; 8]);
        }
    }

    #[test]
    fn raw_identity_preserves_nan_payloads_and_signed_zero() {
        let cases = [
            (Storage::Native(SampleType::F64), 0x7ff8_0000_0000_0123u64.to_ne_bytes().to_vec()),
            (Storage::Native(SampleType::F64), (-0.0f64).to_ne_bytes().to_vec()),
            (Storage::Native(SampleType::F32), 0x7fc0_0123u32.to_ne_bytes().to_vec()),
            (Storage::Native(SampleType::F32), (-0.0f32).to_ne_bytes().to_vec()),
            (Storage::Binary16, 0x7e35u16.to_ne_bytes().to_vec()),
            (Storage::Binary16, 0x8000u16.to_ne_bytes().to_vec()),
        ];
        for (storage, input) in cases {
            let mut output = vec![0xa5; input.len()];
            copy_identity(Operand { bytes: &input, storage, index: 0 }, Destination { bytes: &mut output, storage, index: 0 }).unwrap();
            assert_eq!(output, input);
        }
    }

    #[test]
    fn adjacent_u32_codes_and_f64_low_bits_survive() {
        for code in [0x8000_0000u32, 0x8000_0001, u32::MAX - 1] {
            let input = code.to_ne_bytes();
            let value = decode_finite(Operand { bytes: &input, storage: Storage::Native(SampleType::U32), index: 0 }, Domain::Unit).unwrap();
            let mut output = [0; 4];
            encode_finite(value, Destination { bytes: &mut output, storage: Storage::Native(SampleType::U32), index: 0 }, Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Exact }).unwrap();
            assert_eq!(u32::from_ne_bytes(output), code);
        }
        let value = 1.0 + f64::EPSILON;
        let input = value.to_ne_bytes();
        let zero = 0.0f64.to_ne_bytes();
        let mut output = [0; 8];
        binary(BinaryOperation::Add,
            Operand { bytes: &input, storage: Storage::Native(SampleType::F64), index: 0 },
            Operand { bytes: &zero, storage: Storage::Native(SampleType::F64), index: 0 },
            Destination { bytes: &mut output, storage: Storage::Native(SampleType::F64), index: 0 }, hdr(Rounding::Exact)).unwrap();
        assert_eq!(output, input);
        assert_ne!(value, f64::from(value as f32));
    }

    #[test]
    fn binary16_rounds_directly_beside_midpoint() {
        let midpoint = 1.0f64 + 2.0f64.powi(-11);
        let above = f64::from_bits(midpoint.to_bits() + 1);
        let below = f64::from_bits(midpoint.to_bits() - 1);
        // Binary32 would collapse this distinguishing input onto the midpoint.
        assert_eq!(f64::from(above as f32), midpoint);
        for (value, expected) in [(below, 0x3c00u16), (midpoint, 0x3c00), (above, 0x3c01), (-above, 0xbc01)] {
            let mut output = [0; 2];
            encode_finite(value, Destination { bytes: &mut output, storage: Storage::Binary16, index: 0 }, hdr(Rounding::Nearest)).unwrap();
            assert_eq!(u16::from_ne_bytes(output), expected);
        }
    }
}
