//! Qualified initial GIMP mode dispatch; other inventory modes fail explicitly.
use crate::gimp_composite::{CompositionError, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GimpBlendMode {
    Normal,
    Dissolve,
    Multiply,
    LegacyMultiply,
    LegacyScreen,
    LegacyDifference,
    LegacyAddition,
    LegacySubtract,
    LegacyDarkenOnly,
    LegacyLightenOnly,
    LegacySoftLight,
    LegacyDivide,
    LegacyDodge,
    LegacyBurn,
    LegacyHardLight,
    LegacyGrainExtract,
    LegacyGrainMerge,
    LegacyHsvHue,
    LegacyHsvSaturation,
    LegacyHslColor,
    LegacyHsvValue,
    Screen,
    Difference,
    Addition,
    Subtract,
    Exclusion,
    Overlay,
    GrainMerge,
    GrainExtract,
    LinearLight,
    HardLight,
    HardMix,
    SoftLight,
    PinLight,
    ColorDodge,
    ColorBurn,
    VividLight,
    Divide,
    LinearBurn,
    DarkenOnly,
    LightenOnly,
    HsvHue,
    HsvSaturation,
    HslColor,
    HsvValue,
    LchHue,
    LchChroma,
    LchColor,
    LchLightness,
    LumaDarken,
    LumaLighten,
    Luminance,
    ColorErase,
    Erase,
    Merge,
    Split,
}
pub fn mode(source_mode: i32) -> Result<GimpBlendMode> {
    match source_mode {
        0 | 28 => Ok(GimpBlendMode::Normal),
        1 => Ok(GimpBlendMode::Dissolve),
        3 => Ok(GimpBlendMode::LegacyMultiply),
        4 => Ok(GimpBlendMode::LegacyScreen),
        6 => Ok(GimpBlendMode::LegacyDifference),
        7 => Ok(GimpBlendMode::LegacyAddition),
        8 => Ok(GimpBlendMode::LegacySubtract),
        9 => Ok(GimpBlendMode::LegacyDarkenOnly),
        10 => Ok(GimpBlendMode::LegacyLightenOnly),
        11 => Ok(GimpBlendMode::LegacyHsvHue),
        12 => Ok(GimpBlendMode::LegacyHsvSaturation),
        13 => Ok(GimpBlendMode::LegacyHslColor),
        14 => Ok(GimpBlendMode::LegacyHsvValue),
        15 => Ok(GimpBlendMode::LegacyDivide),
        16 => Ok(GimpBlendMode::LegacyDodge),
        17 => Ok(GimpBlendMode::LegacyBurn),
        18 => Ok(GimpBlendMode::LegacyHardLight),
        19 => Ok(GimpBlendMode::LegacySoftLight),
        20 => Ok(GimpBlendMode::LegacyGrainExtract),
        21 => Ok(GimpBlendMode::LegacyGrainMerge),
        23 => Ok(GimpBlendMode::Overlay),
        24 => Ok(GimpBlendMode::LchHue),
        25 => Ok(GimpBlendMode::LchChroma),
        26 => Ok(GimpBlendMode::LchColor),
        27 => Ok(GimpBlendMode::LchLightness),
        30 => Ok(GimpBlendMode::Multiply),
        31 => Ok(GimpBlendMode::Screen),
        32 => Ok(GimpBlendMode::Difference),
        33 => Ok(GimpBlendMode::Addition),
        34 => Ok(GimpBlendMode::Subtract),
        35 => Ok(GimpBlendMode::DarkenOnly),
        36 => Ok(GimpBlendMode::LightenOnly),
        37 => Ok(GimpBlendMode::HsvHue),
        38 => Ok(GimpBlendMode::HsvSaturation),
        39 => Ok(GimpBlendMode::HslColor),
        40 => Ok(GimpBlendMode::HsvValue),
        41 => Ok(GimpBlendMode::Divide),
        42 => Ok(GimpBlendMode::ColorDodge),
        43 => Ok(GimpBlendMode::ColorBurn),
        44 => Ok(GimpBlendMode::HardLight),
        45 => Ok(GimpBlendMode::SoftLight),
        46 => Ok(GimpBlendMode::GrainExtract),
        47 => Ok(GimpBlendMode::GrainMerge),
        48 => Ok(GimpBlendMode::VividLight),
        49 => Ok(GimpBlendMode::PinLight),
        50 => Ok(GimpBlendMode::LinearLight),
        51 => Ok(GimpBlendMode::HardMix),
        52 => Ok(GimpBlendMode::Exclusion),
        53 => Ok(GimpBlendMode::LinearBurn),
        54 => Ok(GimpBlendMode::LumaDarken),
        55 => Ok(GimpBlendMode::LumaLighten),
        56 => Ok(GimpBlendMode::Luminance),
        57 => Ok(GimpBlendMode::ColorErase),
        58 => Ok(GimpBlendMode::Erase),
        59 => Ok(GimpBlendMode::Merge),
        60 => Ok(GimpBlendMode::Split),
        _ => Err(CompositionError::Unsupported("GIMP blend mode evaluator remains unimplemented")),
    }
}
/// Public XCF mode 0 is legacy Normal, mode 28 is modern Normal.
pub fn require_normal(source_mode: i32) -> Result<()> {
    if matches!(source_mode, 0 | 28) { Ok(()) } else { Err(CompositionError::Unsupported("GIMP blend mode evaluator remains unimplemented")) }
}

/// RGB component replacement from the public GIMP HSV/HSL manual and standard
/// min/max definitions. Achromatic hue is red; Hue preserves a gray source's
/// backdrop hue. This initial domain deliberately excludes HDR colors.
pub(crate) fn components(mode: GimpBlendMode, b: [f64; 3], s: [f64; 3]) -> Result<[f64; 3]> {
    if b.iter().chain(s.iter()).any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(CompositionError::Unsupported("HSV/HSL unit RGB domain is not qualified"));
    }
    let extrema = |v: [f64; 3]| (v.into_iter().fold(f64::INFINITY, f64::min), v.into_iter().fold(f64::NEG_INFINITY, f64::max));
    let (blo, bhi) = extrema(b);
    let (slo, shi) = extrema(s);
    // A hue is the affine position of each channel within its chroma interval.
    let hue = |v: [f64; 3], lo: f64, hi: f64| {
        if hi == lo { [1.0, 0.0, 0.0] } else { v.map(|c| (c - lo) / (hi - lo)) }
    };
    let output = match mode {
        GimpBlendMode::HsvHue => {
            if shi == slo {
                b
            } else {
                hue(s, slo, shi).map(|c| blo + c * (bhi - blo))
            }
        }
        GimpBlendMode::HsvSaturation => {
            let sat = if shi == 0.0 { 0.0 } else { (shi - slo) / shi };
            hue(b, blo, bhi).map(|c| bhi * (1.0 - sat + c * sat))
        }
        GimpBlendMode::HsvValue => {
            if bhi == 0.0 {
                [shi; 3]
            } else {
                b.map(|c| c * shi / bhi)
            }
        }
        GimpBlendMode::HslColor => {
            let light = (blo + bhi) * 0.5;
            let source_light = (slo + shi) * 0.5;
            let denominator = 1.0 - (2.0 * source_light - 1.0).abs();
            let sat = if denominator == 0.0 { 0.0 } else { (shi - slo) / denominator };
            let chroma = (1.0 - (2.0 * light - 1.0).abs()) * sat;
            hue(s, slo, shi).map(|c| light - chroma * 0.5 + c * chroma)
        }
        _ => return Err(CompositionError::Unsupported("not an HSV/HSL component mode")),
    };
    if output.iter().any(|v| !v.is_finite()) {
        return Err(CompositionError::Invalid("HSV/HSL arithmetic overflow"));
    }
    Ok(output)
}

