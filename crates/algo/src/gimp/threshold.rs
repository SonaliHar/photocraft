//! Histogram threshold derived from independent public GIMP response probes.
//! The caller supplies nonlinear profile-encoded unit RGB/Gray. Histogram
//! samples are Float32 even when the drawable stores Double samples.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold {
    /// Value, Red, Green, Blue, Alpha, or Luminance (public enum 0 through 5).
    pub channel: u8,
    pub low: f64,
    pub high: f64,
}
impl Threshold {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if self.channel > 5 || [self.low, self.high].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    pub fn try_eval_pixel(self, input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.validate()?;
        if !matches!(channels, 1 | 3) || !matches!(input.len(), n if n == channels || n == channels + 1) || output.len() != input.len() {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || channels == 1 && matches!(self.channel, 1..=3) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let color = input.get(..channels).ok_or(RasterInputError::LengthMismatch)?;
        let sample = match self.channel {
            0 => color.iter().copied().fold(0.0, f64::max),
            1..=3 => *color.get(usize::from(self.channel - 1)).ok_or(RasterInputError::UnsupportedConversion)?,
            4 => input.get(channels).copied().unwrap_or(1.0),
            5 if channels == 1 => *color.first().ok_or(RasterInputError::LengthMismatch)?,
            5 => color.iter().zip([0.2224884033203125, 0.7169036865234375, 0.06060791015625]).map(|(v, w)| v * w).sum(),
            _ => return Err(RasterInputError::UnsupportedConversion),
        };
        // Conversion occurs after luminance evaluation; rounding each colour
        // before weighting fails independently observed Double boundary probes.
        let sample = f64::from(sample as f32);
        let value = if sample >= self.low && sample <= self.high { 1.0 } else { 0.0 };
        let colors = output.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?;
        colors.fill(value);
        if let Some(alpha) = input.get(channels) {
            *output.get_mut(channels).ok_or(RasterInputError::LengthMismatch)? = *alpha;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_double_luminance_transitions_round_after_weighting() {
        for (rgb, boundary) in [
            ([0.173, 0.367, 0.719], 0.3451712429523468),
            ([0.911, 0.123, 0.419], 0.31626081466674805),
            ([0.017, 0.779, 0.291], 0.5798871517181396),
            ([0.6251, 0.3127, 0.8123], 0.4124850928783417),
            ([0.49999997, 0.50000006, 0.1257], 0.4773145020008087),
            ([0.831, 0.937, 0.053], 0.8598388433456421),
        ] {
            let mut output = [0.5; 3];
            Threshold { channel: 5, low: boundary, high: 1.0 }.try_eval_pixel(&rgb, 3, &mut output).unwrap();
            assert_eq!(output, [1.0; 3]);
            Threshold { channel: 5, low: f64::from_bits(boundary.to_bits() + 1), high: 1.0 }.try_eval_pixel(&rgb, 3, &mut output).unwrap();
            assert_eq!(output, [0.0; 3]);
        }
    }
    #[test]
    fn inclusive_endpoints_float_histogram_alpha_and_reversed_bounds() {
        let operation = Threshold { channel: 0, low: 0.5, high: 0.75 };
        for (sample, expected) in [(0.5 - 2.0_f64.powi(-52), 1.0), (0.5 - 2.0_f64.powi(-25), 0.0), (0.75, 1.0), (1.0, 0.0)] {
            let mut output = [0.0; 2];
            operation.try_eval_pixel(&[sample, -0.0], 1, &mut output).unwrap();
            assert_eq!(output[0], expected);
            assert_eq!(output[1].to_bits(), (-0.0_f64).to_bits());
        }
        let mut output = [0.5; 4];
        Threshold { channel: 4, low: 0.25, high: 0.75 }.try_eval_pixel(&[1.0, 0.0, 0.0, 0.5], 3, &mut output).unwrap();
        assert_eq!(output, [1.0, 1.0, 1.0, 0.5]);
        Threshold { low: 0.75, high: 0.25, ..operation }.try_eval_pixel(&[0.5; 4], 3, &mut output).unwrap();
        assert_eq!(output, [0.0, 0.0, 0.0, 0.5]);
    }
    #[test]
    fn invalid_shapes_samples_and_parameters_are_atomic() {
        let valid = Threshold { channel: 0, low: 0.25, high: 0.75 };
        for (operation, input, channels) in [
            (valid, vec![0.5; 5], 3),
            (valid, vec![0.5; 4], 2),
            (valid, vec![f64::NAN, 0.5], 1),
            (valid, vec![1.001, 0.5], 1),
            (Threshold { channel: 2, ..valid }, vec![0.5; 2], 1),
            (Threshold { channel: 6, ..valid }, vec![0.5; 4], 3),
            (Threshold { low: f64::INFINITY, ..valid }, vec![0.5; 4], 3),
        ] {
            let mut output = vec![0.9375; input.len()];
            let before = output.clone();
            assert!(operation.try_eval_pixel(&input, channels, &mut output).is_err());
            assert_eq!(output, before);
        }
    }
}
