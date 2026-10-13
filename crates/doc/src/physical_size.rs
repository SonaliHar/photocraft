//! Checked per-axis size conversions. Unit factors are units per inch.
//! No scalar DPI collapse, metadata mutation or serialized model additions.
use crate::{PhysicalResolution, PhysicalUnit, Size};

pub const MAX_CANVAS_DIMENSION: u32 = 300_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimensionRounding {
    Nearest,
    Floor,
    Ceil,
    Exact,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalSizeError {
    Resolution,
    Unit,
    Value,
    Range,
    Dimension,
}
impl std::fmt::Display for PhysicalSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Resolution => "physical size: invalid resolution",
            Self::Unit => "physical size: invalid unit",
            Self::Value => "physical size: invalid length",
            Self::Range => "physical size: unrepresentable conversion",
            Self::Dimension => "physical size: dimension must be 1..=300000 pixels",
        })
    }
}
impl std::error::Error for PhysicalSizeError {}

fn context(resolution: &PhysicalResolution, unit: &PhysicalUnit, axis: Axis) -> Result<(f64, Option<f64>), PhysicalSizeError> {
    // Validate borrowed custom strings/factors before any numeric operation.
    resolution.validate().map_err(|_| PhysicalSizeError::Resolution)?;
    let factor = match unit {
        PhysicalUnit::Pixel => None,
        PhysicalUnit::Inch => Some(1.0),
        PhysicalUnit::Millimeter => Some(25.4),
        PhysicalUnit::Point => Some(72.0),
        PhysicalUnit::Pica => Some(6.0),
        PhysicalUnit::Custom(value) => {
            if !value.factor.is_finite()
                || value.factor <= 0.0
                || value.name.is_empty()
                || [&value.name, &value.symbol, &value.abbreviation].iter().any(|s| s.len() > 1 << 20 || s.contains('\0'))
            {
                return Err(PhysicalSizeError::Unit);
            }
            Some(value.factor)
        }
    };
    Ok((
        match axis {
            Axis::X => resolution.x_dpi,
            Axis::Y => resolution.y_dpi,
        },
        factor,
    ))
}
fn length(value: f64) -> Result<(), PhysicalSizeError> {
    if !value.is_finite() || value < 0.0 { Err(PhysicalSizeError::Value) } else { Ok(()) }
}

/// Convert a nonnegative pixel length along exactly the chosen resolution axis.
/// Zero remains zero; a positive conversion underflowing to zero refuses.
pub fn pixels_to_units(pixels: f64, resolution: &PhysicalResolution, unit: &PhysicalUnit, axis: Axis) -> Result<f64, PhysicalSizeError> {
    length(pixels)?;
    let (dpi, factor) = context(resolution, unit, axis)?;
    match factor {
        None => Ok(pixels),
        Some(factor) => product_ratio(pixels, factor, dpi),
    }
}
/// Convert a nonnegative physical length without narrowing axes or custom factors.
pub fn units_to_pixels(value: f64, resolution: &PhysicalResolution, unit: &PhysicalUnit, axis: Axis) -> Result<f64, PhysicalSizeError> {
    length(value)?;
    let (dpi, factor) = context(resolution, unit, axis)?;
    match factor {
        None => Ok(value),
        Some(factor) => product_ratio(value, dpi, factor),
    }
}
pub fn physical_dimensions(size: Size, resolution: &PhysicalResolution, unit: &PhysicalUnit) -> Result<(f64, f64), PhysicalSizeError> {
    Ok((pixels_to_units(f64::from(size.width), resolution, unit, Axis::X)?, pixels_to_units(f64::from(size.height), resolution, unit, Axis::Y)?))
}
/// Convert dimensions and apply the requested integer policy. Nearest uses
/// f64::round (half away from zero); no minimum-one or maximum clamping occurs.
pub fn pixel_dimensions(
    width: f64,
    height: f64,
    resolution: &PhysicalResolution,
    unit: &PhysicalUnit,
    rounding: DimensionRounding,
) -> Result<Size, PhysicalSizeError> {
    let width = units_to_pixels(width, resolution, unit, Axis::X)?;
    let height = units_to_pixels(height, resolution, unit, Axis::Y)?;
    Ok(Size::new(dimension(width, rounding)?, dimension(height, rounding)?))
}
fn dimension(value: f64, rounding: DimensionRounding) -> Result<u32, PhysicalSizeError> {
    let rounded = match rounding {
        DimensionRounding::Nearest => value.round(),
        DimensionRounding::Floor => value.floor(),
        DimensionRounding::Ceil => value.ceil(),
        DimensionRounding::Exact if value.fract() == 0.0 => value,
        DimensionRounding::Exact => return Err(PhysicalSizeError::Dimension),
    };
    if !rounded.is_finite() || rounded < 1.0 || rounded > f64::from(MAX_CANVAS_DIMENSION) {
        return Err(PhysicalSizeError::Dimension);
    }
    Ok(rounded as u32)
}

