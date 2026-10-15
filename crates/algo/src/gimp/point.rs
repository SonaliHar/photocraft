//! Checked point equations from public operation descriptions; profile conversion is caller-owned.
//! https://gegl.org/operations/gegl-brightness-contrast.html specifies scale around
//! 50% gray followed by a constant offset. Exposure uses the documented stop scale
//! https://gegl.org/operations/gegl-exposure.html documents stops and black-level domains.
//! Independent stock native black/white endpoint witnesses qualify affine remapping
//! from black-level to exp2(-stops), only with a positive finite denominator.
//! https://gegl.org/operations/gegl-levels.html defines two input/output endpoints;
//! the unique affine remap is derived from those endpoints without implicit clipping.
//! https://gegl.org/operations/gegl-value-invert.html describes retaining color
//! while inverting brightness; independent GIMP unit-RGB observations qualify
//! scaling RGB ratios from HSV value max(R,G,B) to 1-max(R,G,B).
//! https://gegl.org/operations/gegl-rgb-clip.html documents component limits;
//! independent finite HDR linear RGB probes qualify the closed high-enabled slice.
use photocraft_color::{try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};
#[path = "point_alien_hsl.rs"]
mod alien_hsl;
#[path = "point_color_exchange.rs"]
mod color_exchange;
#[path = "point_component_extract.rs"]
mod component_extract;
#[path = "point_saturation.rs"]
mod saturation;
#[path = "point_temperature.rs"]
mod temperature;
#[path = "point/native_pixel.rs"]
mod native_pixel;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointOperation {
    /// Explicit operation-working Float color. Caller retains and converts the
    /// original nonlinear literal separately, and selects working encoding.
    ColorOverlay {
        color: [f32; 4],
    },
    /// Operative clipping narrows qualified working RGBA to Float32.
    AlphaClipPrecision {
        clip_low: bool,
        low_limit: f32,
        clip_high: bool,
        high_limit: f32,
    },
    /// Finite thresholds with independently observed high-comparison precedence.
    AlphaClipFinitePrecision {
        clip_low: bool,
        low_limit: f32,
        clip_high: bool,
        high_limit: f32,
    },
    /// Eleven measured RGB/alpha, CMYK and YCbCr selectors, producing opaque output.
    ComponentExtract {
        component: u8,
        invert: bool,
        linear: bool,
    },
    /// Nonlinear Float32 RGB literal-background removal; output alpha is operative.
    ColorToAlpha {
        color: [f32; 4],
        transparency: f32,
        opacity: f32,
    },
    /// Exact nonlinear literal colors; all four operand bits are retained,
    /// while only RGB participates in the independently measured equation.
    ColorExchange {
        from: [f32; 4],
        to: [f32; 4],
        thresholds: [f32; 3],
    },
    /// Independently observed public nonlinear RGB trigonometric mapping.
    AlienMap {
        frequency: [f64; 3],
        phase: [f64; 3],
        keep: [bool; 3],
    },
    /// Independently observed HSL conversion, component mapping and RGB return.
    AlienMapHsl {
        frequency: [f64; 3],
        phase: [f64; 3],
        keep: [bool; 3],
    },
    /// Independently measured RGB matrix; caller chooses exact observed encoding.
    Sepia {
        scale: f64,
        srgb: bool,
    },
    /// Alpha-only strict Float32 sample comparison; source colors remain exact.
    ThresholdAlpha {
        value: f64,
    },
    /// Checked public histogram threshold; nonlinear RGB/Gray caller input.
    Threshold {
        channel: u8,
        low: f64,
        high: f64,
    },
    /// Independently observed implicit linear RGB saturation; arbitrary profiles remain unqualified.
    Saturation {
        scale: f64,
        colorspace: u8,
    },
    /// Public GIMP DesaturateMode: Lightness, Luma, Average, Luminance, Value.
    /// Caller supplies linear RGB for Luminance and nonlinear RGB otherwise.
    /// Finite unit RGB only; Gray/HDR/profile generalization is not admitted.
    Desaturate {
        mode: u8,
    },
    /// Independently observed implicit RGB/Gray linear temperature adjustment.
    /// Kelvin parameters are continuous; near-pole amplification and HDR inputs
    /// remain outside the bounded observed evaluator domain.
    ColorTemperature {
        original: f64,
        intended: f64,
    },
    /// Qualified linear Float absolute value; transfer conversion is caller-owned.
    AbsoluteLinear,
    /// Independently observed direct Float32 division in linear RGBA.
    UnpremultiplyLinear,
    /// Caller must explicitly supply drawable-local coordinates and retain
    /// geometry qualification. The coordinate-free evaluator refuses this family.
    VideoDegradation {
        operation: super::point_video_degradation::VideoDegradation,
    },
    Invert,
    /// Invert encoded HSV value while retaining the RGB ratios. Only finite
    /// unit-range RGB is independently qualified; Gray/HDR remain unsupported.
    ValueInvert,
    ColorRotate {
        operation: super::point_color_rotate::ColorRotate,
    },
    RedEyeRemoval {
        threshold: f64,
    },
    /// Independently observed linear RGB mixer, preserve-luminosity=false.
    /// Nonnegative weights with sum <= 1 and unit RGB only in this slice.
    ChannelMixer {
        gains: [[f64; 3]; 3],
        preserve_luminosity: bool,
    },
    MonoMixerSigned {
        red: f64,
        green: f64,
        blue: f64,
        preserve_luminosity: bool,
    },
    MonoMixer {
        red: f64,
        green: f64,
        blue: f64,
    },
    /// Independently observed finite linear RGB clipping, high clipping enabled.
    /// Disabling high clipping and Gray remain outside this qualified slice.
    RgbClip {
        clip_low: bool,
        low_limit: f64,
        clip_high: bool,
        high_limit: f64,
    },
    /// GIMP posterize: nearest of evenly spaced encoded unit-range levels.
    /// HDR/out-of-range behavior is not admitted by this checked slice.
    Posterize {
        levels: u16,
    },
    Exposure {
        stops: f64,
        black_level: f64,
    },
    BrightnessContrast {
        brightness: f64,
        contrast: f64,
    },
    /// Affine range remap; no gamma or implicit clipping.
    Levels {
        input_low: f64,
        input_high: f64,
        output_low: f64,
        output_high: f64,
    },
}

