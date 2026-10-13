//! Eleven independently measured selectors of the public 21-choice operation.
//! https://gegl.org/operations/gegl-component-extract.html defines extraction,
//! inversion and linear output. Nonlinear working input/profile conversion is
//! caller-owned; only the implicit analytic transfer is qualified here.
use photocraft_raster::RasterInputError;

pub(super) fn validate(component: u8) -> Result<(), RasterInputError> {
    if matches!(component, 0 | 1 | 2 | 8..=14 | 20) { Ok(()) } else { Err(RasterInputError::UnsupportedConversion) }
}

pub(super) fn eval(component: u8, invert: bool, linear: bool, input: &[f64], channels: usize, output: &mut [f64]) -> Result<(), RasterInputError> {
    validate(component)?;
    if !matches!(channels, 1 | 3) || input.len() != channels + 1 || output.len() != input.len() {
        return Err(RasterInputError::LengthMismatch);
    }
    if input.iter().any(|sample| !sample.is_finite() || !(0.0..=1.0).contains(sample)) {
        return Err(RasterInputError::InvalidSample);
    }
    let sample = if (8..=14).contains(&component) {
        subtractive_sample(component, input, channels)?
    } else {
        let index = if component == 20 {
            channels
        } else if channels == 1 {
            0
        } else {
            usize::from(component)
        };
        *input.get(index).ok_or(RasterInputError::LengthMismatch)? as f32
    };
    let scalar = if invert { 1.0f32 - sample } else { sample };
    let encoded = if linear {
        let value = f64::from(scalar);
        (if value <= 0.0031308 { 12.92 * value } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 }) as f32
    } else {
        scalar
    };
    if !encoded.is_finite() || !(0.0..=1.0).contains(&encoded) {
        return Err(RasterInputError::InvalidSample);
    }
    let mut staged = [f64::from(encoded); 4];
    let alpha = staged.get_mut(channels).ok_or(RasterInputError::LengthMismatch)?;
    *alpha = 1.0;
    let exact = staged.get(..output.len()).ok_or(RasterInputError::LengthMismatch)?;
    output.copy_from_slice(exact);
    Ok(())
}

/// Public working-space observations qualify linear black-pullout CMYK and
/// BT.601 Y'CbCr. The nonlinear working samples enter the measured Float32
/// boundary once; source-native storage is never clipped by this evaluator.
fn subtractive_sample(component: u8, input: &[f64], channels: usize) -> Result<f32, RasterInputError> {
    let channel = |index| input.get(if channels == 1 { 0 } else { index }).map(|value| f64::from(*value as f32)).ok_or(RasterInputError::LengthMismatch);
    let rgb = [channel(0)?, channel(1)?, channel(2)?];
    let scalar = if (8..=11).contains(&component) {
        let linear = rgb.map(|value| {
            let decoded = if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
            f64::from(decoded as f32)
        });
        let maximum = linear.into_iter().fold(0.0f64, f64::max);
        if maximum == 0.0 {
            1.0
        } else if component == 11 {
            1.0 - maximum
        } else {
            let value = linear.get(usize::from(component - 8)).ok_or(RasterInputError::LengthMismatch)?;
            1.0 - value / maximum
        }
    } else {
        let [red, green, blue] = rgb;
        let luminance = 0.299 * red + 0.587 * green + 0.114 * blue;
        match component {
            12 => luminance,
            13 => (blue - luminance) / 1.772 + 0.5,
            14 => (red - luminance) / 1.402 + 0.5,
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
    };
    if !scalar.is_finite() {
        return Err(RasterInputError::InvalidSample);
    }
    Ok((scalar as f32).clamp(0.0, 1.0))
}
