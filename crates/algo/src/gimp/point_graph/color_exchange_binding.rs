//! Retained literal Color Exchange binding over caller-established nonlinear Float.
//! This is a point-stage schema consumer, never a literal/profile conversion or
//! source/outer-composition credential. All eight literal component bits matter.
use super::*;

pub(super) fn validate(
    binding: &Binding<'_>,
    node: &photocraft_doc::xcf_filters::GraphNode,
    from: [f32; 4],
    to: [f32; 4],
    thresholds: [f32; 3],
) -> Result<(), GraphExecuteError> {
    if binding.identity.name != "gegl:color-exchange"
        || binding.identity.stored_version != "0:0"
        || binding.domain.format.mode != ColorMode::Rgb
        || binding.domain.format.sample != SampleType::F32
        || !binding.domain.format.alpha
        || binding.domain.encoding != SampleEncoding::NonlinearProfile
        || binding.domain.profile_fingerprint.is_none_or(|fingerprint| fingerprint == [0; 32])
        || node.arguments.len() != 5
    {
        return Err(GraphExecuteError::Operation("Color Exchange graph requires explicit profiled nonlinear RGBA Float and exact retained schema"));
    }
    // Reuse the kernel's actual parameter admission; no parallel numeric domain.
    PointOperation::ColorExchange { from, to, thresholds }.prepare().map_err(GraphExecuteError::Raster)?;
    let names = ["from-color", "to-color", "red-threshold", "green-threshold", "blue-threshold"];
    let mut seen = [false; 5];
    for argument in &node.arguments {
        let index = names.iter().position(|name| *name == argument.name).ok_or(GraphExecuteError::Operation("unknown Color Exchange property"))?;
        let slot = seen.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        if std::mem::replace(slot, true) {
            return Err(GraphExecuteError::Operation("duplicate Color Exchange property"));
        }
        if index < 2 {
            let FilterValue::Color { components, encoding: SampleEncoding::NonlinearProfile, profile: None } = &argument.value else {
                return Err(GraphExecuteError::Operation("Color Exchange requires retained nonlinear unassigned literal color"));
            };
            let expected = if index == 0 { from } else { to };
            if components.len() != 4 {
                return Err(GraphExecuteError::Operation("Color Exchange literal RGBA component count"));
            }
            for (number, value) in components.iter().zip(expected) {
                if !matches!(number, ExactNumber::Float32Bits(bits) if *bits == value.to_bits()) {
                    return Err(GraphExecuteError::Operation("Color Exchange literal Float bits differ"));
                }
            }
        } else {
            let expected = thresholds.get(index - 2).ok_or(GraphExecuteError::InvalidPlan)?;
            if !matches!(&argument.value, FilterValue::Number(ExactNumber::Float32Bits(bits)) if *bits == expected.to_bits()) {
                return Err(GraphExecuteError::Operation("Color Exchange threshold Float bits differ"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "color_exchange_binding_tests.rs"]
mod tests;
