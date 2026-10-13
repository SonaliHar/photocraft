//! Clean-room nonlinear Float32 RGB background subtraction.
//! Public cube, off-grid and representable-neighbor observations distinguish
//! the threshold boundary from the comparison tolerance. The stable color
//! expression retains an independently measured unassociated-RGB delta from
//! stock cancellation near zero coverage; premultiplied output is the oracle.
//! This kernel alone grants no import, profile or mask admission.
use photocraft_raster::RasterInputError;
#[derive(Clone, Copy, Debug)]
pub struct ColorToAlpha {
    pub color: [f32; 4],
    pub transparency: f32,
    pub opacity: f32,
}
impl ColorToAlpha {
    pub fn validate(self) -> Result<(), RasterInputError> {
        if self.color.into_iter().chain([self.transparency, self.opacity]).all(|v| v.is_finite() && (0.0..=1.0).contains(&v)) {
            Ok(())
        } else {
            Err(RasterInputError::UnsupportedConversion)
        }
    }
    pub fn try_eval_pixel(self, input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.validate()?;
        if channels != 3 || input.len() != 4 || output.len() != 4 || input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut staged = [0f32; 4];
        for (dst, src) in staged.iter_mut().zip(input) {
            *dst = *src as f32;
        }
        let mut coverage = 0f32;
        let mut original_fraction = 0f32;
        for (value, color) in staged.iter().take(3).zip(self.color) {
            let distance = (*value - color).abs();
            let endpoint = if *value > color { 1. - color } else { color };
            let span = self.opacity.min(endpoint);
            let component = if f64::from(distance) <= f64::from(self.transparency) + 1e-5 {
                0.
            } else if f64::from(distance) >= f64::from(span) - 1e-5 || span <= self.transparency {
                1.
            } else {
                ((distance - self.transparency) / (span - self.transparency)).clamp(0., 1.)
            };
            // Independently tied-axis controls retain the first maximal channel.
            if component > coverage {
                coverage = component;
                original_fraction = if span > 0. { distance / span } else { 0. };
            }
        }
        if coverage > 0. && coverage < 1. {
            if !original_fraction.is_finite() || original_fraction <= 0. {
                return Err(RasterInputError::UnsupportedConversion);
            }
            for (value, color) in staged.iter_mut().take(3).zip(self.color) {
                *value = (*value - color) / original_fraction + color;
            }
        }
        let alpha = staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)?;
        *alpha *= coverage;
        if staged.iter().any(|value| !value.is_finite()) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        for (dst, value) in output.iter_mut().zip(staged) {
            *dst = f64::from(value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_operands_and_pixels_leave_output_unchanged() {
        let base = ColorToAlpha { color: [0.375, 0.5, 0.625, 0.75], transparency: 0.125, opacity: 0.5 };
        for value in [f32::NAN, f32::INFINITY, -0.001, 1.001] {
            for field in 0..6 {
                let mut operation = base;
                match field {
                    0..=3 => operation.color[field] = value,
                    4 => operation.transparency = value,
                    _ => operation.opacity = value,
                }
                let mut output = [17.; 4];
                assert!(operation.try_eval_pixel(&[0.25, 0.5, 0.75, 0.5], 3, &mut output).is_err());
                assert_eq!(output, [17.; 4]);
            }
        }
        for value in [f64::NAN, f64::INFINITY, -0.001, 1.001] {
            for field in 0..4 {
                let mut input = [0.25, 0.5, 0.75, 0.5];
                input[field] = value;
                let mut output = [17.; 4];
                assert!(base.try_eval_pixel(&input, 3, &mut output).is_err());
                assert_eq!(output, [17.; 4]);
            }
        }
        let mut short = [17.; 3];
        assert!(base.try_eval_pixel(&[0.25, 0.5, 0.75, 0.5], 3, &mut short).is_err());
        assert_eq!(short, [17.; 3]);
        let mut output = [17.; 4];
        assert!(base.try_eval_pixel(&[0.25, 0.5], 1, &mut output).is_err());
        assert_eq!(output, [17.; 4]);
    }
    #[test]
    fn literal_alpha_is_inert_and_source_alpha_is_operative() {
        let base = ColorToAlpha { color: [0.375, 0.5, 0.625, 0.], transparency: 0., opacity: 1. };
        let mut first = [0.; 4];
        let mut second = [0.; 4];
        base.try_eval_pixel(&[0.5, 0.6, 0.75, 0.5], 3, &mut first).unwrap();
        ColorToAlpha { color: [0.375, 0.5, 0.625, 1.], ..base }.try_eval_pixel(&[0.5, 0.6, 0.75, 0.5], 3, &mut second).unwrap();
        assert_eq!(first, second);
        assert!(first[3] > 0. && first[3] < 0.5);
        base.try_eval_pixel(&[0.5, 0.6, 0.75, 0.], 3, &mut second).unwrap();
        assert_eq!(second[3], 0.);
        base.try_eval_pixel(&[0.375, 0.5, 0.625, 0.5], 3, &mut second).unwrap();
        assert_eq!(second[3], 0.);
    }
}
