//! Atomic CURRENT coordinate views over actual flat-composite native custody.
//! Selection, locks, rendering and source eligibility remain caller authority.
use super::*;
use crate::{BackedLayer, BackedLayerError, item_identity::ItemRef};

struct Target {
    id: LayerId,
    layer_index: usize,
    item_index: usize,
    translated: BackedLayer,
}
pub(super) struct Plan {
    targets: Vec<Target>,
    dx: i32,
    dy: i32,
    _fee: QuotaLease,
}
fn preflight(owner: &AccountedDocument, ids: &[LayerId], c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    c.check(1)?;
    if !owner.is_readonly_flat_composite_storage() {
        return Err(BackingError::Unsupported);
    }
    let logical = measured(owner.document(), owner.quota(), c)?;
    if ids.is_empty() || ids.len() > 4096 {
        return Err(BackingError::Bounds);
    }
    for (index, id) in ids.iter().enumerate() {
        c.check(1)?;
        if id.0 == 0 {
            return Err(BackingError::Bounds);
        }
        for previous in ids.get(..index).ok_or(BackingError::Integrity)? {
            c.check(1)?;
            if previous == id {
                return Err(BackingError::Integrity);
            }
        }
    }
    Ok(logical)
}
fn target(owner: &AccountedDocument, id: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Target, BackingError> {
    let doc = owner.document();
    let mut layer_index = None;
    for (index, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        if layer.id == id && layer_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let layer = doc.layers.get(layer_index).ok_or(BackingError::Integrity)?;
    if layer.link_group.is_some() || layer.mask.is_some() || layer.vector_mask.is_some() {
        return Err(BackingError::Unsupported);
    }
    let LayerContent::BackedRaster(backed) = &layer.content else {
        return Err(BackingError::Unsupported);
    };
    let mut item_index = None;
    for (index, item) in doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?.items.iter().enumerate() {
        c.check(1)?;
        if item.item == ItemRef::Layer(id) && item_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    let translated = backed.try_translated(dx, dy, c).map_err(|e| match e {
        BackedLayerError::Backing(e) => e,
        BackedLayerError::DescriptorMismatch => BackingError::Integrity,
    })?;
    Ok(Target { id, layer_index, item_index, translated })
}
pub(super) fn apply(doc: &mut Document, plan: &Plan, c: &ReadControl<'_>) -> Result<(), BackingError> {
    for target in &plan.targets {
        c.check(1)?;
        let layer = doc.layers.get_mut(target.layer_index).ok_or(BackingError::Integrity)?;
        if layer.id != target.id {
            return Err(BackingError::Integrity);
        }
        layer.content = LayerContent::BackedRaster(target.translated.clone());
        // Reuse ordinary Move's exact arithmetic, including the zero axis of a
        // nonzero vector and nonfinite/signed-zero inert reference components.
        imported_translation::shift_reference(layer, plan.dx, plan.dy, c)?;
        let item = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?.items.get_mut(target.item_index).ok_or(BackingError::Integrity)?;
        if item.item != ItemRef::Layer(target.id) {
            return Err(BackingError::Integrity);
        }
        item.raster_bounds = Some(target.translated.operative_bounds());
    }
    c.check(1)
}
impl AccountedDocument {
    /// Bounded allocation-free target preflight; this grants no command or lock
    /// authority and performs no native pixel materialization.
    pub fn validate_flat_composite_layers_translation(&self, ids: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<(), BackingError> {
        preflight(self, ids, c)?;
        for id in ids {
            target(self, *id, dx, dy, c)?;
        }
        c.check(1)
    }
    /// One private paid model copy for every requested fixed-root native view.
    pub fn try_with_flat_composite_layers_translation(&self, ids: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        if dx == 0 && dy == 0 {
            self.validate_flat_composite_layers_translation(ids, dx, dy, c)?;
            return Ok(self.clone());
        }
        let logical = preflight(self, ids, c)?;
        let ceiling = ids.len().checked_mul(2).ok_or(BackingError::Length)?;
        let envelope =
            ceiling.checked_mul(std::mem::size_of::<Target>()).and_then(|v| v.checked_add(std::mem::size_of::<Plan>())).ok_or(BackingError::Length)?;
        let fee = self.quota().reserve(QuotaKind::Index, envelope as u64).map_err(|_| BackingError::Quota)?;
        let mut plan = Plan { targets: Vec::new(), dx, dy, _fee: fee };
        c.check(envelope as u64)?;
        plan.targets.try_reserve_exact(ids.len()).map_err(|_| BackingError::Allocation)?;
        if plan.targets.capacity() > ceiling {
            return Err(BackingError::Allocation);
        }
        let actual = plan
            .targets
            .capacity()
            .checked_mul(std::mem::size_of::<Target>())
            .and_then(|v| v.checked_add(std::mem::size_of::<Plan>()))
            .ok_or(BackingError::Length)?;
        plan._fee.reconcile_capacity(actual as u64).map_err(|_| BackingError::Quota)?;
        for id in ids {
            c.check(std::mem::size_of::<Target>() as u64)?;
            plan.targets.push(target(self, *id, dx, dy, c)?);
        }
        current_copy(self, logical, CopyKind::Translation(&plan), c)
    }
}

#[cfg(test)]
#[path = "accounted_flat_composite_readonly_translation_tests.rs"]
mod tests;
