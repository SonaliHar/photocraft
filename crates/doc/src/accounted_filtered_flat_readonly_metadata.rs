//! Compound CURRENT layer metadata only. No native, selection, Move, mask,
//! individual-property or composition authority follows from this plan.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Plan<'a> {
    layer: LayerId,
    layer_index: usize,
    item_index: usize,
    name: Option<&'a str>,
    visible: Option<bool>,
    opacity: Option<f32>,
}
impl Plan<'_> {
    pub(super) fn replacement_name_bytes(self) -> Result<u64, BackingError> {
        // The common copy envelope doubles this slack, paying both replacement
        // Strings at up to twice their borrowed length before either allocates.
        self.name.map_or(Ok(0), |v| (v.len() as u64).checked_mul(2).ok_or(BackingError::Length))
    }
}
fn prepare<'a>(owner: &AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<(ModelCensus, Option<Plan<'a>>), BackingError> {
    c.check(1)?;
    if !accepts(owner) {
        return Err(BackingError::Unsupported);
    }
    let AccountedEdit::LayerMetadata { layer, name, visible, opacity } = edit else {
        return Err(BackingError::Unsupported);
    };
    if let Some(name) = name {
        imported_edits::validate_name(name, c)?;
    }
    if opacity.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
        return Err(BackingError::Bounds);
    }
    // Full actual storage/census and coherent canonical prestate precede every
    // all-None or raw-bit no-op. Omitted fields never repair malformed state.
    let logical = measured_owner(owner, c)?;
    let mut target = None;
    for (index, candidate) in owner.document().layers.iter().enumerate() {
        c.check(1)?;
        if candidate.id == layer {
            if target.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let layer_index = target.ok_or(BackingError::Bounds)?;
    let target = owner.document().layers.get(layer_index).ok_or(BackingError::Integrity)?;
    let name_changed = match name {
        Some(name) => !imported_edits::names_equal(&target.name, name, c)?,
        None => false,
    };
    let changed = name_changed || visible.is_some_and(|v| v != target.visible) || opacity.is_some_and(|v| v.to_bits() != target.opacity.to_bits());
    let state = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut item_index = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Layer(layer) && item_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    c.check(1)?;
    Ok((logical, changed.then_some(Plan { layer, layer_index, item_index, name: if name_changed { name } else { None }, visible, opacity })))
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
    let Some(plan) = plan else {
        return Ok(owner.clone());
    };
    current_copy(owner, logical, CopyKind::Metadata(plan), c)
}
pub(super) fn apply(doc: &mut Document, plan: Plan<'_>, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if state.items.get(plan.item_index).is_none_or(|i| i.item != ItemRef::Layer(plan.layer))
        || doc.layers.get(plan.layer_index).is_none_or(|l| l.id != plan.layer)
    {
        return Err(BackingError::Integrity);
    }
    let name = plan.name.map(|v| imported_edits::copy_name(v, c)).transpose()?;
    let target = doc.layers.get_mut(plan.layer_index).ok_or(BackingError::Integrity)?;
    if let Some(name) = name {
        target.name = name;
    }
    if let Some(visible) = plan.visible {
        target.visible = visible;
    }
    if let Some(opacity) = plan.opacity {
        target.opacity = opacity;
    }
    imported_edits::synchronize(doc, AccountedEdit::LayerMetadata { layer: plan.layer, name: plan.name, visible: plan.visible, opacity: plan.opacity }, c)
}
