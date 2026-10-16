//! Checked finite subset of the existing HSL arithmetic, not Source authority.
use super::{PointOperation, PreparedPointOperation};
use photocraft_raster::RasterInputError;

fn finite(value: f32) -> Result<(), RasterInputError> {
    if value.is_finite() { Ok(()) } else { Err(RasterInputError::UnsupportedConversion) }
}
fn finite64(value: f64) -> Result<(), RasterInputError> {
    if value.is_finite() { Ok(()) } else { Err(RasterInputError::UnsupportedConversion) }
}
// Deliberately separate from the old unit map. Every numeric expression retains
// its existing precision and ordering; checks add refusal, never recovery.
fn try_map_derived(rgb: [f64; 3], frequency: [f64; 3], phase: [f64; 3], keep: [bool; 3]) -> Result<[f64; 3], RasterInputError> {
    if rgb.iter().any(|v| !v.is_finite())
        || frequency.iter().any(|v| !v.is_finite() || !(0.0..=20.0).contains(v))
        || phase.iter().any(|v| !v.is_finite() || !(0.0..=360.0).contains(v))
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let [r, g, b] = rgb.map(|v| v as f32);
    for v in [r, g, b] {
        finite(v)?;
    }
    let maximum = r.max(g).max(b);
    let minimum = r.min(g).min(b);
    finite(maximum)?;
    finite(minimum)?;
    let chroma = maximum - minimum;
    finite(chroma)?;
    let sum = maximum + minimum;
    finite(sum)?;
    let lightness = (maximum + minimum) * 0.5;
    finite(lightness)?;
    let (hue, saturation) = if chroma == 0.0 {
        (0.0, 0.0)
    } else {
        let denominator = if lightness <= 0.5 { maximum + minimum } else { 2.0 - maximum - minimum };
        finite(denominator)?;
        if denominator == 0.0 {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let saturation = if lightness <= 0.5 { chroma / (maximum + minimum) } else { chroma / (2.0 - maximum - minimum) };
        finite(saturation)?;
        let sector = if maximum == r {
            (g - b) / chroma
        } else if maximum == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        };
        finite(sector)?;
        ((sector / 6.0).rem_euclid(1.0), saturation)
    };
    finite(hue)?;
    finite(saturation)?;
    let mut mapped = [0.0; 3];
    for ((((output, value), frequency), phase), keep) in mapped.iter_mut().zip([hue, saturation, lightness]).zip(frequency).zip(phase).zip(keep) {
        *output = if keep {
            f64::from(value)
        } else {
            let angle = (2.0 * f64::from(value) - 1.0) * std::f64::consts::PI * frequency + phase.to_radians();
            finite64(angle)?;
            f64::from(((1.0 + ((2.0 * f64::from(value) - 1.0) * std::f64::consts::PI * frequency + phase.to_radians()).sin()) * 0.5) as f32)
        };
        finite64(*output)?;
    }
    let [h, s, l] = mapped;
    if s == 0.0 {
        return Ok([l; 3]);
    }
    let upper = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    finite64(upper)?;
    let lower = 2.0 * l - upper;
    finite64(lower)?;
    let channel = |hue: f64| -> Result<f64, RasterInputError> {
        finite64(hue)?;
        let hue = hue.rem_euclid(1.0);
        finite64(hue)?;
        let value = if hue < 1.0 / 6.0 {
            lower + (upper - lower) * 6.0 * hue
        } else if hue < 0.5 {
            upper
        } else if hue < 2.0 / 3.0 {
            lower + (upper - lower) * (2.0 / 3.0 - hue) * 6.0
        } else {
            lower
        };
        finite64(value)?;
        let result = f64::from(value as f32);
        finite64(result)?;
        Ok(result)
    };
    Ok([channel(h + 1.0 / 3.0)?, channel(h)?, channel(h - 1.0 / 3.0)?])
}
impl PreparedPointOperation {
    /// Additive checked domain: finite RGB/Float intermediates, nonsingular selected HSL
    /// denominator and unit alpha. Does not extend old strict/Gray entry points.
    pub fn try_eval_derived_hsl_pixel(&self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        let PointOperation::AlienMapHsl { frequency, phase, keep } = self.operation else {
            return Err(RasterInputError::UnsupportedConversion);
        };
        let rgba: [f64; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
        let [r, g, b, alpha] = rgba;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let rgb = try_map_derived([r, g, b], frequency, phase, keep)?;
        let result = [rgb[0], rgb[1], rgb[2], alpha];
        output.copy_from_slice(&result);
        Ok(())
    }
}

#[cfg(test)]
#[path = "derived_hsl_tests.rs"]
mod tests;
