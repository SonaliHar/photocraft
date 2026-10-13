//! Linear working Absolute operation, implemented from the public GEGL contract.
//! Source/current profile, precision, cancellation and storage remain caller-owned.
//! https://gegl.org/operations/gegl-absolute.html
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, Default)]
pub struct Absolute;

impl Absolute {
    /// Four unassociated linear Float samples; operative alpha remains unchanged.
    pub fn try_eval_linear_float(self, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if input.iter().any(|v| !v.is_finite()) || input.get(3).is_none_or(|v| !(0.0..=1.0).contains(v)) {
            return Err(RasterInputError::InvalidSample);
        }
        let mut staged = [0.0; 4];
        for (slot, sample) in staged.iter_mut().zip(input).take(3) {
            *slot = sample.abs();
        }
        let Some(alpha) = input.get(3) else { return Err(RasterInputError::LengthMismatch) };
        let Some(slot) = staged.get_mut(3) else { return Err(RasterInputError::LengthMismatch) };
        *slot = *alpha;
        output.copy_from_slice(&staged);
        Ok(())
    }
}
