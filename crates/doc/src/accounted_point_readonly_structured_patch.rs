//! Borrowed, bounded structured replacements for genuine readonly Point custody.
//! No raw mutable model callback or operation-specific policy is introduced.
use super::*;
use crate::{LayerId, item_identity::FilterId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointStructuredArgumentPatch<'a> {
    pub name: &'a str,
    pub value: &'a FilterValue,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointStructuredFilterPatch<'a> {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub arguments: &'a [PointStructuredArgumentPatch<'a>],
    pub enabled: Option<bool>,
}
#[derive(Clone, Copy)]
pub(in crate::accounted_document) struct Plan<'a> {
    layer: LayerId,
    filter: FilterId,
    node_index: usize,
    old_revision: u64,
    revision: u64,
    replacements: [Option<(usize, &'a FilterValue)>; 10],
    enabled: Option<bool>,
    copy_bytes: u64,
}
impl Plan<'_> {
    pub(super) fn copy_index_bytes(self) -> u64 {
        self.copy_bytes
    }
}
// Allocation-free stable-ID selection is independent of operation semantics.
// Repeat it on the private copied stack before applying the captured plan.
fn node_index(nodes: &[FilterNode], filter: FilterId, control: &ReadControl<'_>) -> Result<usize, BackingError> {
    if nodes.is_empty() || nodes.len() > 2 || filter.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let mut found = None;
    for (index, node) in nodes.iter().enumerate() {
        control.check(1)?;
        if node.id == filter {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(index);
        }
    }
    found.ok_or(BackingError::Bounds)
}
impl AccountedDocument {
    /// Replace existing argument values of the exact same bounded raw tree
    /// shape. Config names/order, Curve layout and numeric representation stay
    /// fixed. All input is borrowed until one real paid copy has been admitted;
    /// semantic parameter ranges and complete rendering remain caller policy.
    pub fn try_with_readonly_point_structured_filter(&self, patch: PointStructuredFilterPatch<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        if !self.is_readonly_point_storage() {
            return refused();
        }
        if patch.arguments.len() > 10 {
            return Err(BackingError::Length);
        }
        for argument in patch.arguments {
            control.check(1)?;
            if argument.name.is_empty() || argument.name.len() > 4096 {
                return Err(BackingError::Length);
            }
        }
        let logical = measured(self.document(), self.quota(), control)?;
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
        let node_index = node_index(&stack.nodes, patch.filter, control)?;
        let node = stack.nodes.get(node_index).ok_or(BackingError::Integrity)?;
        if layer.id != patch.layer || stack.owner != ItemRef::Layer(patch.layer) || node.id != patch.filter || stack.revision != patch.expected_stack_revision {
            return Err(BackingError::Integrity);
        }
        let mut replacements = [None; 10];
        let mut changed = patch.enabled.is_some_and(|enabled| enabled != node.enabled);
        let mut copy_bytes = 0u64;
        for (slot, replacement) in patch.arguments.iter().enumerate() {
            control.check(1)?;
            for previous in patch.arguments.get(..slot).ok_or(BackingError::Length)? {
                if imported_edits::names_equal(previous.name, replacement.name, control)? {
                    return Err(BackingError::Integrity);
                }
            }
            let shape = point_value_shape(replacement.value, control)?;
            let mut found = None;
            for (index, argument) in node.arguments.iter().enumerate() {
                control.check(1)?;
                if !imported_edits::names_equal(&argument.name, replacement.name, control)? {
                    continue;
                }
                if found.is_some() {
                    return Err(BackingError::Integrity);
                }
                if !point_values_same_shape(&argument.value, replacement.value, control)? {
                    return refused();
                }
                changed |= !point_values_equal(&argument.value, replacement.value, control)?;
                found = Some(index);
            }
            let index = found.ok_or(BackingError::Bounds)?;
            // The source whole-node census bounded aggregate words. Exact tree
            // compatibility and unique names preserve that aggregate after the
            // replacements; no extra recursive work allowance is introduced.
            copy_bytes = copy_bytes.checked_add(shape.copy_index_bytes).ok_or(BackingError::Length)?;
            *replacements.get_mut(slot).ok_or(BackingError::Length)? = Some((index, replacement.value));
        }
        if !changed {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        current_copy(
            self,
            logical,
            CopyKind::Structured(Plan {
                layer: patch.layer,
                filter: patch.filter,
                node_index,
                old_revision: stack.revision,
                revision,
                replacements,
                enabled: patch.enabled,
                copy_bytes,
            }),
            control,
        )
    }
}
pub(super) fn apply(document: &mut Document, plan: Plan<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
    let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer) || stack.revision != plan.old_revision {
        return Err(BackingError::Integrity);
    }
    if node_index(&stack.nodes, plan.filter, control)? != plan.node_index {
        return Err(BackingError::Integrity);
    }
    let node = stack.nodes.get_mut(plan.node_index).ok_or(BackingError::Integrity)?;
    if node.id != plan.filter {
        return Err(BackingError::Integrity);
    }
    for (index, value) in plan.replacements.into_iter().flatten() {
        control.check(1)?;
        let argument = node.arguments.get_mut(index).ok_or(BackingError::Integrity)?;
        // The caller's replacement envelope is held while the old copied value
        // and this new fallible copy overlap. A failure stays wholly private.
        let copied = structured::copy_value(value, control)?;
        argument.value = copied;
    }
    if let Some(enabled) = plan.enabled {
        node.enabled = enabled;
    }
    stack.revision = plan.revision;
    control.check(1)
}
