//! Exact caller-owned Red Eye graph arguments over explicit nonlinear RGB Float.
//! The retained working kernel is reused; no outer/owner/profile source admission.
use super::*;

pub(super) fn validate(binding: &Binding<'_>, node: &photocraft_doc::xcf_filters::GraphNode, threshold: f64) -> Result<(), GraphExecuteError> {
    if binding.domain.format.mode != ColorMode::Rgb
        || binding.domain.format.sample != SampleType::F32
        || !binding.domain.format.alpha
        || binding.domain.encoding != SampleEncoding::NonlinearProfile
        || binding.domain.profile_fingerprint.is_none_or(|fingerprint| fingerprint == [0; 32])
        || !threshold.is_finite()
        || !(0.0..=0.8).contains(&threshold)
    {
        return Err(GraphExecuteError::Operation("Red Eye graph requires explicit profiled nonlinear RGB Float and finite operative threshold"));
    }
    // Original Float32 and explicit current Double authorities stay distinct:
    // supplied operation must match the exact promoted or retained number bits.
    validate_values(binding, node, "gegl:red-eye-removal", "0:0", SampleEncoding::NonlinearProfile, &[("threshold", SourceValue::Float(threshold))])
}

#[cfg(test)]
#[path = "red_eye_binding_tests.rs"]
mod tests;
