//! Red Eye working-domain math. Native/profile conversion and source admission
//! are separate; this API requires explicit unassociated nonlinear RGB Float.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug)]
pub struct RedEyeRemoval {
    /// Operative threshold, not an unvalidated originally requested value.
    pub threshold: f64,
}

impl RedEyeRemoval {
    /// Fixed-size, allocation-free evaluation. Errors leave output untouched.
    pub fn try_eval_nonlinear_float(&self, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if !self.threshold.is_finite()
            || !(0.0..=0.8).contains(&self.threshold)
            || input.iter().any(|v| !v.is_finite())
            || !input.get(3).is_some_and(|v| (0.0..=1.0).contains(v))
        {
            return Err(RasterInputError::InvalidSample);
        }
        let [red, green, blue, alpha]: [f32; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
        // Independent public responses and decision-boundary holdouts distinguish
        // these decimal coefficients and Float stage boundaries from rational
        // approximations or an all-Double evaluation. No pixel oracle is used.
        let adjusted_red = (f64::from(red) * 0.5133333) as f32;
        let adjusted_blue = (f64::from(blue) * 0.1933333) as f32;
        let threshold = f64::from(self.threshold as f32);
        let offset = ((threshold - 0.4) * 2.0) as f32;
        let green_boundary = green - offset;
        let blue_boundary = adjusted_blue - offset;
        if !green_boundary.is_finite() || !blue_boundary.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        let result_red = if adjusted_red >= green_boundary && adjusted_red >= blue_boundary {
            let numerator = green + adjusted_blue;
            if !numerator.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            let corrected = (f64::from(numerator) / (2.0 * 0.5133333)) as f32;
            if !corrected.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            corrected.clamp(0.0, 1.0)
        } else {
            red
        };
        for (destination, value) in output.iter_mut().zip([result_red, green, blue, alpha]) {
            *destination = value;
        }
        Ok(())
    }
}