#[cfg(test)]
mod component_tests {
    use super::*;
    #[test]
    fn component_replacement_handles_achromatic_and_primary_extremes() {
        let b = [0.2, 0.4, 0.8];
        assert_eq!(components(GimpBlendMode::HsvHue, b, [0.5; 3]).unwrap(), b);
        assert_eq!(components(GimpBlendMode::HsvValue, [0.0; 3], [0.2, 0.4, 0.8]).unwrap(), [0.8; 3]);
        assert_eq!(components(GimpBlendMode::HsvSaturation, [0.5; 3], [0.0, 1.0, 0.0]).unwrap(), [0.5, 0.0, 0.0]);
        assert_eq!(components(GimpBlendMode::HslColor, b, [0.0; 3]).unwrap(), [0.5; 3]);
        assert_eq!(components(GimpBlendMode::HslColor, [1.0; 3], [0.0, 1.0, 0.0]).unwrap(), [1.0; 3]);
        for mode in [GimpBlendMode::HsvHue, GimpBlendMode::HsvSaturation, GimpBlendMode::HslColor, GimpBlendMode::HsvValue] {
            assert!(components(mode, [-0.1, 0.2, 0.3], b).is_err());
            assert!(components(mode, b, [f64::NAN, 0.0, 1.0]).is_err());
            assert!(components(mode, b, [0.0, 0.0, 1.1]).is_err());
            let marker = 0.5 + 2.0_f64.powi(-40);
            assert_eq!(components(mode, [marker; 3], [marker; 3]).unwrap(), [marker; 3]);
        }
    }

