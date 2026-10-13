//! Private candidate for the public GEGL Unpremultiply alpha contract.
//! https://gegl.org/operations/gegl-unpremultiply.html
//! Independent public working-format observations qualify direct Float32 division,
//! with zero alpha producing positive-zero colors and retaining the alpha bits.
//! Profile conversion and source/storage admission remain caller-owned.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug, Default)]
pub struct Unpremultiply;

impl Unpremultiply {
    /// Four unassociated linear Float samples, with finite color/unit alpha.
    /// Refuses overflow atomically; observed nonfinite output is not clamped.
    pub fn try_eval_linear_float(self, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        let Some(alpha) = input.get(3).copied() else { return Err(RasterInputError::LengthMismatch) };
        if input.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::InvalidSample);
        }
        let mut staged = [0.0; 4];
        if alpha != 0.0 {
            for (slot, sample) in staged.iter_mut().zip(input).take(3) {
                *slot = *sample / alpha;
                if !slot.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
            }
        }
        let Some(slot) = staged.get_mut(3) else { return Err(RasterInputError::LengthMismatch) };
        *slot = alpha;
        output.copy_from_slice(&staged);
        Ok(())
    }
}
