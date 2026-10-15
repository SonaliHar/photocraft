//! Bounded raw Point argument storage. No operation, profile or numeric meaning
//! follows from this walk. Copy callers must hold the reported real quota fee.
use crate::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl},
};
use std::mem::size_of;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointArgumentShape {
    pub f64_words: u64,
    pub config_entries: u64,
    pub curve_arrays: u64,
    pub utf8_bytes: u64,
    /// Dynamic input allocation capacities. For the root argument slice only,
    /// counts len*sizeof(FilterArgument); callers must add its spare capacity.
    pub input_index_bytes: u64,
    /// Prospective dynamic destination bytes, allowing at most twice each
    /// requested allocation length. Inline owner/header fees are caller duties.
    pub copy_index_bytes: u64,
}
type Check<'a> = dyn Fn(u64) -> Result<(), BackingError> + 'a;
fn add(to: &mut u64, value: u64) -> Result<(), BackingError> {
    *to = to.checked_add(value).ok_or(BackingError::Length)?;
    Ok(())
}
fn bytes<T>(count: usize) -> Result<u64, BackingError> {
    (count as u64).checked_mul(size_of::<T>() as u64).ok_or(BackingError::Length)
}
fn allocation<T>(shape: &mut PointArgumentShape, len: usize, capacity: usize) -> Result<(), BackingError> {
    add(&mut shape.input_index_bytes, bytes::<T>(capacity)?)?;
    add(&mut shape.copy_index_bytes, bytes::<T>(len)?.checked_mul(2).ok_or(BackingError::Length)?)
}
fn text(value: &String, shape: &mut PointArgumentShape, check: &Check<'_>) -> Result<(), BackingError> {
    check(1)?;
    if value.is_empty() || value.len() > 4096 {
        return Err(BackingError::Unsupported);
    }
    add(&mut shape.utf8_bytes, value.len() as u64)?;
    allocation::<u8>(shape, value.len(), value.capacity())
}
fn words(shape: &mut PointArgumentShape, n: u64) -> Result<(), BackingError> {
    add(&mut shape.f64_words, n)?;
    if shape.f64_words > 1280 {
        return Err(BackingError::Length);
    }
    Ok(())
}
fn walk_value(value: &FilterValue, depth: usize, shape: &mut PointArgumentShape, check: &Check<'_>) -> Result<(), BackingError> {
    check(1)?;
    match value {
        FilterValue::Number(ExactNumber::Float32Bits(_)) | FilterValue::Bool(_) => {}
        FilterValue::Number(ExactNumber::Float64Bits(_)) => words(shape, 1)?,
        FilterValue::Number(ExactNumber::Signed { width: 32, value }) if i32::try_from(*value).is_ok() => {}
        FilterValue::Enum { domain, .. } => text(domain, shape, check)?,
        FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile | SampleEncoding::Linear, profile: None } if components.len() == 4 => {
            allocation::<ExactNumber>(shape, components.len(), components.capacity())?;
            for component in components {
                check(1)?;
                if !matches!(component, ExactNumber::Float32Bits(_)) {
                    return Err(BackingError::Unsupported);
                }
            }
        }
        FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => {
            if points_f64_bits.len() > 256 || point_types.len() > 256 || samples_f64_bits.len() > 256 {
                return Err(BackingError::Length);
            }
            words(
                shape,
                (points_f64_bits.len() as u64).checked_mul(2).and_then(|v| v.checked_add(samples_f64_bits.len() as u64)).ok_or(BackingError::Length)?,
            )?;
            add(&mut shape.curve_arrays, 3)?;
            allocation::<[u64; 2]>(shape, points_f64_bits.len(), points_f64_bits.capacity())?;
            allocation::<i32>(shape, point_types.len(), point_types.capacity())?;
            allocation::<u64>(shape, samples_f64_bits.len(), samples_f64_bits.capacity())?;
            // Poll/debit every raw input entry even though shape never evaluates
            // numeric words, point types or a curve interpolation algorithm.
            for _ in points_f64_bits {
                check(2)?;
            }
            for _ in point_types {
                check(1)?;
            }
            for _ in samples_f64_bits {
                check(1)?;
            }
        }
        FilterValue::Config(arguments) => {
            if depth >= 2 || arguments.len() > 5 {
                return Err(BackingError::Length);
            }
            add(&mut shape.config_entries, arguments.len() as u64)?;
            allocation::<FilterArgument>(shape, arguments.len(), arguments.capacity())?;
            for argument in arguments {
                text(&argument.name, shape, check)?;
                walk_value(&argument.value, depth + 1, shape, check)?;
            }
        }
        _ => return Err(BackingError::Unsupported),
    }
    Ok(())
}

