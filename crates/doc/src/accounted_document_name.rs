//! Paid file-identity name preparation for the closed imported plain class.
//! This changes no pixels, original evidence, layer name, or undo cursor.
use super::*;
impl AccountedDocument {
    pub fn try_with_document_name(&self, name: &str, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        if self.owner._storage_class == StorageClass::ReadonlyFlatComposite {
            return flat_composite_readonly::renamed(self, name, control);
        }
        if self.owner._storage_class == StorageClass::ReadonlyGaussianGroup {
            return gaussian_group_readonly::renamed(self, name, control);
        }
        if matches!(self.owner._storage_class, StorageClass::ReadonlyPoint | StorageClass::ReadonlyAssignedPoint) {
            return point_readonly::renamed(self, name, control);
        }
        if self.owner._storage_class == StorageClass::ReadonlyThresholdAlpha {
            return filter_readonly::renamed(self, name, control);
        }
        if self.owner._storage_class == StorageClass::ReadonlyHistogramThreshold {
            return threshold_readonly::renamed(self, name, control);
        }
        if !imported_selection::applies(self.document()) || name.len() > 4096 {
            return Err(BackingError::Unsupported);
        }
        imported_edits::validate_name(name, control)?;
        // Dispatch only this file-identity copy; generic selection and layer
        // edits retain the original strict unmasked validator.
        let isolated_group =
            self.document().layers.len() == 2 && self.document().layers.get(1).is_some_and(|layer| matches!(layer.content, LayerContent::Group(_)));
        let masked_normal = self.document().layers.len() == 2 && self.document().layers.get(1).is_some_and(|layer| layer.mask.is_some());
        if isolated_group {
            imported_isolated_group::validate(self.document(), self.quota(), control)?;
        } else if masked_normal {
            imported_selection::validate_assigned_geometry(self.document(), self.quota(), control)?;
        } else {
            imported_selection::validate(self.document(), self.quota(), control)?;
        }
        let logical = selection_census(self.document(), false, self.owner._selection.as_ref(), self.quota(), control)?;
        if imported_edits::names_equal(&self.document().name, name, control)? {
            return Ok(self.clone());
        }
        let owner_bytes = (std::mem::size_of::<ModelOwner>() + 128) as u64;
        let index = logical.index.checked_sub(owner_bytes).ok_or(BackingError::Integrity)?;
        let index_envelope = index
            .checked_add((name.len() as u64).checked_mul(2).ok_or(BackingError::Length)?)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(4096))
            .ok_or(BackingError::Length)?;
        let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
        let allocation = self.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
        let mut index_lease = self.quota().reserve(QuotaKind::Index, index_envelope).map_err(|_| BackingError::Quota)?;
        let mut payload_lease = self.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
        control.check(logical.total()?)?;
        let view = self.owner._selection.as_ref().map(|guard| guard.clone_view(control)).transpose()?;
        let document = if isolated_group {
            imported_selection::copy_isolated_group(self.document(), self.quota(), control)?
        } else if masked_normal {
            imported_selection::copy_masked_normal(self.document(), self.quota(), control)?
        } else {
            imported_selection::copy_document(self.document(), control)?
        };
        let mut staged = ModelStaging { document, selection_view: None };
        if let Some(SelectionView { surface, charges }) = view {
            staged.document.selection = Some(surface);
            staged.selection_view = Some(charges);
        }
        staged.document.name = imported_edits::copy_name(name, control)?;
        let retained = selection_census(&staged.document, true, self.owner._selection.as_ref(), self.quota(), control)?;
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
                _selection: self.owner._selection.clone(),
                _selection_view: selection_view,
                retention: self.owner.retention.clone(),
                quota: self.quota().clone(),
                _model: Some(ModelCharges { _index: index_lease, _payload: payload_lease }),
                _allocation: Arc::new(allocation),
            }),
        })
    }
}
