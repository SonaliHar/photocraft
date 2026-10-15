//! Typed replacements of existing flat Point words. L1 stores these words;
//! exact operation/range/current rendering qualification belongs to its caller.
use super::*;
use crate::{LayerId, item_identity::FilterId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointArgumentValue {
    Float32Bits(u32),
    Signed32(i32),
    Bool(bool),
    /// Replaces only the integer; the original independent enum domain stays.
    Enum(i32),
    /// Four literal words only. The existing NL/profile-None tags stay with
    /// the representation; a changed paid copy may shrink spare capacity.
    Color4F32([u32; 4]),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointArgumentPatch<'a> {
    pub name: &'a str,
    pub value: PointArgumentValue,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointFilterPatch<'a> {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
    pub arguments: &'a [PointArgumentPatch<'a>],
    pub enabled: Option<bool>,
}

#[derive(Clone, Copy)]
pub(in crate::accounted_document) struct Plan {
    layer: LayerId,
    filter: FilterId,
    node_index: usize,
    old_revision: u64,
    revision: u64,
    replacements: [Option<(usize, PointArgumentValue)>; 10],
    enabled: Option<bool>,
}
fn original(value: &FilterValue, control: &ReadControl<'_>) -> Result<PointArgumentValue, BackingError> {
    match value {
        FilterValue::Number(ExactNumber::Float32Bits(bits)) => Ok(PointArgumentValue::Float32Bits(*bits)),
        FilterValue::Number(ExactNumber::Signed { width: 32, value }) => {
            Ok(PointArgumentValue::Signed32(i32::try_from(*value).map_err(|_| BackingError::Length)?))
        }
        FilterValue::Bool(value) => Ok(PointArgumentValue::Bool(*value)),
        FilterValue::Enum { value, .. } => Ok(PointArgumentValue::Enum(*value)),
        FilterValue::Color { components, encoding: crate::SampleEncoding::NonlinearProfile, profile: None } if components.len() == 4 => {
            control.check(4)?;
            let mut words = [0; 4];
            for (word, value) in words.iter_mut().zip(components) {
                let ExactNumber::Float32Bits(bits) = value else { return refused() };
                *word = *bits;
            }
            Ok(PointArgumentValue::Color4F32(words))
        }
        _ => refused(),
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
    /// Replace at most ten uniquely named, existing arguments of the same raw
    /// type and/or the activation bit in one paid current-model copy. The enum
    /// domain, argument order, original source and all other metadata survive.
    /// This accepts raw f32 words without a semantic numeric/range assertion.
    pub fn try_with_readonly_point_filter(&self, patch: PointFilterPatch<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        self.with_point_filter_patch(patch, control, false)
    }
    /// Replace one existing signed-32 scalar and/or activation on an assigned
    /// singleton, retaining both independent ICC owners. This is structural
    /// custody only: operation, scalar range and rendering eligibility must be
    /// established by the caller before publishing the returned model.
    pub fn try_with_readonly_assigned_point_filter(&self, patch: PointFilterPatch<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        self.with_point_filter_patch(patch, control, true)
    }
    fn with_point_filter_patch(&self, patch: PointFilterPatch<'_>, control: &ReadControl<'_>, assigned: bool) -> Result<Self, BackingError> {
        control.check(1)?;
        if assigned {
            if !self.is_readonly_assigned_point_storage() {
                return refused();
            }
            if patch.arguments.len() > 1 {
                return Err(BackingError::Length);
            }
            for argument in patch.arguments {
                control.check(1)?;
                if !matches!(argument.value, PointArgumentValue::Signed32(_)) {
                    return refused();
                }
            }
        } else if !self.is_readonly_point_storage() {
            return refused();
        }
        if patch.arguments.len() > 10 {
            return Err(BackingError::Length);
        }
        for replacement in patch.arguments {
            control.check(1)?;
            if replacement.name.is_empty() || replacement.name.len() > 4096 {
                return Err(BackingError::Length);
            }
        }
        let logical = if assigned { measured_assigned(self.document(), self.quota(), control)? } else { measured(self.document(), self.quota(), control)? };
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
        if assigned && (stack.nodes.len() != 1 || patch.layer.0 == 0 || patch.filter.0 == 0) {
            return Err(BackingError::Integrity);
        }
        let node_index = node_index(&stack.nodes, patch.filter, control)?;
        let node = stack.nodes.get(node_index).ok_or(BackingError::Integrity)?;
        if layer.id != patch.layer || stack.owner != ItemRef::Layer(patch.layer) || node.id != patch.filter || stack.revision != patch.expected_stack_revision {
            return Err(BackingError::Integrity);
        }
        let mut replacements = [None; 10];
        let mut changed = patch.enabled.is_some_and(|enabled| enabled != node.enabled);
        for (slot, replacement) in patch.arguments.iter().enumerate() {
            control.check(1)?;
            for earlier in patch.arguments.get(..slot).ok_or(BackingError::Length)? {
                if imported_edits::names_equal(earlier.name, replacement.name, control)? {
                    return Err(BackingError::Integrity);
                }
            }
            let mut found = None;
            for (index, argument) in node.arguments.iter().enumerate() {
                control.check(1)?;
                if imported_edits::names_equal(&argument.name, replacement.name, control)? {
                    if found.is_some() {
                        return Err(BackingError::Integrity);
                    }
                    let before = original(&argument.value, control)?;
                    if std::mem::discriminant(&before) != std::mem::discriminant(&replacement.value) {
                        return refused();
                    }
                    changed |= before != replacement.value;
                    found = Some(index);
                }
            }
            let index = found.ok_or(BackingError::Bounds)?;
            *replacements.get_mut(slot).ok_or(BackingError::Length)? = Some((index, replacement.value));
        }
        if !changed {
            control.check(1)?;
            return Ok(self.clone());
        }
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        let plan = Plan {
            layer: patch.layer,
            filter: patch.filter,
            node_index,
            old_revision: stack.revision,
            revision,
            replacements,
            enabled: patch.enabled,
        };
        let kind = if assigned { CopyKind::AssignedFilter(plan) } else { CopyKind::Filter(plan) };
        current_copy(self, logical, kind, control)
    }
}
pub(super) fn apply(document: &mut Document, plan: Plan, control: &ReadControl<'_>) -> Result<(), BackingError> {
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
    for (index, replacement) in plan.replacements.into_iter().flatten() {
        control.check(1)?;
        let argument = node.arguments.get_mut(index).ok_or(BackingError::Integrity)?;
        match (&mut argument.value, replacement) {
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), PointArgumentValue::Float32Bits(value)) => *bits = value,
            (FilterValue::Number(ExactNumber::Signed { width: 32, value: before }), PointArgumentValue::Signed32(value)) => *before = i64::from(value),
            (FilterValue::Bool(before), PointArgumentValue::Bool(value)) => *before = value,
            (FilterValue::Enum { value: before, .. }, PointArgumentValue::Enum(value)) => *before = value,
            (FilterValue::Color { components, encoding: crate::SampleEncoding::NonlinearProfile, profile: None }, PointArgumentValue::Color4F32(words))
                if components.len() == 4 =>
            {
                for (component, word) in components.iter_mut().zip(words) {
                    control.check(1)?;
                    let ExactNumber::Float32Bits(bits) = component else { return Err(BackingError::Integrity) };
                    *bits = word;
                }
            }
            _ => return Err(BackingError::Integrity),
        }
    }
    if let Some(enabled) = plan.enabled {
        node.enabled = enabled;
    }
    stack.revision = plan.revision;
    control.check(1)
}
