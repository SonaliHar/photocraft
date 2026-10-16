//! Structural owner-local movement only; command and full operative authority
//! remain external. No insertion, removal or mask/canonical reordering.
use super::*;
use crate::accounted_document::PointFilterMovePatch;

#[derive(Clone, Copy)]
pub(super) struct Plan {
    layer: LayerId,
    filter: crate::item_identity::FilterId,
    layer_index: usize,
    stack_index: usize,
    from: usize,
    to: usize,
    old_revision: u64,
    revision: u64,
}
fn position(nodes: &[FilterNode], id: crate::item_identity::FilterId, c: &ReadControl<'_>) -> Result<usize, BackingError> {
    if nodes.is_empty() || nodes.len() > 1024 || id.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let mut found = None;
    for (index, node) in nodes.iter().enumerate() {
        c.check(1)?;
        if node.id == id && found.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    found.ok_or(BackingError::Bounds)
}
fn prepare(owner: &AccountedDocument, patch: PointFilterMovePatch, c: &ReadControl<'_>) -> Result<(ModelCensus, Option<Plan>), BackingError> {
    c.check(1)?;
    if !owner.is_readonly_filtered_flat_point_storage() {
        return refused();
    }
    let logical = measured(owner.document(), owner.quota(), c)?;
    if patch.layer.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let mut layer_index = None;
    for (index, layer) in owner.document().layers.iter().enumerate() {
        c.check(1)?;
        if layer.id == patch.layer && layer_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut stack_index = None;
    for (index, stack) in state.filter_stacks.iter().enumerate() {
        c.check(1)?;
        if stack.owner == ItemRef::Layer(patch.layer) && stack_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let stack_index = stack_index.ok_or(BackingError::Bounds)?;
    let stack = state.filter_stacks.get(stack_index).ok_or(BackingError::Integrity)?;
    if stack.revision != patch.expected_stack_revision {
        return Err(BackingError::Integrity);
    }
    let from = position(&stack.nodes, patch.filter, c)?;
    if patch.index >= stack.nodes.len() {
        return Err(BackingError::Bounds);
    }
    c.check(1)?;
    if from == patch.index {
        return Ok((logical, None));
    }
    let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
    Ok((
        logical,
        Some(Plan { layer: patch.layer, filter: patch.filter, layer_index, stack_index, from, to: patch.index, old_revision: stack.revision, revision }),
    ))
}
impl AccountedDocument {
    /// Structural preflight only. Full OLD/history/STAGED operative and command
    /// fences remain mandatory externally, including a same-position request.
    pub fn validate_readonly_filtered_flat_point_filter_move(&self, patch: PointFilterMovePatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        prepare(self, patch, c)?;
        Ok(())
    }
    /// Move an existing stable ID in one paid whole-document copy. The data API
    /// permits a destination within the actual bounded stack; command wire limits
    /// remain external. Same-position requests share the model without revision.
    pub fn try_move_readonly_filtered_flat_point_filter(&self, patch: PointFilterMovePatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, patch, c)?;
        let Some(plan) = plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        current_copy(self, logical, CopyKind::Move(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if doc.layers.get(plan.layer_index).is_none_or(|layer| layer.id != plan.layer) {
        return Err(BackingError::Integrity);
    }
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.get_mut(plan.stack_index).ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer)
        || stack.revision != plan.old_revision
        || position(&stack.nodes, plan.filter, c)? != plan.from
        || plan.to >= stack.nodes.len()
    {
        return Err(BackingError::Integrity);
    }
    // Adjacent swaps have remove-then-insert order without allocation or payload
    // cloning. Every step is controlled and only the unpublished copy is mutated.
    let mut current = plan.from;
    while current != plan.to {
        c.check(1)?;
        let begin = if current < plan.to { current } else { current.checked_sub(1).ok_or(BackingError::Integrity)? };
        let end = begin.checked_add(2).ok_or(BackingError::Length)?;
        let pair = stack.nodes.get_mut(begin..end).ok_or(BackingError::Integrity)?;
        let [first, second] = pair else { return Err(BackingError::Integrity) };
        std::mem::swap(first, second);
        current = if current < plan.to { current.checked_add(1).ok_or(BackingError::Length)? } else { begin };
    }
    stack.revision = plan.revision;
    c.check(1)
}
