//! Clean-room HSL geometry from CSS Color 4 and independent public GIMP responses.
//! https://www.w3.org/TR/css-color-4/#hsl-to-rgb
//! Achromatic input has zero hue/saturation; alpha is handled by the caller.
pub(super) fn map(rgb: [f64; 3], frequency: [f64; 3], phase: [f64; 3], keep: [bool; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(|v| v as f32);
    let maximum = r.max(g).max(b);
    let minimum = r.min(g).min(b);
    let chroma = maximum - minimum;
    let lightness = (maximum + minimum) * 0.5;
    let (hue, saturation) = if chroma == 0.0 {
        (0.0, 0.0)
    } else {
        let saturation = if lightness <= 0.5 { chroma / (maximum + minimum) } else { chroma / (2.0 - maximum - minimum) };
        let sector = if maximum == r {
            (g - b) / chroma
        } else if maximum == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        };
        ((sector / 6.0).rem_euclid(1.0), saturation)
    };
    let mut mapped = [0.0; 3];
    for ((((output, value), frequency), phase), keep) in mapped.iter_mut().zip([hue, saturation, lightness]).zip(frequency).zip(phase).zip(keep) {
        *output = if keep {
            f64::from(value)
        } else {
            f64::from(((1.0 + ((2.0 * f64::from(value) - 1.0) * std::f64::consts::PI * frequency + phase.to_radians()).sin()) * 0.5) as f32)
        };
    }
    let [h, s, l] = mapped;
    if s == 0.0 {
        return [l; 3];
    }
    let upper = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let lower = 2.0 * l - upper;
    let channel = |hue: f64| {
        let hue = hue.rem_euclid(1.0);
        let value = if hue < 1.0 / 6.0 {
            lower + (upper - lower) * 6.0 * hue
        } else if hue < 0.5 {
            upper
        } else if hue < 2.0 / 3.0 {
            lower + (upper - lower) * (2.0 / 3.0 - hue) * 6.0
        } else {
            lower
        };
        f64::from(value as f32)
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}