/// Checked immutable preparation; coefficients are calculated once per job,
/// while each pixel still validates shape and touched sample domains.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPointOperation {
    operation: PointOperation,
    temperature_gains: Option<[f64; 3]>,
}
impl PreparedPointOperation {
    pub fn try_eval_pixel_at(&self, position: (i32, i32), input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        if matches!(self.operation, PointOperation::VideoDegradation { .. }) {
            self.operation.try_eval_pixel_at(position, input, color_channels, output)
        } else {
            self.try_eval_pixel(input, color_channels, output)
        }
    }
    pub fn try_eval_pixel(&self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.operation.eval_prepared(input, color_channels, output, self.temperature_gains)
    }
    /// Independently observed live integer-Gray temperature boundary: clamp
    /// the three working RGB components before reducing them to luminance.
    /// Baked Gray storage follows a different boundary and uses `try_eval_pixel`.
    pub fn try_eval_integer_gray_temperature(&self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        if !matches!(self.operation, PointOperation::ColorTemperature { .. }) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        if output.len() != 2 {
            return Err(RasterInputError::LengthMismatch);
        }
        let alpha = *input.get(1).ok_or(RasterInputError::LengthMismatch)?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut staged = [0.0; 2];
        self.try_eval_pixel(input, 1, &mut staged)?;
        let gray = *input.first().ok_or(RasterInputError::LengthMismatch)?;
        let gains = self.temperature_gains.ok_or(RasterInputError::UnsupportedConversion)?;
        staged[0] = gains.iter().zip(temperature::GRAY_WEIGHTS).map(|(gain, weight)| f64::from((gray as f32) * (*gain as f32)).clamp(0.0, 1.0) * weight).sum();
        output.copy_from_slice(&staged);
        Ok(())
    }
}

