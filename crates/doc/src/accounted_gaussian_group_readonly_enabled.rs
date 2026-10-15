//! Compatibility activation API over the same closed CURRENT typed patch.
use super::*;
use crate::item_identity::FilterId;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaussianGroupEnabledPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub enabled: bool,
}
fn combined(patch: GaussianGroupEnabledPatch) -> GaussianGroupPatch {
    GaussianGroupPatch {
        layer: patch.layer,
        filter: patch.filter,
        expected_stack_revision: patch.expected_stack_revision,
        std_dev_x_f32_bits: None,
        std_dev_y_f32_bits: None,
        enabled: Some(patch.enabled),
    }
}
impl AccountedDocument {
    pub fn validate_gaussian_group_enabled(&self, patch: GaussianGroupEnabledPatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        self.validate_gaussian_group_patch(combined(patch), c)
    }
    pub fn try_with_gaussian_group_enabled(&self, patch: GaussianGroupEnabledPatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        self.try_with_gaussian_group_patch(combined(patch), c)
    }
}
#[cfg(test)]
#[path = "accounted_gaussian_group_readonly_enabled_tests.rs"]
mod tests;
