//! Activation-only copies for the actual isolated small-Auto group class.
//! Command authorization, locks and current profile/rendering proof remain
//! with the engine. No argument, tuple, mask or source mutation is admitted.
use super::*;
use crate::item_identity::FilterId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaussianGroupEnabledPatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Plan {
    patch: GaussianGroupEnabledPatch,
    revision: u64,
}
fn plan(owner: &AccountedDocument, patch: GaussianGroupEnabledPatch, c: &ReadControl<'_>) -> Result<Option<(ModelCensus, Plan)>, BackingError> {
    c.check(1)?;
    if !owner.is_readonly_gaussian_group_storage() {
        return Err(BackingError::Unsupported);
    }
    // Validate the entire current tree, inert metadata and retained ownership
    // even for a bool no-op. A stale/malformed owner is never rehabilitated.
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
    // The alternate disabled-only opaque tuple carries no activation grant.
    // Never normalize it to the original tuple to make an edit succeed.
    if patch.enabled && raw != [-643996976; 3] {
        return Err(BackingError::Unsupported);
    }
    if node.enabled == patch.enabled {
        c.check(1)?;
        return Ok(None);
    }
    let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
    Ok(Some((logical, Plan { patch, revision })))
}
impl AccountedDocument {
    /// Allocation-free activation planning. This is storage eligibility only;
    /// caller command locks and complete current renderer/profile checks apply.
    pub fn validate_gaussian_group_enabled(&self, patch: GaussianGroupEnabledPatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        plan(self, patch, c)?;
        Ok(())
    }
    /// Copy only the exact selected enabled flag and changed stack revision.
    /// A valid bool no-op shares the current owner without allocation or a
    /// revision increment, including at the maximum revision.
    pub fn try_with_gaussian_group_enabled(&self, patch: GaussianGroupEnabledPatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let Some((logical, plan)) = plan(self, patch, c)? else { return Ok(self.clone()) };
        current_copy(self, logical, CopyKind::Enabled(plan), c)
    }
}
// Called only with the new, privately owned model after prospective copy fees
// and work. Recheck the captured stable identities before either scalar write.
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
    let node = stack.nodes.first_mut().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.patch.layer) || node.id != plan.patch.filter || stack.revision != plan.patch.expected_stack_revision {
        return Err(BackingError::Integrity);
    }
    node.enabled = plan.patch.enabled;
    stack.revision = plan.revision;
    Ok(())
}

#[cfg(test)]
#[path = "accounted_gaussian_group_readonly_enabled_tests.rs"]
mod tests;
