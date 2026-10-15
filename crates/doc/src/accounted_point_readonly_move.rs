//! Stable-ID reordering of existing bounded Point storage nodes only.
//! Current rendering and source qualification remain the caller's responsibility.
use super::*;
use crate::{LayerId, item_identity::FilterId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointFilterMovePatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub index: usize,
}

#[derive(Clone, Copy)]
pub(in crate::accounted_document) struct Plan {
    layer: LayerId,
    filter: FilterId,
    from: usize,
    to: usize,
    old_revision: u64,
    revision: u64,
}

fn position(nodes: &[FilterNode], id: FilterId, control: &ReadControl<'_>) -> Result<usize, BackingError> {
    if nodes.is_empty() || nodes.len() > 2 || id.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let mut found = None;
    for (index, node) in nodes.iter().enumerate() {
        control.check(1)?;
        if node.id == id {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(index);
        }
    }
    found.ok_or(BackingError::Bounds)
}

impl AccountedDocument {
    /// Move one existing stable filter ID to an owner-local node index. This
    /// preserves every raw node and mask, with one checked stack revision.
    /// Same-position requests share the original model before allocating or
    /// incrementing the revision. No insertion or removal is supported.
    pub fn try_move_readonly_point_filter(&self, patch: PointFilterMovePatch, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if !self.is_readonly_point_storage() {
            return refused();
        }
        let logical = measured(self.document(), self.quota(), control)?;
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
        if layer.id != patch.layer || stack.owner != ItemRef::Layer(patch.layer) || stack.revision != patch.expected_stack_revision {
            return Err(BackingError::Integrity);
        }
        let from = position(&stack.nodes, patch.filter, control)?;
        if patch.index >= stack.nodes.len() {
            return Err(BackingError::Bounds);
        }
        if from == patch.index {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        current_copy(
            self,
            logical,
            CopyKind::Move(Plan { layer: patch.layer, filter: patch.filter, from, to: patch.index, old_revision: stack.revision, revision }),
            control,
        )
    }
}

pub(super) fn apply(document: &mut Document, plan: Plan, control: &ReadControl<'_>) -> Result<(), BackingError> {
    let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer) || stack.revision != plan.old_revision || position(&stack.nodes, plan.filter, control)? != plan.from {
        return Err(BackingError::Integrity);
    }
    // With exactly two possible positions a checked swap is the same ordering
    // as removing the selected node and reinserting it at the requested index.
    // No vector allocation or payload cloning occurs in this private mutation.
    if plan.to >= stack.nodes.len() {
        return Err(BackingError::Integrity);
    }
    control.check(1)?;
    let [first, second] = stack.nodes.as_mut_slice() else { return Err(BackingError::Integrity) };
    std::mem::swap(first, second);
    stack.revision = plan.revision;
    control.check(1)
}

#[cfg(test)]
#[path = "accounted_point_readonly_move_tests.rs"]
mod tests;
