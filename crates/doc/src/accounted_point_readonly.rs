//! Generic readonly Point storage custody, not an operation or source proof.
//! The independently paid IO source selector and compose current context must
//! qualify exact semantics before any Session publication. Nothing is persisted
//! to enable this runtime class, and generic edits/selection remain closed.
use super::*;
use crate::{
    FilterMask,
    item_identity::ItemRef,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
};
use photocraft_raster::Surface;
#[path = "accounted_point_readonly_patch.rs"]
mod patch;
pub use patch::{PointArgumentPatch, PointArgumentValue, PointFilterPatch};
#[path = "accounted_point_readonly/structured.rs"]
mod structured;
pub use structured::{PointArgumentShape, point_argument_shape, point_argument_shape_with, point_value_shape, point_values_equal, point_values_same_shape};
#[path = "accounted_point_readonly_structured_patch.rs"]
mod structured_patch;
pub use structured_patch::{PointStructuredArgumentPatch, PointStructuredFilterPatch};
#[path = "accounted_point_readonly/properties.rs"]
mod properties;
pub(super) fn layer_named(owner: &AccountedDocument, patch: ReadonlyLayerNamePatch<'_>, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    properties::layer_named(owner, patch, control)
}
pub(super) fn layer_properties(
    owner: &AccountedDocument,
    patch: ReadonlyLayerPropertiesPatch<'_>,
    control: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    properties::layer_properties(owner, patch, control)
}

const SIDE: u32 = 262_144;
const PIXELS: u64 = 1 << 30;
const MAP_ENTRY: u64 = 256;
fn refused<T>() -> Result<T, BackingError> {
    Err(BackingError::Unsupported)
}

fn rectangle(bounds: Rect) -> Result<(), BackingError> {
    for coordinate in [bounds.x0, bounds.y0, bounds.x1, bounds.y1] {
        if !(-(1 << 29)..=(1 << 29)).contains(&coordinate) {
            return Err(BackingError::Bounds);
        }
    }
    if bounds.is_empty() || bounds.width() > SIDE || bounds.height() > SIDE {
        return Err(BackingError::Bounds);
    }
    let pixels = u64::from(bounds.width()).checked_mul(u64::from(bounds.height())).ok_or(BackingError::Length)?;
    if pixels > PIXELS {
        return Err(BackingError::Length);
    }
    Ok(())
}
fn native_sample(sample: crate::SampleType) -> bool {
    matches!(sample, crate::SampleType::U8 | crate::SampleType::U16 | crate::SampleType::U32 | crate::SampleType::F32 | crate::SampleType::F64)
}
fn precision(doc: &Document) -> bool {
    use crate::{SampleEncoding as E, SampleType as S};
    let Some(source) = &doc.source_color_context else { return false };
    // This checks only literal metadata correspondence. In particular, Half
    // source precision names canonical F32 storage; no sample is reinterpreted
    // or asserted exactly quantized by this L1 storage gate.
    matches!(
        (source.source_precision, doc.depth, doc.sample_encoding),
        (100, S::U8, Some(E::Linear))
            | (150, S::U8, Some(E::NonlinearProfile))
            | (175, S::U8, Some(E::Perceptual))
            | (200, S::U16, Some(E::Linear))
            | (250, S::U16, Some(E::NonlinearProfile))
            | (275, S::U16, Some(E::Perceptual))
            | (300, S::U32, Some(E::Linear))
            | (350, S::U32, Some(E::NonlinearProfile))
            | (375, S::U32, Some(E::Perceptual))
            | (500 | 600, S::F32, Some(E::Linear))
            | (550 | 650, S::F32, Some(E::NonlinearProfile))
            | (575 | 675, S::F32, Some(E::Perceptual))
            | (700, S::F64, Some(E::Linear))
            | (750, S::F64, Some(E::NonlinearProfile))
            | (775, S::F64, Some(E::Perceptual))
    )
}
fn node_shape(node: &FilterNode, control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(1)?;
    if node.id.0 == 0
        || node.arguments.len() > 10
        || !node.auxiliary.is_empty()
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
    {
        return refused();
    }
    // No operation-name/version/schema registry lives at L1. These bounded
    // strings and raw arguments are retained for the separate exact evaluator.
    for text in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
        control.check(1)?;
        if text.is_empty() || text.len() > 4096 {
            return refused();
        }
    }
    let composition = node.composition.as_ref().ok_or(BackingError::Unsupported)?;
    if composition.compatibility_epoch.is_empty()
        || composition.compatibility_epoch.len() > 4096
        || composition.source_mode != 62
        || composition.legacy
        || composition.pass_through
        || composition.resolved.is_some()
    {
        return refused();
    }
    point_argument_shape(&node.arguments, control)?;
    Ok(())
}

