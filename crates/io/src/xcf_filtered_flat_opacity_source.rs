//! Fresh mixed-source opacity only; never an ordinary Point admission.
use super::*;
pub(crate) fn raw_opacity<P>(effect: &Effect<P>) -> Option<u32> {
    // Alien Map has eight common fields plus its released ten typed arguments.
    let property_limit = if effect.operation == "gegl:alien-map" { 18 } else { 16 };
    if effect.properties.len() > property_limit || effect.state.opacity_u8.is_some() {
        return None;
    }
    let mut found = None;
    for p in &effect.properties {
        if p.tag == 6 {
            return None;
        }
        if p.tag == 33 {
            if found.is_some() || p.declared_bytes != 4 || p.payload.len() != 4 {
                return None;
            }
            let bits = u32::from_be_bytes(p.payload.as_slice().try_into().ok()?);
            let value = f32::from_bits(bits);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return None;
            }
            found = Some(bits);
        }
    }
    found.filter(|bits| effect.state.opacity_bits == Some(*bits))
}
pub(crate) fn envelope<P>(effect: &Effect<P>) -> bool {
    let property_limit = if effect.operation == "gegl:alien-map" { 18 } else { 16 };
    if effect.properties.len() > property_limit {
        return false;
    }
    let parameters = match effect.operation.as_str() {
        "gimp:semi-flatten" => crate::xcf_point_replace_source::semi_flatten_source::parameters(effect).is_some(),
        "gegl:color-temperature" | "gegl:sepia" | "gegl:alien-map" => mixed_stock_source::kind(effect).is_some(),
        "gegl:brightness-contrast" | "gegl:levels" => mixed_affine_source::parameters(effect).is_some(),
        "gegl:hue-chroma" => crate::xcf_point_replace_source::hue_chroma_source::parameters(effect).is_some(),
        "gegl:vibrance" => crate::xcf_point_replace_source::vibrance_source::parameters(effect).is_some(),
        "gegl:exposure" => {
            mixed_exposure_source::parameters(effect).is_some() || crate::xcf_point_replace_source::loaded_exposure_source::parameters(effect).is_some()
        }
        "gegl:invert-linear" | "gegl:invert-gamma" => mixed_invert_source::kind(effect).is_some(),
        _ => false,
    };
    let state = &effect.state;
    parameters
        && state.enabled.is_some()
        && state.mode == Some(62)
        && raw_opacity(effect).is_some()
        && state.opacity_u8.is_none()
        && state.clipping == Some(0)
        && state.region == Some(0)
        && state.blend_space.is_some()
        && state.composite_space.is_some()
        && state.composite_mode.is_some()
        && effect.properties.iter().all(|p| matches!(p.tag, 6 | 7 | 8 | 33 | 35 | 36 | 37 | 44 | 45 | 46))
}
/// Exact per-node source policy; no root-first domain is reused for later nodes.
pub(crate) fn family<P>(effect: &Effect<P>) -> Option<PointReplaceFamily> {
    if !envelope(effect) {
        return None;
    }
    match effect.operation.as_str() {
        "gimp:semi-flatten" => Some(PointReplaceFamily::SemiFlatten),
        "gegl:color-temperature" => Some(PointReplaceFamily::MixedTemperature),
        "gegl:sepia" => Some(PointReplaceFamily::MixedSepia),
        "gegl:alien-map" => match mixed_stock_source::kind(effect)? {
            mixed_stock_source::Kind::AlienRgb => Some(PointReplaceFamily::MixedAlienRgb),
            mixed_stock_source::Kind::AlienHsl => Some(PointReplaceFamily::MixedAlienHsl),
            _ => None,
        },
        "gegl:brightness-contrast" => Some(PointReplaceFamily::MixedBrightnessContrast),
        "gegl:levels" => Some(PointReplaceFamily::MixedGeglLevels),
        "gegl:hue-chroma" => Some(PointReplaceFamily::HueChroma),
        "gegl:vibrance" => Some(PointReplaceFamily::Vibrance),
        "gegl:exposure" if crate::xcf_point_replace_source::loaded_exposure_source::parameters(effect).is_some() => {
            Some(PointReplaceFamily::MixedLoadedExposure)
        }
        "gegl:exposure" => Some(PointReplaceFamily::MixedExposure),
        "gegl:invert-linear" => Some(PointReplaceFamily::MixedInvertLinear),
        "gegl:invert-gamma" => Some(PointReplaceFamily::MixedInvertGamma),
        _ => None,
    }
}
