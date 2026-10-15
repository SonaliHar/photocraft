//! Name-only current-layer copies for genuine readonly native storage.
//! This grants no generic metadata, native-pixel or original-source edit.
use super::*;
use crate::item_identity::ItemRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadonlyLayerNamePatch<'a> {
    pub layer: LayerId,
    pub name: &'a str,
}

impl AccountedDocument {
    /// Retain native planes, raw masks, profiles and original source owners;
    /// replace only the current layer and canonical item name. Command locks
    /// and history/view publication remain the engine's responsibility.
    pub fn try_with_readonly_layer_name(&self, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        match self.owner._storage_class {
            StorageClass::ReadonlyThresholdAlpha => filter_readonly::layer_named(self, patch, control),
            StorageClass::ReadonlyHistogramThreshold => threshold_readonly::layer_named(self, patch, control),
            StorageClass::ReadonlyPoint => point_readonly::layer_named(self, patch, control),
            StorageClass::ReadonlyAssignedPoint | StorageClass::Legacy | StorageClass::ReadonlyGaussianGroup | StorageClass::ReadonlyFlatComposite => {
                Err(BackingError::Unsupported)
            }
        }
    }
}

pub(super) fn changed(doc: &Document, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<bool, BackingError> {
    let layer = doc.layers.first().filter(|layer| layer.id == patch.layer).ok_or(BackingError::Bounds)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let mut item = None;
    for candidate in &state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(patch.layer) {
            if item.is_some() {
                return Err(BackingError::Integrity);
            }
            item = Some(candidate);
        }
    }
    let item = item.ok_or(BackingError::Integrity)?;
    let layer_equal = imported_edits::names_equal(&layer.name, patch.name, control)?;
    let item_equal = imported_edits::names_equal(&item.name, patch.name, control)?;
    Ok(!layer_equal || !item_equal)
}

pub(super) fn apply(doc: &mut Document, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<(), BackingError> {
    let layer_name = imported_edits::copy_name(patch.name, control)?;
    let item_name = imported_edits::copy_name(patch.name, control)?;
    let layer = doc.layers.first_mut().filter(|layer| layer.id == patch.layer).ok_or(BackingError::Bounds)?;
    layer.name = layer_name;
    let state = doc.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    let mut item = None;
    for candidate in &mut state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(patch.layer) {
            if item.is_some() {
                return Err(BackingError::Integrity);
            }
            item = Some(candidate);
        }
    }
    let item = item.ok_or(BackingError::Integrity)?;
    // Ordinary layer.setProps first changes Layer.name; reconciliation marks
    // the canonical item custom only when its own name actually differs.
    if !imported_edits::names_equal(&item.name, patch.name, control)? {
        item.name = item_name;
        item.custom_name = true;
    }
    Ok(())
}

#[cfg(test)]
#[path = "accounted_readonly_layer_name_tests.rs"]
mod tests;