    #[test]
    fn checked_components_reject_unqualified_coverage_and_hidden_hdr() {
        use crate::gimp_composite::WorkingSpace;
        use photocraft_doc::{
            SampleEncoding,
            import_compositing::{GimpOperator, GimpSpace},
        };
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let blend = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let composite = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [GimpBlendMode::HsvHue, GimpBlendMode::HsvSaturation, GimpBlendMode::HslColor, GimpBlendMode::HsvValue] {
            let b = [0.2, 0.4, 0.8, 0.5];
            let s = [0.7, 0.3, 0.1, 0.4];
            let out = composite.separable(&blend, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).unwrap();
            assert_eq!(out[3], b[3]);
            assert!(composite.separable(&blend, b, s, 0.75, GimpOperator::Union, mode).is_err());
            assert!(blend.separable(&blend, b, s, 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(composite.separable(&blend, b, [-1.0, 0.0, 0.0, 0.0], 0.75, GimpOperator::ClipToBackdrop, mode).is_err());
            assert!(composite.separable(&blend, b, s, f64::NAN, GimpOperator::ClipToBackdrop, mode).is_err());
        }
    }
    #[test]
    #[ignore = "24MP release composition throughput measurement"]
    fn component_composition_24mp_release() {
        use crate::gimp_composite::WorkingSpace;
        use photocraft_doc::{
            SampleEncoding,
            import_compositing::{GimpOperator, GimpSpace},
        };
        let profile = photocraft_cms::Builtin::Srgb.profile();
        let blend = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::ProfileRgb).unwrap();
        let composite = WorkingSpace::new(profile, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb).unwrap();
        for mode in [GimpBlendMode::HsvHue, GimpBlendMode::HsvSaturation, GimpBlendMode::HslColor, GimpBlendMode::HsvValue] {
            let start = std::time::Instant::now();
            for i in 0..24_000_000 {
                let b = std::hint::black_box([0.15 + f64::from(i % 128) * 0.001, 0.6, 0.85, 0.7]);
                std::hint::black_box(composite.separable(&blend, b, [0.8, 0.25, 0.4, 0.6], 0.7125, GimpOperator::ClipToBackdrop, mode).unwrap());
            }
            println!("{mode:?}: 24MP {:?}", start.elapsed());
        }
    }
}

#[path = "gimp_lch.rs"]
mod lch;
pub(crate) use lch::replace as lch_components;

#[path = "gimp_luma.rs"]
mod luma;
pub(crate) use luma::{coefficients as luminance_coefficients, replace as luminance_components};

#[path = "gimp_alpha.rs"]
mod alpha;
pub(crate) use alpha::replace as alpha_components;

#[path = "gimp_color_erase.rs"]
mod color_erase;
pub(crate) use color_erase::erase as erase_color;

#[path = "gimp_dissolve.rs"]
mod dissolve;
pub(crate) use dissolve::endpoint as dissolve_endpoint;
