//! Structural singleton metadata copying, independent of operation eligibility.
//! A caller publishing an operative command must separately prove its complete
//! CURRENT recipe. This adapter grants no source, profile or renderer authority.
use super::*;

fn prepare<'a>(
    owner: &'a AccountedDocument,
    edit: AccountedEdit<'a>,
    control: &ReadControl<'_>,
) -> Result<(ModelCensus, Option<readonly_layer_properties::Plan<'a>>), BackingError> {
    control.check(1)?;
    if !owner.is_readonly_point_storage() && !owner.is_readonly_assigned_point_storage() {
        return refused();
    }
    let AccountedEdit::LayerMetadata { layer, name, visible, opacity } = edit else { return refused() };
    let patch = ReadonlyLayerPropertiesPatch { layer, name, visible, opacity_f32_bits: opacity.map(f32::to_bits) };
    readonly_layer_properties::validate(patch, control)?;
    let doc = owner.document();
    let logical = measured_owner(owner, doc, control)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if doc.layers.len() != 1 || state.filter_stacks.len() != 1 || state.filter_stacks.first().is_none_or(|stack| stack.nodes.len() != 1) {
        return refused();
    }
    let target = doc.layers.first().filter(|target| target.id == layer).ok_or(BackingError::Bounds)?;
    if !target.opacity.is_finite() || !(0.0..=1.0).contains(&target.opacity) {
        return Err(BackingError::Bounds);
    }
    let mut item = None;
    for candidate in &state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(layer) && item.replace(candidate).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item = item.ok_or(BackingError::Integrity)?;
    // Malformed native canonical divergence refuses even an all-None request;
    // omitted fields do not repair unrelated state or allocate hidden names.
    if !imported_edits::names_equal(&target.name, &item.name, control)? || target.visible != item.visible {
        return Err(BackingError::Integrity);
    }
    let plan = readonly_layer_properties::plan(doc, patch, control)?;
    Ok((logical, plan))
}

pub(in crate::accounted_document) fn validate(owner: &AccountedDocument, edit: AccountedEdit<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
    prepare(owner, edit, control)?;
    Ok(())
}

pub(in crate::accounted_document) fn edited(
    owner: &AccountedDocument,
    edit: AccountedEdit<'_>,
    control: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    let (logical, plan) = prepare(owner, edit, control)?;
    let Some(plan) = plan else {
        control.check(1)?;
        return Ok(owner.clone());
    };
    let kind = if owner.is_readonly_assigned_point_storage() { CopyKind::AssignedLayerProperties(plan) } else { CopyKind::LayerProperties(plan) };
    current_copy(owner, logical, kind, control)
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
