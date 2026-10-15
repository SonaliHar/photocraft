//! Closed immutable-pixel filter storage with paid file-title and typed current
//! threshold/activation copies. No source correspondence, evaluator, profile,
//! generic pixel or selection edit authority is granted.
//! Runtime adoption is explicit; no persisted field can enable the class.
use super::*;
use crate::{
    FilterMask,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterStack, FilterValue},
};
use photocraft_raster::Surface;
#[path = "accounted_filter_readonly_patch.rs"]
mod patch;
pub use patch::ThresholdAlphaPatch;

// Same established logical publication envelope as raster::staged. BTreeMap
// nodes have no stable fallible allocator/capacity API: at most one prepaid,
// polled insertion is allowed here, not a complete allocator/RSS claim.
const MAP_ENTRY_ENVELOPE: u64 = 256;
const EPOCH: &str = "xcf-22-stock-threshold-alpha-precision-v1";
fn unsupported<T>() -> Result<T, BackingError> {
    Err(BackingError::Unsupported)
}
struct MaskCopyPlan {
    bytes: u64,
}
impl MaskCopyPlan {
    fn new(surface: &Surface, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        let format = surface.format();
        if format.mode != crate::ColorMode::Grayscale
            || format.alpha
            || !matches!(format.sample, crate::SampleType::U16 | crate::SampleType::F32 | crate::SampleType::F64)
            || surface.default_bytes().len() != format.bytes_per_pixel()
            || surface.tile_count() > 1
        {
            return unsupported();
        }
        let expected = (photocraft_geom::TILE_SIZE as usize)
            .checked_mul(photocraft_geom::TILE_SIZE as usize)
            .and_then(|n| n.checked_mul(format.bytes_per_pixel()))
            .ok_or(BackingError::Length)?;
        for (coordinate, tile) in surface.tiles() {
            control.check(1)?;
            if coordinate.tx != 0 || coordinate.ty != 0 {
                return Err(BackingError::Bounds);
            }
            for v in [coordinate.tx, coordinate.ty] {
                let begin = v.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
                let end = begin.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
                if begin < -(1 << 29) || end > 1 << 29 {
                    return Err(BackingError::Bounds);
                }
            }
            if tile.bytes().len() != expected {
                return Err(BackingError::Integrity);
            }
        }
        let bytes = (surface.default_bytes().len() as u64)
            .checked_mul(2)
            .and_then(|n| n.checked_add((surface.tile_count() as u64).checked_mul(MAP_ENTRY_ENVELOPE)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Surface>() as u64 + 128))
            .ok_or(BackingError::Length)?;
        Ok(Self { bytes })
    }
    // Caller has reserved this plan in the new model's real metadata lease.
    fn copy(&self, source: &Surface, control: &ReadControl<'_>) -> Result<Surface, BackingError> {
        if Self::new(source, control)?.bytes > self.bytes {
            return Err(BackingError::Integrity);
        }
        control.check(source.default_bytes().len() as u64 + 1)?;
        let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|_| BackingError::Allocation)?;
        for (coordinate, tile) in source.tiles() {
            control.check(1)?;
            output.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
        control.check(1)?;
        Ok(output)
    }
}
pub(super) fn mask_census(masks: &Vec<FilterMask>, census: &mut CanonicalCensus<'_, '_>) -> Result<(), BackingError> {
    if masks.len() > 1 {
        return unsupported();
    }
    census.vec(masks, 1)?;
    for mask in masks {
        let plan = MaskCopyPlan::new(&mask.surface, census.control)?;
        census.counted.index = census.counted.index.checked_add(plan.bytes).ok_or(BackingError::Length)?;
        // Existing native Tile payload/Arc owners remain with the original
        // accounted codec guard. This is only a new map/default metadata charge.
    }
    Ok(())
}
fn shape(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(1)?;
    if doc.layers.len() != 1
        || doc.size.width != 8
        || doc.size.height != 6
        || !matches!(doc.mode, crate::ColorMode::Rgb | crate::ColorMode::Grayscale)
        || !matches!(doc.depth, crate::SampleType::U16 | crate::SampleType::F32 | crate::SampleType::F64)
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
    {
        return unsupported();
    }
    validate_backing(doc, quota, control)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    let source = doc.source_color_context.as_ref().ok_or(BackingError::Unsupported)?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 22
        || state.items.len() != 1usize.checked_add(state.filter_masks.len()).ok_or(BackingError::Length)?
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() > 1
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || state.selection.selected.len() > 1
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
        || source.original_icc.is_some()
    {
        return unsupported();
    }
    let precision = match (source.source_precision, doc.depth, doc.sample_encoding) {
        (200, crate::SampleType::U16, Some(crate::SampleEncoding::Linear))
        | (250, crate::SampleType::U16, Some(crate::SampleEncoding::NonlinearProfile))
        | (600, crate::SampleType::F32, Some(crate::SampleEncoding::Linear))
        | (650, crate::SampleType::F32, Some(crate::SampleEncoding::NonlinearProfile))
        | (700, crate::SampleType::F64, Some(crate::SampleEncoding::Linear))
        | (750, crate::SampleType::F64, Some(crate::SampleEncoding::NonlinearProfile)) => true,
        _ => false,
    };
    if !precision {
        return unsupported();
    }
    let layer = doc.layers.first().ok_or(BackingError::Integrity)?;
    let LayerContent::BackedRaster(backed) = &layer.content else { return unsupported() };
    let mut found = None;
    for candidate in &state.items {
        control.check(1)?;
        if candidate.item == ItemRef::Layer(layer.id) {
            if found.is_some() {
                return Err(BackingError::Integrity);
            }
            found = Some(candidate);
        }
    }
    let item = found.ok_or(BackingError::Integrity)?;
    let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
    if layer.id.0 == 0
        || backed.format().mode != doc.mode
        || backed.format().sample != doc.depth
        || !backed.format().alpha
        || backed.operative_bounds() != doc.bounds()
        || item.item != ItemRef::Layer(layer.id)
        || item.parent.is_some()
        || item.mask.is_some()
        || item.raster_bounds != Some(backed.operative_bounds())
        || stack.owner != ItemRef::Layer(layer.id)
        || stack.nodes.len() != 1
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.indexed_source.is_some()
        || !layer.effects.items.is_empty()
        || layer.fill_cache.is_some()
        || layer.video.is_some()
    {
        return unsupported();
    }
    for selected in &state.selection.selected {
        control.check(1)?;
        if *selected != item.item {
            return Err(BackingError::Integrity);
        }
    }
    if state.selection.active.is_some_and(|id| id != item.item) {
        return Err(BackingError::Integrity);
    }
    let node = stack.nodes.first().ok_or(BackingError::Integrity)?;
    if node.id.0 == 0
        || node.operation.name != "gimp:threshold-alpha"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != EPOCH
        || node.arguments.len() != 1
        || !node.auxiliary.is_empty()
    {
        return unsupported();
    }
    let argument = node.arguments.first().ok_or(BackingError::Integrity)?;
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else { return unsupported() };
    let scalar = f32::from_bits(bits);
    if argument.name != "value" || !scalar.is_finite() || !(0.0..=1.0).contains(&scalar) {
        return unsupported();
    }
    match (node.mask, state.filter_masks.first()) {
        (None, None) => {}
        (Some(ItemRef::Mask(id)), Some(mask)) if id == mask.id && mask.id.0 != 0 && mask.owner == node.id => {
            if mask.bounds != doc.bounds() || mask.surface.format().sample != doc.depth {
                return unsupported();
            }
            let mut metadata = None;
            for candidate in &state.items {
                control.check(1)?;
                if candidate.item == ItemRef::Mask(mask.id) {
                    if metadata.is_some() {
                        return Err(BackingError::Integrity);
                    }
                    metadata = Some(candidate);
                }
            }
            let metadata = metadata.ok_or(BackingError::Integrity)?;
            let policy = metadata.mask.as_ref().ok_or(BackingError::Integrity)?;
            if metadata.parent.is_some()
                || metadata.composition.is_some()
                || metadata.raster_bounds != Some(mask.bounds)
                || !metadata.visible
                || metadata.source_flags != 0
                || policy.owner != ItemRef::Layer(layer.id)
                || policy.bounds != mask.bounds
                || policy.offset != [0, 0]
                || policy.outside_f64_bits != 0f64.to_bits()
                || !policy.enabled
                || policy.shown
                || policy.editing
                || policy.linked
            {
                return unsupported();
            }
            MaskCopyPlan::new(&mask.surface, control)?;
        }
        _ => return Err(BackingError::Integrity),
    }
    Ok(())
}
fn measured(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    shape(doc, quota, control)?;
    // This census observes current storage only. The old source/codec owner
    // retains existing raw/profile/Tile payload charges independently.
    census_inner(doc, true, true, None, quota, control, true)
}
impl AccountedDocument {
    /// Explicit codec-owned adoption for this closed immutable storage class.
    /// The supplied retention must already own every existing model/default/map/
    /// native allocation on quota. No serialized flag or epoch string is proof
    /// of source correspondence, rendering support or profile interpretation.
    pub fn from_owned_readonly_threshold_alpha_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        measured(&incoming.document, &quota, control)?;
        let allocation = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyThresholdAlpha,
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
    /// Runtime storage-class routing only; final validation is still mandatory.
    /// This grants no source, read, renderer or profile authority.
    pub fn is_readonly_threshold_alpha_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyThresholdAlpha
    }
    /// Read-only bounded model/native-metadata census, not selection/edit or
    /// source/render authority. No sample values are read or normalized.
    pub fn validate_readonly_threshold_alpha_storage(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if self.owner._storage_class != StorageClass::ReadonlyThresholdAlpha {
            return unsupported();
        }
        measured(self.document(), self.quota(), control)?;
        Ok(())
    }
}
fn copy_node(node: &crate::xcf_filters::FilterNode, c: &ReadControl<'_>) -> Result<crate::xcf_filters::FilterNode, BackingError> {
    use crate::xcf_filters::{FilterArgument, FilterNode, OperationIdentity};
    let FilterNode {
        id,
        operation,
        source_display_name,
        source_icon,
        arguments,
        enabled,
        opacity_f64_bits,
        source_clipping,
        source_region,
        mask,
        auxiliary: _,
        composition,
    } = node;
    let argument = arguments.first().ok_or(BackingError::Integrity)?;
    let FilterValue::Number(ExactNumber::Float32Bits(bits)) = argument.value else { return unsupported() };
    let mut copied = Vec::new();
    copied.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if copied.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    copied.push(FilterArgument { name: imported_edits::copy_name(&argument.name, c)?, value: FilterValue::Number(ExactNumber::Float32Bits(bits)) });
    let composition = composition
        .as_ref()
        .map(|v| {
            let crate::import_compositing::GimpComposition {
                compatibility_epoch,
                source_mode,
                legacy,
                stored_blend_space,
                stored_composite_space,
                stored_operator,
                resolved,
                pass_through,
            } = v;
            Ok::<_, BackingError>(crate::import_compositing::GimpComposition {
                compatibility_epoch: imported_edits::copy_name(compatibility_epoch, c)?,
                source_mode: *source_mode,
                legacy: *legacy,
                stored_blend_space: *stored_blend_space,
                stored_composite_space: *stored_composite_space,
                stored_operator: *stored_operator,
                resolved: resolved.clone(),
                pass_through: *pass_through,
            })
        })
        .transpose()?;
    Ok(FilterNode {
        id: *id,
        operation: OperationIdentity {
            name: imported_edits::copy_name(&operation.name, c)?,
            stored_version: imported_edits::copy_name(&operation.stored_version, c)?,
            schema_epoch: imported_edits::copy_name(&operation.schema_epoch, c)?,
        },
        source_display_name: source_display_name.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
        source_icon: source_icon.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
        arguments: copied,
        enabled: *enabled,
        opacity_f64_bits: *opacity_f64_bits,
        source_clipping: *source_clipping,
        source_region: *source_region,
        mask: *mask,
        auxiliary: Vec::new(),
        composition,
    })
}

