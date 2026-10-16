//! Bounded Point removal with inert source-mask archival and retained empty stack.
use super::*;
use crate::{
    LayerId,
    item_identity::{FilterId, MaskId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointFilterRemovePatch {
    pub layer: LayerId,
    pub filter: FilterId,
    pub expected_stack_revision: u64,
}
#[derive(Clone, Copy)]
pub(in crate::accounted_document) struct Plan {
    layer: LayerId,
    filter: FilterId,
    index: usize,
    mask: Option<MaskId>,
    old_revision: u64,
    revision: u64,
    bytes: u64,
    work: u64,
}
impl Plan {
    pub(super) fn copy_index_bytes(self) -> u64 {
        self.bytes
    }
    pub(super) fn copy_work(self) -> u64 {
        self.work
    }
}

fn target(nodes: &[FilterNode], id: FilterId, c: &ReadControl<'_>) -> Result<usize, BackingError> {
    if nodes.is_empty() || nodes.len() > 2 || id.0 == 0 {
        return Err(BackingError::Integrity);
    }
    let mut found = None;
    for (index, node) in nodes.iter().enumerate() {
        c.check(1)?;
        if node.id == id {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(index);
        }
    }
    found.ok_or(BackingError::Bounds)
}
// ASCII prefix assembled on the stack so archive collision preflight allocates
// nothing. Twenty decimal digits cover every u64 mask ID without truncation.
fn prefix(mask: MaskId) -> Result<([u8; 64], usize), BackingError> {
    let literal = b"deleted-filter-mask/";
    let mut output = [0; 64];
    output.get_mut(..literal.len()).ok_or(BackingError::Length)?.copy_from_slice(literal);
    let mut digits = [0; 20];
    let mut used = 0usize;
    let mut number = mask.0;
    loop {
        *digits.get_mut(used).ok_or(BackingError::Length)? = b'0' + (number % 10) as u8;
        used = used.checked_add(1).ok_or(BackingError::Length)?;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    let mut length = literal.len();
    for digit in digits.get(..used).ok_or(BackingError::Length)?.iter().rev() {
        *output.get_mut(length).ok_or(BackingError::Length)? = *digit;
        length = length.checked_add(1).ok_or(BackingError::Length)?;
    }
    *output.get_mut(length).ok_or(BackingError::Length)? = b'/';
    Ok((output, length.checked_add(1).ok_or(BackingError::Length)?))
}
fn archived_equal(candidate: &str, original: &str, mask: MaskId, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    let (bytes, length) = prefix(mask)?;
    let total = length.checked_add(original.len()).ok_or(BackingError::Length)?;
    c.check(64)?;
    if candidate.len() != total {
        return Ok(false);
    }
    c.check(total as u64)?;
    Ok(candidate.as_bytes().iter().copied().eq(bytes.get(..length).ok_or(BackingError::Length)?.iter().copied().chain(original.bytes())))
}
pub(in crate::accounted_document) fn archive_plan(state: &crate::XcfState, mask: Option<MaskId>, filter: FilterId, c: &ReadControl<'_>) -> Result<(u64, u64), BackingError> {
    for record in &state.opaque {
        c.check(1)?;
        if let Some(owner) = record.owner {
            let mut found = false;
            for item in &state.items {
                c.check(1)?;
                found |= item.item == owner;
            }
            if !found {
                return Err(BackingError::Integrity);
            }
        }
    }
    let Some(mask) = mask else {
        return Ok((0, 0));
    };
    let reference = ItemRef::Mask(mask);
    // The structural owner already excludes these families. Keep explicit
    // deletion dependency checks rather than dropping or repairing authority.
    for source in &state.source_items {
        c.check(1)?;
        if source.owner == reference {
            return refused();
        }
    }
    for stack in &state.filter_stacks {
        for node in &stack.nodes {
            c.check(1)?;
            if node.id != filter && (node.mask == Some(reference) || node.auxiliary.iter().any(|(_, input)| *input == reference)) {
                return refused();
            }
        }
    }
    let (_, prefix_len) = prefix(mask)?;
    let mut bytes = 0u64;
    let mut work = 0u64;
    for (index, record) in state.opaque.iter().enumerate() {
        c.check(1)?;
        if record.owner != Some(reference) {
            continue;
        }
        if record.rendering_required || !imported_edits::names_equal(&record.namespace, "gimp/xcf/source-property", c)? {
            return refused();
        }
        if record.identity.len() > 4096 {
            return Err(BackingError::Length);
        }
        let length = prefix_len.checked_add(record.identity.len()).ok_or(BackingError::Length)? as u64;
        let extra = length.checked_mul(2).ok_or(BackingError::Length)?;
        bytes = bytes.checked_add(extra).ok_or(BackingError::Length)?;
        work = work.checked_add(extra).and_then(|n| n.checked_add(64)).ok_or(BackingError::Length)?;
        for (other_index, other) in state.opaque.iter().enumerate() {
            c.check(1)?;
            if !imported_edits::names_equal(&record.namespace, &other.namespace, c)? {
                continue;
            }
            if other.owner.is_none() && archived_equal(&other.identity, &record.identity, mask, c)?
                || other_index < index && other.owner == Some(reference) && imported_edits::names_equal(&other.identity, &record.identity, c)?
            {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok((bytes, work))
}
impl AccountedDocument {
    /// Remove an existing node from a one- or two-node structural Point stack.
    /// Its inert source-property records are archived with exact original bytes;
    /// last-node removal retains the empty stack owner/revision and storage class.
    pub fn try_remove_readonly_point_filter(&self, patch: PointFilterRemovePatch, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        c.check(1)?;
        if !self.is_readonly_point_storage() {
            return refused();
        }
        let logical = measured(self.document(), self.quota(), c)?;
        let layer = self.document().layers.first().ok_or(BackingError::Integrity)?;
        let state = self.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
        let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
        if layer.id != patch.layer || stack.owner != ItemRef::Layer(patch.layer) || stack.revision != patch.expected_stack_revision {
            return Err(BackingError::Integrity);
        }
        let index = target(&stack.nodes, patch.filter, c)?;
        let node = stack.nodes.get(index).ok_or(BackingError::Integrity)?;
        let mask = match node.mask {
            Some(ItemRef::Mask(id)) => Some(id),
            None => None,
            _ => return Err(BackingError::Integrity),
        };
        let (bytes, work) = archive_plan(state, mask, patch.filter, c)?;
        let revision = stack.revision.checked_add(1).ok_or(BackingError::Length)?;
        current_copy(
            self,
            logical,
            CopyKind::Remove(Plan { layer: patch.layer, filter: patch.filter, index, mask, old_revision: stack.revision, revision, bytes, work }),
            c,
        )
    }
}
pub(in crate::accounted_document) fn identity(original: &str, mask: MaskId, c: &ReadControl<'_>) -> Result<String, BackingError> {
    let (prefix, count) = prefix(mask)?;
    let length = count.checked_add(original.len()).ok_or(BackingError::Length)?;
    let mut result = String::new();
    result.try_reserve_exact(length).map_err(|_| BackingError::Allocation)?;
    if result.capacity() > length.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    c.check(count as u64)?;
    result.push_str(std::str::from_utf8(prefix.get(..count).ok_or(BackingError::Length)?).map_err(|_| BackingError::Integrity)?);
    // Original identity is bounded to 4096 UTF-8 bytes. This paid push never
    // splits a character or needs more than the admitted destination capacity.
    c.check(original.len() as u64)?;
    result.push_str(original);
    Ok(result)
}
pub(super) fn apply(document: &mut Document, plan: Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(plan.layer) || stack.revision != plan.old_revision || target(&stack.nodes, plan.filter, c)? != plan.index {
        return Err(BackingError::Integrity);
    }
    let node = stack.nodes.get(plan.index).ok_or(BackingError::Integrity)?;
    if node.mask != plan.mask.map(ItemRef::Mask) {
        return Err(BackingError::Integrity);
    }
    let (bytes, work) = archive_plan(state, plan.mask, plan.filter, c)?;
    if bytes != plan.bytes || work != plan.work {
        return Err(BackingError::Integrity);
    }
    if let Some(mask) = plan.mask {
        for record in &mut state.opaque {
            c.check(1)?;
            if record.owner == Some(ItemRef::Mask(mask)) {
                record.identity = identity(&record.identity, mask, c)?;
                record.owner = None;
            }
        }
        state.filter_masks.retain(|value| value.id != mask);
        state.items.retain(|item| item.item != ItemRef::Mask(mask));
    }
    let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
    // Private copied vector, checked index, length one or two. Removal shifts
    // the survivor without allocation and preserves all its raw fields/order.
    if plan.index >= stack.nodes.len() {
        return Err(BackingError::Integrity);
    }
    stack.nodes.remove(plan.index);
    stack.revision = plan.revision;
    state.reconcile_filter_output_precision();
    c.check(1)
}

#[cfg(test)]
#[path = "accounted_point_readonly_remove_tests.rs"]
mod tests;
