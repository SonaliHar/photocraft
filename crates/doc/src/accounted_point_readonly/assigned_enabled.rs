//! Activation-only CURRENT storage mutation. Full operation/profile validation,
//! command locks and transaction publication remain the engine's responsibility.
use super::*;
use crate::item_identity::FilterId;

/// Structural data mutation only, like the implicit Point raw-word patches.
/// This type grants no user-visible command or operation eligibility. The engine
/// independently restricts its first command adapter to fully qualified Box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssignedPointEnabledPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Copy)]
pub(in crate::accounted_document) struct Plan {
    layer: LayerId,
    filter: FilterId,
    old_revision: u64,
    revision: u64,
    old_enabled: bool,
    enabled: bool,
}
impl AccountedDocument {
    /// Change only a singleton assigned node's activation and stack revision.
    /// This does not qualify an operation, profile, source or command edit.
    /// Equal activation shares the owner after full structural/target checks.
    pub fn try_with_readonly_assigned_point_enabled(&self, patch: AssignedPointEnabledPatch, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if !self.is_readonly_assigned_point_storage() {
            return refused();
        }
        let logical = measured_assigned(self.document(), self.quota(), control)?;
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let [stack] = state.filter_stacks.as_slice() else { return refused() };
        let [node] = stack.nodes.as_slice() else { return refused() };
        control.check(1)?;
        if patch.layer.0 == 0
            || patch.filter.0 == 0
            || layer.id != patch.layer
            || stack.owner != ItemRef::Layer(patch.layer)
            || node.id != patch.filter
            || stack.revision != patch.expected_stack_revision
        {
            return Err(BackingError::Integrity);
        }
        if node.enabled == patch.enabled {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        let plan = Plan { layer: patch.layer, filter: patch.filter, old_revision: stack.revision, revision, old_enabled: node.enabled, enabled: patch.enabled };
        current_copy(self, logical, CopyKind::AssignedEnabled(plan), control)
    }
}
pub(in crate::accounted_document) fn apply(document: &mut Document, plan: Plan, control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(1)?;
    let [layer] = document.layers.as_slice() else { return Err(BackingError::Integrity) };
    if layer.id != plan.layer {
        return Err(BackingError::Integrity);
    }
    let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let [stack] = state.filter_stacks.as_mut_slice() else { return Err(BackingError::Integrity) };
    let [node] = stack.nodes.as_mut_slice() else { return Err(BackingError::Integrity) };
    if stack.owner != ItemRef::Layer(plan.layer)
        || stack.revision != plan.old_revision
        || node.id != plan.filter
        || node.enabled != plan.old_enabled
        || plan.old_revision.checked_add(1) != Some(plan.revision)
        || plan.enabled == plan.old_enabled
    {
        return Err(BackingError::Integrity);
    }
    node.enabled = plan.enabled;
    stack.revision = plan.revision;
    Ok(())
}
#[cfg(test)]
#[path = "assigned_enabled_tests.rs"]
mod tests;
