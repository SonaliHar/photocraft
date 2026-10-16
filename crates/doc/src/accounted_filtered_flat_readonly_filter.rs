//! Target-aware raw Point argument/activation data changes. Exact operation,
//! parameter ranges and full old/staged operative proof belong to the engine.
use super::*;
use crate::item_identity::FilterId;
#[derive(Clone, Copy)]
pub(super) struct Plan {
    layer: LayerId,
    filter: FilterId,
    layer_index: usize,
    stack_index: usize,
    node_index: usize,
    old_revision: u64,
    revision: u64,
    replacements: [Option<(usize, PointArgumentValue)>; 10],
    enabled: Option<bool>,
}
fn prepare(owner: &AccountedDocument, patch: PointFilterPatch<'_>, c: &ReadControl<'_>) -> Result<(ModelCensus, Option<Plan>), BackingError> {
    c.check(1)?;
    if !owner.is_readonly_filtered_flat_point_storage() {
        return refused();
    }
    if patch.arguments.len() > 10 {
        return Err(BackingError::Length);
    }
    for argument in patch.arguments {
        c.check(1)?;
        if argument.name.is_empty() || argument.name.len() > 4096 {
            return Err(BackingError::Length);
        }
        imported_edits::validate_name(argument.name, c)?;
    }
    // Full whole-document structure, canonical prestate and actual census
    // precede every stable-ID/revision/noop decision and every model allocation.
    let logical = measured(owner.document(), owner.quota(), c)?;
    if patch.layer.0 == 0 || patch.filter.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let doc = owner.document();
    let mut layer_index = None;
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        if layer.id == patch.layer {
            if layer_index.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut target = None;
    for (index, stack) in state.filter_stacks.iter().enumerate() {
        c.check(1)?;
        if stack.owner == ItemRef::Layer(patch.layer) {
            if target.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let stack_index = target.ok_or(BackingError::Bounds)?;
    let stack = state.filter_stacks.get(stack_index).ok_or(BackingError::Integrity)?;
    let [node] = stack.nodes.as_slice() else { return Err(BackingError::Integrity) };
    if node.id != patch.filter || stack.revision != patch.expected_stack_revision {
        return Err(BackingError::Integrity);
    }
    let mut replacements = [None; 10];
    let mut changed = patch.enabled.is_some_and(|v| v != node.enabled);
    for (slot, replacement) in patch.arguments.iter().enumerate() {
        c.check(1)?;
        for previous in patch.arguments.get(..slot).ok_or(BackingError::Length)? {
            if imported_edits::names_equal(previous.name, replacement.name, c)? {
                return Err(BackingError::Integrity);
            }
        }
        let mut found = None;
        for (index, argument) in node.arguments.iter().enumerate() {
            c.check(1)?;
            if imported_edits::names_equal(&argument.name, replacement.name, c)? {
                if found.is_some() {
                    return Err(BackingError::Integrity);
                }
                let before = point_readonly::original_argument_value(&argument.value, replacement.value, c)?;
                if std::mem::discriminant(&before) != std::mem::discriminant(&replacement.value) {
                    return refused();
                }
                changed |= before != replacement.value;
                found = Some(index);
            }
        }
        *replacements.get_mut(slot).ok_or(BackingError::Length)? = Some((found.ok_or(BackingError::Bounds)?, replacement.value));
    }
    c.check(1)?;
    if !changed {
        return Ok((logical, None));
    }
    let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
    Ok((
        logical,
        Some(Plan {
            layer: patch.layer,
            filter: patch.filter,
            layer_index,
            stack_index,
            node_index: 0,
            old_revision: stack.revision,
            revision,
            replacements,
            enabled: patch.enabled,
        }),
    ))
}
impl AccountedDocument {
    /// Structural preflight only. Caller must independently prove the complete
    /// current document, operation/range/locks and publication fences, even noop.
    pub fn validate_readonly_filtered_flat_point_filter(&self, patch: PointFilterPatch<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
        prepare(self, patch, c)?;
        Ok(())
    }
    /// Replace existing named raw words and/or one activation bit in one paid
    /// whole-document copy. Other roots/stacks/masks/source/profile stay exact.
    pub fn try_with_readonly_filtered_flat_point_filter(&self, patch: PointFilterPatch<'_>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, patch, c)?;
        let Some(plan) = plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        current_copy(self, logical, CopyKind::Filter(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if doc.layers.get(plan.layer_index).is_none_or(|layer| layer.id != plan.layer) {
        return Err(BackingError::Integrity);
    }
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.get_mut(plan.stack_index).ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer) || stack.revision != plan.old_revision || stack.nodes.len() != 1 {
        return Err(BackingError::Integrity);
    }
    let node = stack.nodes.get_mut(plan.node_index).ok_or(BackingError::Integrity)?;
    if node.id != plan.filter {
        return Err(BackingError::Integrity);
    }
    for (index, replacement) in plan.replacements.into_iter().flatten() {
        c.check(1)?;
        let argument = node.arguments.get_mut(index).ok_or(BackingError::Integrity)?;
        point_readonly::apply_argument_value(&mut argument.value, replacement, c)?;
    }
    if let Some(enabled) = plan.enabled {
        node.enabled = enabled;
    }
    stack.revision = plan.revision;
    c.check(1)
}
