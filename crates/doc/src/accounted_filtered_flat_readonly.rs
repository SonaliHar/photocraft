//! Readonly filtered-flat structural custody. This is not an operation, source,
//! profile or render grant. Generic edits and selection remain closed.
use super::*;
use crate::{
    FilterMask, FilterOutputPrecision,
    item_identity::ItemRef,
    xcf_filters::{FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
};
use photocraft_raster::Surface;
#[path = "accounted_filtered_flat_readonly_filter.rs"]
mod filter;
#[path = "accounted_filtered_flat_readonly_move.rs"]
mod filter_move;
#[path = "accounted_filtered_flat_readonly_remove.rs"]
mod filter_remove;
#[path = "accounted_filtered_flat_readonly_metadata.rs"]
pub(super) mod metadata;
fn refused<T>() -> Result<T, BackingError> {
    Err(BackingError::Unsupported)
}
fn native_sample(sample: crate::SampleType) -> bool {
    matches!(sample, crate::SampleType::F32 | crate::SampleType::F64)
}
fn rectangle(r: Rect) -> Result<(), BackingError> {
    if r.x0 != 0
        || r.y0 != 0
        || r.is_empty()
        || r.width() > 262144
        || r.height() > 262144
        || u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or(BackingError::Length)? > 1 << 30
    {
        return Err(BackingError::Bounds);
    }
    Ok(())
}
pub(super) fn text(value: &str, c: &dyn ModelControl) -> Result<(), BackingError> {
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
pub(super) fn hash(value: &str, c: &dyn ModelControl) -> Result<(), BackingError> {
    if value.len() != 64 {
        return Err(BackingError::Integrity);
    }
    c.check(64)?;
    if !value.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(BackingError::Integrity);
    }
    Ok(())
}
pub(super) fn digest(bytes: &[u8], expected: &str, c: &dyn ModelControl) -> Result<(), BackingError> {
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
pub(super) struct MaskPlan {
    bytes: u64,
    bounds: Rect,
    entries: usize,
    format: crate::PixelFormat,
}
impl MaskPlan {
    pub(super) fn new(surface: &Surface, bounds: Rect, control: &dyn ModelControl) -> Result<Self, BackingError> {
        control.check(1)?;
        rectangle(bounds)?;
        if surface.tile_count() > 4096 {
            return Err(BackingError::Length);
        }
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
            .and_then(|n| n.checked_add((surface.tile_count() as u64).checked_mul(256)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Surface>() as u64 + 128))
            .ok_or(BackingError::Length)?;
        Ok(Self { bytes, bounds, entries: surface.tile_count(), format })
    }
    pub(super) fn copy(&self, source: &Surface, control: &ReadControl<'_>) -> Result<Surface, BackingError> {
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
pub(super) fn node_shape(node: &FilterNode, control: &dyn ModelControl) -> Result<PointArgumentShape, BackingError> {
    control.check(1)?;
    let opacity = f64::from_bits(node.opacity_f64_bits);
    if node.id.0 == 0
        || node.arguments.len() > 10
        || !node.auxiliary.is_empty()
        || !opacity.is_finite()
        || !(0.0..=1.0).contains(&opacity)
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
    for (index, argument) in node.arguments.iter().enumerate() {
        text(&argument.name, control)?;
        for old in node.arguments.get(..index).ok_or(BackingError::Length)? {
            control.check(1)?;
            if old.name == argument.name {
                return Err(BackingError::Integrity);
            }
        }
        match &argument.value {
            FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Color { .. } => {}
            FilterValue::Enum { domain, .. } => text(domain, control)?,
            _ => return refused(),
        }
    }
    text(&composition.compatibility_epoch, control)?;
    point_argument_shape(&node.arguments, control.sync_control())
}

pub(super) fn present(doc: &Document, id: ItemRef, c: &dyn ModelControl) -> Result<(), BackingError> {
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    for item in &state.items {
        c.check(1)?;
        if item.item == id {
            return Ok(());
        }
    }
    Err(BackingError::Integrity)
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TopologyPurpose {
    Source,
    Current,
}
/// Current structural custody only. Source construction uses its separate
/// strict topology purpose and cannot infer authority from this wrapper.
pub(super) fn shape(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    shape_with_purpose(doc, q, c, TopologyPurpose::Current)
}
pub(super) fn shape_with_purpose(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>, purpose: TopologyPurpose) -> Result<(), BackingError> {
    shape_with_purpose_selected(doc, q, c, purpose, None)
}
pub(super) fn shape_with_purpose_selected(
    doc: &Document,
    q: &BackingQuota,
    c: &ReadControl<'_>,
    purpose: TopologyPurpose,
    selection: Option<&SelectionPermit<'_>>,
) -> Result<(), BackingError> {
    c.check(1)?;
    if !(1..=4096).contains(&doc.layers.len())
        || doc.mode != crate::ColorMode::Rgb
        || !native_sample(doc.depth)
        || doc.sample_encoding != Some(crate::SampleEncoding::NonlinearProfile)
        || selection_disallowed(doc, selection)
        || doc.selection_channel_id.is_some()
    {
        return refused();
    }
    let canvas = doc.bounds();
    rectangle(canvas)?;
    if canvas.size() != doc.size {
        return Err(BackingError::Bounds);
    }
    let source = doc.source_color_context.as_ref().ok_or(BackingError::Unsupported)?;
    if source.original_icc.is_some()
        || !matches!((doc.depth, source.source_precision), (crate::SampleType::F32, 650) | (crate::SampleType::F64, 750))
        || doc.icc_profile.as_ref().is_none_or(|v| v.is_empty() || v.len() > 16 << 20)
    {
        return refused();
    }
    let WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } = &doc.working_profile_interpretation else { return refused() };
    hash(icc_blake3, c)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 22
        || state.current_filter_output_precision != FilterOutputPrecision::Canonical
        || state.filter_stacks.is_empty()
        || state.filter_stacks.len() > doc.layers.len()
        || state.filter_masks.len() > 4096
        || state.items.len() != doc.layers.len().checked_add(state.filter_masks.len()).ok_or(BackingError::Length)?
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || state.selection.selected.len() > doc.layers.len()
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
        || state.opaque.len() > 4096
        || state.provenance.protected_source_fingerprints.len() > 4096
    {
        return refused();
    }
    // Original source retains nonempty stacks and its singleton discriminator.
    // CURRENT may retain empty stacks/revisions after explicit typed removal;
    // at least one real stack owner is still required by the header above.
    let mut node_count = 0usize;
    for stack in &state.filter_stacks {
        c.check(1)?;
        if purpose == TopologyPurpose::Source && stack.nodes.is_empty() || stack.nodes.len() > 1024 {
            return Err(BackingError::Length);
        }
        node_count = node_count.checked_add(stack.nodes.len()).ok_or(BackingError::Length)?;
        if node_count > 4096 {
            return Err(BackingError::Length);
        }
    }
    if state.filter_masks.len() != node_count || purpose == TopologyPurpose::Source && doc.layers.len() == 1 && node_count < 2 {
        return refused();
    }
    // Aggregate retained physical map facts and their work are checked before
    // scans; each root retains its existing4096 tile bound, with no cap raise.
    let mut total_tiles = 0u64;
    let mut map_bytes = 0u64;
    for mask in &state.filter_masks {
        c.check(1)?;
        if mask.surface.tile_count() > 4096 {
            return Err(BackingError::Length);
        }
        total_tiles = total_tiles.checked_add(mask.surface.tile_count() as u64).ok_or(BackingError::Length)?;
        map_bytes = map_bytes
            .checked_add((mask.surface.tile_count() as u64).checked_mul(256).ok_or(BackingError::Length)?)
            .and_then(|n| n.checked_add((mask.surface.default_bytes().len() as u64).checked_mul(2)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Surface>() as u64 + 128))
            .ok_or(BackingError::Length)?;
    }
    c.check(total_tiles.checked_add(map_bytes).ok_or(BackingError::Length)?)?;
    // Bound the aggregate for each actual root before any physical tile walk.
    // This is not a new global4096 tile cap or a4096 allowance per node.
    for stack in &state.filter_stacks {
        c.check(1)?;
        let mut root_tiles = 0usize;
        for node in &stack.nodes {
            c.check(1)?;
            let Some(ItemRef::Mask(id)) = node.mask else { return Err(BackingError::Integrity) };
            let mut matched = false;
            for mask in &state.filter_masks {
                c.check(1)?;
                if mask.id == id {
                    if matched {
                        return Err(BackingError::Integrity);
                    }
                    matched = true;
                    root_tiles = root_tiles.checked_add(mask.surface.tile_count()).ok_or(BackingError::Length)?;
                    if root_tiles > 4096 {
                        return Err(BackingError::Length);
                    }
                }
            }
            if !matched {
                return Err(BackingError::Integrity);
            }
        }
    }

    if doc.metadata.text.len() > 4096 || doc.metadata.psd_resources.len() > 4096 {
        return Err(BackingError::Length);
    }
    if let Some(resolution) = &doc.physical_resolution
        && let crate::physical_resolution::PhysicalUnit::Custom(unit) = &resolution.unit
    {
        for value in [&unit.name, &unit.symbol, &unit.abbreviation] {
            text(value, c)?;
        }
    }
    text(&doc.name, c)?;
    for value in [&doc.metadata.xmp].into_iter().flatten() {
        text(value, c)?;
    }
    for (key, value) in &doc.metadata.text {
        c.check(1)?;
        text(key, c)?;
        text(value, c)?;
    }
    for (_, name, _) in &doc.metadata.psd_resources {
        c.check(1)?;
        text(name, c)?;
    }
    text(&doc.measurement.scale.units, c)?;
    text(&doc.text_styles.basic.name, c)?;
    text(&doc.slices.group_name, c)?;
    for (i, layer) in doc.layers.iter().enumerate() {
        c.check(1)?;
        let LayerContent::BackedRaster(backed) = &layer.content else { return refused() };
        if layer.id.0 == 0
            || !backed.quota().shares_ledger(q)
            || backed.operative_bounds() != canvas
            || backed.format() != crate::PixelFormat::new(doc.mode, doc.depth, true)
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.link_group.is_some()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
        {
            return refused();
        }
        text(&layer.name, c)?;
        for old in doc.layers.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if old.id == layer.id {
                return Err(BackingError::Integrity);
            }
        }
        let mut item = None;
        for row in &state.items {
            c.check(1)?;
            if row.item == ItemRef::Layer(layer.id) {
                if item.is_some() {
                    return Err(BackingError::Integrity);
                }
                item = Some(row)
            }
        }
        let item = item.ok_or(BackingError::Integrity)?;
        if item.parent.is_some()
            || item.mask.is_some()
            || item.raster_bounds != Some(canvas)
            || item.visible != layer.visible
            || !imported_edits::names_equal(&item.name, &layer.name, c)?
        {
            return Err(BackingError::Integrity);
        }
    }
    for (i, stack) in state.filter_stacks.iter().enumerate() {
        c.check(1)?;
        let ItemRef::Layer(id) = stack.owner else { return Err(BackingError::Integrity) };
        if !doc.layers.iter().try_fold(false, |found, l| {
            c.check(1)?;
            Ok::<_, BackingError>(found || l.id == id)
        })? {
            return Err(BackingError::Integrity);
        }
        for old in state.filter_stacks.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if old.owner == stack.owner {
                return Err(BackingError::Integrity);
            }
        }
        for (node_index, node) in stack.nodes.iter().enumerate() {
            node_shape(node, c)?;
            for previous in stack.nodes.get(..node_index).ok_or(BackingError::Length)? {
                c.check(1)?;
                if previous.id == node.id || previous.mask == node.mask {
                    return Err(BackingError::Integrity);
                }
            }
            for old in state.filter_stacks.get(..i).ok_or(BackingError::Length)? {
                c.check(1)?;
                for previous in &old.nodes {
                    c.check(1)?;
                    if previous.id == node.id || previous.mask == node.mask {
                        return Err(BackingError::Integrity);
                    }
                }
            }
            for value in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
                text(value, c)?;
            }
            for value in [node.source_display_name.as_deref(), node.source_icon.as_deref()].into_iter().flatten() {
                text(value, c)?;
            }
            let ItemRef::Mask(mask_id) = node.mask.ok_or(BackingError::Integrity)? else { return Err(BackingError::Integrity) };
            let mut matched = None;
            for mask in &state.filter_masks {
                c.check(1)?;
                if mask.id == mask_id {
                    if matched.is_some() {
                        return Err(BackingError::Integrity);
                    }
                    matched = Some(mask);
                }
            }
            let mask = matched.ok_or(BackingError::Integrity)?;
            if mask.id.0 == 0 || mask.owner != node.id || mask.bounds != canvas || mask.surface.format().sample != doc.depth {
                return Err(BackingError::Integrity);
            }
        }
    }
    for (i, mask) in state.filter_masks.iter().enumerate() {
        for old in state.filter_masks.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if old.id == mask.id {
                return Err(BackingError::Integrity);
            }
        }
        MaskPlan::new(&mask.surface, canvas, c)?;
        let mut metadata = None;
        for item in &state.items {
            c.check(1)?;
            if item.item == ItemRef::Mask(mask.id) {
                if metadata.is_some() {
                    return Err(BackingError::Integrity);
                }
                metadata = Some(item)
            }
        }
        let item = metadata.ok_or(BackingError::Integrity)?;
        let policy = item.mask.as_ref().ok_or(BackingError::Integrity)?;
        let mut stack = None;
        for candidate in &state.filter_stacks {
            c.check(1)?;
            for node in &candidate.nodes {
                c.check(1)?;
                if node.id == mask.owner {
                    if stack.is_some() {
                        return Err(BackingError::Integrity);
                    }
                    stack = Some(candidate);
                }
            }
        }
        let stack = stack.ok_or(BackingError::Integrity)?;
        if item.parent.is_some()
            || item.composition.is_some()
            || item.raster_bounds != Some(canvas)
            || !item.visible
            || item.source_flags != 0
            || policy.owner != stack.owner
            || policy.bounds != canvas
            || policy.offset != [0, 0]
            || policy.outside_f64_bits != 0f64.to_bits()
            || !policy.enabled
            || policy.shown
            || policy.editing
            || policy.linked
        {
            return Err(BackingError::Integrity);
        }
    }
    for (i, item) in state.items.iter().enumerate() {
        c.check(1)?;
        text(&item.name, c)?;
        for old in state.items.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if old.item == item.item {
                return Err(BackingError::Integrity);
            }
        }
        match item.item {
            ItemRef::Layer(id) => {
                let mut found = false;
                for layer in &doc.layers {
                    c.check(1)?;
                    found |= layer.id == id;
                }
                if !found {
                    return Err(BackingError::Integrity);
                }
            }
            ItemRef::Mask(id) => {
                let mut found = false;
                for mask in &state.filter_masks {
                    c.check(1)?;
                    found |= mask.id == id;
                }
                if !found {
                    return Err(BackingError::Integrity);
                }
            }
            _ => return Err(BackingError::Integrity),
        }
        if let Some(p) = &item.composition {
            text(&p.compatibility_epoch, c)?;
            c.check(128)?;
            p.checked_resolved().map_err(|_| BackingError::Integrity)?;
        }
    }
    for (i, selected) in state.selection.selected.iter().enumerate() {
        c.check(1)?;
        let ItemRef::Layer(id) = selected else { return Err(BackingError::Integrity) };
        let mut found = false;
        for layer in &doc.layers {
            c.check(1)?;
            found |= layer.id == *id;
        }
        if !found {
            return Err(BackingError::Integrity);
        }
        for previous in state.selection.selected.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous == selected {
                return Err(BackingError::Integrity);
            }
        }
    }
    if let Some(active) = state.selection.active {
        let mut found = false;
        for selected in &state.selection.selected {
            c.check(1)?;
            found |= *selected == active;
        }
        if !found {
            return Err(BackingError::Integrity);
        }
    }
    let p = &state.provenance;
    for v in [&p.format, &p.container, &p.source_label] {
        text(v, c)?;
    }
    for v in [&p.producer, &p.release_id, &p.original_locator].into_iter().flatten() {
        text(v, c)?;
    }
    if let Some(h) = &p.source_hash {
        hash(h, c)?;
    }
    for h in &p.protected_source_fingerprints {
        hash(h, c)?;
    }
    let mut payload = 0usize;
    for raw in &state.opaque {
        c.check(1)?;
        if raw.rendering_required {
            return refused();
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
    Ok(())
}
fn measured_owner(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    if owner.document().selection.is_some() {
        return programme_selection::measured_owner(owner, c);
    }
    if owner.is_readonly_masked_filtered_flat_point_storage() {
        masked_filtered_flat_readonly::measured(owner.document(), owner.quota(), c, false)
    } else {
        measured(owner.document(), owner.quota(), c)
    }
}
fn accepts(owner: &AccountedDocument) -> bool {
    owner.is_readonly_filtered_flat_point_storage() || owner.is_readonly_masked_filtered_flat_point_storage()
}
fn measured(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    measured_with_purpose(doc, q, c, TopologyPurpose::Current)
}
fn measured_with_purpose(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>, purpose: TopologyPurpose) -> Result<ModelCensus, BackingError> {
    shape_with_purpose(doc, q, c, purpose)?;
    census_with_mask_policy(doc, true, true, None, q, c, ReadonlyMaskPolicy::FilteredFlat)
}
pub(super) fn mask_census(masks: &Vec<FilterMask>, census: &mut CanonicalCensus<'_>) -> Result<(), BackingError> {
    census.vec(masks, 4096)?;
    for mask in masks {
        let plan = MaskPlan::new(&mask.surface, mask.bounds, census.control)?;
        census.counted.index = census.counted.index.checked_add(plan.bytes).ok_or(BackingError::Length)?;
    }
    Ok(())
}
impl AccountedDocument {
    /// Source-topology additional owner header only. Existing whole
    /// model/source/native/profile allocations must already be genuinely
    /// retained on the same quota; empty/derived singleton topology is refused.
    pub fn readonly_filtered_flat_point_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured_with_purpose(doc, q, c, TopologyPurpose::Source)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    /// Current-only additional header requirements. This is structural custody,
    /// not source qualification or proof that an edit occurred historically.
    pub fn readonly_filtered_flat_point_current_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, c)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn new_readonly_filtered_flat_point_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_filtered_flat_point_storage_with_purpose(document, retention, quota, c, TopologyPurpose::Source)
    }
    /// Explicit CURRENT handoff retaining the same actual runtime class. Empty
    /// stacks and single-root zero/one-node states are structurally representable;
    /// caller must independently prove the complete current recipe/publication.
    /// Incoming model/native/profile/source allocations must already be retained
    /// on this quota. No source epoch or persisted hint grants authority here.
    pub fn new_readonly_filtered_flat_point_current_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_filtered_flat_point_storage_with_purpose(document, retention, quota, c, TopologyPurpose::Current)
    }
    fn new_filtered_flat_point_storage_with_purpose(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
        purpose: TopologyPurpose,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        let required = match purpose {
            TopologyPurpose::Source => Self::readonly_filtered_flat_point_storage_requirements(&incoming.document, &quota, c)?,
            TopologyPurpose::Current => Self::readonly_filtered_flat_point_current_storage_requirements(&incoming.document, &quota, c)?,
        };
        let allocation = quota.reserve(QuotaKind::Index, required).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        let Incoming { document, retention } = incoming;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyFilteredFlatPoint,
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
    pub fn is_readonly_filtered_flat_point_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyFilteredFlatPoint
    }
    pub fn validate_readonly_filtered_flat_point_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_filtered_flat_point_storage() {
            return refused();
        }
        programme_measured(self, c)?;
        Ok(())
    }
}
pub(super) fn vector<T>(len: usize) -> Result<Vec<T>, BackingError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| BackingError::Allocation)?;
    if v.capacity() > len.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    Ok(v)
}
fn copy_value(v: &FilterValue, c: &ReadControl<'_>) -> Result<FilterValue, BackingError> {
    c.check(1)?;
    Ok(match v {
        FilterValue::Number(n) => FilterValue::Number(n.clone()),
        FilterValue::Bool(b) => FilterValue::Bool(*b),
        FilterValue::Enum { domain, value } => FilterValue::Enum { domain: imported_edits::copy_name(domain, c)?, value: *value },
        FilterValue::Color { components, encoding, profile } => {
            let mut out = vector(components.len())?;
            for word in components {
                c.check(1)?;
                out.push(word.clone());
            }
            FilterValue::Color { components: out, encoding: *encoding, profile: *profile }
        }
        _ => return refused(),
    })
}
pub(super) fn copy_node(node: &FilterNode, control: &ReadControl<'_>) -> Result<FilterNode, BackingError> {
    node_shape(node, control)?;
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(node.arguments.len()).map_err(|_| BackingError::Allocation)?;
    if arguments.capacity() > node.arguments.len().checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation);
    }
    for argument in &node.arguments {
        control.check(1)?;
        let value = copy_value(&argument.value, control)?;
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

pub(super) fn renamed(owner: &AccountedDocument, name: &str, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    if name.len() > 4096 {
        return Err(BackingError::Length);
    }
    imported_edits::validate_name(name, c)?;
    let logical = measured_owner(owner, c)?;
    if imported_edits::names_equal(&owner.document().name, name, c)? {
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Title(name), c)
}
#[derive(Clone, Copy)]
// Keep prospectively priced copy plans inline; boxing changes allocation and quota semantics.
#[allow(clippy::large_enum_variant)]
enum CopyKind<'a> {
    Selection(Option<&'a AccountedSelection>),
    ProgrammeNative(programme_native::Plan<'a>),
    Title(&'a str),
    Filter(filter::Plan),
    Move(filter_move::Plan),
    Remove(filter_remove::Plan),
    Metadata(metadata::Plan<'a>),
}
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    if owner.is_readonly_masked_filtered_flat_point_storage() {
        owner.document().checked_f32_filter_mask_coverage_set_requirements(crate::xcf_filter_masks::MaskCoverageSetLimits::STANDARD, c)?;
    }
    let replacement_selection = match kind {
        CopyKind::Selection(g) => Some(g),
        _ => None,
    };
    let selection_copy = if owner.document().selection.is_some() || replacement_selection.is_some() {
        Some(programme_selection::SelectionCopy::new(owner, replacement_selection, c)?)
    } else {
        None
    };
    let name_bytes = match kind {
        CopyKind::Title(name) => (name.len() as u64).checked_mul(2).ok_or(BackingError::Length)?,
        CopyKind::Selection(_) | CopyKind::ProgrammeNative(_) | CopyKind::Filter(_) | CopyKind::Move(_) | CopyKind::Remove(_) => 0,
        CopyKind::Metadata(plan) => plan.replacement_name_bytes()?,
    };
    let archive_bytes = match kind {
        CopyKind::Remove(plan) => plan.copy_index_bytes(),
        _ => 0,
    };
    let extra_work = match kind {
        CopyKind::ProgrammeNative(plan) => plan.copy_work(),
        CopyKind::Remove(plan) => plan.copy_work()?,
        _ => 0,
    };
    let header = AccountedDocument::owned_model_reservation_bytes();
    let envelope = logical
        .index
        .checked_sub(header)
        .and_then(|v| v.checked_add(name_bytes))
        .and_then(|v| v.checked_mul(2))
        .and_then(|v| v.checked_add(archive_bytes))
        .and_then(|v| v.checked_add(4096))
        .ok_or(BackingError::Length)?;
    let payload = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, header).map_err(|_| BackingError::Quota)?;
    let mut index = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let mut payload_lease = owner.quota().reserve(QuotaKind::Resident, payload).map_err(|_| BackingError::Quota)?;
    c.check(logical.total()?)?;
    if extra_work != 0 {
        c.check(extra_work)?;
    }
    let mut staged = ModelStaging {
        document: if owner.document().selection.is_some() {
            let p = programme_selection::owner_permit(owner, c)?;
            imported_selection::copy_filtered_flat_selected(owner.document(), owner.quota(), &p, c)?
        } else {
            imported_selection::copy_filtered_flat(owner.document(), owner.quota(), c)?
        },
        selection_view: None,
    };
    let selection_guard = if let Some(copy) = selection_copy { copy.install(&mut staged) } else { None };
    let source = owner.document().xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    let destination = staged.document.xcf_state.as_mut().ok_or(BackingError::Integrity)?;
    destination.filter_stacks = vector(source.filter_stacks.len())?;
    destination.filter_masks = vector(source.filter_masks.len())?;
    for stack in &source.filter_stacks {
        c.check(1)?;
        let mut nodes = vector(stack.nodes.len())?;
        for node in &stack.nodes {
            nodes.push(copy_node(node, c)?);
        }
        destination.filter_stacks.push(FilterStack { owner: stack.owner, nodes, revision: stack.revision });
    }
    for mask in &source.filter_masks {
        let plan = MaskPlan::new(&mask.surface, mask.bounds, c)?;
        destination.filter_masks.push(FilterMask { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: plan.copy(&mask.surface, c)? });
    }
    match kind {
        CopyKind::Selection(_) => {}
        CopyKind::ProgrammeNative(plan) => programme_native::apply(&mut staged.document, plan, c)?,
        CopyKind::Title(name) => staged.document.name = imported_edits::copy_name(name, c)?,
        CopyKind::Filter(plan) => filter::apply(&mut staged.document, plan, c)?,
        CopyKind::Move(plan) => filter_move::apply(&mut staged.document, plan, c)?,
        CopyKind::Remove(plan) => filter_remove::apply(&mut staged.document, plan, c)?,
        CopyKind::Metadata(plan) => metadata::apply(&mut staged.document, plan, c)?,
    }
    let actual = if selection_guard.is_some() {
        let p = programme_selection::staged_permit(&staged.document, selection_guard, owner.quota(), c)?;
        programme_selection::measured(&staged.document, owner.quota(), programme_native::kind(owner)?, &p, c)?
    } else {
        if owner.is_readonly_masked_filtered_flat_point_storage() {
            masked_filtered_flat_readonly::measured(&staged.document, owner.quota(), c, false)?
        } else {
            measured(&staged.document, owner.quota(), c)?
        }
    };
    let actual_index = actual.index.checked_sub(header).ok_or(BackingError::Integrity)?;
    if actual_index > envelope || actual.payload > payload {
        return Err(BackingError::Allocation);
    }
    index.reconcile_capacity(actual_index).map_err(|_| BackingError::Quota)?;
    payload_lease.reconcile_capacity(actual.payload).map_err(|_| BackingError::Quota)?;
    c.check(1)?;
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: owner.owner._storage_class,
            document: staged.document,
            _selection: selection_guard.cloned(),
            _selection_view: staged.selection_view,
            retention: owner.owner.retention.clone(),
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index, _payload: payload_lease }),
            _allocation: Arc::new(allocation),
        }),
    })
}

