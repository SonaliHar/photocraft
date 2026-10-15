//! Atomic bounded plain targets; no selection, lock, link-expansion or render
//! authority follows from the caller's borrowed ID list.
use super::*;

fn validate_ids(owner: &AccountedDocument, layers: &[LayerId], c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    if layers.is_empty() || layers.len() > 4096 {
        return Err(BackingError::Bounds);
    }
    for (index, id) in layers.iter().enumerate() {
        c.check(1)?;
        if id.0 == 0 {
            return Err(BackingError::Bounds);
        }
        for previous in layers.get(..index).ok_or(BackingError::Integrity)? {
            c.check(1)?;
            if previous == id {
                return Err(BackingError::Integrity);
            }
        }
    }
    validate_owner(owner, *layers.first().ok_or(BackingError::Bounds)?, c)
}
fn checked_target(owner: &AccountedDocument, id: LayerId, dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Plan, BackingError> {
    let plan = target_plan(owner, id, dx, dy, c)?;
    if owner.document().layers.get(plan.layer_index).ok_or(BackingError::Integrity)?.link_group.is_some() {
        return Err(BackingError::Unsupported);
    }
    Ok(plan)
}
impl AccountedDocument {
    /// Allocation-free bounded preflight. A singleton keeps the existing single
    /// API's validation/control schedule; multiple IDs are unique actual roots.
    pub fn validate_imported_layers_translation(&self, layers: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if let [id] = layers {
            return self.validate_imported_layer_translation(*id, dx, dy, c);
        }
        validate_ids(self, layers, c)?;
        for id in layers {
            checked_target(self, *id, dx, dy, c)?;
        }
        c.check(1)
    }
    /// One private metadata copy and one final actual census for all targets.
    /// No target mutation is published if any target or admission fails.
    pub fn try_with_imported_layers_translation(&self, layers: &[LayerId], dx: i32, dy: i32, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        if let [id] = layers {
            return self.try_with_imported_layer_translation(*id, dx, dy, c);
        }
        if dx == 0 && dy == 0 {
            self.validate_imported_layers_translation(layers, dx, dy, c)?;
            return Ok(self.clone());
        }
        validate_ids(self, layers, c)?;
        let requested = layers.len();
        let plan_envelope = (requested as u64)
            .checked_mul(std::mem::size_of::<Plan>() as u64)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(128))
            .ok_or(BackingError::Length)?;
        let mut plan_fee = self.quota().reserve(QuotaKind::Index, plan_envelope).map_err(|_| BackingError::Quota)?;
        c.check(plan_envelope)?;
        let mut plans = Vec::new();
        plans.try_reserve_exact(requested).map_err(|_| BackingError::Allocation)?;
        if plans.capacity() > requested.checked_mul(2).ok_or(BackingError::Length)? {
            return Err(BackingError::Allocation);
        }
        let actual = (plans.capacity() as u64).checked_mul(std::mem::size_of::<Plan>() as u64).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
        plan_fee.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
        for id in layers {
            c.check(std::mem::size_of::<Plan>() as u64)?;
            plans.push(checked_target(self, *id, dx, dy, c)?);
        }
        let logical = selection_census(self.document(), false, None, self.quota(), c)?;
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index_envelope =
            logical.index.checked_sub(owner_bytes).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(4096)).ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        c.check(logical.total()?)?;
        let document = imported_selection::copy_document(self.document(), c)?;
        let mut staged = ModelStaging { document, selection_view: None };
        for plan in &plans {
            c.check(1)?;
            let bounds = plan.translated.operative_bounds();
            let target = staged.document.layers.get_mut(plan.layer_index).ok_or(BackingError::Integrity)?;
            target.content = LayerContent::BackedRaster(plan.translated.clone());
            shift_reference(target, dx, dy, c)?;
            staged.document.xcf_state.as_mut().ok_or(BackingError::Integrity)?.items.get_mut(plan.item_index).ok_or(BackingError::Integrity)?.raster_bounds =
                Some(bounds);
        }
        imported_selection::validate(&staged.document, self.quota(), c)?;
        let retained = selection_census(&staged.document, true, None, self.quota(), c)?;
        let retained_index = retained.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        if retained_index > index_envelope || retained.payload > payload_envelope {
            return Err(BackingError::Allocation);
        }
        index.reconcile_capacity(retained_index).map_err(|_| BackingError::Quota)?;
        payload.reconcile_capacity(retained.payload).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let ModelStaging { document, selection_view } = staged;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::Legacy,
                document,
                _selection: None,
                _selection_view: selection_view,
                retention: self.owner.retention.clone(),
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index, _payload: payload }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
}

#[cfg(test)]
#[path = "accounted_imported_translation_bulk_tests.rs"]
mod tests;
