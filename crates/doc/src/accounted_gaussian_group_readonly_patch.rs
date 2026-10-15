//! Bounded CURRENT Gaussian axes and activation only. This stores exact words;
//! it does not qualify a source, profile or numerical evaluator.
use super::*;
use crate::item_identity::FilterId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaussianGroupPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub std_dev_x_f32_bits: Option<u32>,
    pub std_dev_y_f32_bits: Option<u32>,
    pub enabled: Option<bool>,
}
pub(super) fn valid_axis(bits: u32) -> bool {
    let value = f32::from_bits(bits);
    value.is_finite() && (0.0..=15.0).contains(&value)
}
#[derive(Clone, Copy)]
pub(super) struct Plan {
    patch: GaussianGroupPatch,
    revision: u64,
}
fn plan(owner: &AccountedDocument, patch: GaussianGroupPatch, c: &ReadControl<'_>) -> Result<Option<(ModelCensus, Plan)>, BackingError> {
    c.check(1)?;
    if !owner.is_readonly_gaussian_group_storage() {
        return Err(BackingError::Unsupported);
    }
    for bits in [patch.std_dev_x_f32_bits, patch.std_dev_y_f32_bits].into_iter().flatten() {
        c.check(1)?;
        if !valid_axis(bits) {
            return Err(BackingError::Bounds);
        }
    }
    let logical = measured(owner.document(), owner.quota(), c)?;
    let layer = owner.document().layers.get(1).ok_or(BackingError::Integrity)?;
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
    let node = stack.nodes.first().ok_or(BackingError::Integrity)?;
    c.check(1)?;
    if patch.layer != layer.id || stack.owner != ItemRef::Layer(patch.layer) || patch.filter != node.id || patch.expected_stack_revision != stack.revision {
        return Err(BackingError::Integrity);
    }
    let policy = node.composition.as_ref().ok_or(BackingError::Integrity)?;
    let raw = [policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator];
    if patch.enabled.unwrap_or(node.enabled) && raw != [-643996976; 3] {
        return Err(BackingError::Unsupported);
    }
    let mut changed = patch.enabled.is_some_and(|next| next != node.enabled);
    for arg in &node.arguments {
        c.check(arg.name.len() as u64 + 1)?;
        match (arg.name.as_str(), &arg.value) {
            ("std-dev-x", FilterValue::Number(ExactNumber::Float32Bits(bits))) => changed |= patch.std_dev_x_f32_bits.is_some_and(|next| next != *bits),
            ("std-dev-y", FilterValue::Number(ExactNumber::Float32Bits(bits))) => changed |= patch.std_dev_y_f32_bits.is_some_and(|next| next != *bits),
            _ => {}
        }
    }
    if !changed {
        c.check(1)?;
        return Ok(None);
    }
    let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
    Ok(Some((logical, Plan { patch, revision })))
}
impl AccountedDocument {
    /// Allocation-free typed CURRENT preflight. Command locks and complete
    /// renderer/profile qualification remain the caller's responsibility.
    pub fn validate_gaussian_group_patch(&self, patch: GaussianGroupPatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        plan(self, patch, c)?;
        Ok(())
    }
    /// Replace at most the two existing F32 axes and activation atomically.
    /// Raw -0 is retained; any changed word/bool increments the stack once.
    pub fn try_with_gaussian_group_patch(&self, patch: GaussianGroupPatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let Some((logical, plan)) = plan(self, patch, c)? else { return Ok(self.clone()) };
        current_copy(self, logical, CopyKind::Filter(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
    let node = stack.nodes.first_mut().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.patch.layer) || node.id != plan.patch.filter || stack.revision != plan.patch.expected_stack_revision {
        return Err(BackingError::Integrity);
    }
    for arg in &mut node.arguments {
        c.check(arg.name.len() as u64 + 1)?;
        match (arg.name.as_str(), &mut arg.value) {
            ("std-dev-x", FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                if let Some(next) = plan.patch.std_dev_x_f32_bits {
                    *bits = next;
                }
            }
            ("std-dev-y", FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                if let Some(next) = plan.patch.std_dev_y_f32_bits {
                    *bits = next;
                }
            }
            _ => {}
        }
    }
    if let Some(enabled) = plan.patch.enabled {
        node.enabled = enabled;
    }
    stack.revision = plan.revision;
    Ok(())
}
#[cfg(test)]
#[path = "accounted_gaussian_group_readonly_patch_tests.rs"]
mod tests;
