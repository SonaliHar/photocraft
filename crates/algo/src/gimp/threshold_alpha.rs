//! Alpha-only threshold from independently observed public GIMP boundary responses.
//! Input alpha is rounded to Float32, then compared strictly with the Double
//! threshold. Source color components remain authoritative and untouched.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThresholdAlpha {
    pub value: f64,
}
impl ThresholdAlpha {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if !self.value.is_finite() || !(0.0..=1.0).contains(&self.value) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    pub fn try_eval_pixel(self, input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.validate()?;
        if !matches!(channels, 1 | 3) || input.len() != channels + 1 || output.len() != input.len() {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let alpha = input.get(channels).ok_or(RasterInputError::LengthMismatch)?;
        let next = if f64::from(*alpha as f32) > self.value { 1.0 } else { 0.0 };
        output.copy_from_slice(input);
        *output.get_mut(channels).ok_or(RasterInputError::LengthMismatch)? = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_float32_alpha_and_double_threshold_boundaries() {
        let operation = ThresholdAlpha { value: 0.5 };
        for (alpha, expected) in
            [(0., 0.), (0.5 - 2_f64.powi(-25), 0.), (0.5, 0.), (0.5 + 2_f64.powi(-52), 0.), (0.5 + 2_f64.powi(-25), 0.), (0.5 + 2_f64.powi(-24), 1.), (1., 1.)]
        {
            let input = [0.173, -0.0, 0.719, alpha];
            let mut output = [0.; 4];
            operation.try_eval_pixel(&input, 3, &mut output).unwrap();
            assert_eq!(output[3], expected);
            for index in 0..3 {
                assert_eq!(output[index].to_bits(), input[index].to_bits());
            }
        }
        let mut output = [0.; 2];
        ThresholdAlpha { value: 0.5 - 2_f64.powi(-52) }.try_eval_pixel(&[0.367, 0.5], 1, &mut output).unwrap();
        assert_eq!(output, [0.367, 1.]);
        ThresholdAlpha { value: 0. }.try_eval_pixel(&[0.367, 0.], 1, &mut output).unwrap();
        assert_eq!(output, [0.367, 0.]);
        ThresholdAlpha { value: 1. }.try_eval_pixel(&[0.367, 1.], 1, &mut output).unwrap();
        assert_eq!(output, [0.367, 0.]);
    }
    #[test]
    fn invalid_input_is_atomic() {
        for (value, input, channels) in [
            (0.5, vec![0.5], 1),
            (0.5, vec![0.5; 3], 2),
            (f64::NAN, vec![0.5; 4], 3),
            (1.001, vec![0.5; 4], 3),
            (0.5, vec![f64::INFINITY, 0.5], 1),
            (0.5, vec![0.5, -0.001], 1),
        ] {
            let mut output = vec![0.9375; input.len()];
            let before = output.clone();
            assert!(ThresholdAlpha { value }.try_eval_pixel(&input, channels, &mut output).is_err());
            assert_eq!(output, before);
        }
    }
}
