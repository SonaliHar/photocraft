//! Flat immutable imported composition custody. This runtime class proves no
//! operation, profile, source, renderer or editable-domain eligibility.
use super::*;
use crate::{ColorMode, FilterOutputPrecision, SampleType, item_identity::ItemRef};
#[path = "accounted_flat_composite_readonly_composition.rs"]
mod composition;
#[path = "accounted_flat_composite_readonly_metadata.rs"]
pub(super) mod metadata;
pub use composition::FlatCompositeCompositionPatch;

fn text(value: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if value.len() > 1 << 20 {
        return Err(BackingError::Length);
    }
    for chunk in value.as_bytes().chunks(32 << 10) {
        c.check(chunk.len() as u64)?;
        if chunk.contains(&0) {
            return Err(BackingError::Integrity);
        }
    }
    c.check(1)
}
fn hash(value: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if value.len() != 64 {
        return Err(BackingError::Integrity);
    }
    c.check(64)?;
    if !value.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(BackingError::Integrity);
    }
    Ok(())
}
fn digest(bytes: &[u8], expected: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if bytes.len() > 64 << 20 {
        return Err(BackingError::Length);
    }
    hash(expected, c)?;
    c.check(std::mem::size_of::<blake3::Hasher>() as u64 + 128)?;
    let mut digest = blake3::Hasher::new();
    for chunk in bytes.chunks(32 << 10) {
        c.check(chunk.len() as u64 + 1)?;
        digest.update(chunk);
    }
    c.check(128)?;
    if digest.finalize().to_hex().as_str() != expected {
        return Err(BackingError::Integrity);
    }
    c.check(1)
}
fn present(doc: &Document, id: ItemRef, c: &ReadControl<'_>) -> Result<(), BackingError> {
    let ItemRef::Layer(id) = id else {
        return Err(BackingError::Integrity);
    };
    for layer in &doc.layers {
        c.check(1)?;
        if layer.id == id {
            return Ok(());
        }
    }
    Err(BackingError::Integrity)
}
fn shape(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    if doc.size.width == 0
        || doc.size.height == 0
        || doc.size.width > 262144
        || doc.size.height > 262144
        || u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or(BackingError::Length)? > 1 << 30
        || !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(doc.depth, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.current_filter_output_precision != FilterOutputPrecision::Canonical
        || state.opaque.len() > 4096
        || state.provenance.protected_source_fingerprints.len() > 4096
    {
        return Err(BackingError::Unsupported);
    }
    // Existing flat relationship validator is allocation-free and checks every
    // native ledger, unique layer/item identity and exact operative rectangle.
    imported_selection::validate(doc, q, c)?;
    text(&doc.name, c)?;
    // Syntax is an inert hash invariant. Matching this marker to actual current
    // ICC bytes and interpreting either profile remain the renderer's duty.
    match &doc.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => {}
        WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } | WorkingProfileInterpretation::ImplicitSGray { icc_blake3 } => hash(icc_blake3, c)?,
    }
    for layer in &doc.layers {
        c.check(1)?;
        let LayerContent::BackedRaster(backed) = &layer.content else {
            return Err(BackingError::Unsupported);
        };
        let bounds = backed.operative_bounds();
        let width = i64::from(bounds.x1) - i64::from(bounds.x0);
        let height = i64::from(bounds.y1) - i64::from(bounds.y0);
        if width <= 0 || height <= 0 || width > 262144 || height > 262144 {
            return Err(BackingError::Bounds);
        }
        if backed.format().mode != doc.mode || backed.format().sample != doc.depth || layer.link_group.is_some() {
            return Err(BackingError::Unsupported);
        }
        text(&layer.name, c)?;
    }
    for item in &state.items {
        c.check(1)?;
        text(&item.name, c)?;
        let ItemRef::Layer(id) = item.item else {
            return Err(BackingError::Integrity);
        };
        let mut matched = false;
        for layer in &doc.layers {
            c.check(1)?;
            if layer.id == id {
                if item.visible != layer.visible || !imported_edits::names_equal(&item.name, &layer.name, c)? {
                    return Err(BackingError::Integrity);
                }
                matched = true;
            }
        }
        if !matched {
            return Err(BackingError::Integrity);
        }
        if let Some(policy) = &item.composition {
            // Preserve raw/resolved policy without treating Auto, epochs or
            // source modes as a semantic operation or colour-space grant.
            text(&policy.compatibility_epoch, c)?;
            if policy.compatibility_epoch.is_empty() {
                return Err(BackingError::Integrity);
            }
            if !(-4..=4).contains(&policy.stored_blend_space)
                || !(-4..=4).contains(&policy.stored_composite_space)
                || !(-2..=4).contains(&policy.stored_operator)
            {
                return Err(BackingError::Integrity);
            }
            let resolved = policy.resolved.as_ref().ok_or(BackingError::Integrity)?;
            use crate::import_compositing::{GimpOperator, GimpSpace};
            let space = |raw| match raw {
                1 => Some(GimpSpace::LinearRgb),
                2 => Some(GimpSpace::ProfileRgb),
                3 => Some(GimpSpace::Lab),
                4 => Some(GimpSpace::PerceptualRgb),
                _ => None,
            };
            let operator = match policy.stored_operator {
                1 => Some(GimpOperator::Union),
                2 => Some(GimpOperator::ClipToBackdrop),
                3 => Some(GimpOperator::ClipToLayer),
                4 => Some(GimpOperator::Intersection),
                _ => None,
            };
            if space(policy.stored_blend_space).is_some_and(|v| v != resolved.blend_space)
                || space(policy.stored_composite_space).is_some_and(|v| v != resolved.composite_space)
                || operator.is_some_and(|v| v != resolved.operator)
            {
                return Err(BackingError::Integrity);
            }
        }
    }
    for (index, id) in state.selection.selected.iter().enumerate() {
        present(doc, *id, c)?;
        for previous in state.selection.selected.get(..index).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous == id {
                return Err(BackingError::Integrity);
            }
        }
    }
    if let Some(active) = state.selection.active {
        let mut selected = false;
        for id in &state.selection.selected {
            c.check(1)?;
            selected |= *id == active;
        }
        if !selected {
            return Err(BackingError::Integrity);
        }
    }
    let p = &state.provenance;
    for value in [&p.format, &p.container, &p.source_label] {
        text(value, c)?;
    }
    for value in [&p.producer, &p.release_id, &p.original_locator].into_iter().flatten() {
        text(value, c)?;
    }
    if p.format != "xcf" {
        return Err(BackingError::Unsupported);
    }
    if let Some(value) = &p.source_hash {
        hash(value, c)?;
    }
    for value in &p.protected_source_fingerprints {
        hash(value, c)?;
    }
    let mut payload = 0usize;
    for raw in &state.opaque {
        c.check(1)?;
        if raw.rendering_required {
            return Err(BackingError::Unsupported);
        }
        if let Some(id) = raw.owner {
            present(doc, id, c)?;
        }
        text(&raw.namespace, c)?;
        text(&raw.identity, c)?;
        payload = payload.checked_add(raw.bytes.len()).ok_or(BackingError::Length)?;
        if payload > 256 << 20 {
            return Err(BackingError::Length);
        }
        digest(&raw.bytes, &raw.blake3, c)?;
    }
    c.check(1)
}
fn measured(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    shape(doc, q, c)?;
    selection_census(doc, true, None, q, c)
}
impl AccountedDocument {
    /// Structural allocation facts only, not a receipt or source/render grant.
    /// The consuming codec must already retain the model and immutable payloads.
    pub fn readonly_flat_composite_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, c)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    /// Consume a genuinely paid codec model and its same-ledger source/native
    /// retention. Only the additional model-owner/Arc header is charged here.
    pub fn from_owned_readonly_flat_composite(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        measured(&incoming.document, &quota, c)?;
        let allocation = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyFlatComposite,
                document,
                _selection: None,
                _selection_view: None,
                retention,
                quota,
                _model: None,
                _allocation: Arc::new(allocation),
            }),
        })
    }
    pub fn is_readonly_flat_composite_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyFlatComposite
    }
    pub fn validate_readonly_flat_composite_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        c.check(1)?;
        if !self.is_readonly_flat_composite_storage() {
            return Err(BackingError::Unsupported);
        }
        measured(self.document(), self.quota(), c)?;
        Ok(())
    }
}

