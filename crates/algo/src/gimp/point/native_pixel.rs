//! Exact native pixel boundary shared with point::try_apply. No schema/CMS authority.
use super::*;
use photocraft_color::{ColorMode, PixelFormat};
fn changes_alpha(operation: PointOperation) -> bool {
    matches!(
        operation,
        PointOperation::AlphaClipFinitePrecision { clip_low: true, .. }
            | PointOperation::AlphaClipFinitePrecision { clip_high: true, .. }
            | PointOperation::AlphaClipPrecision { clip_low: true, .. }
            | PointOperation::AlphaClipPrecision { clip_high: true, .. }
            | PointOperation::ThresholdAlpha { .. }
            | PointOperation::ChannelMixer { .. }
            | PointOperation::MonoMixerSigned { .. }
            | PointOperation::RedEyeRemoval { .. }
            | PointOperation::ColorOverlay { .. }
            | PointOperation::ColorToAlpha { .. }
            | PointOperation::ComponentExtract { .. }
    )
}
impl PreparedPointOperation {
    /// The unchanged Surface point boundary: RGB/Gray and a real alpha channel
    /// for coverage-changing operations. This does not infer working transfer,
    /// native stock precision, schema, profile, outer state or source admission.
    pub fn validate_native_format(&self, format: PixelFormat) -> Result<(), RasterInputError> {
        if changes_alpha(self.operation) && !format.alpha {
            return Err(RasterInputError::UnsupportedConversion);
        }
        if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(())
    }
    /// Native-endian pixel application of the existing Surface equation/write
    /// schedule. Color-only alpha bytes and alpha-only color bytes remain raw.
    /// Exact identity copies the complete raw word; it does not normalize -0/NaN.
    /// Output is unchanged on failure. No position-dependent variant is enrolled.
    pub fn try_eval_native_pixel(&self, input: &[u8], format: PixelFormat, output: &mut [u8]) -> Result<(), RasterInputError> {
        self.validate_native_format(format)?;
        let bpp = format.bytes_per_pixel();
        if bpp == 0 || input.len() != bpp || output.len() != bpp {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut native = [0u8; 40];
        let pixel = native.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        pixel.copy_from_slice(input);
        if self.operation.identity() {
            output.copy_from_slice(pixel);
            return Ok(());
        }
        let colors = format.mode.color_channels();
        let mut values = [0.0; 4];
        for (channel, value) in values.iter_mut().take(colors).enumerate() {
            *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        }
        let alpha = changes_alpha(self.operation);
        let count = if alpha {
            *values.get_mut(colors).ok_or(RasterInputError::LengthMismatch)? =
                try_read_sample_f64(pixel, format.sample, colors).map_err(|_| RasterInputError::InvalidSample)?;
            colors.checked_add(1).ok_or(RasterInputError::LengthMismatch)?
        } else {
            colors
        };
        let mut evaluated = [0.0; 4];
        self.try_eval_pixel(
            values.get(..count).ok_or(RasterInputError::LengthMismatch)?,
            colors,
            evaluated.get_mut(..count).ok_or(RasterInputError::LengthMismatch)?,
        )?;
        if alpha {
            try_write_sample_f64(pixel, format.sample, colors, *evaluated.get(colors).ok_or(RasterInputError::LengthMismatch)?)
                .map_err(|_| RasterInputError::InvalidSample)?;
        }
        if !alpha
            || matches!(
                self.operation,
                PointOperation::ColorOverlay { .. }
                    | PointOperation::ChannelMixer { .. }
                    | PointOperation::MonoMixerSigned { .. }
                    | PointOperation::RedEyeRemoval { .. }
                    | PointOperation::ColorToAlpha { .. }
                    | PointOperation::ComponentExtract { .. }
            )
        {
            for (channel, value) in evaluated.iter().take(colors).enumerate() {
                try_write_sample_f64(pixel, format.sample, channel, *value).map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
        output.copy_from_slice(pixel);
        Ok(())
    }
}

#[cfg(test)]
#[path = "native_pixel_tests.rs"]
mod tests;