// Private additive programme-native bridge. Existing entrypoints and copier
// schedules retain their own class-specific custody and final census.
pub(super) fn programme_measured(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    if owner.document().selection.is_some() {
        return programme_selection::measured_owner(owner, c);
    }
    measured_owner(owner, c)
}
pub(super) fn programme_copy(
    owner: &AccountedDocument,
    logical: ModelCensus,
    plan: programme_native::Plan<'_>,
    c: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    current_copy(owner, logical, CopyKind::ProgrammeNative(plan), c)
}

pub(super) fn programme_selected_measured(
    doc: &Document,
    q: &BackingQuota,
    masked: bool,
    p: &SelectionPermit<'_>,
    c: &ReadControl<'_>,
) -> Result<ModelCensus, BackingError> {
    if masked {
        return masked_filtered_flat_readonly::measured_policy(doc, q, c, false, Some(p));
    }
    shape_with_purpose_selected(doc, q, c, TopologyPurpose::Current, Some(p))?;
    census_with_mask_policy(doc, true, true, p.guard(), q, c, ReadonlyMaskPolicy::FilteredFlat)
}

pub(super) fn programme_selection_copy(
    owner: &AccountedDocument,
    logical: ModelCensus,
    selection: Option<&AccountedSelection>,
    c: &ReadControl<'_>,
) -> Result<AccountedDocument, BackingError> {
    current_copy(owner, logical, CopyKind::Selection(selection), c)
}

#[cfg(test)]
#[path = "accounted_filtered_flat_readonly_tests.rs"]
mod tests;
