//! Additive finite-derived RGB lane. No source or working-space authority.
use super::{PointOperation, PreparedPointOperation};
use photocraft_raster::RasterInputError;

impl PreparedPointOperation {
    /// Finite derived RGB with unit alpha for three explicitly selected kinds.
    /// The caller supplies the existing kind's working encoding and pays work.
    /// Old strict/Gray evaluators remain independent. Output is atomic.
    pub fn try_eval_derived_rgb_pixel(&self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        let source: [f64; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
        let alpha = *source.get(3).ok_or(RasterInputError::LengthMismatch)?;
        if source.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut result = source;
        let colors = result.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
        match self.operation {
            PointOperation::ColorTemperature { .. } => {
                let gains = self.temperature_gains.ok_or(RasterInputError::UnsupportedConversion)?;
                for (value, gain) in colors.iter_mut().zip(gains) {
                    if !(*value as f32).is_finite() {
                        return Err(RasterInputError::UnsupportedConversion);
                    }
                    *value = f64::from((*value as f32) * (gain as f32));
                }
            }
            PointOperation::AlienMap { frequency, phase, keep } => {
                for (((value, frequency), phase), keep) in colors.iter_mut().zip(frequency).zip(phase).zip(keep) {
                    if !keep {
                        if !(*value as f32).is_finite() {
                            return Err(RasterInputError::UnsupportedConversion);
                        }
                        let angle = (2.0 * f64::from(*value as f32) - 1.0) * std::f64::consts::PI * frequency + phase.to_radians();
                        if !angle.is_finite() {
                            return Err(RasterInputError::UnsupportedConversion);
                        }
                        *value = f64::from(((1.0 + angle.sin()) * 0.5) as f32);
                    }
                }
            }
            PointOperation::Sepia { scale, .. } => {
                let matrix = [[0x3ec9374c, 0x3f44dd2f, 0x3e418937], [0x3eb2b021, 0x3f2f9db2, 0x3e2c0831], [0x3e8b4396, 0x3f08b439, 0x3e0624dd]];
                let rgb = [
                    *colors.first().ok_or(RasterInputError::LengthMismatch)? as f32,
                    *colors.get(1).ok_or(RasterInputError::LengthMismatch)? as f32,
                    *colors.get(2).ok_or(RasterInputError::LengthMismatch)? as f32,
                ];
                if rgb.iter().any(|v| !v.is_finite()) {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                for (index, (value, row)) in colors.iter_mut().zip(matrix).enumerate() {
                    let transformed = rgb.into_iter().zip(row).map(|(v, bits)| v * f32::from_bits(bits)).sum::<f32>();
                    let original = *rgb.get(index).ok_or(RasterInputError::LengthMismatch)?;
                    *value = f64::from(original * (1.0 - scale as f32) + transformed * scale as f32);
                }
            }
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
        if result.iter().any(|v| !v.is_finite()) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        output.copy_from_slice(&result);
        Ok(())
    }
}

#[cfg(test)]
#[path = "derived_rgb_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::unreachable)]
mod tests;