impl PointOperation {
    /// Explicit drawable-local phase; source/world geometry is caller-owned.
    pub fn try_eval_pixel_at(self, position: (i32, i32), input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        if let Self::VideoDegradation { operation } = self {
            if color_channels != 3 || input.len() != 4 || output.len() != 4 {
                return Err(RasterInputError::LengthMismatch);
            }
            if input.iter().any(|value| !value.is_finite() || f64::from(*value as f32) != *value) {
                return Err(RasterInputError::InvalidSample);
            }
            let mut native = [0.; 4];
            for (target, value) in native.iter_mut().zip(input) {
                *target = *value as f32;
            }
            let mut result = [0.; 4];
            operation.try_eval_nonlinear_float(position.0, position.1, &native, &mut result)?;
            for (target, value) in output.iter_mut().zip(result) {
                *target = f64::from(value);
            }
            return Ok(());
        }
        self.try_eval_pixel(input, color_channels, output)
    }
    /// Independently qualified RGB float Yuv route; double and Gray semantics
    /// require separate source/current admission. Failure leaves output intact.
    pub fn try_eval_float_yuv_pixel(self, input: &[f64], output: &mut [f64]) -> Result<(), RasterInputError> {
        let Self::Saturation { scale, colorspace: 2 } = self else { return Err(RasterInputError::UnsupportedConversion) };
        if input.len() != 4 || output.len() != 4 {
            return Err(RasterInputError::LengthMismatch);
        }
        let alpha = *input.get(3).ok_or(RasterInputError::LengthMismatch)?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let color = saturation::eval_float_yuv(
            [
                *input.first().ok_or(RasterInputError::LengthMismatch)?,
                *input.get(1).ok_or(RasterInputError::LengthMismatch)?,
                *input.get(2).ok_or(RasterInputError::LengthMismatch)?,
            ],
            scale,
        )?;
        output.copy_from_slice(&[color[0], color[1], color[2], alpha]);
        Ok(())
    }
    pub fn prepare(self) -> Result<PreparedPointOperation, RasterInputError> {
        self.validate()?;
        let temperature_gains = if let Self::ColorTemperature { original, intended } = self { Some(temperature::gains(original, intended)?) } else { None };
        Ok(PreparedPointOperation { operation: self, temperature_gains })
    }
    /// Bounded per-pixel ordered-stack evaluator. Caller supplies the member's
    /// working encoding. Alpha changes only for explicit coverage operations; output is atomic on failure.
    pub fn try_eval_pixel(self, input: &[f64], color_channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
        self.prepare()?.try_eval_pixel(input, color_channels, output)
    }
    fn eval_prepared(self, input: &[f64], color_channels: usize, output: &mut [f64], temperature_gains: Option<[f64; 3]>) -> Result<(), RasterInputError> {
        if !matches!(color_channels, 1 | 3) || input.len() != output.len() || !(color_channels..=color_channels + 1).contains(&input.len()) {
            return Err(RasterInputError::LengthMismatch);
        }
        if let Self::AlphaClipFinitePrecision { clip_low, low_limit, clip_high, high_limit } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            return super::alpha_clip_finite::AlphaClipFinite { clip_low, low_limit, clip_high, high_limit }.try_eval_pixel(input, output);
        }
        if let Self::AlphaClipPrecision { clip_low, low_limit, clip_high, high_limit } = self {
            return super::alpha_clip::AlphaClip { clip_low, low_limit, clip_high, high_limit }.try_eval_pixel(input, color_channels, output);
        }
        if let Self::ColorToAlpha { color, transparency, opacity } = self {
            return super::color_to_alpha::ColorToAlpha { color, transparency, opacity }.try_eval_pixel(input, color_channels, output);
        }
        if let Self::ColorRotate { operation } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let rgba = [
                *input.first().ok_or(RasterInputError::LengthMismatch)? as f32,
                *input.get(1).ok_or(RasterInputError::LengthMismatch)? as f32,
                *input.get(2).ok_or(RasterInputError::LengthMismatch)? as f32,
                input.get(3).copied().unwrap_or(1.0) as f32,
            ];
            let mut result = [0.; 4];
            operation.try_eval_perceptual_float(&rgba, &mut result)?;
            for (destination, value) in output.iter_mut().zip(result) {
                *destination = f64::from(value);
            }
            return Ok(());
        }
        if let Self::UnpremultiplyLinear = self {
            if color_channels != 3 || input.len() != 4 || output.len() != 4 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let input: [f64; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
            let mut result = [0.; 4];
            super::point_unpremultiply::Unpremultiply.try_eval_linear_float(&input.map(|v| v as f32), &mut result)?;
            for (destination, value) in output.iter_mut().zip(result) { *destination = f64::from(value); }
            return Ok(());
        }
        if let Self::AbsoluteLinear = self {
            let alpha = *input.get(color_channels).ok_or(RasterInputError::LengthMismatch)?;
            let red = *input.first().ok_or(RasterInputError::LengthMismatch)?;
            let rgba = if color_channels == 1 { [red, red, red, alpha] } else { input.try_into().map_err(|_| RasterInputError::LengthMismatch)? };
            let mut result = [0f32; 4];
            super::point_absolute::Absolute.try_eval_linear_float(&rgba.map(|v| v as f32), &mut result)?;
            if color_channels == 1 {
                output.copy_from_slice(&[f64::from(result[0]), f64::from(result[3])]);
            } else {
                for (destination, value) in output.iter_mut().zip(result) {
                    *destination = f64::from(value);
                }
            }
            return Ok(());
        }
        if let Self::RedEyeRemoval { threshold } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let rgba: [f64; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
            let mut result = [0f32; 4];
            super::point_red_eye::RedEyeRemoval { threshold }.try_eval_nonlinear_float(&rgba.map(|v| v as f32), &mut result)?;
            for (destination, value) in output.iter_mut().zip(result) {
                *destination = f64::from(value);
            }
            return Ok(());
        }
        if let Self::ChannelMixer { gains, preserve_luminosity } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let rgba: [f64; 4] = input.try_into().map_err(|_| RasterInputError::LengthMismatch)?;
            let mut result = [0f32; 4];
            super::point_channel_mixer::ChannelMixer { gains, preserve_luminosity }.try_eval_linear_float(&rgba.map(|v| v as f32), &mut result)?;
            for (destination, value) in output.iter_mut().zip(result) {
                *destination = f64::from(value);
            }
            return Ok(());
        }
        if let Self::MonoMixerSigned { red, green, blue, preserve_luminosity } = self {
            let alpha = *input.get(color_channels).ok_or(RasterInputError::LengthMismatch)?;
            let red_input = *input.first().ok_or(RasterInputError::LengthMismatch)?;
            let rgba = if color_channels == 1 {
                [red_input, red_input, red_input, alpha]
            } else {
                [red_input, *input.get(1).ok_or(RasterInputError::LengthMismatch)?, *input.get(2).ok_or(RasterInputError::LengthMismatch)?, alpha]
            };
            let mut result = [0.; 4];
            super::point_mono_mixer::MonoMixer { red, green, blue, preserve_luminosity }.try_eval_pixel_with(
                &rgba,
                &mut result,
                true,
                &Interrupt::NONE,
                &|_| Ok(()),
            )?;
            if color_channels == 1 {
                let staged = [result[0], result[3]];
                output.copy_from_slice(&staged);
            } else {
                output.copy_from_slice(&result);
            }
            return Ok(());
        }
        if let Self::ColorOverlay { color } = self {
            return super::point_color_overlay::ColorOverlay { color }.try_eval_pixel(input, color_channels as u8, output);
        }
        if let Self::ComponentExtract { component, invert, linear } = self {
            return component_extract::eval(component, invert, linear, input, color_channels, output);
        }
        if let Self::ColorExchange { from, to, thresholds } = self {
            return color_exchange::eval(from, to, thresholds, input, color_channels, output);
        }
        if let Self::Threshold { channel, low, high } = self {
            return super::threshold::Threshold { channel, low, high }.try_eval_pixel(input, color_channels, output);
        }
        if let Self::ThresholdAlpha { value } = self {
            return super::threshold_alpha::ThresholdAlpha { value }.try_eval_pixel(input, color_channels, output);
        }
        if matches!(self, Self::Saturation { .. }) && color_channels == 1 {
            let value = input.first().ok_or(RasterInputError::LengthMismatch)?;
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            // Independent Gray ramps for every public interpolation choice
            // qualify identity display semantics. Current native bits stay exact;
            // this does not manufacture GIMP's tiny bake-conversion rounding.
            output.copy_from_slice(input);
            return Ok(());
        }
        if matches!(self, Self::RgbClip { .. }) && color_channels != 3 {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut scratch = [0.0; 4];
        let values = scratch.get_mut(..input.len()).ok_or(RasterInputError::LengthMismatch)?;
        values.copy_from_slice(input);
        if matches!(self, Self::ColorTemperature { .. }) {
            let gains = temperature_gains.ok_or(RasterInputError::UnsupportedConversion)?;
            let colors = values.get_mut(..color_channels).ok_or(RasterInputError::LengthMismatch)?;
            if colors.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if color_channels == 3 {
                for (value, gain) in colors.iter_mut().zip(gains) {
                    *value = f64::from((*value as f32) * (gain as f32));
                }
            } else {
                let gain = gains.iter().zip(temperature::GRAY_WEIGHTS).map(|(gain, weight)| gain * weight).sum::<f64>();
                for value in colors {
                    *value = f64::from(*value as f32) * gain;
                }
            }
        } else if let Self::AlienMap { frequency, phase, keep } = self {
            if color_channels != 3 || input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            for (((value, frequency), phase), keep) in colors.iter_mut().zip(frequency).zip(phase).zip(keep) {
                if !keep {
                    let angle = (2.0 * f64::from(*value as f32) - 1.0) * std::f64::consts::PI * frequency + phase.to_radians();
                    *value = f64::from(((1.0 + angle.sin()) * 0.5) as f32);
                }
            }
        } else if let Self::AlienMapHsl { frequency, phase, keep } = self {
            if color_channels != 3 || input.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            let rgb = [
                *colors.first().ok_or(RasterInputError::LengthMismatch)?,
                *colors.get(1).ok_or(RasterInputError::LengthMismatch)?,
                *colors.get(2).ok_or(RasterInputError::LengthMismatch)?,
            ];
            let result = alien_hsl::map(rgb, frequency, phase, keep);
            colors.copy_from_slice(&result);
        } else if let Self::Sepia { scale, .. } = self {
            if input.get(3).is_some_and(|alpha| !alpha.is_finite() || !(0.0..=1.0).contains(alpha)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            if colors.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            // Exact independently observed scale-one linear RGB primary bits.
            // Public 125-color holdouts and five coupled scales qualify this matrix.
            let matrix = [[0x3ec9374c, 0x3f44dd2f, 0x3e418937], [0x3eb2b021, 0x3f2f9db2, 0x3e2c0831], [0x3e8b4396, 0x3f08b439, 0x3e0624dd]];
            let rgb = [
                *colors.first().ok_or(RasterInputError::LengthMismatch)? as f32,
                *colors.get(1).ok_or(RasterInputError::LengthMismatch)? as f32,
                *colors.get(2).ok_or(RasterInputError::LengthMismatch)? as f32,
            ];
            for (index, (value, row)) in colors.iter_mut().zip(matrix).enumerate() {
                let transformed = rgb.into_iter().zip(row).map(|(v, bits)| v * f32::from_bits(bits)).sum::<f32>();
                let original = *rgb.get(index).ok_or(RasterInputError::LengthMismatch)?;
                *value = f64::from(original * (1.0 - scale as f32) + transformed * scale as f32);
            }
        } else if let Self::Saturation { scale, colorspace } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            if colors.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if colorspace == 0 {
                let r = *colors.first().ok_or(RasterInputError::LengthMismatch)? as f32;
                let g = *colors.get(1).ok_or(RasterInputError::LengthMismatch)? as f32;
                let b = *colors.get(2).ok_or(RasterInputError::LengthMismatch)? as f32;
                let gray = r * f32::from_bits(0x3e63d3e1) + g * f32::from_bits(0x3f378703) + b * f32::from_bits(0x3d78405d);
                for value in colors {
                    *value = f64::from(gray + (*value as f32 - gray) * scale as f32);
                }
            } else {
                let rgb = [
                    *colors.first().ok_or(RasterInputError::LengthMismatch)?,
                    *colors.get(1).ok_or(RasterInputError::LengthMismatch)?,
                    *colors.get(2).ok_or(RasterInputError::LengthMismatch)?,
                ];
                colors.copy_from_slice(&saturation::eval(rgb, scale, colorspace)?);
            }
        } else if let Self::Desaturate { mode } = self {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            if colors.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            // Actual independent RGB primary witnesses identify these D50 weights;
            // native operation arithmetic is f32 even for DOUBLE source storage.
            let r = *colors.first().ok_or(RasterInputError::LengthMismatch)? as f32;
            let g = *colors.get(1).ok_or(RasterInputError::LengthMismatch)? as f32;
            let b = *colors.get(2).ok_or(RasterInputError::LengthMismatch)? as f32;
            let gray = match mode {
                0 => (r.max(g).max(b) + r.min(g).min(b)) * 0.5,
                1 | 3 => r * f32::from_bits(0x3e63d3e1) + g * f32::from_bits(0x3f378703) + b * f32::from_bits(0x3d78405d),
                2 => (r + g + b) / 3.0,
                4 => r.max(g).max(b),
                _ => return Err(RasterInputError::UnsupportedConversion),
            };
            colors.fill(f64::from(gray));
        } else if matches!(self, Self::ValueInvert | Self::MonoMixer { .. }) {
            if color_channels != 3 {
                return Err(RasterInputError::UnsupportedConversion);
            }
            let colors = values.get_mut(..3).ok_or(RasterInputError::LengthMismatch)?;
            if colors.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                return Err(RasterInputError::UnsupportedConversion);
            }
            if let Self::MonoMixer { red, green, blue } = self {
                let gray = colors.iter().zip([red, green, blue]).map(|(value, weight)| value * weight).sum();
                colors.fill(gray);
            } else {
                let maximum = colors.iter().copied().fold(0.0_f64, f64::max);
                if maximum == 0.0 {
                    colors.fill(1.0);
                } else {
                    for value in colors {
                        *value = (*value / maximum) * (1.0 - maximum);
                    }
                }
            }
        } else if !self.identity() {
            for value in values.iter_mut().take(color_channels) {
                *value = self.eval_color(*value)?;
            }
        }
        output.copy_from_slice(values);
        Ok(())
    }

    fn eval_color(self, value: f64) -> Result<f64, RasterInputError> {
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        let result = match self {
            Self::Invert => 1.0 - value,
            Self::AbsoluteLinear | Self::UnpremultiplyLinear | Self::VideoDegradation { .. } => return Err(RasterInputError::UnsupportedConversion),
            Self::AlphaClipPrecision { .. }
            | Self::AlphaClipFinitePrecision { .. }
            | Self::ComponentExtract { .. }
            | Self::ColorToAlpha { .. }
            | Self::ColorOverlay { .. }
            | Self::ColorExchange { .. }
            | Self::Threshold { .. }
            | Self::ThresholdAlpha { .. }
            | Self::Saturation { .. }
            | Self::Sepia { .. }
            | Self::AlienMap { .. }
            | Self::AlienMapHsl { .. }
            | Self::Desaturate { .. }
            | Self::ValueInvert
            | Self::ChannelMixer { .. }
            | Self::MonoMixerSigned { .. }
            | Self::ColorRotate { .. }
            | Self::RedEyeRemoval { .. }
            | Self::MonoMixer { .. }
            | Self::ColorTemperature { .. } => {
                return Err(RasterInputError::UnsupportedConversion);
            }
            Self::RgbClip { clip_low, low_limit, high_limit, .. } => {
                if clip_low && value < low_limit {
                    low_limit
                } else if value > high_limit {
                    high_limit
                } else {
                    value
                }
            }
            Self::Posterize { levels } => {
                if !(0.0..=1.0).contains(&value) {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                let steps = f64::from(levels.checked_sub(1).filter(|steps| *steps != 0).ok_or(RasterInputError::UnsupportedConversion)?);
                (value * steps + 0.5).floor() / steps
            }
            Self::Exposure { stops, black_level } => {
                if black_level == 0.0 {
                    value * stops.exp2()
                } else {
                    (value - black_level) / ((-stops).exp2() - black_level)
                }
            }
            Self::BrightnessContrast { brightness, contrast } => (value - 0.5) * contrast + 0.5 + brightness,
            Self::Levels { input_low, input_high, output_low, output_high } => {
                (value - input_low) / (input_high - input_low) * (output_high - output_low) + output_low
            }
        };
        if !result.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        Ok(result)
    }

    pub fn validate(self) -> Result<(), RasterInputError> {
        match self {
            Self::AlphaClipFinitePrecision { clip_low, low_limit, clip_high, high_limit } => {
                super::alpha_clip_finite::AlphaClipFinite { clip_low, low_limit, clip_high, high_limit }.validate()
            }
            Self::AlphaClipPrecision { clip_low, low_limit, clip_high, high_limit } => {
                super::alpha_clip::AlphaClip { clip_low, low_limit, clip_high, high_limit }.validate()
            }
            Self::ComponentExtract { component, .. } => component_extract::validate(component),
            Self::ColorToAlpha { color, transparency, opacity } => super::color_to_alpha::ColorToAlpha { color, transparency, opacity }.validate(),
            Self::ColorOverlay { color } => {
                if color.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                    Err(RasterInputError::InvalidSample)
                } else {
                    Ok(())
                }
            }
            Self::ColorExchange { from, to, thresholds } => color_exchange::validate(from, to, thresholds),
            Self::AlienMap { frequency, phase, .. } | Self::AlienMapHsl { frequency, phase, .. }
                if frequency.iter().all(|v| v.is_finite() && (0.0..=20.0).contains(v)) && phase.iter().all(|v| v.is_finite() && (0.0..=360.0).contains(v)) =>
            {
                Ok(())
            }
            Self::ThresholdAlpha { value } => super::threshold_alpha::ThresholdAlpha { value }.validate(),
            Self::Threshold { channel, low, high } => super::threshold::Threshold { channel, low, high }.validate(),
            Self::Sepia { scale, .. } if scale.is_finite() && (0.0..=1.0).contains(&scale) => Ok(()),
            Self::Saturation { scale, colorspace } if colorspace <= 2 && scale.is_finite() && (0.0..=10.0).contains(&scale) => Ok(()),
            Self::Desaturate { mode } if mode <= 4 => Ok(()),
            Self::ColorTemperature { original, intended } => temperature::gains(original, intended).map(|_| ()),
            Self::Invert | Self::AbsoluteLinear | Self::UnpremultiplyLinear | Self::ValueInvert => Ok(()),
            Self::VideoDegradation { operation } => operation.try_eval_nonlinear_float(0, 0, &[0.; 4], &mut [0.; 4]),
            Self::ColorRotate { operation } => operation.validate(),
            Self::RedEyeRemoval { threshold } if threshold.is_finite() && (0.0..=0.8).contains(&threshold) => Ok(()),
            Self::ChannelMixer { gains, preserve_luminosity } => {
                super::point_channel_mixer::ChannelMixer { gains, preserve_luminosity }.try_eval_linear_float(&[0.; 4], &mut [0.; 4])
            }
            Self::MonoMixerSigned { red, green, blue, preserve_luminosity } => {
                super::point_mono_mixer::MonoMixer { red, green, blue, preserve_luminosity }.validate()
            }
            Self::MonoMixer { red, green, blue }
                if [red, green, blue].iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)) && red + green + blue <= 1.0 =>
            {
                Ok(())
            }
            Self::RgbClip { low_limit, clip_high: true, high_limit, .. }
                if low_limit.is_finite()
                    && high_limit.is_finite()
                    && (0.0..=1.0).contains(&low_limit)
                    && (0.0..=1.0).contains(&high_limit)
                    && low_limit <= high_limit =>
            {
                Ok(())
            }
            Self::Posterize { levels } if (2..=256).contains(&levels) => Ok(()),
            // Independent stock originals qualify positive finite white-minus-black denominators.
            Self::Exposure { stops, black_level }
                if stops.is_finite()
                    && black_level.is_finite()
                    && (-0.1..=0.1).contains(&black_level)
                    && ((black_level == 0.0 && stops.exp2().is_finite())
                        || (black_level != 0.0 && {
                            let denominator = (-stops).exp2() - black_level;
                            denominator.is_finite() && denominator > 0.0 && denominator.recip().is_finite()
                        })) =>
            {
                Ok(())
            }
            Self::BrightnessContrast { brightness, contrast }
                if brightness.is_finite() && (-3.0..=3.0).contains(&brightness) && contrast.is_finite() && (-5.0..=5.0).contains(&contrast) =>
            {
                Ok(())
            }
            Self::Levels { input_low, input_high, output_low, output_high }
                if [input_low, input_high, output_low, output_high].iter().all(|v| v.is_finite())
                    && input_high > input_low
                    && (input_high - input_low).is_finite()
                    && (output_high - output_low).is_finite() =>
            {
                Ok(())
            }
            _ => Err(RasterInputError::UnsupportedConversion),
        }
    }

    fn identity(self) -> bool {
        matches!(self, Self::Exposure { stops: 0.0, black_level: 0.0 } | Self::BrightnessContrast { brightness: 0.0, contrast: 1.0 })
            || matches!(self,Self::Levels{input_low,input_high,output_low,output_high} if input_low==output_low && input_high==output_high)
            || matches!(self,
                Self::AlphaClipPrecision { clip_low: false, clip_high: false, .. }
                    | Self::AlphaClipFinitePrecision { clip_low: false, clip_high: false, .. })
    }
}

