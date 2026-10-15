//! Structural singleton coordinate views. Command, source, profile and rendering
//! eligibility remain separate; this grants no generic edit or mask authority.
use super::*;

pub(in crate::accounted_document) struct Plan {
    layer: LayerId,
    item_index: usize,
    translated: crate::BackedLayer,
    dx: i32,
    dy: i32,
}

fn prepare(owner: &AccountedDocument, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<(ModelCensus, Plan), BackingError> {
    c.check(1)?;
    if !owner.is_readonly_point_storage() && !owner.is_readonly_assigned_point_storage() {
        return refused();
    }
    let doc = owner.document();
    let logical = measured_owner(owner, doc, c)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    if doc.layers.len() != 1 || state.filter_stacks.len() != 1 || state.filter_stacks.first().is_none_or(|s| s.nodes.len() != 1) {
        return refused();
    }
    let target = doc.layers.first().filter(|v| v.id == layer).ok_or(BackingError::Bounds)?;
    if target.link_group.is_some() || target.mask.is_some() || target.vector_mask.is_some() {
        return refused();
    }
    if !target.opacity.is_finite() || !(0.0..=1.0).contains(&target.opacity) {
        return Err(BackingError::Bounds);
    }
    let mut item_index = None;
    for (index, item) in state.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Layer(layer) && item_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    let item = state.items.get(item_index).ok_or(BackingError::Integrity)?;
    if !imported_edits::names_equal(&target.name, &item.name, c)? || target.visible != item.visible {
        return Err(BackingError::Integrity);
    }
    let LayerContent::BackedRaster(backed) = &target.content else { return refused() };
    let translated = backed.try_translated(dx, dy, c).map_err(|e| match e {
        BackedLayerError::Backing(e) => e,
        BackedLayerError::DescriptorMismatch => BackingError::Integrity,
    })?;
    // The raster descriptor admits checked i32 coordinates. Point storage has
    // its own existing tighter coordinate bounds, checked before model copying.
    rectangle(translated.operative_bounds())?;
    c.check(1)?;
    Ok((logical, Plan { layer, item_index, translated, dx, dy }))
}

pub(super) fn apply(doc: &mut Document, plan: &Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let target = doc.layers.first_mut().filter(|v| v.id == plan.layer).ok_or(BackingError::Integrity)?;
    target.content = LayerContent::BackedRaster(plan.translated.clone());
    imported_translation::shift_reference(target, plan.dx, plan.dy, c)?;
    let item = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?.items.get_mut(plan.item_index).ok_or(BackingError::Integrity)?;
    if item.item != ItemRef::Layer(plan.layer) {
        return Err(BackingError::Integrity);
    }
    item.raster_bounds = Some(plan.translated.operative_bounds());
    // Effect-selection masks intentionally stay canvas-bound. They are not
    // linked physical layer masks; no node, mask or stack revision is changed.
    c.check(1)
}

impl AccountedDocument {
    /// Allocation-free structural preflight, including valid zero vectors. It
    /// grants no command/lock or operative Box/profile eligibility.
    pub fn validate_readonly_point_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<(), BackingError> {
        prepare(self, layer, dx, dy, c)?;
        c.check(1)
    }

    /// Stage one paid fixed-root native view and canonical layer bounds while
    /// retaining both profile roles, source custody and canvas filter masks.
    pub fn try_with_readonly_point_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        let (logical, plan) = prepare(self, layer, dx, dy, c)?;
        if dx == 0 && dy == 0 {
            c.check(1)?;
            return Ok(self.clone());
        }
        let kind = if self.is_readonly_assigned_point_storage() { CopyKind::AssignedTranslation(&plan) } else { CopyKind::Translation(&plan) };
        current_copy(self, logical, kind, c)
    }
}

#[cfg(test)]
#[path = "translation_tests.rs"]
mod tests;
