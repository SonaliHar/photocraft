//! Color Overlay point math; source/profile admission remains separate.
use photocraft_raster::RasterInputError;

/// Explicit operation-working Float color. Alpha belongs to the overlay color.
#[derive(Clone, Copy, Debug)]
pub struct ColorOverlay {
    pub color: [f32; 4],
}

impl ColorOverlay {
    /// Evaluate explicit working-domain samples, without profile conversion.
    pub fn try_eval_pixel(&self, input: &[f64], channels: u8, output: &mut [f64]) -> Result<(), RasterInputError> {
        if channels != 3 || input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if self.color.iter().any(|v| !v.is_finite())
            || !(0.0..=1.0).contains(&self.color[3])
            || input.iter().any(|v| !v.is_finite() || !(*v as f32).is_finite())
            || !input.get(3).is_some_and(|v| (0.0..=1.0).contains(v))
        {
            return Err(RasterInputError::InvalidSample);
        }
        // Public behavior: interpolate paint color by its own alpha, while
        // retaining input coverage even when it is zero. Explicit working Float
        // conversion belongs to the caller; this has no implicit gamma/profile.
        let alpha = self.color[3];
        let retained = 1.0 - alpha;
        let mut staged = [0.0f32; 4];
        for ((destination, source), color) in staged.iter_mut().take(3).zip(input).zip(self.color) {
            *destination = (*source as f32) * retained + color * alpha;
        }
        let source_alpha = input.get(3).ok_or(RasterInputError::LengthMismatch)?;
        let output_alpha = staged.get_mut(3).ok_or(RasterInputError::LengthMismatch)?;
        *output_alpha = *source_alpha as f32;
        if staged.iter().any(|v| !v.is_finite()) {
            return Err(RasterInputError::InvalidSample);
        }
        for (destination, value) in output.iter_mut().zip(staged) {
            *destination = f64::from(value);
        }
        Ok(())
    }
}