pub fn point_argument_shape(arguments: &[FilterArgument], control: &ReadControl<'_>) -> Result<PointArgumentShape, BackingError> {
    point_argument_shape_with(arguments, control.interrupt, control.work)
}
/// Non-Sync work callback permits an existing decoder's same interior ledger.
/// The callback is borrowed for this traversal only and is never retained.
pub fn point_argument_shape_with(
    arguments: &[FilterArgument],
    interrupt: &Interrupt<'_>,
    work: &dyn Fn(u64) -> Result<(), BackingError>,
) -> Result<PointArgumentShape, BackingError> {
    let check = |units| {
        interrupt.check().map_err(|_| BackingError::Cancelled)?;
        work(units)?;
        interrupt.check().map_err(|_| BackingError::Cancelled)
    };
    check(1)?;
    if arguments.len() > 10 {
        return Err(BackingError::Length);
    }
    let mut shape = PointArgumentShape::default();
    allocation::<FilterArgument>(&mut shape, arguments.len(), arguments.len())?;
    for argument in arguments {
        text(&argument.name, &mut shape, &check)?;
        walk_value(&argument.value, 0, &mut shape, &check)?;
    }
    check(1)?;
    Ok(shape)
}
pub fn point_value_shape(input: &FilterValue, control: &ReadControl<'_>) -> Result<PointArgumentShape, BackingError> {
    let mut shape = PointArgumentShape::default();
    walk_value(input, 0, &mut shape, &|n| control.check(n))?;
    control.check(1)?;
    Ok(shape)
}

