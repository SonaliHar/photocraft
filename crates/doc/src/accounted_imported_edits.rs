//! Closed current metadata edits. Original imported evidence remains inert and
//! shared; this is neither source qualification nor permission to edit pixels.
use super::*;

pub(super) fn validate_owner(owner: &AccountedDocument, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    let imported = imported_selection::applies(owner.document());
    if imported {
        validate(owner.document(), edit, owner.quota(), c)?;
        selection_census(owner.document(), false, owner.owner._selection.as_ref(), owner.quota(), c)?;
    } else if matches!(edit, AccountedEdit::LayerMetadata { .. }) {
        return Err(BackingError::Unsupported);
    }
    Ok(imported)
}

pub(super) fn validate(doc: &Document, edit: AccountedEdit<'_>, quota: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    match edit {
        AccountedEdit::LayerMetadata { name, opacity, .. } => {
            if let Some(name) = name {
                validate_name(name, c)?;
            }
            if opacity.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
                return Err(BackingError::Bounds);
            }
        }
        AccountedEdit::LayerName { name, .. } => {
            validate_name(name, c)?;
        }
        AccountedEdit::LayerVisibility { .. } => {}
        AccountedEdit::LayerOpacity { opacity, .. } => {
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                return Err(BackingError::Bounds);
            }
        }
        _ => return Err(BackingError::Unsupported),
    }
    imported_selection::validate(doc, quota, c)?;
    // Preserve one unambiguous current name/visibility representation. No
    // source payload is rewritten, and arbitrary composition remains unchanged.
    if let Some(state) = &doc.xcf_state {
        for item in &state.items {
            c.check(1)?;
            let crate::item_identity::ItemRef::Layer(id) = item.item else {
                return Err(BackingError::Unsupported);
            };
            let mut found = None;
            for layer in &doc.layers {
                c.check(1)?;
                if layer.id == id {
                    found = Some(layer);
                }
            }
            let layer = found.ok_or(BackingError::Integrity)?;
            if !names_equal(&item.name, &layer.name, c)? || item.visible != layer.visible {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok(())
}

pub(super) fn synchronize(doc: &mut Document, edit: AccountedEdit<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if let AccountedEdit::LayerMetadata { layer, name, visible, .. } = edit {
        if let Some(name) = name {
            synchronize(doc, AccountedEdit::LayerName { layer, name }, c)?;
        }
        if let Some(visible) = visible {
            synchronize(doc, AccountedEdit::LayerVisibility { layer, visible }, c)?;
        }
        return Ok(());
    }
    let Some(state) = &mut doc.xcf_state else {
        return Ok(());
    };
    let id = crate::item_identity::ItemRef::Layer(edit.layer_id());
    let mut found = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == id {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(index);
        }
    }
    let item = state.items.get_mut(found.ok_or(BackingError::Integrity)?).ok_or(BackingError::Integrity)?;
    match edit {
        AccountedEdit::LayerName { name, .. } => {
            if !names_equal(&item.name, name, c)? {
                item.name = copy_name(name, c)?;
                item.custom_name = true;
            }
        }
        AccountedEdit::LayerVisibility { visible, .. } => item.visible = visible,
        AccountedEdit::LayerOpacity { .. } => {} // No interpretation of original composition.
        _ => return Err(BackingError::Unsupported),
    }
    Ok(())
}

pub(super) fn validate_name(name: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if name.len() > 1 << 20 {
        return Err(BackingError::Length);
    }
    for chunk in name.as_bytes().chunks(32768) {
        c.check(chunk.len() as u64)?;
        if chunk.contains(&0) {
            return Err(BackingError::Length);
        }
    }
    Ok(())
}

pub(super) fn names_equal(a: &str, b: &str, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    if a.len() != b.len() {
        c.check(1)?;
        return Ok(false);
    }
    for (a, b) in a.as_bytes().chunks(32768).zip(b.as_bytes().chunks(32768)) {
        c.check((a.len() as u64).checked_mul(2).ok_or(BackingError::Length)?)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn copy_name(name: &str, c: &ReadControl<'_>) -> Result<String, BackingError> {
    let mut owned = String::new();
    owned.try_reserve_exact(name.len()).map_err(|_| BackingError::Allocation)?;
    if owned.capacity() > name.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    for ch in name.chars() {
        c.check(ch.len_utf8() as u64)?;
        owned.push(ch);
    }
    Ok(owned)
}

#[cfg(test)]
#[path = "accounted_imported_edits_tests.rs"]
mod tests;
