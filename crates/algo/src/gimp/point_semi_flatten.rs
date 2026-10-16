//! Semi-Flatten working-domain compatibility policy from independent public
//! GIMP observations. Source/profile admission and transfer conversion are
//! caller-owned; exact producer FMA/rounding equivalence remains unproven.
use photocraft_raster::RasterInputError;

/// Prepared operation-working background operand, rather than a raw serialized
/// color literal. The caller interprets the literal's encoding/profile first.
/// The fourth word is retained and validated, but does not affect the equation.
#[derive(Clone, Copy, Debug)]
pub struct SemiFlatten {
    pub color: [f32; 4],
}

impl SemiFlatten {
    pub fn validate(&self) -> Result<(), RasterInputError> {
        if self.color.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::InvalidSample);
        }
        Ok(())
    }

    /// Evaluate straight Linear working RGB with an explicit Float boundary.
    /// This does not bypass the caller's forward/inverse transfer for zero or
    /// full alpha. All validation completes before any output word is written.
    pub fn try_eval_pixel(&self, input: &[f64], channels: u8, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.try_eval_with_input_domain(input, channels, output, false)
    }

    /// Evaluate an already derived Linear RGB Float stage. RGB may be finite
    /// HDR; alpha and the prepared background retain their unit domains. This
    /// does not admit HDR source storage or choose a filter graph's formats.
    pub fn try_eval_derived_pixel(&self, input: &[f64], channels: u8, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.try_eval_with_input_domain(input, channels, output, true)
    }

    fn try_eval_with_input_domain(&self, input: &[f64], channels: u8, output: &mut [f64], derived: bool) -> Result<(), RasterInputError> {
        if channels != 3 || input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        self.validate()?;
        let invalid = if derived {
            input.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite())
                || input.get(3).is_none_or(|alpha| !(0.0..=1.0).contains(alpha))
        } else {
            input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        };
        if invalid {
            return Err(RasterInputError::InvalidSample);
        }
        let alpha = *input.get(3).ok_or(RasterInputError::LengthMismatch)? as f32;
        let mut staged = [0.0f32; 4];
        for (destination, source) in staged.iter_mut().zip(input) {
            *destination = *source as f32;
        }
        if alpha > 0.0 && alpha < 1.0 {
            let retained_background = 1.0f32 - alpha;
            for (destination, background) in staged.iter_mut().take(3).zip(self.color) {
                // Explicit separately rounded Float products/addition are a
                // declared policy, not a claim about producer FMA selection.
                let foreground = *destination * alpha;
                let background = background * retained_background;
                *destination = foreground + background;
            }
            *staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)? = 1.0;
        }
        if staged.iter().any(|v| !v.is_finite()) {
            return Err(RasterInputError::InvalidSample);
        }
        for (destination, value) in output.iter_mut().zip(staged) {
            *destination = f64::from(value);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "point_semi_flatten_tests.rs"]
mod tests;