fn text_equal(a: &str, b: &str, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    control.check(1)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.as_bytes().chunks(1024).zip(b.as_bytes().chunks(1024)) {
        control.check((a.len() as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn equal(a: &FilterValue, b: &FilterValue, raw: bool, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    control.check(1)?;
    Ok(match (a, b) {
        (FilterValue::Number(ExactNumber::Float32Bits(a)), FilterValue::Number(ExactNumber::Float32Bits(b))) => !raw || a == b,
        (FilterValue::Number(ExactNumber::Float64Bits(a)), FilterValue::Number(ExactNumber::Float64Bits(b))) => !raw || a == b,
        (FilterValue::Number(ExactNumber::Signed { width: 32, value: a }), FilterValue::Number(ExactNumber::Signed { width: 32, value: b })) => !raw || a == b,
        (FilterValue::Bool(a), FilterValue::Bool(b)) => !raw || a == b,
        (FilterValue::Enum { domain: a, value: av }, FilterValue::Enum { domain: b, value: bv }) => text_equal(a, b, control)? && (!raw || av == bv),
        (FilterValue::Color { components: a, encoding: ae, profile: ap }, FilterValue::Color { components: b, encoding: be, profile: bp }) => {
            if ae != be || ap != bp || a.len() != b.len() {
                false
            } else {
                let mut same = true;
                for (a, b) in a.iter().zip(b) {
                    control.check(1)?;
                    same &= !raw || a == b;
                }
                same
            }
        }
        (
            FilterValue::Curve { free: af, points_f64_bits: a, point_types: at, samples_f64_bits: as_ },
            FilterValue::Curve { free: bf, points_f64_bits: b, point_types: bt, samples_f64_bits: bs },
        ) => {
            if af != bf || a.len() != b.len() || at.len() != bt.len() || as_.len() != bs.len() {
                false
            } else {
                let mut same = true;
                for (a, b) in a.iter().zip(b) {
                    control.check(2)?;
                    same &= !raw || a == b;
                }
                for (a, b) in at.iter().zip(bt) {
                    control.check(1)?;
                    same &= !raw || a == b;
                }
                for (a, b) in as_.iter().zip(bs) {
                    control.check(1)?;
                    same &= !raw || a == b;
                }
                same
            }
        }
        (FilterValue::Config(a), FilterValue::Config(b)) => {
            if a.len() != b.len() {
                false
            } else {
                let mut same = true;
                for (a, b) in a.iter().zip(b) {
                    if !text_equal(&a.name, &b.name, control)? || !equal(&a.value, &b.value, raw, control)? {
                        same = false;
                    }
                }
                same
            }
        }
        _ => false,
    })
}
pub fn point_values_same_shape(a: &FilterValue, b: &FilterValue, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    point_value_shape(a, control)?;
    point_value_shape(b, control)?;
    equal(a, b, false, control)
}
pub fn point_values_equal(a: &FilterValue, b: &FilterValue, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    point_value_shape(a, control)?;
    point_value_shape(b, control)?;
    equal(a, b, true, control)
}

fn vector<T>(len: usize) -> Result<Vec<T>, BackingError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| BackingError::Allocation)?;
    if v.capacity() > len.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    Ok(v)
}
fn copy_text(input: &str, control: &ReadControl<'_>) -> Result<String, BackingError> {
    let mut output = String::new();
    output.try_reserve_exact(input.len()).map_err(|_| BackingError::Allocation)?;
    if output.capacity() > input.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    for ch in input.chars() {
        control.check(ch.len_utf8() as u64)?;
        output.push(ch);
    }
    Ok(output)
}
fn copy(input: &FilterValue, control: &ReadControl<'_>) -> Result<FilterValue, BackingError> {
    control.check(1)?;
    Ok(match input {
        FilterValue::Number(n) => FilterValue::Number(n.clone()),
        FilterValue::Bool(v) => FilterValue::Bool(*v),
        FilterValue::Enum { domain, value } => FilterValue::Enum { domain: copy_text(domain, control)?, value: *value },
        FilterValue::Color { components, encoding, profile } => {
            let mut output = vector(components.len())?;
            for v in components {
                control.check(1)?;
                output.push(v.clone());
            }
            FilterValue::Color { components: output, encoding: *encoding, profile: *profile }
        }
        FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => {
            let mut points = vector(points_f64_bits.len())?;
            let mut types = vector(point_types.len())?;
            let mut samples = vector(samples_f64_bits.len())?;
            for p in points_f64_bits {
                control.check(2)?;
                points.push(*p);
            }
            for t in point_types {
                control.check(1)?;
                types.push(*t);
            }
            for s in samples_f64_bits {
                control.check(1)?;
                samples.push(*s);
            }
            FilterValue::Curve { free: *free, points_f64_bits: points, point_types: types, samples_f64_bits: samples }
        }
        FilterValue::Config(arguments) => {
            let mut output = vector(arguments.len())?;
            for a in arguments {
                output.push(FilterArgument { name: copy_text(&a.name, control)?, value: copy(&a.value, control)? });
            }
            FilterValue::Config(output)
        }
        _ => return Err(BackingError::Unsupported),
    })
}
/// Caller holds copy_index_bytes plus its inline owner/header allocation fees.
/// No arbitrary raw payload or quota receipt can be adopted through this copier.
pub(super) fn copy_value(input: &FilterValue, control: &ReadControl<'_>) -> Result<FilterValue, BackingError> {
    point_value_shape(input, control)?;
    let output = copy(input, control)?;
    control.check(1)?;
    Ok(output)
}

#[cfg(test)]
#[path = "structured_tests.rs"]
mod tests;