/// Only opaque map/default metadata is copied. Native tile words, padding and
/// payload owners remain physically shared with the original codec guard.
pub(super) struct MaskPlan {
    bytes: u64,
    bounds: Rect,
    entries: usize,
    format: crate::PixelFormat,
}
impl MaskPlan {
    fn new(surface: &Surface, bounds: Rect, control: &ReadControl<'_>) -> Result<Self, BackingError> {
        control.check(1)?;
        rectangle(bounds)?;
        if bounds.x0 != 0 || bounds.y0 != 0 {
            return Err(BackingError::Bounds);
        }
        let format = surface.format();
        if format.mode != crate::ColorMode::Grayscale
            || format.alpha
            || !native_sample(format.sample)
            || surface.default_bytes().len() != format.bytes_per_pixel()
        {
            return refused();
        }
        let side = u64::try_from(photocraft_geom::TILE_SIZE).map_err(|_| BackingError::Length)?;
        let ceiling = side.checked_sub(1).ok_or(BackingError::Length)?;
        let columns = u64::from(bounds.width()).checked_add(ceiling).ok_or(BackingError::Length)? / side;
        let rows = u64::from(bounds.height()).checked_add(ceiling).ok_or(BackingError::Length)? / side;
        let maximum = columns.checked_mul(rows).ok_or(BackingError::Length)?;
        if u64::try_from(surface.tile_count()).map_err(|_| BackingError::Length)? > maximum {
            return Err(BackingError::Length);
        }
        let length = side.checked_mul(side).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(BackingError::Length)?;
        for (coordinate, tile) in surface.tiles() {
            control.check(1)?;
            if u64::try_from(coordinate.tx).map_err(|_| BackingError::Bounds)? >= columns
                || u64::try_from(coordinate.ty).map_err(|_| BackingError::Bounds)? >= rows
            {
                return Err(BackingError::Bounds);
            }
            for v in [coordinate.tx, coordinate.ty] {
                let begin = v.checked_mul(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
                let end = begin.checked_add(photocraft_geom::TILE_SIZE).ok_or(BackingError::Bounds)?;
                if begin < -(1 << 29) || end > 1 << 29 {
                    return Err(BackingError::Bounds);
                }
            }
            if tile.bytes().len() as u64 != length {
                return Err(BackingError::Integrity);
            }
        }
        let bytes = (surface.default_bytes().len() as u64)
            .checked_mul(2)
            .and_then(|n| n.checked_add((surface.tile_count() as u64).checked_mul(MAP_ENTRY)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Surface>() as u64 + 128))
            .ok_or(BackingError::Length)?;
        Ok(Self { bytes, bounds, entries: surface.tile_count(), format })
    }
    fn copy(&self, source: &Surface, control: &ReadControl<'_>) -> Result<Surface, BackingError> {
        let actual = Self::new(source, self.bounds, control)?;
        if actual.bytes != self.bytes || actual.entries != self.entries || actual.format != self.format {
            return Err(BackingError::Integrity);
        }
        control.check(source.default_bytes().len() as u64 + 1)?;
        let mut copied = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|_| BackingError::Allocation)?;
        for (coordinate, tile) in source.tiles() {
            control.check(1)?;
            copied.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
        control.check(1)?;
        Ok(copied)
    }
}
// Parent's Point-only census policy calls this; old strict mask policies must
// continue calling their unchanged Threshold helper.
pub(super) fn mask_census(masks: &Vec<FilterMask>, census: &mut CanonicalCensus<'_, '_>) -> Result<(), BackingError> {
    if masks.len() != 1 {
        return refused();
    }
    census.vec(masks, 1)?;
    for mask in masks {
        let plan = MaskPlan::new(&mask.surface, mask.bounds, census.control)?;
        census.counted.index = census.counted.index.checked_add(plan.bytes).ok_or(BackingError::Length)?;
    }
    Ok(())
}
fn shape(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<(), BackingError> {
    control.check(1)?;
    if doc.layers.len() != 1
        || !matches!(doc.mode, crate::ColorMode::Rgb | crate::ColorMode::Grayscale)
        || !native_sample(doc.depth)
        || !precision(doc)
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
    {
        return refused();
    }
    let width = i32::try_from(doc.size.width).map_err(|_| BackingError::Bounds)?;
    let height = i32::try_from(doc.size.height).map_err(|_| BackingError::Bounds)?;
    let canvas = Rect::new(0, 0, width, height);
    rectangle(canvas)?;
    if doc.bounds() != canvas {
        return Err(BackingError::Bounds);
    }
    validate_backing(doc, quota, control)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    let source = doc.source_color_context.as_ref().ok_or(BackingError::Unsupported)?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 22
        || state.items.len() != 2
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
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
        return refused();
    }
    let layer = doc.layers.first().ok_or(BackingError::Integrity)?;
    let LayerContent::BackedRaster(backed) = &layer.content else { return refused() };
    let bounds = backed.operative_bounds();
    rectangle(bounds)?;
    if layer.id.0 == 0
        || backed.format().mode != doc.mode
        || backed.format().sample != doc.depth
        || !backed.format().alpha
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || layer.indexed_source.is_some()
        || !layer.effects.items.is_empty()
        || layer.fill_cache.is_some()
        || layer.video.is_some()
    {
        return refused();
    }
    let stack = state.filter_stacks.first().ok_or(BackingError::Integrity)?;
    if stack.owner != ItemRef::Layer(layer.id) || stack.nodes.len() != 1 {
        return refused();
    }
    let node = stack.nodes.first().ok_or(BackingError::Integrity)?;
    node_shape(node, control)?;
    let mask = state.filter_masks.first().ok_or(BackingError::Integrity)?;
    if mask.id.0 == 0
        || mask.owner != node.id
        || node.mask != Some(ItemRef::Mask(mask.id))
        || mask.bounds != canvas
        || mask.surface.format().sample != doc.depth
    {
        return Err(BackingError::Integrity);
    }
    let mut layer_item = None;
    let mut mask_item = None;
    for item in &state.items {
        control.check(1)?;
        match item.item {
            ItemRef::Layer(id) if id == layer.id && layer_item.is_none() => layer_item = Some(item),
            ItemRef::Mask(id) if id == mask.id && mask_item.is_none() => mask_item = Some(item),
            _ => return Err(BackingError::Integrity),
        }
    }
    let item = layer_item.ok_or(BackingError::Integrity)?;
    if item.parent.is_some() || item.mask.is_some() || item.raster_bounds != Some(bounds) {
        return refused();
    }
    let metadata = mask_item.ok_or(BackingError::Integrity)?;
    let policy = metadata.mask.as_ref().ok_or(BackingError::Integrity)?;
    if metadata.parent.is_some()
        || metadata.composition.is_some()
        || metadata.raster_bounds != Some(canvas)
        || !metadata.visible
        || metadata.source_flags != 0
        || policy.owner != ItemRef::Layer(layer.id)
        || policy.bounds != canvas
        || policy.offset != [0, 0]
        || policy.outside_f64_bits != 0f64.to_bits()
        || !policy.enabled
        || policy.shown
        || policy.editing
        || policy.linked
    {
        return refused();
    }
    for selected in &state.selection.selected {
        control.check(1)?;
        if *selected != ItemRef::Layer(layer.id) {
            return Err(BackingError::Integrity);
        }
    }
    if state.selection.active.is_some_and(|id| id != ItemRef::Layer(layer.id)) {
        return Err(BackingError::Integrity);
    }
    MaskPlan::new(&mask.surface, canvas, control)?;
    Ok(())
}
pub(super) fn measured(doc: &Document, quota: &BackingQuota, control: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    shape(doc, quota, control)?;
    // Private parent seam selects this module's mask census, with every other
    // original full capacity/payload bound and accounting rule unchanged.
    point_readonly_census(doc, quota, control)
}
impl AccountedDocument {
    /// Prospective additional owner metadata only. All existing model/source/
    /// native/default/map allocations must already be genuinely retained on q.
    /// This traversal/capacity report is not a receipt or evaluator proof.
    pub fn readonly_point_storage_requirements(doc: &Document, q: &BackingQuota, control: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, control)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    /// Trusted consuming codec/model ownership transfer. The supplied guard
    /// must own the actual existing charges, not numeric asserted totals. The
    /// native BackedRaster independently verifies the same actual quota.
    pub fn new_readonly_point_storage(
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
        let required = Self::readonly_point_storage_requirements(&incoming.document, &quota, control)?;
        let allocation = quota.reserve(QuotaKind::Index, required).map_err(|_| BackingError::Quota)?;
        control.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyPoint,
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
    pub fn is_readonly_point_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyPoint
    }
    pub fn validate_readonly_point_storage(&self, control: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_point_storage() {
            return refused();
        }
        measured(self.document(), self.quota(), control)?;
        Ok(())
    }
}

fn copy_node(node: &FilterNode, control: &ReadControl<'_>) -> Result<FilterNode, BackingError> {
    node_shape(node, control)?;
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(node.arguments.len()).map_err(|_| BackingError::Allocation)?;
    if arguments.capacity() > node.arguments.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    for argument in &node.arguments {
        control.check(1)?;
        let value = structured::copy_value(&argument.value, control)?;
        arguments.push(FilterArgument { name: imported_edits::copy_name(&argument.name, control)?, value });
    }
    let old = node.composition.as_ref().ok_or(BackingError::Integrity)?;
    let composition = crate::import_compositing::GimpComposition {
        compatibility_epoch: imported_edits::copy_name(&old.compatibility_epoch, control)?,
        source_mode: old.source_mode,
        legacy: old.legacy,
        stored_blend_space: old.stored_blend_space,
        stored_composite_space: old.stored_composite_space,
        stored_operator: old.stored_operator,
        resolved: old.resolved.clone(),
        pass_through: old.pass_through,
    };
    Ok(FilterNode {
        id: node.id,
        operation: OperationIdentity {
            name: imported_edits::copy_name(&node.operation.name, control)?,
            stored_version: imported_edits::copy_name(&node.operation.stored_version, control)?,
            schema_epoch: imported_edits::copy_name(&node.operation.schema_epoch, control)?,
        },
        source_display_name: node.source_display_name.as_deref().map(|v| imported_edits::copy_name(v, control)).transpose()?,
        source_icon: node.source_icon.as_deref().map(|v| imported_edits::copy_name(v, control)).transpose()?,
        arguments,
        enabled: node.enabled,
        opacity_f64_bits: node.opacity_f64_bits,
        source_clipping: node.source_clipping,
        source_region: node.source_region,
        mask: node.mask,
        auxiliary: Vec::new(),
        composition: Some(composition),
    })
}
struct OriginalRetention {
    original: AccountedDocument,
}
#[derive(Clone, Copy)]
pub(super) enum CopyKind<'a> {
    Rename(&'a str),
    LayerName(ReadonlyLayerNamePatch<'a>),
    LayerProperties(readonly_layer_properties::Plan<'a>),
    Filter(patch::Plan),
    Structured(structured_patch::Plan<'a>),
}
pub(super) fn renamed(owner: &AccountedDocument, name: &str, control: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    imported_edits::validate_name(name, control)?;
    if name.len() > 4096 {
        return Err(BackingError::Length);
    }
    let logical = measured(owner.document(), owner.quota(), control)?;
    if imported_edits::names_equal(&owner.document().name, name, control)? {
        control.check(1)?;
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Rename(name), control)
}
/// Only the sealed typed mutations above share this private paid copier. There
/// is no public callback, mutable Document access or generic edit admission.
pub(super) fn current_copy(
    owner: &AccountedDocument,
    logical: ModelCensus,
    kind: CopyKind<'_>,
    control: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    let replacement_name_bytes = match kind {
        CopyKind::Rename(name) => name.len() as u64,
        CopyKind::LayerName(patch) => (patch.name.len() as u64).checked_mul(2).ok_or(BackingError::Length)?,
        CopyKind::LayerProperties(plan) => plan.replacement_name_bytes()?,
        CopyKind::Filter(_) | CopyKind::Structured(_) => 0,
    };
    let replacement_value_bytes = match kind {
        CopyKind::Structured(plan) => plan.copy_index_bytes(),
        _ => 0,
    };
    let envelope = logical
        .index
        .checked_mul(2)
        .and_then(|v| v.checked_add(replacement_name_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(replacement_value_bytes))
        .and_then(|v| v.checked_add(4096 + std::mem::size_of::<OriginalRetention>() as u64 + 128))
        .ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, AccountedDocument::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
    let mut index = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let payload = owner.quota().reserve(QuotaKind::Resident, 0).map_err(|_| BackingError::Quota)?;
    control.check(logical.total()?)?;
    if replacement_value_bytes != 0 {
        // The replacement's newly initialized dynamic bytes are additional to
        // the old-model copy above, and receive their own nonrefundable work.
        control.check(replacement_value_bytes)?;
    }
    let mut document = imported_selection::copy_document(owner.document(), control)?;
    let source = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let destination = document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    destination.filter_stacks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    destination.filter_masks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if destination.filter_stacks.capacity() > 2 || destination.filter_masks.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    let stack = source.filter_stacks.first().ok_or(BackingError::Integrity)?;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if nodes.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    nodes.push(copy_node(stack.nodes.first().ok_or(BackingError::Integrity)?, control)?);
    destination.filter_stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
    for mask in &source.filter_masks {
        let plan = MaskPlan::new(&mask.surface, mask.bounds, control)?;
        destination.filter_masks.push(FilterMask { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: plan.copy(&mask.surface, control)? });
    }
    match kind {
        CopyKind::Rename(name) => document.name = imported_edits::copy_name(name, control)?,
        CopyKind::LayerName(patch) => readonly_layer_name::apply(&mut document, patch, control)?,
        CopyKind::LayerProperties(plan) => readonly_layer_properties::apply(&mut document, plan, control)?,
        CopyKind::Filter(plan) => patch::apply(&mut document, plan, control)?,
        CopyKind::Structured(plan) => structured_patch::apply(&mut document, plan, control)?,
    }
    let actual =
        measured(&document, owner.quota(), control)?.index.checked_add(std::mem::size_of::<OriginalRetention>() as u64 + 128).ok_or(BackingError::Length)?;
    if actual > envelope {
        return Err(BackingError::Allocation);
    }
    index.reconcile_capacity(actual).map_err(|_| BackingError::Quota)?;
    control.check(1)?;
    let original = owner.owner.retention.downcast_ref::<OriginalRetention>().map_or_else(|| owner.clone(), |r| r.original.clone());
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: StorageClass::ReadonlyPoint,
            document,
            _selection: None,
            _selection_view: None,
            retention: Arc::new(OriginalRetention { original }),
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index, _payload: payload }),
            _allocation: Arc::new(allocation),
        }),
    })
}
#[cfg(test)]
#[path = "accounted_point_readonly_tests.rs"]
mod tests;
