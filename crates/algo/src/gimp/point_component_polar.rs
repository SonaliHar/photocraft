//! Polar Component model math. This module does not authorize a source epoch or
//! supply RGB/profile conversion. Inputs are explicit nonlinear Double samples.
//! HSV hexcone and HSL double-cone coordinates are derived from extrema/chroma;
//! HSL coordinate definition: https://www.w3.org/TR/css-color-4/#hsl-color
use photocraft_raster::RasterInputError;

/// Compute normalized HSV and HSL triples from unit nonlinear RGB Double.
/// Exact neutrals have hue zero; no near-neutral epsilon is applied.
pub fn hsv_hsl(rgb: [f64; 3]) -> Result<([f64; 3], [f64; 3]), RasterInputError> {
    if rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err(RasterInputError::InvalidSample);
    }
    let [red, green, blue] = rgb;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let chroma = maximum - minimum;
    let lightness = (maximum + minimum) / 2.0;
    if chroma == 0.0 {
        return Ok(([0.0, 0.0, maximum], [0.0, 0.0, lightness]));
    }
    let sector = if maximum == red {
        (green - blue) / chroma
    } else if maximum == green {
        (blue - red) / chroma + 2.0
    } else {
        (red - green) / chroma + 4.0
    };
    let hue = (sector / 6.0).rem_euclid(1.0);
    let saturation = chroma / maximum;
    let denominator = if lightness <= 0.5 { maximum + minimum } else { 2.0 - maximum - minimum };
    if denominator <= 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(([hue, saturation, maximum], [hue, chroma / denominator, lightness]))
}

/// Extract an explicitly supplied model triple through the observed
/// Float32 boundary. This does not convert RGB to a model or quantize storage.
/// `model` is HSV for selectors3..5, HSL for6..7; hue is shared by both models.
/// For15..17 it is CIE Lab (L*,a*,b*);18..19 use CIE LCh (L*,C*,h degrees).
/// Their observed component normalization is L/100,a/255+.5,b/255+.5,
/// C/200,h/360 followed by clipping. Tiny nonzero chroma/hue is retained.
/// Caller chooses the separately qualified nonlinear output/profile conversion.
pub fn extract_model(component: u8, model: [f64; 3], invert: bool, linear: bool) -> Result<[f64; 4], RasterInputError> {
    let axis = match component {
        3 => 0,
        4 | 6 => 1,
        5 | 7 => 2,
        15 => 0,
        16 | 18 => 1,
        17 | 19 => 2,
        _ => return Err(RasterInputError::UnsupportedConversion),
    };
    if model.iter().any(|value| !value.is_finite() || !(*value as f32).is_finite() || (component <= 7 && !(0.0..=1.0).contains(value))) {
        return Err(RasterInputError::InvalidSample);
    }
    let sample = *model.get(axis).ok_or(RasterInputError::LengthMismatch)? as f32;
    let sample = match component {
        15 => sample / 100.0,
        16 | 17 => sample / 255.0 + 0.5,
        18 => sample / 200.0,
        19 => sample / 360.0,
        _ => sample,
    }
    .clamp(0.0, 1.0);
    let sample = if invert { 1.0f32 - sample } else { sample };
    let output = if linear {
        let sample = f64::from(sample);
        (if sample <= 0.0031308 { 12.92 * sample } else { 1.055 * sample.powf(1.0 / 2.4) - 0.055 }) as f32
    } else {
        sample
    };
    if !output.is_finite() || !(0.0..=1.0).contains(&output) {
        return Err(RasterInputError::InvalidSample);
    }
    Ok([f64::from(output), f64::from(output), f64::from(output), 1.0])
}
