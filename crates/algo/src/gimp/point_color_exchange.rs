//! Clean-room literal color exchange from independent public GEGL response.
//! Strict Float32 coordinate bounds, including the independently measured
//! 1e-5 boundary expansion, were distinguished from fabs/inclusive bounds by
//! 1,552 original off-grid and representable-neighbor controls. Cached Float32
//! to-minus-from arithmetic matches another 27,648 source/output components.
//! Original observation manifest: 7cc62d4b0a62751aa567630d163bdb7b1011701d471643b18d7d4757f086bad0.
use photocraft_raster::RasterInputError;

pub(super) fn validate(from: [f32; 4], to: [f32; 4], thresholds: [f32; 3]) -> Result<(), RasterInputError> {
    if from.into_iter().chain(to).chain(thresholds).all(|value| value.is_finite() && (0.0..=1.0).contains(&value)) {
        Ok(())
    } else {
        Err(RasterInputError::UnsupportedConversion)
    }
}

/// Nonlinear RGB operation input; converting a Gray document is caller-owned.
/// Every sample is validated before output publication. Alpha is copied exactly.
pub(super) fn eval(from: [f32; 4], to: [f32; 4], thresholds: [f32; 3], input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
    validate(from, to, thresholds)?;
    if channels != 3 || input.len() != output.len() || !(3..=4).contains(&input.len()) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if input.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let mut values = [0.; 4];
    let dst = values.get_mut(..input.len()).ok_or(RasterInputError::LengthMismatch)?;
    dst.copy_from_slice(input);
    let colors = dst.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
    let mut inside = true;
    for ((value, from), threshold) in colors.iter_mut().zip(from).zip(thresholds) {
        *value = f64::from(*value as f32);
        let low = (f64::from(from) - f64::from(threshold) - 1e-5) as f32;
        let high = (f64::from(from) + f64::from(threshold) + 1e-5) as f32;
        inside &= *value > f64::from(low) && *value < f64::from(high);
    }
    if inside {
        for ((value, from), to) in colors.iter_mut().zip(from).zip(to) {
            let offset = to - from;
            *value = f64::from((*value as f32 + offset).clamp(0., 1.));
        }
    }
    output.copy_from_slice(dst);
    Ok(())
}
