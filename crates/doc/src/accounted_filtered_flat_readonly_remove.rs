//! CURRENT removal of an existing mixed node and its own selection mask.
//! Original mask source records become paid inert archives; empty stacks retain
//! their actual owner and checked revision in the same CURRENT storage class.
use super::*;
use crate::{
    accounted_document::PointFilterRemovePatch,
    item_identity::{FilterId, MaskId},
};
#[derive(Clone, Copy)]
pub(super) struct Plan {
    layer: LayerId,
    filter: FilterId,
    mask: MaskId,
    layer_index: usize,
    stack_index: usize,
    node_index: usize,
    mask_index: usize,
    item_index: usize,
    roots: usize,
    nodes: usize,
    masks: usize,
    items: usize,
    old_revision: u64,
    revision: u64,
    archive_bytes: u64,
    archive_work: u64,
    shift_work: u64,
}
impl Plan {
    pub(super) fn copy_index_bytes(self) -> u64 {
        self.archive_bytes
    }
    pub(super) fn copy_work(self) -> Result<u64, BackingError> {
        self.archive_work.checked_add(self.shift_work).ok_or(BackingError::Length)
    }
}
fn shifted_bytes<T>(len: usize, index: usize) -> Result<u64, BackingError> {
    let remaining = len.checked_sub(index.checked_add(1).ok_or(BackingError::Length)?).ok_or(BackingError::Integrity)?;
    u64::try_from(remaining.checked_mul(std::mem::size_of::<T>()).ok_or(BackingError::Length)?).map_err(|_| BackingError::Length)
}
fn prepare(owner: &AccountedDocument, patch: PointFilterRemovePatch, c: &ReadControl<'_>) -> Result<(ModelCensus, Plan), BackingError> {
    c.check(1)?;
    if !owner.is_readonly_filtered_flat_point_storage() {
        return refused();
    }
    let logical = measured(owner.document(), owner.quota(), c)?;
    if patch.layer.0 == 0 || patch.filter.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let doc = owner.document();
    let mut layer_index = None;
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        if layer.id == patch.layer && layer_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
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
    let mut node_index = None;
    for (index, node) in stack.nodes.iter().enumerate() {
        c.check(1)?;
        if node.id == patch.filter && node_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let node_index = node_index.ok_or(BackingError::Bounds)?;
    // The selected node exists. Complete CURRENT removal retains this actual
    // stack owner/revision even when its node vector becomes empty.
    let node = stack.nodes.get(node_index).ok_or(BackingError::Integrity)?;
    let Some(ItemRef::Mask(mask)) = node.mask else { return Err(BackingError::Integrity) };
    let mut mask_index = None;
    for (index, value) in state.filter_masks.iter().enumerate() {
        c.check(1)?;
        if value.id == mask {
            if value.owner != patch.filter || mask_index.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let mask_index = mask_index.ok_or(BackingError::Integrity)?;
    let mut item_index = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Mask(mask) {
            if item.mask.as_ref().is_none_or(|value| value.owner != ItemRef::Layer(patch.layer)) || item_index.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    let (archive_bytes, archive_work) = point_readonly::removed_mask_archive_plan(state, Some(mask), patch.filter, c)?;
    let node_shift = shifted_bytes::<FilterNode>(stack.nodes.len(), node_index)?;
    let mask_shift = shifted_bytes::<FilterMask>(state.filter_masks.len(), mask_index)?;
    let item_shift = shifted_bytes::<crate::xcf_state::ItemState>(state.items.len(), item_index)?;
    let shift_work = node_shift.checked_add(mask_shift).and_then(|n| n.checked_add(item_shift)).ok_or(BackingError::Length)?;
    let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
    Ok((
        logical,
        Plan {
            layer: patch.layer,
            filter: patch.filter,
            mask,
            layer_index,
            stack_index,
            node_index,
            mask_index,
            item_index,
            roots: doc.layers.len(),
            nodes: stack.nodes.len(),
            masks: state.filter_masks.len(),
            items: state.items.len(),
            old_revision: stack.revision,
            revision,
            archive_bytes,
            archive_work,
            shift_work,
        },
    ))
}
impl AccountedDocument {
    /// Data-only removal preflight. Operation, lock, OLD/history/STAGED proofs
    /// and publication fences are mandatory externally; no source grant.
    pub fn validate_readonly_filtered_flat_point_filter_remove(&self, patch: PointFilterRemovePatch, c: &ReadControl<'_>) -> Result<(), BackingError> {
        prepare(self, patch, c)?;
        Ok(())
    }
    /// Remove one actual stable node/mask/item while retaining this mixed class.
    /// Inert source-mask properties are archived with their exact payload Arcs.
    /// Last-node removal retains the empty stack owner and checked revision.
    pub fn try_remove_readonly_filtered_flat_point_filter(&self, patch: PointFilterRemovePatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, patch, c)?;
        current_copy(self, logical, CopyKind::Remove(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if doc.layers.len() != plan.roots || doc.layers.get(plan.layer_index).is_none_or(|layer| layer.id != plan.layer) {
        return Err(BackingError::Integrity);
    }
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.get(plan.stack_index).ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer)
        || stack.revision != plan.old_revision
        || stack.nodes.len() != plan.nodes
        || stack.nodes.get(plan.node_index).is_none_or(|node| node.id != plan.filter || node.mask != Some(ItemRef::Mask(plan.mask)))
        || state.filter_masks.len() != plan.masks
        || state.items.len() != plan.items
        || state.filter_masks.get(plan.mask_index).is_none_or(|mask| mask.id != plan.mask || mask.owner != plan.filter)
        || state
            .items
            .get(plan.item_index)
            .is_none_or(|item| item.item != ItemRef::Mask(plan.mask) || item.mask.as_ref().is_none_or(|mask| mask.owner != ItemRef::Layer(plan.layer)))
    {
        return Err(BackingError::Integrity);
    }
    let (bytes, work) = point_readonly::removed_mask_archive_plan(state, Some(plan.mask), plan.filter, c)?;
    if bytes != plan.archive_bytes || work != plan.archive_work {
        return Err(BackingError::Integrity);
    }
    for record in &mut state.opaque {
        c.check(1)?;
        if record.owner == Some(ItemRef::Mask(plan.mask)) {
            record.identity = point_readonly::removed_mask_archive_identity(&record.identity, plan.mask, c)?;
            record.owner = None;
        }
    }
    // All three shifts were prospectively charged before model allocation.
    // Checked indices above refer to distinct vectors and cannot be invalidated
    // by removing from the other two. Only this unpublished model is modified.
    c.check(1)?;
    state.filter_masks.remove(plan.mask_index);
    state.items.remove(plan.item_index);
    let stack = state.filter_stacks.get_mut(plan.stack_index).ok_or(BackingError::Integrity)?;
    stack.nodes.remove(plan.node_index);
    stack.revision = plan.revision;
    c.check(1)
}
