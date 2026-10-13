//! CIE luminance Y from the current RGB profile's ICC PCS D50 colorants.
//! The unadapted native-white Y hypothesis is separately disproved by the
//! original Luminance pair; these coefficients are profile data, never fitted.
use super::{CompositionError, GimpBlendMode, Result};
pub(crate) fn coefficients(profile: &photocraft_cms::Profile) -> Option<[f64; 3]> {
    if profile.channels() != 3 || profile.a2b.iter().any(Option::is_some) || profile.b2a.iter().any(Option::is_some) {
        return None;
    }
    let row = profile.matrix.as_ref()?[1];
    if row.iter().any(|v| !v.is_finite() || *v <= 0.0) { None } else { Some(row) }
}
pub(crate) fn replace(mode: GimpBlendMode, b: [f64; 3], s: [f64; 3], weights: [f64; 3]) -> Result<[f64; 3]> {
    if b.iter().chain(s.iter()).any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || weights.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(CompositionError::Unsupported("linear luminance finite unit domain is not qualified"));
    }
    let y = |v: [f64; 3]| v[0] * weights[0] + v[1] * weights[1] + v[2] * weights[2];
    let by = y(b);
    let sy = y(s);
    if !by.is_finite() || !sy.is_finite() {
        return Err(CompositionError::Invalid("luminance coefficient overflow"));
    }
    let output = match mode {
        GimpBlendMode::LumaDarken | GimpBlendMode::LumaLighten => {
            if by == sy && b != s {
                return Err(CompositionError::Unsupported("equal-luminance color selection tie is not qualified"));
            }
            if (mode == GimpBlendMode::LumaDarken && by <= sy) || (mode == GimpBlendMode::LumaLighten && by >= sy) { b } else { s }
        }
        GimpBlendMode::Luminance => {
            if by <= 0.0 {
                return Err(CompositionError::Unsupported("zero-backdrop luminance is not qualified"));
            }
            b.map(|v| (v / by) * sy)
        }
        _ => return Err(CompositionError::Unsupported("not a luminance mode")),
    };
    if output.iter().any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("luminance arithmetic overflow"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_color_selection_ratio_and_singular_domains_are_explicit() {
        let weights = [0.25, 0.5, 0.25];
        let b = [1.0, 0.0, 0.0];
        let s = [0.0, 1.0, 0.0];
        assert_eq!(replace(GimpBlendMode::LumaDarken, b, s, weights).unwrap(), b);
        assert_eq!(replace(GimpBlendMode::LumaLighten, b, s, weights).unwrap(), s);
        assert_eq!(replace(GimpBlendMode::Luminance, b, s, weights).unwrap(), [2.0, 0.0, 0.0]);
        assert!(replace(GimpBlendMode::Luminance, [0.0; 3], s, weights).is_err());
        assert!(replace(GimpBlendMode::LumaDarken, b, [0.0, 0.0, 1.0], weights).is_err());
        assert!(replace(GimpBlendMode::LumaLighten, b, [f64::NAN, 0.0, 0.0], weights).is_err());
        assert!(replace(GimpBlendMode::Luminance, b, [-1.0, 0.0, 0.0], weights).is_err());
        assert!(replace(GimpBlendMode::LumaDarken, [1.0; 3], [1.0; 3], [f64::MAX; 3]).is_err());
        let marker = 0.5 + 2.0_f64.powi(-40);
        assert_eq!(replace(GimpBlendMode::Luminance, [0.5; 3], [marker; 3], weights).unwrap(), [marker; 3]);
        assert_eq!(replace(GimpBlendMode::Luminance, [f64::MIN_POSITIVE; 3], [0.5; 3], weights).unwrap(), [0.5; 3]);
    }
}
