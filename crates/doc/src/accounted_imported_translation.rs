//! Closed current coordinate translation of imported plain native layers.
//! This is model custody only: it establishes no source, profile, rendering,
//! filesystem or lock authority. The engine must enforce its command policy.
use super::*;
use crate::item_identity::ItemRef;

struct Plan {
    layer_index: usize,
    item_index: usize,
    translated: crate::BackedLayer,
}

fn plan(owner: &AccountedDocument, layer: LayerId, dx: i32, dy: i32, control: &ReadControl<'_>) -> Result<Plan, BackingError> {
    control.check(1)?;
    let doc = owner.document();
    if owner.owner._storage_class != StorageClass::Legacy
        || owner.owner._selection.is_some()
        || owner.owner._selection_view.is_some()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || doc.source_color_context.is_none()
        || doc.xcf_state.as_ref().is_none_or(|s| s.provenance.format != "xcf")
    {
        return Err(BackingError::Unsupported);
    }
    // Existing plain current validation covers all layers and canonical rows,
    // exact native bounds, shared ledger, and the closed mask/filter/group gates.
    imported_selection::validate(doc, owner.quota(), control)?;
    owner.validate_imported_metadata_edit(AccountedEdit::LayerMetadata { layer, name: None, visible: None, opacity: None }, control)?;
    let mut layer_index = None;
    for (index, candidate) in doc.layers.iter().enumerate() {
        control.check(1)?;
        if candidate.id == layer {
            if layer_index.replace(index).is_some() {
                return Err(BackingError::Integrity);
            }
        }
    }
    let layer_index = layer_index.ok_or(BackingError::Bounds)?;
    let LayerContent::BackedRaster(backed) = &doc.layers.get(layer_index).ok_or(BackingError::Bounds)?.content else {
        return Err(BackingError::Unsupported);
    };
    let mut item_index = None;
    for (index, item) in doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?.items.iter().enumerate() {
        control.check(1)?;
        if item.item == ItemRef::Layer(layer) && item_index.replace(index).is_some() {
            return Err(BackingError::Integrity);
        }
    }
    let item_index = item_index.ok_or(BackingError::Integrity)?;
    let translated = backed.try_translated(dx, dy, control).map_err(|e| match e {
        BackedLayerError::Backing(e) => e,
        BackedLayerError::DescriptorMismatch => BackingError::Integrity,
    })?;
    Ok(Plan { layer_index, item_index, translated })
}

impl AccountedDocument {
    /// Allocation-free typed preflight over genuine current plain storage.
    /// This does not authorize the command or bypass effective layer locks.
    pub fn validate_imported_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, control: &ReadControl<'_>) -> Result<(), BackingError> {
        plan(self, layer, dx, dy, control)?;
        control.check(1)
    }

    /// Stage a coordinate view and its exact canonical raster bounds. Immutable
    /// source/native/default/profile owners and the original retention guard are
    /// shared; no full image allocation, numeric conversion or locator read occurs.
    pub fn try_with_imported_layer_translation(&self, layer: LayerId, dx: i32, dy: i32, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        let plan = plan(self, layer, dx, dy, control)?;
        if dx == 0 && dy == 0 {
            control.check(1)?;
            return Ok(self.clone());
        }
        let logical = selection_census(self.document(), false, None, self.quota(), control)?;
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index = logical.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        let index_envelope = index.checked_mul(2).and_then(|n| n.checked_add(4096)).ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index_lease = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload_lease = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        control.check(logical.total()?)?;
        let document = imported_selection::copy_document(self.document(), control)?;
        let mut staged = ModelStaging { document, selection_view: None };
        let bounds = plan.translated.operative_bounds();
        staged.document.layers.get_mut(plan.layer_index).ok_or(BackingError::Integrity)?.content = LayerContent::BackedRaster(plan.translated);
        staged.document.xcf_state.as_mut().ok_or(BackingError::Integrity)?.items.get_mut(plan.item_index).ok_or(BackingError::Integrity)?.raster_bounds =
            Some(bounds);
        imported_selection::validate(&staged.document, self.quota(), control)?;
        let retained = selection_census(&staged.document, true, None, self.quota(), control)?;
        let retained_index = retained.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        if retained_index > index_envelope || retained.payload > payload_envelope {
            return Err(BackingError::Allocation);
        }
        index_lease.reconcile_capacity(retained_index).map_err(|_| BackingError::Quota)?;
        payload_lease.reconcile_capacity(retained.payload).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let ModelStaging { document, selection_view } = staged;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: None,
                _selection_view: selection_view,
                retention: self.owner.retention.clone(),
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index_lease, _payload: payload_lease }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
}

#[cfg(test)]
#[path = "accounted_imported_translation_tests.rs"]
mod tests;
