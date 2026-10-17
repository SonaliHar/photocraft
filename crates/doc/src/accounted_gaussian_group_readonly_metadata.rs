//! Closed CURRENT layer metadata and physical group-mask activation. The saved
//! filter-selection mask is a distinct role even though its owner is identical.
//! Source correspondence, renderer/profile checks and command locks are external.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Plan<'a> {
    edit: AccountedEdit<'a>,
    mask_index: Option<usize>,
}
impl Plan<'_> {
    pub(super) fn replacement_name_bytes(self) -> Result<u64, BackingError> {
        // The shared envelope doubles this value: two replacement Strings,
        // each with actual capacity bounded by twice its borrowed name length.
        match self.edit {
            AccountedEdit::LayerMetadata { name: Some(name), .. } => (name.len() as u64).checked_mul(2).ok_or(BackingError::Length),
            _ => Ok(0),
        }
    }
}
fn canonical_prestate(doc: &Document, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    for item in &state.items {
        c.check(1)?;
        if let ItemRef::Layer(id) = item.item {
            let layer = accounted_layer_access(doc, id, c)?.ok_or(BackingError::Integrity)?.layer;
            if !imported_edits::names_equal(&layer.name, &item.name, c)? || item.visible != layer.visible {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok(())
}
fn physical_index(doc: &Document, layer: LayerId, c: &ReadControl<'_>) -> Result<usize, BackingError> {
    let group = doc.layers.get(1).ok_or(BackingError::Integrity)?;
    if group.id != layer || group.mask.is_none() {
        return Err(BackingError::Unsupported);
    }
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let filter = state.filter_masks.first().ok_or(BackingError::Integrity)?;
    let mut found = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        let ItemRef::Mask(id) = item.item else { continue };
        if id == filter.id {
            continue;
        }
        if item.mask.as_ref().is_some_and(|m| m.owner == ItemRef::Layer(layer)) && found.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    found.ok_or(BackingError::Integrity)
}
fn prepare<'a>(owner: &AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<(ModelCensus, Option<Plan<'a>>), BackingError> {
    c.check(1)?;
    if !owner.is_readonly_gaussian_group_storage() {
        return Err(BackingError::Unsupported);
    }
    if !matches!(edit, AccountedEdit::LayerMetadata { .. } | AccountedEdit::MaskEnabled { .. }) {
        return Err(BackingError::Unsupported);
    }
    if let AccountedEdit::LayerMetadata { name, opacity, .. } = edit {
        if let Some(name) = name {
            imported_edits::validate_name(name, c)?;
        }
        if opacity.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
            return Err(BackingError::Bounds);
        }
    }
    // Complete current storage and all four canonical Layer rows precede even
    // an all-None/no-op. Omitted properties never repair malformed native state.
    let logical = measured(owner.document(), owner.quota(), c)?;
    canonical_prestate(owner.document(), c)?;
    let layer = accounted_layer_access(owner.document(), edit.layer_id(), c)?.ok_or(BackingError::Bounds)?.layer;
    let (changed, planned, mask_index) = match edit {
        AccountedEdit::LayerMetadata { layer: id, name, visible, opacity } => {
            let name_changed = match name {
                Some(name) => !imported_edits::names_equal(&layer.name, name, c)?,
                None => false,
            };
            let changed = name_changed || visible.is_some_and(|v| v != layer.visible) || opacity.is_some_and(|v| v.to_bits() != layer.opacity.to_bits());
            (changed, AccountedEdit::LayerMetadata { layer: id, name: if name_changed { name } else { None }, visible, opacity }, None)
        }
        AccountedEdit::MaskEnabled { layer: id, enabled } => {
            let index = physical_index(owner.document(), id, c)?;
            let native = layer.mask.as_ref().ok_or(BackingError::Unsupported)?;
            (native.enabled != enabled, edit, Some(index))
        }
        _ => return Err(BackingError::Unsupported),
    };
    c.check(1)?;
    Ok((logical, changed.then_some(Plan { edit: planned, mask_index })))
}
pub(in crate::accounted_document) fn validate(owner: &AccountedDocument, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    prepare(owner, edit, c)?;
    Ok(())
}
pub(in crate::accounted_document) fn noop(owner: &AccountedDocument, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    Ok(prepare(owner, edit, c)?.1.is_none())
}
pub(in crate::accounted_document) fn edited(
    owner: &AccountedDocument,
    edit: AccountedEdit<'_>,
    c: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    let (logical, plan) = prepare(owner, edit, c)?;
    let Some(plan) = plan else { return Ok(owner.clone()) };
    current_copy(owner, logical, CopyKind::Metadata(plan), c)
}
pub(super) fn apply(doc: &mut Document, plan: Plan<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    match plan.edit {
        AccountedEdit::LayerMetadata { layer, name, visible, opacity } => {
            let new_name = name.map(|name| imported_edits::copy_name(name, c)).transpose()?;
            let target = find_layer_mut(&mut doc.layers, layer, 0, c)?.ok_or(BackingError::Bounds)?;
            if let Some(name) = new_name {
                target.name = name;
            }
            if let Some(visible) = visible {
                target.visible = visible;
            }
            if let Some(opacity) = opacity {
                target.opacity = opacity;
            }
            imported_edits::synchronize(doc, plan.edit, c)?;
        }
        AccountedEdit::MaskEnabled { layer, enabled } => {
            let index = physical_index(doc, layer, c)?;
            if Some(index) != plan.mask_index {
                return Err(BackingError::Integrity);
            }
            let target = find_layer_mut(&mut doc.layers, layer, 0, c)?.ok_or(BackingError::Bounds)?;
            target.mask.as_mut().ok_or(BackingError::Unsupported)?.enabled = enabled;
            let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
            c.check(1)?;
            state.items.get_mut(index).and_then(|item| item.mask.as_mut()).ok_or(BackingError::Integrity)?.enabled = enabled;
        }
        _ => return Err(BackingError::Unsupported),
    }
    Ok(())
}
#[cfg(test)]
#[path = "accounted_gaussian_group_readonly_metadata_tests.rs"]
mod tests;
