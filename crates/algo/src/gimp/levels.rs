//! Independent channel range/gamma equations; caller supplies the exact filter TRC.
//! Public behavior: https://docs.gimp.org/3.0/en/gimp-tool-levels.html
use photocraft_raster::RasterInputError;

#[derive(Clone, Copy, Debug)]
pub struct LevelsChannel {
    pub low_input: f64,
    pub high_input: f64,
    pub gamma: f64,
    pub low_output: f64,
    pub high_output: f64,
}
impl Default for LevelsChannel {
    fn default() -> Self {
        Self { low_input: 0.0, high_input: 1.0, gamma: 1.0, low_output: 0.0, high_output: 1.0 }
    }
}
impl LevelsChannel {
    fn validate(self) -> Result<(), RasterInputError> {
        if [self.low_input, self.high_input, self.gamma, self.low_output, self.high_output].iter().any(|v| !v.is_finite())
            || self.high_input <= self.low_input
            || !(self.high_input - self.low_input).is_finite()
            || !(0.1..=10.0).contains(&self.gamma)
            || !(self.high_output - self.low_output).is_finite()
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    fn eval(self, input: f64, clamp_input: bool, clamp_output: bool) -> Result<f64, RasterInputError> {
        if self.low_input == 0.0
            && self.high_input == 1.0
            && self.gamma == 1.0
            && self.low_output == 0.0
            && self.high_output == 1.0
            && ((!clamp_input && !clamp_output) || (input.is_finite() && (0.0..=1.0).contains(&input)))
        {
            return Ok(input);
        }
        if !input.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        let mut value = (input - self.low_input) / (self.high_input - self.low_input);
        if clamp_input {
            value = value.clamp(0.0, 1.0);
        }
        if self.gamma != 1.0 && value > 0.0 {
            value = value.powf(1.0 / self.gamma);
        }
        value = value * (self.high_output - self.low_output) + self.low_output;
        if clamp_output {
            value = value.clamp(0.0, 1.0);
        }
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LevelsConfiguration {
    /// Fixed selector order: value, red, green, blue, alpha. No cached proxy inference.
    pub channels: [LevelsChannel; 5],
    pub clamp_input: bool,
    pub clamp_output: bool,
}
impl LevelsConfiguration {
    pub fn validate(self) -> Result<(), RasterInputError> {
        for channel in self.channels {
            channel.validate()?;
        }
        Ok(())
    }
    /// Bounded atomic RGB/Gray math. Alpha is affected only by its explicit selector;
    /// filter working encoding/profile conversion and admission remain caller-owned.
    pub fn try_eval_pixel(self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.validate()?;
        if !matches!(color_channels, 1 | 3) || input.len() != output.len() || !(color_channels..=color_channels + 1).contains(&input.len()) {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut scratch = [0.0; 4];
        let values = scratch.get_mut(..input.len()).ok_or(RasterInputError::LengthMismatch)?;
        values.copy_from_slice(input);
        let value = self.channels.first().copied().ok_or(RasterInputError::LengthMismatch)?;
        for (index, sample) in values.iter_mut().take(color_channels).enumerate() {
            if color_channels == 3 {
                *sample = self.channels.get(index + 1).copied().ok_or(RasterInputError::LengthMismatch)?.eval(*sample, self.clamp_input, self.clamp_output)?;
            }
            *sample = value.eval(*sample, self.clamp_input, self.clamp_output)?;
        }
        if input.len() > color_channels {
            let alpha = values.get_mut(color_channels).ok_or(RasterInputError::LengthMismatch)?;
            *alpha = self.channels.get(4).copied().ok_or(RasterInputError::LengthMismatch)?.eval(*alpha, self.clamp_input, self.clamp_output)?;
        }
        output.copy_from_slice(values);
        Ok(())
    }
}