fn product_ratio(value: f64, numerator: f64, denominator: f64) -> Result<f64, PhysicalSizeError> {
    if value == 0.0 {
        return Ok(value);
    }
    // Preserve the ordinary divide-then-multiply schedule, without rejecting a
    // finite final result merely because its intermediate overflowed/underflowed.
    let divided = value / denominator;
    let ordinary = divided * numerator;
    if divided.is_finite() && divided > 0.0 && ordinary.is_finite() && ordinary > 0.0 {
        return Ok(ordinary);
    }
    let (a, ea) = normalized(value);
    let (b, eb) = normalized(numerator);
    let (c, ec) = normalized(denominator);
    let mut mantissa = (a * b) / c;
    let mut exponent = ea + eb - ec;
    while mantissa >= 2.0 {
        mantissa *= 0.5;
        exponent += 1;
    }
    while mantissa < 1.0 {
        mantissa *= 2.0;
        exponent -= 1;
    }
    if !(-1075..=1023).contains(&exponent) {
        return Err(PhysicalSizeError::Range);
    }
    let result = if exponent >= -1022 {
        mantissa * f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        // Scale in two exact powers so subnormal rounding happens at the final
        // operation, rather than truncating an intermediate significand.
        let first = mantissa * f64::from_bits(1_u64 << 52);
        first * f64::from_bits(((exponent + 1022 + 1023) as u64) << 52)
    };
    if !result.is_finite() || result <= 0.0 {
        return Err(PhysicalSizeError::Range);
    }
    Ok(result)
}
/// Finite positive binary64 → exact normalized significand and binary exponent.
fn normalized(value: f64) -> (f64, i32) {
    let bits = value.to_bits();
    let stored = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);
    if stored != 0 {
        return (f64::from_bits((1023_u64 << 52) | fraction), stored - 1023);
    }
    let highest = 63 - fraction.leading_zeros() as i32;
    let shift = (52 - highest) as u32;
    let shifted = fraction << shift;
    (f64::from_bits((1023_u64 << 52) | (shifted & ((1_u64 << 52) - 1))), highest - 1074)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolution() -> PhysicalResolution {
        PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: PhysicalUnit::Inch }
    }
    #[test]
    fn distinct_axes_and_units_preserve_physical_size() {
        let size = pixel_dimensions(1.0, 1.0, &resolution(), &PhysicalUnit::Inch, DimensionRounding::Exact).unwrap();
        assert_eq!(size, Size::new(144, 288));
        assert_eq!(physical_dimensions(size, &resolution(), &PhysicalUnit::Point).unwrap(), (72.0, 72.0));
    }
    #[test]
    fn rounding_has_no_silent_minimum_or_maximum_clamp() {
        assert!(pixel_dimensions(0.0, 1.0, &resolution(), &PhysicalUnit::Pixel, DimensionRounding::Ceil).is_err());
        assert!(pixel_dimensions(300001.0, 1.0, &resolution(), &PhysicalUnit::Pixel, DimensionRounding::Nearest).is_err());
        assert!(pixel_dimensions(1.5, 1.0, &resolution(), &PhysicalUnit::Pixel, DimensionRounding::Exact).is_err());
        assert_eq!(pixel_dimensions(1.5, 1.0, &resolution(), &PhysicalUnit::Pixel, DimensionRounding::Nearest).unwrap(), Size::new(2, 1));
    }
    #[test]
    fn subnormal_equal_factors_do_not_lose_positive_value() {
        let tiny = f64::from_bits(1);
        let unit = PhysicalUnit::Custom(crate::physical_resolution::CustomUnit {
            name: "tiny".into(),
            factor: tiny,
            digits: 2,
            symbol: String::new(),
            abbreviation: String::new(),
        });
        let res = PhysicalResolution { x_dpi: tiny, y_dpi: tiny, unit: PhysicalUnit::Inch };
        assert_eq!(units_to_pixels(2.0, &res, &unit, Axis::X).unwrap(), 2.0);
        assert_eq!(pixels_to_units(2.0, &res, &unit, Axis::Y).unwrap(), 2.0);
    }
    #[test]
    fn invalid_and_unrepresentable_values_refuse() {
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(units_to_pixels(value, &resolution(), &PhysicalUnit::Inch, Axis::X).is_err());
        }
        let res = PhysicalResolution::square(f64::from_bits(1));
        assert!(pixels_to_units(1.0, &res, &PhysicalUnit::Inch, Axis::X).is_err());
        assert_eq!(pixels_to_units(0.0, &res, &PhysicalUnit::Inch, Axis::X).unwrap(), 0.0);
    }
}
