//! Exact directed-hue schema consumer in caller-established Perceptual Float.
//! Does not infer native/profile conversion, source triplets or owner geometry.
use super::super::point_color_rotate::{ColorRotate, GrayMode};
use super::*;

pub(super) fn validate(binding: &Binding<'_>, node: &photocraft_doc::xcf_filters::GraphNode, operation: ColorRotate) -> Result<(), GraphExecuteError> {
    if binding.identity.name != "gegl:color-rotate"
        || binding.identity.stored_version != "0:0"
        || binding.domain.format.mode != ColorMode::Rgb
        || binding.domain.format.sample != SampleType::F32
        || !binding.domain.format.alpha
        || binding.domain.encoding != SampleEncoding::Perceptual
        || binding.domain.profile_fingerprint.is_none_or(|fingerprint| fingerprint == [0; 32])
        || node.arguments.len() != 10
    {
        return Err(GraphExecuteError::Operation("Color Rotate graph requires exact schema and explicit profiled Perceptual RGBA Float"));
    }
    operation.validate().map_err(GraphExecuteError::Raster)?;
    let names = ["src-clockwise", "src-from", "src-to", "dest-clockwise", "dest-from", "dest-to", "threshold", "gray-mode", "hue", "saturation"];
    let mut seen = [false; 10];
    for argument in &node.arguments {
        let index = names.iter().position(|name| *name == argument.name).ok_or(GraphExecuteError::Operation("unknown Color Rotate property"))?;
        let slot = seen.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        if std::mem::replace(slot, true) {
            return Err(GraphExecuteError::Operation("duplicate Color Rotate property"));
        }
        let exact = match (index, &argument.value) {
            (0, FilterValue::Bool(value)) => *value == operation.source.clockwise,
            (3, FilterValue::Bool(value)) => *value == operation.destination.clockwise,
            (7, FilterValue::Enum { domain, value }) => {
                domain == "gegl:color-rotate/gray-mode"
                    && *value
                        == match operation.gray_mode {
                            GrayMode::TreatAs => 0,
                            GrayMode::ChangeTo => 1,
                        }
            }
            (_, FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                let expected = match index {
                    1 => operation.source.from_degrees,
                    2 => operation.source.to_degrees,
                    4 => operation.destination.from_degrees,
                    5 => operation.destination.to_degrees,
                    6 => operation.gray_threshold,
                    8 => operation.gray_hue_degrees,
                    9 => operation.gray_saturation,
                    _ => return Err(GraphExecuteError::Operation("Color Rotate Boolean/enum cannot be a number")),
                };
                *bits == expected.to_bits()
            }
            _ => false,
        };
        if !exact {
            return Err(GraphExecuteError::Operation("Color Rotate retained Float32/Boolean/enum differs from explicit operation"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "color_rotate_binding_tests.rs"]
mod tests;
