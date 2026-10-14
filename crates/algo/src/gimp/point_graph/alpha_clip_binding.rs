//! Exact Alpha Clip argument binding over caller-established RGB Float storage.
//! Unit and finite/crossed limit policies remain distinct existing kernel variants.
//! No stored outer tuple, Gray conversion or source working encoding is inferred.
use super::*;

pub(super) fn validate(binding: &Binding<'_>, node: &photocraft_doc::xcf_filters::GraphNode) -> Result<(), GraphExecuteError> {
    let (clip_low, low_limit, clip_high, high_limit) = match binding.operation {
        PointOperation::AlphaClipPrecision { clip_low, low_limit, clip_high, high_limit }
        | PointOperation::AlphaClipFinitePrecision { clip_low, low_limit, clip_high, high_limit } => (clip_low, low_limit, clip_high, high_limit),
        _ => return Err(GraphExecuteError::Operation("Alpha Clip binding requires an explicit kernel limit policy")),
    };
    if binding.identity.name != "gegl:alpha-clip"
        || binding.identity.stored_version != "0:0"
        || binding.domain.format.mode != ColorMode::Rgb
        || binding.domain.format.sample != SampleType::F32
        || !binding.domain.format.alpha
        || !matches!(binding.domain.encoding, SampleEncoding::Linear | SampleEncoding::NonlinearProfile)
        || binding.domain.profile_fingerprint.is_none_or(|fingerprint| fingerprint == [0; 32])
        || node.arguments.len() != 4
    {
        return Err(GraphExecuteError::Operation("Alpha Clip graph requires exact schema and explicit profiled Linear or nonlinear RGBA Float"));
    }
    // Reuse the unit/finite kernel's own domain validation, including crossed
    // thresholds only for the explicit finite variant. Never normalize limits.
    binding.operation.prepare().map_err(GraphExecuteError::Raster)?;
    let mut seen = [false; 4];
    for argument in &node.arguments {
        let index = ["clip-low", "low-limit", "clip-high", "high-limit"]
            .iter()
            .position(|name| *name == argument.name)
            .ok_or(GraphExecuteError::Operation("unknown Alpha Clip property"))?;
        let flag = seen.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        if std::mem::replace(flag, true) {
            return Err(GraphExecuteError::Operation("duplicate Alpha Clip property"));
        }
        let exact = match (index, &argument.value) {
            (0, FilterValue::Bool(value)) => *value == clip_low,
            (2, FilterValue::Bool(value)) => *value == clip_high,
            (1, FilterValue::Number(ExactNumber::Float32Bits(bits))) => *bits == low_limit.to_bits(),
            (3, FilterValue::Number(ExactNumber::Float32Bits(bits))) => *bits == high_limit.to_bits(),
            _ => false,
        };
        if !exact {
            return Err(GraphExecuteError::Operation("Alpha Clip requires exact Boolean flags and Float32 limit bits"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "alpha_clip_binding_tests.rs"]
mod tests;