pub(super) fn renamed(owner: &AccountedDocument, name: &str, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    if name.len() > 4096 {
        return Err(BackingError::Length);
    }
    imported_edits::validate_name(name, c)?;
    let logical = measured(owner.document(), owner.quota(), c)?;
    if imported_edits::names_equal(&owner.document().name, name, c)? {
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Title(name), c)
}
#[derive(Clone, Copy)]
enum CopyKind<'a> {
    Title(&'a str),
    Metadata(metadata::Plan<'a>),
    Composition(composition::Plan),
}
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    let header = AccountedDocument::owned_model_reservation_bytes();
    let index = logical.index.checked_sub(header).ok_or(BackingError::Integrity)?;
    let name_bytes = match kind {
        CopyKind::Title(name) => (name.len() as u64).checked_mul(2).ok_or(BackingError::Length)?,
        CopyKind::Metadata(plan) => plan.replacement_name_bytes()?,
        CopyKind::Composition(_) => 0,
    };
    let envelope = index.checked_add(name_bytes).and_then(|v| v.checked_mul(2)).and_then(|v| v.checked_add(4096)).ok_or(BackingError::Length)?;
    let payload = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, header).map_err(|_| BackingError::Quota)?;
    let mut index_lease = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let mut payload_lease = owner.quota().reserve(QuotaKind::Resident, payload).map_err(|_| BackingError::Quota)?;
    c.check(logical.total()?)?;
    let mut staged = ModelStaging { document: imported_selection::copy_document(owner.document(), c)?, selection_view: None };
    match kind {
        CopyKind::Title(name) => staged.document.name = imported_edits::copy_name(name, c)?,
        CopyKind::Metadata(plan) => metadata::apply(&mut staged.document, plan, c)?,
        CopyKind::Composition(plan) => composition::apply(&mut staged.document, plan, c)?,
    }
    let actual = measured(&staged.document, owner.quota(), c)?;
    let actual_index = actual.index.checked_sub(header).ok_or(BackingError::Integrity)?;
    if actual_index > envelope || actual.payload > payload {
        return Err(BackingError::Allocation);
    }
    index_lease.reconcile_capacity(actual_index).map_err(|_| BackingError::Quota)?;
    payload_lease.reconcile_capacity(actual.payload).map_err(|_| BackingError::Quota)?;
    c.check(1)?;
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: StorageClass::ReadonlyFlatComposite,
            document: staged.document,
            _selection: None,
            _selection_view: None,
            // Direct codec retention, never a previous model owner/title chain.
            retention: owner.owner.retention.clone(),
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index_lease, _payload: payload_lease }),
            _allocation: Arc::new(allocation),
        }),
    })
}

#[cfg(test)]
#[path = "accounted_flat_composite_readonly_tests.rs"]
mod tests;
