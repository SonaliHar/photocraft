//! Physical CIE Lab D50 component replacement. Chroma is hypot(a,b);
//! hue is the signed direction in the a/b plane, so no angular wrap or
//! degree/radian conversion is required. No ICC normalized encoding here.
use super::{CompositionError, GimpBlendMode, Result};
pub(crate) fn replace(mode: GimpBlendMode, b: [f64; 3], s: [f64; 3]) -> Result<[f64; 3]> {
    if b.iter().chain(s.iter()).any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("nonfinite physical Lab"));
    }
    let bc = b[1].hypot(b[2]);
    let sc = s[1].hypot(s[2]);
    let output = match mode {
        GimpBlendMode::LchHue => {
            if sc == 0.0 {
                b
            } else {
                [b[0], (s[1] / sc) * bc, (s[2] / sc) * bc]
            }
        }
        GimpBlendMode::LchChroma => {
            if bc == 0.0 {
                [b[0], 0.0, 0.0]
            } else {
                [b[0], (b[1] / bc) * sc, (b[2] / bc) * sc]
            }
        }
        GimpBlendMode::LchColor => [b[0], s[1], s[2]],
        GimpBlendMode::LchLightness => [s[0], b[1], b[2]],
        _ => return Err(CompositionError::Unsupported("not an LCh mode")),
    };
    if output.iter().any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("LCh arithmetic overflow"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_axes_zero_chroma_and_wide_chroma_are_not_normalized_icc_codes() {
        let b = [53.0, -30.0, 40.0];
        let s = [71.0, 0.0, -20.0];
        assert_eq!(replace(GimpBlendMode::LchHue, b, s).unwrap(), [53.0, 0.0, -50.0]);
        assert_eq!(replace(GimpBlendMode::LchChroma, b, s).unwrap(), [53.0, -12.0, 16.0]);
        assert_eq!(replace(GimpBlendMode::LchColor, b, s).unwrap(), [53.0, 0.0, -20.0]);
        assert_eq!(replace(GimpBlendMode::LchLightness, b, s).unwrap(), [71.0, -30.0, 40.0]);
        assert_eq!(replace(GimpBlendMode::LchHue, b, [10.0, 0.0, 0.0]).unwrap(), b);
        assert_eq!(replace(GimpBlendMode::LchChroma, [10.0, 0.0, 0.0], s).unwrap(), [10.0, 0.0, 0.0]);
        assert_eq!(replace(GimpBlendMode::LchColor, b, [71.0, 200.0, -300.0]).unwrap(), [53.0, 200.0, -300.0]);
        let output = replace(GimpBlendMode::LchHue, b, [71.0, f64::MIN_POSITIVE, 0.0]).unwrap();
        assert_eq!(output, [53.0, 50.0, 0.0]);
        assert!(replace(GimpBlendMode::LchHue, b, [0.0, f64::NAN, 0.0]).is_err());
    }
}
