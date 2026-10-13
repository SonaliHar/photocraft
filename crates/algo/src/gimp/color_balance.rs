//! Color Balance in profile-encoded RGB; source/context validation is caller-owned.
//!
//! Independent public-setter response probes establish piecewise linear tonal
//! weights against HSL lightness. Grayscale ramps identify the segments; separate
//! RGB cube holdouts and coupled/preserve-luminosity originals check the model.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug)]
pub struct ColorBalance {
    /// Shadows, midtones, highlights; each cyan-red, magenta-green, yellow-blue.
    pub ranges: [[f64; 3]; 3],
    pub preserve_luminosity: bool,
}
impl ColorBalance {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if self.ranges.iter().flatten().any(|value| !value.is_finite() || !(-1.0..=1.0).contains(value)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    pub fn try_eval_pixel(self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        self.validate()?;
        if !matches!(input.len(), 3 | 4) || output.len() != input.len() {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        if self.ranges.iter().flatten().all(|value| *value == 0.0) {
            output.copy_from_slice(input);
            return Ok(());
        }
        let color = input.get(..3).ok_or(RasterInputError::LengthMismatch)?;
        let minimum = color.iter().copied().fold(1.0_f64, f64::min);
        let maximum = color.iter().copied().fold(0.0_f64, f64::max);
        let lightness = (minimum + maximum) * 0.5;
        let shadows = (1.2824 - 2.8 * lightness).clamp(0.0, 0.7);
        let highlights = (-1.5176 + 2.8 * lightness).clamp(0.0, 0.7);
        let weights = [shadows, (0.7 - shadows - highlights).max(0.0), highlights];
        let mut scratch = [0.0; 4];
        let adjusted = scratch.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
        for (channel, (value, input)) in adjusted.iter_mut().zip(color).enumerate() {
            let mut change = 0.0;
            for (range, weight) in self.ranges.iter().zip(weights) {
                change += range.get(channel).copied().ok_or(RasterInputError::LengthMismatch)? * weight;
            }
            *value = (input + change).clamp(0.0, 1.0);
        }
        if self.preserve_luminosity {
            // Retain adjusted HSL hue/saturation, restore the original lightness.
            // The achromatic branch also avoids undefined hue/division by zero.
            let low = adjusted.iter().copied().fold(1.0_f64, f64::min);
            let high = adjusted.iter().copied().fold(0.0_f64, f64::max);
            let span = high - low;
            if span == 0.0 {
                adjusted.fill(lightness);
            } else {
                let adjusted_lightness = (low + high) * 0.5;
                let denominator = 1.0 - (2.0 * adjusted_lightness - 1.0).abs();
                if denominator <= 0.0 {
                    return Err(RasterInputError::InvalidSample);
                }
                let chroma = (span / denominator) * (1.0 - (2.0 * lightness - 1.0).abs());
                for value in adjusted {
                    *value = (lightness - chroma * 0.5 + (*value - low) / span * chroma).clamp(0.0, 1.0);
                }
            }
        }
        if let Some(alpha) = input.get(3) {
            *scratch.get_mut(3).ok_or(RasterInputError::LengthMismatch)? = *alpha;
        }
        output.copy_from_slice(scratch.get(..input.len()).ok_or(RasterInputError::LengthMismatch)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_samples_shapes_and_parameters_leave_output_untouched() {
        let config = ColorBalance { ranges: [[0.125, -0.25, 0.375]; 3], preserve_luminosity: true };
        for input in [
            vec![0.5],
            vec![0.5, 1.0],
            vec![0.5; 5],
            vec![f64::NAN, 0.5, 0.5],
            vec![0.5, f64::INFINITY, 0.5],
            vec![-0.001, 0.5, 0.5],
            vec![0.5, 1.001, 0.5],
            vec![0.5, 0.5, 0.5, -0.001],
        ] {
            let mut output = vec![0.9375; input.len()];
            let before = output.clone();
            assert!(config.try_eval_pixel(&input, &mut output).is_err());
            assert_eq!(output, before);
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.001, 1.001] {
            let bad = ColorBalance { ranges: [[value; 3]; 3], ..config };
            let mut output = [0.9375; 4];
            assert!(bad.try_eval_pixel(&[0.5; 4], &mut output).is_err());
            assert_eq!(output, [0.9375; 4]);
        }
        let mut output = [0.9375; 3];
        assert!(config.try_eval_pixel(&[0.5; 4], &mut output).is_err());
        assert_eq!(output, [0.9375; 3]);
    }

    #[test]
    fn neutral_preserves_native_bits_and_luminosity_handles_achromatic_endpoints() {
        for preserve_luminosity in [false, true] {
            let config = ColorBalance { ranges: [[0.0; 3]; 3], preserve_luminosity };
            let input = [-0.0, 0.123456789012345, 1.0, -0.0];
            let mut output = [0.5; 4];
            config.try_eval_pixel(&input, &mut output).unwrap();
            assert_eq!(input.map(f64::to_bits), output.map(f64::to_bits));
        }
        let config = ColorBalance { ranges: [[1.0; 3]; 3], preserve_luminosity: true };
        for level in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let input = [level, level, level, 0.375];
            let mut output = [0.0; 4];
            config.try_eval_pixel(&input, &mut output).unwrap();
            assert_eq!(input, output);
        }
    }
}