struct OriginalRetention {
    original: AccountedDocument,
}
#[derive(Clone, Copy)]
enum CopyKind<'n> {
    Rename(&'n str),
    ThresholdAlpha { value_f32_bits: Option<u32>, enabled: Option<bool>, revision: u64 },
}

pub(super) fn renamed(owner: &AccountedDocument, name: &str, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    imported_edits::validate_name(name, control)?;
    if name.len() > 4096 {
        return Err(BackingError::Length);
    }
    let logical = measured(owner.document(), owner.quota(), control)?;
    if imported_edits::names_equal(&owner.document().name, name, control)? {
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Rename(name), control)
}
// Both routes copy current metadata under the original paid recipe. Only the
// Rename case includes replacement-name slack, exactly as before this split.
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    let replacement_name_bytes = match kind {
        CopyKind::Rename(name) => name.len() as u64,
        CopyKind::ThresholdAlpha { .. } => 0,
    };
    let owner_bytes = AccountedDocument::owned_model_reservation_bytes();
    let envelope = logical
        .index
        .checked_mul(2)
        .and_then(|n| n.checked_add(replacement_name_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(4096 + std::mem::size_of::<OriginalRetention>() as u64 + 128))
        .ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, owner_bytes).map_err(|_| BackingError::Quota)?;
    let mut index = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let payload = owner.quota().reserve(QuotaKind::Resident, 0).map_err(|_| BackingError::Quota)?;
    control.check(logical.total()?)?;
    let mut document = imported_selection::copy_document(owner.document(), control)?;
    let source = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let destination = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    destination.filter_stacks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    let stack = source.filter_stacks.first().ok_or(BackingError::Integrity)?;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if nodes.capacity() > 2 || destination.filter_stacks.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    nodes.push(copy_node(stack.nodes.first().ok_or(BackingError::Integrity)?, control)?);
    destination.filter_stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
    destination.filter_masks.try_reserve_exact(source.filter_masks.len()).map_err(|_| BackingError::Allocation)?;
    if destination.filter_masks.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    for mask in &source.filter_masks {
        let plan = MaskCopyPlan::new(&mask.surface, control)?;
        destination.filter_masks.push(FilterMask { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: plan.copy(&mask.surface, control)? });
    }
    match kind {
        CopyKind::Rename(name) => document.name = imported_edits::copy_name(name, control)?,
        CopyKind::ThresholdAlpha { value_f32_bits, enabled, revision } => {
            let state = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
            let stack = state.filter_stacks.first_mut().ok_or(BackingError::Integrity)?;
            let node = stack.nodes.first_mut().ok_or(BackingError::Integrity)?;
            control.check(1)?;
            if let Some(bits) = value_f32_bits {
                let argument = node.arguments.first_mut().ok_or(BackingError::Integrity)?;
                let FilterValue::Number(ExactNumber::Float32Bits(_)) = argument.value else { return unsupported() };
                argument.value = FilterValue::Number(ExactNumber::Float32Bits(bits));
            }
            if let Some(value) = enabled {
                node.enabled = value;
            }
            stack.revision = revision;
        }
    }
    let actual =
        measured(&document, owner.quota(), control)?.index.checked_add(std::mem::size_of::<OriginalRetention>() as u64 + 128).ok_or(BackingError::Length)?;
    if actual > envelope {
        return Err(BackingError::Allocation);
    }
    index.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    // Flatten repeated title/current-node changes to the original real owner rather than form
    // an unbounded recursive retention/drop chain. Native tile Arc identity is
    // unchanged; no second tile payload receipt is fabricated for those Arcs.
    let original = owner.owner.retention.downcast_ref::<OriginalRetention>().map_or_else(|| owner.clone(), |r| r.original.clone());
    let retention = Arc::new(OriginalRetention { original });
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: StorageClass::ReadonlyThresholdAlpha,
            document,
            _selection: None,
            _selection_view: None,
            retention,
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index, _payload: payload }),
            _allocation: Arc::new(allocation),
        }),
    })
}
#[cfg(test)]
#[path = "accounted_filter_readonly_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "accounted_filter_readonly_patch_tests.rs"]
mod patch_tests;
