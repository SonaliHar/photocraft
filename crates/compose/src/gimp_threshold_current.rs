//! Exact current Histogram Threshold node recipe, separate from source/storage
//! qualification. This bounded allocation-free validator does not inspect pixels,
//! profiles, masks, namespaces or original source correspondence. Its caller must
//! establish those independently before using the returned typed operation.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_algo::gimp::threshold::Threshold;
use photocraft_doc::{
    ColorMode, SampleEncoding,
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};

fn refused() -> CompositionError {
    CompositionError::Unsupported("exact nonlinear RGB current Histogram Threshold recipe")
}
/// Exactly one current typed node, including a disabled node. Disabled consumers
/// must bypass raw samples; neither zero thresholds nor a positive stored name
/// establish identity. There are exactly three constant-sized argument values;
/// their original order/bits remain authoritative in the borrowed node.
pub(crate) fn validate(nodes: &[FilterNode], mode: ColorMode, stored: SampleEncoding, version: u32) -> Result<(Threshold, bool)> {
    let [node] = nodes else { return Err(refused()) };
    let policy = node.composition.as_ref().ok_or_else(refused)?;
    let epoch = crate::gimp_filters::THRESHOLD_PRECISION_EPOCH;
    if mode != ColorMode::Rgb
        || stored != SampleEncoding::NonlinearProfile
        || version != 22
        || node.id.0 == 0
        || node.operation.name != "gimp:threshold"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != epoch
        || policy.compatibility_epoch != epoch
        || policy.source_mode != 62
        || policy.legacy
        || policy.pass_through
        || policy.resolved.is_some()
        || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != (-1565818287, -1565818287, -1565818287)
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || node.arguments.len() != 3
    {
        return Err(refused());
    }
    let mut channel = None;
    let mut low = None;
    let mut high = None;
    for argument in &node.arguments {
        match (argument.name.as_str(), &argument.value) {
            ("channel", FilterValue::Enum { domain, value }) if domain == "GimpHistogramChannel" && (0..=5).contains(value) && channel.is_none() => {
                channel = Some(u8::try_from(*value).map_err(|_| refused())?);
            }
            (name @ ("low" | "high"), FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                let value = f32::from_bits(*bits);
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(refused());
                }
                let slot = if name == "low" { &mut low } else { &mut high };
                if slot.is_some() {
                    return Err(refused());
                }
                *slot = Some(f64::from(value));
            }
            _ => return Err(refused()),
        }
    }
    let operation = Threshold { channel: channel.ok_or_else(refused)?, low: low.ok_or_else(refused)?, high: high.ok_or_else(refused)? };
    // Reversed finite-unit thresholds are a legitimate existing operation. Do
    // not sort/clamp coefficients or turn them into a different stored recipe.
    operation.validate()?;
    Ok((operation, node.enabled))
}

#[cfg(test)]
#[path = "gimp_threshold_current_tests.rs"]
mod tests;
