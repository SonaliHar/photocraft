//! Explicit same-format LCh Float stage, separate from RGB/ICC conversion.
//! Original arithmetic retained from the independently observed Stage A capsule.
//! It does not resolve source composition tuples or grant evaluator admission.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    pub hue: f64,
    pub chroma: f64,
    pub lightness: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LchFloatContext {
    pub profile_identity: [u8; 32],
    pub space_identity: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub work_units: u64,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub fn validate(p: Parameters) -> Result<(), E> {
    for (v, lo, hi) in [(p.hue, -180.0, 180.0), (p.chroma, -100.0, 100.0), (p.lightness, -100.0, 100.0)] {
        if !v.is_finite() || v < lo || v > hi {
            return Err(E::UnsupportedConversion);
        }
    }
    Ok(())
}
/// f64 parameters are narrowed before f32 addition. Only chroma is clamped;
/// hue stays unwrapped and lightness unclamped. Alpha is copied word-for-word.
pub fn evaluate_lch_float(input: [f32; 4], p: Parameters) -> Result<[f32; 4], E> {
    validate(p)?;
    if input.iter().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    let [lightness, chroma, hue, alpha] = input;
    let chromatic = chroma.abs() > 0.0001_f32;
    let out = [
        lightness + p.lightness as f32,
        (if chromatic { chroma + p.chroma as f32 } else { chroma }).clamp(0.0, 300.0),
        if chromatic { hue + p.hue as f32 } else { hue },
        alpha,
    ];
    if out.iter().any(|v| !v.is_finite()) {
        return Err(E::InvalidSample);
    }
    Ok(out)
}
pub fn requirements(pixels: usize, limits: Limits) -> Result<Requirements, E> {
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 1 << 30 {
        return Err(E::LimitExceeded);
    }
    let bytes = (pixels as u64).checked_mul(16).ok_or(E::LimitExceeded)?;
    let work_units = (pixels as u64).checked_mul(64).ok_or(E::LimitExceeded)?;
    if bytes.checked_mul(2).is_none_or(|n| n > limits.resident_bytes) || work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    Ok(Requirements { input_bytes: bytes, output_bytes: bytes, work_units })
}
/// Exact in-place private preparation. Callers must not publish partial results.
pub fn evaluate_private_with_interrupt(
    pixels: &mut [[f32; 4]],
    p: Parameters,
    expected: LchFloatContext,
    actual: LchFloatContext,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<(), E> {
    poll(c)?;
    validate(p)?;
    if expected != actual {
        return Err(E::UnsupportedConversion);
    }
    requirements(pixels.len(), limits)?;
    for pixel in pixels {
        poll(c)?;
        *pixel = evaluate_lch_float(*pixel, p)?;
    }
    poll(c)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_gate_and_raw_alpha_words() {
        let p = Parameters { hue: 37.5, chroma: -12.5, lightness: 8.25 };
        let out = evaluate_lch_float([50.0, 40.0, 320.0, -0.0], p).unwrap();
        assert_eq!(out[..3], [58.25, 27.5, 357.5]);
        assert_eq!(out[3].to_bits(), (-0.0f32).to_bits());
        let gate = 0.0001_f32;
        for c in [0.0, -0.0, f32::from_bits(gate.to_bits() - 1), gate, -gate] {
            let o = evaluate_lch_float([10.0, c, 20.0, 1.0], p).unwrap();
            assert_eq!(o[2], 20.0);
        }
        let o = evaluate_lch_float([110.0, 280.0, 350.0, 0.0], Parameters { hue: 180.0, chroma: 100.0, lightness: 100.0 }).unwrap();
        assert_eq!(o, [210.0, 300.0, 530.0, 0.0]);
    }

    #[test]
    fn retained_stage_a_selected_channel_word_controls() {
        let input = [[3240099840, 0, 3274964992, 2147483648], [0, 2147483648, 0, 2147483648], [1112014848, 953267990, 1134559232, 2147483648]];
        let expected = [[3219128320, 0, 3274964992, 2147483648], [1090781184, 2147483648, 0, 2147483648], [1114177536, 953267990, 1134559232, 2147483648]];
        let p = Parameters { hue: 37.5, chroma: -12.5, lightness: 8.25 };
        for (i, e) in input.into_iter().zip(expected) {
            assert_eq!(evaluate_lch_float(i.map(f32::from_bits), p).unwrap().map(f32::to_bits), e);
        }
    }
    #[test]
    fn invalid_and_rounding_sensitive_double_parameters() {
        let p = Parameters { hue: 37.50000001, chroma: 0.0, lightness: 8.25000001 };
        assert_eq!(evaluate_lch_float([50.0, 40.0, 1.0, 1.0], p).unwrap(), [58.25, 40.0, 38.5, 1.0]);
        assert!(evaluate_lch_float([f32::NAN, 0.0, 0.0, 1.0], p).is_err());
        assert!(validate(Parameters { hue: 180.0001, ..p }).is_err());
        assert!(requirements(100, Limits { resident_bytes: 1, work_units: 1 }).is_err());
    }
}
