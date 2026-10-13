//! Standalone public Channel Mixer foundation; registry and source admission are separate.
//! Public gains define a three-by-three linear remix. Independent off-grid native
//! controls distinguish Double gains/sums and normalization before Float output.
//! https://gegl.org/operations/gegl-channel-mixer.html documents gain bounds.
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug)]
pub struct ChannelMixer {
    pub gains: [[f64; 3]; 3],
    pub preserve_luminosity: bool,
}

impl ChannelMixer {
    /// Four unassociated linear working Float samples. Caller owns profile/transfer,
    /// cancellation, storage, Half policy, and admission; this fixed O(1) operation
    /// stages all output before committing. Disabled identity is caller-owned.
    pub fn try_eval_linear_float(self, input: &[f32], output: &mut [f32]) -> Result<(), RasterInputError> {
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        if self.gains.iter().flatten().any(|v| !v.is_finite() || !(-2.0..=2.0).contains(v))
            || input.iter().any(|v| !v.is_finite())
            || input.get(3).is_none_or(|v| !(0.0..=1.0).contains(v))
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut staged = [0.0_f32; 4];
        for (slot, gains) in staged.iter_mut().take(3).zip(self.gains) {
            let mut dot = 0.0_f64;
            let mut sum = 0.0_f64;
            for (sample, gain) in input.iter().take(3).zip(gains) {
                dot += f64::from(*sample) * gain;
                sum += gain;
            }
            let factor = if self.preserve_luminosity && sum != 0.0 { 1.0 / sum.abs() } else { 1.0 };
            let value = (dot * factor) as f32;
            if !value.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            *slot = value;
        }
        let Some(alpha) = input.get(3) else { return Err(RasterInputError::LengthMismatch) };
        let Some(slot) = staged.get_mut(3) else { return Err(RasterInputError::LengthMismatch) };
        *slot = *alpha;
        output.copy_from_slice(&staged);
        Ok(())
    }
}