/// Exact raw alpha/outside-ROI preservation for color operations; Alpha Threshold
/// changes only raw alpha samples and leaves color bytes untouched. Input interpretation is
/// defined by the admitted registry member, never inferred from sample depth.
/// The job owns aggregate reservations/deadline; limits bound this invocation.
pub fn try_apply(
    source: &Surface,
    area: Rect,
    operation: PointOperation,
    resident_limit: u64,
    work_limit: u64,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    let prepared = operation.prepare()?;
    prepared.validate_native_format(source.format())?;
    if source.resident_payload_bytes()? > resident_limit.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let bytes = checked_region_byte_len(area, source.format())? as u64;
    let work = bytes.checked_mul(16).ok_or(RasterInputError::InvalidBounds)?;
    if work > work_limit {
        return Err(RasterInputError::LimitExceeded);
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    if operation.identity() {
        return Ok(source.clone());
    }
    let format = source.format();
    let mut out = source.clone();
    for tile in area.tiles() {
        let rect = tile.rect().intersect(&area);
        let scratch = checked_region_byte_len(rect, format)? as u64;
        let peak = source
            .resident_payload_bytes()?
            .checked_add(out.resident_payload_bytes()?)
            .and_then(|v| v.checked_add(scratch.checked_mul(3)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        if peak.checked_add((256 * 256 * format.bytes_per_pixel()) as u64).ok_or(RasterInputError::LimitExceeded)?
            > resident_limit.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut raw = source.try_read_interleaved_with(rect, control)?;
        for (index, pixel) in raw.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
            if index.is_multiple_of(256) && control.cancelled() {
                return Err(RasterInputError::Cancelled);
            }
            let mut staged = [0u8; 40];
            let target = staged.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
            prepared.try_eval_native_pixel(pixel, format, target)?;
            pixel.copy_from_slice(target);
        }
        out.try_write_interleaved_with(rect, &raw, control)?;
    }
    if control.cancelled() {
        return Err(RasterInputError::Cancelled);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "point/alpha_clip_identity_tests.rs"]
mod alpha_clip_identity_tests;

#[cfg(test)]
mod integer_gray_temperature_tests {
    use super::*;

    #[test]
    fn live_integer_boundary_clamps_rgb_before_gray_and_preserves_alpha() {
        let operation = PointOperation::ColorTemperature { original: 6500.0, intended: 5500.0 }.prepare().unwrap();
        let input = [1.0, -0.0];
        let mut wide = [0.0; 2];
        let mut integer = [0.0; 2];
        operation.try_eval_pixel(&input, 1, &mut wide).unwrap();
        operation.try_eval_integer_gray_temperature(&input, &mut integer).unwrap();
        assert!(wide[0] > integer[0]);
        assert!(integer[0] < 1.0);
        assert_eq!(integer[1].to_bits(), input[1].to_bits());
        let mut repeated_wide = [0.0; 2];
        operation.try_eval_pixel(&input, 1, &mut repeated_wide).unwrap();
        assert_eq!(wide.map(f64::to_bits), repeated_wide.map(f64::to_bits));
    }

    #[test]
    fn invalid_integer_temperature_is_atomic() {
        let prepared = PointOperation::ColorTemperature { original: 6500.0, intended: 5500.0 }.prepare().unwrap();
        for input in [vec![], vec![f64::NAN, 0.5], vec![0.5, f64::INFINITY]] {
            let mut output = [0.25, 0.75];
            assert!(prepared.try_eval_integer_gray_temperature(&input, &mut output).is_err());
            assert_eq!(output, [0.25, 0.75]);
        }
        let mut output = [0.25, 0.75];
        assert!(PointOperation::Invert.prepare().unwrap().try_eval_integer_gray_temperature(&[0.5, 0.5], &mut output).is_err());
        assert_eq!(output, [0.25, 0.75]);
    }
}
