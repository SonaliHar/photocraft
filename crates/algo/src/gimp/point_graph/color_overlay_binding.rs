//! Retained nonlinear Color Overlay literal binding, not profile conversion.
//! `srgb:false` requires a separately prepared literal-to-working conversion.
//! Literal alpha is paint opacity; original filter opacity stays outside this node.
use super::*;

pub(super) fn validate(binding: &Binding<'_>, node: &photocraft_doc::xcf_filters::GraphNode, color: [f32; 4]) -> Result<(), GraphExecuteError> {
    if binding.identity.name != "gegl:color-overlay"
        || binding.identity.stored_version != "0:0"
        || binding.domain.format.mode != ColorMode::Rgb
        || binding.domain.format.sample != SampleType::F32
        || !binding.domain.format.alpha
        || binding.domain.encoding != SampleEncoding::NonlinearProfile
        || binding.domain.profile_fingerprint.is_none_or(|fingerprint| fingerprint == [0; 32])
        || node.arguments.len() != 2
    {
        return Err(GraphExecuteError::Operation("Color Overlay graph requires exact retained schema and explicit profiled nonlinear RGBA Float"));
    }
    PointOperation::ColorOverlay { color }.prepare().map_err(GraphExecuteError::Raster)?;
    let mut value_seen = false;
    let mut flag_seen = false;
    for argument in &node.arguments {
        match argument.name.as_str() {
            "value" if !std::mem::replace(&mut value_seen, true) => {
                let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = &argument.value else {
                    return Err(GraphExecuteError::Operation("Color Overlay requires retained nonlinear unassigned literal"));
                };
                if components.len() != 4 {
                    return Err(GraphExecuteError::Operation("Color Overlay literal RGBA count"));
                }
                for (number, expected) in components.iter().zip(color) {
                    if !matches!(number, ExactNumber::Float32Bits(bits) if *bits == expected.to_bits()) {
                        return Err(GraphExecuteError::Operation("Color Overlay literal Float32 bits differ"));
                    }
                }
            }
            "srgb" if !std::mem::replace(&mut flag_seen, true) => {
                if !matches!(argument.value, FilterValue::Bool(true)) {
                    return Err(GraphExecuteError::Operation("Color Overlay linear mode needs separate prepared literal conversion"));
                }
            }
            _ => return Err(GraphExecuteError::Operation("Color Overlay unknown or duplicate property")),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "color_overlay_binding_tests.rs"]
mod tests;
