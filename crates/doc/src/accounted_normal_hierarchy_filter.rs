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
    opacity: Option<(u64, u64)>,
}
fn prepare(
    owner: &AccountedDocument,
    patch: PointFilterPatch<'_>,
    opacity_f64_bits: Option<u64>,
    c: &ReadControl<'_>,
) -> Result<(ModelCensus, Option<Plan>), BackingError> {
    c.check(1)?;
    if !accepts(owner) {
        return refused();
    }
    if let Some(bits) = opacity_f64_bits {
        c.check(1)?;
        let value = f64::from_bits(bits);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(BackingError::Bounds);
        }
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
    let logical = measured_owner(owner, c)?;
    if patch.layer.0 == 0 || patch.filter.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let doc = owner.document();
    let layer_index = target_ordinal(owner.document(), patch.layer, c)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut target = None;
    for (index, stack) in state.filter_stacks.iter().enumerate() {
        c.check(1)?;
        if stack.owner == ItemRef::Layer(patch.layer) && target.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let stack_index = target.ok_or(BackingError::Bounds)?;
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
    let node = stack.nodes.get(node_index).ok_or(BackingError::Integrity)?;
    let mut replacements = [None; 10];
    if opacity_f64_bits.is_some_and(|bits| bits != 1.0f64.to_bits()) {
        let index = owner.checked_readonly_normal_hierarchy_current_index(c)?;
        let binding = index.bindings_for_stack(stack_index).and_then(|b| b.get(node_index)).ok_or(BackingError::Integrity)?;
        if binding.filter_id != node.id {
            return Err(BackingError::Integrity);
        }
        if binding.classification == crate::xcf_filter_masks::FilterMaskCoverageClass::RawNonemptyCoverage {
            return refused();
        }
    }
    let opacity = opacity_f64_bits.map(|after| (node.opacity_f64_bits, after));
    let mut changed = patch.enabled.is_some_and(|v| v != node.enabled) || opacity.is_some_and(|(before, after)| before != after);
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
            node_index,
            old_revision: stack.revision,
            revision,
            replacements,
            enabled: patch.enabled,
            opacity,
        }),
    ))
}
impl AccountedDocument {
    /// Structural preflight only. Caller must independently prove the complete
    /// current document, operation/range/locks and publication fences, even noop.
    pub fn validate_readonly_normal_hierarchy_filter(&self, patch: PointFilterPatch<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
        self.validate_readonly_normal_hierarchy_filter_with_opacity(patch, None, c)
    }
    /// Atomic structural arguments/activation/opacity preflight. Exact finite
    /// unit F64 words are data, not operation/source/renderer authority. OLD and
    /// STAGED operative proofs and locks remain external, including raw noops.
    pub fn validate_readonly_normal_hierarchy_filter_with_opacity(
        &self,
        patch: PointFilterPatch<'_>,
        opacity_f64_bits: Option<u64>,
        c: &ReadControl<'_>,
    ) -> Result<(), BackingError> {
        prepare(self, patch, opacity_f64_bits, c)?;
        Ok(())
    }
    /// Replace existing named raw words and/or one activation bit in one paid
    /// whole-document copy. Other roots/stacks/masks/source/profile stay exact.
    pub fn try_with_readonly_normal_hierarchy_filter(&self, patch: PointFilterPatch<'_>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        self.try_with_readonly_normal_hierarchy_filter_with_opacity(patch, None, c)
    }
    /// Atomically replace existing raw arguments, enabled and/or opacity in one
    /// paid whole-model copy, with one changed-only revision. +/-zero words stay
    /// distinct; omitted opacity preserves its original word. No other runtime
    /// class, mask, source property or rendering permission is granted.
    pub fn try_with_readonly_normal_hierarchy_filter_with_opacity(
        &self,
        patch: PointFilterPatch<'_>,
        opacity_f64_bits: Option<u64>,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, patch, opacity_f64_bits, c)?;
        let Some(plan) = plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        current_copy(self, logical, CopyKind::Filter(plan), c)
    }
}
pub(super) fn apply(doc: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if layer_at(doc, plan.layer_index, c)?.id != plan.layer {
        return Err(BackingError::Integrity);
    }
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.get_mut(plan.stack_index).ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer) || stack.revision != plan.old_revision || stack.nodes.is_empty() || stack.nodes.len() > 1024 {
        return Err(BackingError::Integrity);
    }
    let node = stack.nodes.get_mut(plan.node_index).ok_or(BackingError::Integrity)?;
    if node.id != plan.filter {
        return Err(BackingError::Integrity);
    }
    if let Some((before, _)) = plan.opacity {
        c.check(1)?;
        if node.opacity_f64_bits != before {
            return Err(BackingError::Integrity);
        }
    }
    for (index, replacement) in plan.replacements.into_iter().flatten() {
        c.check(1)?;
        let argument = node.arguments.get_mut(index).ok_or(BackingError::Integrity)?;
        point_readonly::apply_argument_value(&mut argument.value, replacement, c)?;
    }
    if let Some(enabled) = plan.enabled {
        node.enabled = enabled;
    }
    if let Some((_, after)) = plan.opacity {
        node.opacity_f64_bits = after;
    }
    stack.revision = plan.revision;
    c.check(1)
}
