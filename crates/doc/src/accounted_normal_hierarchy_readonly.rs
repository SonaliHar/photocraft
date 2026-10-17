//! Explicit isolated Normal tree structural custody, without an operation registry.
use super::*;
use crate::{
    FilterMask,
    xcf_filters::{FilterNode, FilterStack},
};
use crate::{
    FilterOutputPrecision,
    item_identity::ItemRef,
    normal_hierarchy_index::{CheckedNormalHierarchyIndex, NormalHierarchyLimits},
};
use filtered_flat_readonly::TopologyPurpose;
use filtered_flat_readonly::{MaskPlan, copy_node, digest, hash, node_shape, present, text, vector};
use photocraft_raster::Surface;
#[path = "accounted_normal_hierarchy_filter.rs"]
mod filter;
#[path = "accounted_normal_hierarchy_move.rs"]
mod filter_move;
#[path = "accounted_normal_hierarchy_remove.rs"]
mod filter_remove;
#[path = "accounted_normal_hierarchy_metadata.rs"]
pub(super) mod metadata;
#[path = "accounted_normal_hierarchy_native.rs"]
mod native;
#[path = "accounted_normal_hierarchy_plain.rs"]
mod plain;
fn refused<T>() -> Result<T, BackingError> {
    Err(BackingError::Unsupported)
}
fn native_sample(s: crate::SampleType) -> bool {
    matches!(s, crate::SampleType::F32 | crate::SampleType::F64)
}
fn rectangle(r: Rect) -> Result<(), BackingError> {
    if r.x0 != 0
        || r.y0 != 0
        || r.is_empty()
        || r.width() > 262144
        || r.height() > 262144
        || u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or(BackingError::Length)? > 1 << 30
    {
        Err(BackingError::Bounds)
    } else {
        Ok(())
    }
}
fn names_equal_local(a: &str, b: &str, c: &dyn ModelControl) -> Result<bool, BackingError> {
    if a.len() != b.len() {
        c.check(1)?;
        return Ok(false);
    }
    for (a, b) in a.as_bytes().chunks(32768).zip(b.as_bytes().chunks(32768)) {
        c.check(a.len() as u64 + 1)?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn item_composition<'a>(doc: &'a Document, id: LayerId, c: &dyn ModelControl) -> Result<&'a crate::import_compositing::GimpComposition, BackingError> {
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Integrity)?;
    for item in &state.items {
        c.check(1)?;
        if item.item == ItemRef::Layer(id) {
            return item.composition.as_ref().ok_or(BackingError::Unsupported);
        }
    }
    Err(BackingError::Integrity)
}
pub(super) fn shape(doc: &Document, q: &BackingQuota, c: &dyn ModelControl, purpose: TopologyPurpose) -> Result<(), BackingError> {
    let index = doc.checked_normal_hierarchy_index_with_reservation(NormalHierarchyLimits::STANDARD, c.sync_control().interrupt, &|n| c.check(n), |n| {
        q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
    })?;
    shape_rows(doc, q, c, purpose, index.rows(), false)
}
fn shape_rows(
    doc: &Document,
    q: &BackingQuota,
    c: &dyn ModelControl,
    purpose: TopologyPurpose,
    layers: &[crate::normal_hierarchy_index::NormalHierarchyRow<'_>],
    allow_quantized: bool,
) -> Result<(), BackingError> {
    shape_rows_selected(doc, q, c, purpose, layers, allow_quantized, None)
}
fn shape_rows_selected(
    doc: &Document,
    q: &BackingQuota,
    c: &dyn ModelControl,
    purpose: TopologyPurpose,
    layers: &[crate::normal_hierarchy_index::NormalHierarchyRow<'_>],
    allow_quantized: bool,
    selection: Option<&SelectionPermit<'_>>,
) -> Result<(), BackingError> {
    c.check(1)?;
    if !(1..=4096).contains(&layers.len())
        || doc.mode != crate::ColorMode::Rgb
        || !(native_sample(doc.depth) || allow_quantized && matches!(doc.depth, crate::SampleType::U8 | crate::SampleType::U16 | crate::SampleType::U32))
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
        || !(matches!((doc.depth, source.source_precision), (crate::SampleType::F32, 650) | (crate::SampleType::F64, 750))
            || allow_quantized
                && matches!((doc.depth, source.source_precision), (crate::SampleType::U8, 150) | (crate::SampleType::U16, 250) | (crate::SampleType::U32, 350)))
        || doc.icc_profile.as_ref().is_none_or(|v| v.is_empty() || v.len() > 16 << 20)
    {
        return refused();
    }
    let WorkingProfileInterpretation::ImplicitSrgb { icc_blake3 } = &doc.working_profile_interpretation else { return refused() };
    hash(icc_blake3, c)?;
    let state = doc.xcf_state.as_ref().ok_or(BackingError::Unsupported)?;
    if state.provenance.format != "xcf"
        || !matches!(state.provenance.format_version, 12 | 13 | 22)
        || state.current_filter_output_precision != FilterOutputPrecision::Canonical
        || state.filter_stacks.len() > layers.len()
        || state.filter_masks.len() > 4096
        || state.items.len() != layers.len().checked_add(state.filter_masks.len()).ok_or(BackingError::Length)?
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.resources.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || state.selection.selected.len() > layers.len()
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
        || state.opaque.len() > 4096
        || state.provenance.protected_source_fingerprints.len() > 4096
    {
        return refused();
    }
    // Both purposes retain real leaf stack ownership; an unfiltered tree needs no stack.
    let mut node_count = 0usize;
    for stack in &state.filter_stacks {
        c.check(1)?;
        if stack.nodes.len() > 1024 {
            return Err(BackingError::Length);
        }
        node_count = node_count.checked_add(stack.nodes.len()).ok_or(BackingError::Length)?;
        if node_count > 4096 {
            return Err(BackingError::Length);
        }
    }
    if state.filter_masks.len() != node_count {
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
    for (i, row) in layers.iter().enumerate() {
        let layer = row.layer;
        c.check(1)?;
        match &layer.content {
            LayerContent::BackedRaster(backed)
                if backed.quota().shares_ledger(q)
                    && backed.operative_bounds() == canvas
                    && backed.format() == crate::PixelFormat::new(doc.mode, doc.depth, true) => {}
            LayerContent::Group(g) if g.artboard.is_none() => {}
            _ => return refused(),
        }
        if layer.id.0 == 0
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.indexed_source.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.link_group.is_some()
            || layer.import_compositing.is_some_and(|p| p.pass_through)
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
        {
            return refused();
        }
        text(&layer.name, c)?;
        if layer.blend != crate::BlendMode::Normal {
            return refused();
        }
        let composition = item_composition(doc, layer.id, c)?;
        if composition.source_mode != 28
            || composition.legacy
            || composition.pass_through
            || composition.checked_resolved().map_err(|_| BackingError::Integrity)?.operator != crate::import_compositing::GimpOperator::Union
        {
            return refused();
        }

        for old in layers.get(..i).ok_or(BackingError::Length)? {
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
        if item.parent != row.parent.and_then(|p| layers.get(p).map(|r| ItemRef::Layer(r.id)))
            || item.mask.is_some()
            || item.raster_bounds != if layer.is_group() { None } else { Some(canvas) }
            || item.visible != layer.visible
            || !names_equal_local(&item.name, &layer.name, c)?
        {
            return Err(BackingError::Integrity);
        }
    }
    for (i, stack) in state.filter_stacks.iter().enumerate() {
        c.check(1)?;
        let ItemRef::Layer(id) = stack.owner else { return Err(BackingError::Integrity) };
        if !layers.iter().try_fold(false, |found, l| {
            c.check(1)?;
            Ok::<_, BackingError>(found || l.id == id && !l.layer.is_group())
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
            c.check(0)?;
            if purpose == TopologyPurpose::Source && node.opacity_f64_bits != 1f64.to_bits() {
                return refused();
            }
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
                for layer in layers {
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
        for layer in layers {
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
pub(super) fn measured(doc: &Document, q: &BackingQuota, c: &dyn ModelControl, purpose: TopologyPurpose) -> Result<ModelCensus, BackingError> {
    shape(doc, q, c, purpose)?;
    census_with_mask_policy(doc, true, true, None, q, c, ReadonlyMaskPolicy::FilteredFlat)
}
fn measured_current(doc: &Document, q: &BackingQuota, c: &dyn ModelControl, purpose: TopologyPurpose) -> Result<ModelCensus, BackingError> {
    let index =
        doc.checked_normal_hierarchy_mask_index_with_reservation(NormalHierarchyLimits::STANDARD, c.sync_control().interrupt, &|n| c.check(n), |n| {
            q.reserve(QuotaKind::Index, n).map_err(|_| BackingError::Quota)
        })?;
    if purpose == TopologyPurpose::Source && (doc.xcf_state.as_ref().is_none_or(|s| s.provenance.format_version != 22) || index.raw_nonempty_count() == 0) {
        return refused();
    }
    shape_rows(doc, q, c, purpose, index.rows(), purpose == TopologyPurpose::Current)?;
    census_with_mask_policy(doc, true, true, None, q, c, ReadonlyMaskPolicy::FilteredFlat)
}
impl AccountedDocument {
    pub fn readonly_normal_hierarchy_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, c, TopologyPurpose::Source)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn readonly_normal_hierarchy_current_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured(doc, q, c, TopologyPurpose::Current)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn new_readonly_normal_hierarchy_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_normal_hierarchy(document, retention, quota, c, TopologyPurpose::Source)
    }
    pub fn new_readonly_normal_hierarchy_current_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_normal_hierarchy(document, retention, quota, c, TopologyPurpose::Current)
    }
    fn new_normal_hierarchy(
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
        measured(&incoming.document, &quota, c, purpose)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyNormalHierarchy,
                document: incoming.document,
                _selection: None,
                _selection_view: None,
                retention: incoming.retention,
                quota,
                _model: None,
                _allocation: Arc::new(lease),
            }),
        })
    }
    pub fn is_readonly_normal_hierarchy_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyNormalHierarchy
    }
    pub fn validate_readonly_normal_hierarchy_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_normal_hierarchy_storage() {
            return refused();
        }
        measured(self.document(), self.quota(), c, TopologyPurpose::Current)?;
        Ok(())
    }
    pub fn checked_readonly_normal_hierarchy_index(&self, c: &ReadControl<'_>) -> Result<CheckedNormalHierarchyIndex<'_>, BackingError> {
        if !self.is_readonly_normal_hierarchy_storage() {
            return refused();
        }
        self.document().checked_normal_hierarchy_index_with(NormalHierarchyLimits::STANDARD, self.quota(), c)
    }
}

impl AccountedDocument {
    pub fn validate_readonly_normal_hierarchy_current_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_normal_hierarchy_storage() {
            return refused();
        }
        programme_measured(self, c)?;
        Ok(())
    }
    pub fn checked_readonly_normal_hierarchy_current_index(
        &self,
        c: &ReadControl<'_>,
    ) -> Result<crate::normal_hierarchy_mask_index::CheckedNormalHierarchyMaskIndex<'_>, BackingError> {
        if !self.is_readonly_normal_hierarchy_storage() {
            return refused();
        }
        if self.document().selection.is_some() {
            let p = programme_selection::owner_permit(self, c)?;
            return self.document().checked_normal_hierarchy_mask_index_selected(NormalHierarchyLimits::STANDARD, self.quota(), c, &p);
        }
        self.document().checked_normal_hierarchy_mask_index_with(NormalHierarchyLimits::STANDARD, self.quota(), c)
    }
    pub fn readonly_masked_normal_hierarchy_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured_current(doc, q, c, TopologyPurpose::Source)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn readonly_masked_normal_hierarchy_current_storage_requirements(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<u64, BackingError> {
        measured_current(doc, q, c, TopologyPurpose::Current)?;
        Ok(Self::owned_model_reservation_bytes())
    }
    pub fn new_readonly_masked_normal_hierarchy_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_masked_normal_hierarchy(document, retention, quota, c, TopologyPurpose::Source)
    }
    pub fn new_readonly_masked_normal_hierarchy_current_storage(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        Self::new_masked_normal_hierarchy(document, retention, quota, c, TopologyPurpose::Current)
    }
    fn new_masked_normal_hierarchy(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &dyn ModelControl,
        purpose: TopologyPurpose,
    ) -> Result<Self, BackingError> {
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        measured_current(&incoming.document, &quota, c, purpose)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        c.check(1)?;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyNormalHierarchy,
                document: incoming.document,
                _selection: None,
                _selection_view: None,
                retention: incoming.retention,
                quota,
                _model: None,
                _allocation: Arc::new(lease),
            }),
        })
    }
    pub fn new_readonly_masked_normal_hierarchy_storage_with_local_work(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
        work: &dyn Fn(u64) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        struct Local<'a, 'b> {
            base: &'a ReadControl<'b>,
            work: &'a dyn Fn(u64) -> Result<(), BackingError>,
        }
        impl ModelControl for Local<'_, '_> {
            fn check(&self, n: u64) -> Result<(), BackingError> {
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)?;
                (self.work)(n)?;
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)
            }
            fn sync_control(&self) -> &ReadControl<'_> {
                self.base
            }
        }
        Self::new_masked_normal_hierarchy(document, retention, quota, &Local { base: c, work }, TopologyPurpose::Source)
    }
}

fn accepts(owner: &AccountedDocument) -> bool {
    owner.is_readonly_normal_hierarchy_storage()
}
fn measured_owner(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    if owner.document().selection.is_some() {
        return programme_selection::measured_owner(owner, c);
    }
    if !accepts(owner) {
        return refused();
    }
    measured_current(owner.document(), owner.quota(), c, TopologyPurpose::Current)
}
fn layer_at<'a>(doc: &'a Document, ordinal: usize, c: &ReadControl<'_>) -> Result<&'a crate::Layer, BackingError> {
    fn find<'a>(
        layers: &'a [crate::Layer],
        ordinal: usize,
        count: &mut usize,
        depth: usize,
        c: &ReadControl<'_>,
    ) -> Result<Option<&'a crate::Layer>, BackingError> {
        if depth > 32 {
            return Err(BackingError::Length);
        }
        for l in layers {
            c.check(1)?;
            let n = *count;
            *count = count.checked_add(1).ok_or(BackingError::Length)?;
            if n == ordinal {
                return Ok(Some(l));
            }
            if let LayerContent::Group(g) = &l.content
                && let Some(found) = find(&g.children, ordinal, count, depth + 1, c)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
    find(&doc.layers, ordinal, &mut 0, 0, c)?.ok_or(BackingError::Bounds)
}
fn layer_at_mut<'a>(doc: &'a mut Document, ordinal: usize, c: &ReadControl<'_>) -> Result<&'a mut crate::Layer, BackingError> {
    fn find<'a>(
        layers: &'a mut [crate::Layer],
        ordinal: usize,
        count: &mut usize,
        depth: usize,
        c: &ReadControl<'_>,
    ) -> Result<Option<&'a mut crate::Layer>, BackingError> {
        if depth > 32 {
            return Err(BackingError::Length);
        }
        for l in layers {
            c.check(1)?;
            let n = *count;
            *count = count.checked_add(1).ok_or(BackingError::Length)?;
            if n == ordinal {
                return Ok(Some(l));
            }
            if let LayerContent::Group(g) = &mut l.content
                && let Some(found) = find(&mut g.children, ordinal, count, depth + 1, c)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
    find(&mut doc.layers, ordinal, &mut 0, 0, c)?.ok_or(BackingError::Bounds)
}
fn target_ordinal(doc: &Document, id: LayerId, c: &ReadControl<'_>) -> Result<usize, BackingError> {
    for ordinal in 0..4096 {
        match layer_at(doc, ordinal, c) {
            Ok(layer) => {
                if layer.id == id {
                    return Ok(ordinal);
                }
            }
            Err(BackingError::Bounds) => break,
            Err(e) => return Err(e),
        }
    }
    Err(BackingError::Bounds)
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
    Native(native::Plan<'a>),
    Filter(filter::Plan),
    Move(filter_move::Plan),
    Remove(filter_remove::Plan),
    Metadata(metadata::Plan<'a>),
    Expanded { target: Option<(usize, LayerId, bool)>, value: bool },
}
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    if owner.document().selection.is_some() {
        let p = programme_selection::owner_permit(owner, c)?;
        owner.document().checked_normal_hierarchy_mask_index_requirements_selected(NormalHierarchyLimits::STANDARD, c, &p)?;
    } else {
        owner.document().checked_normal_hierarchy_mask_index_requirements(NormalHierarchyLimits::STANDARD, c.interrupt, &|n| c.check(n))?;
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
        CopyKind::Selection(_)
        | CopyKind::ProgrammeNative(_)
        | CopyKind::Native(_)
        | CopyKind::Filter(_)
        | CopyKind::Move(_)
        | CopyKind::Remove(_)
        | CopyKind::Expanded { .. } => 0,
        CopyKind::Metadata(plan) => plan.replacement_name_bytes()?,
    };
    let archive_bytes = match kind {
        CopyKind::Remove(plan) => plan.copy_index_bytes(),
        _ => 0,
    };
    let extra_work = match kind {
        CopyKind::ProgrammeNative(plan) => plan.copy_work(),
        CopyKind::Native(plan) => plan.copy_work(),
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
    let mut staged = ModelStaging { document: imported_selection::copy_normal_hierarchy(owner.document(), c)?, selection_view: None };
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
        CopyKind::Native(plan) => native::apply(&mut staged.document, plan, c)?,
        CopyKind::Filter(plan) => filter::apply(&mut staged.document, plan, c)?,
        CopyKind::Move(plan) => filter_move::apply(&mut staged.document, plan, c)?,
        CopyKind::Remove(plan) => filter_remove::apply(&mut staged.document, plan, c)?,
        CopyKind::Metadata(plan) => metadata::apply(&mut staged.document, plan, c)?,
        CopyKind::Expanded { target, value } => {
            if let Some((ordinal, id, old)) = target {
                let layer = layer_at_mut(&mut staged.document, ordinal, c)?;
                if layer.id != id {
                    return Err(BackingError::Integrity);
                }
                let LayerContent::Group(group) = &mut layer.content else { return Err(BackingError::Integrity) };
                if group.expanded != old {
                    return Err(BackingError::Integrity);
                }
                c.check(1)?;
                group.expanded = value;
            } else {
                fn expand(layers: &mut [crate::Layer], depth: usize, value: bool, c: &ReadControl<'_>) -> Result<(), BackingError> {
                    if depth > 32 {
                        return Err(BackingError::Length);
                    }
                    for layer in layers {
                        c.check(1)?;
                        if let LayerContent::Group(group) = &mut layer.content {
                            group.expanded = value;
                            expand(&mut group.children, depth + 1, value, c)?;
                        }
                    }
                    Ok(())
                }
                expand(&mut staged.document.layers, 0, value, c)?;
            }
        }
    }
    let actual = if selection_guard.is_some() {
        let p = programme_selection::staged_permit(&staged.document, selection_guard, owner.quota(), c)?;
        programme_selection::measured(&staged.document, owner.quota(), programme_native::kind(owner)?, &p, c)?
    } else {
        measured_current(&staged.document, owner.quota(), c, TopologyPurpose::Current)?
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

impl AccountedDocument {
    pub fn validate_readonly_normal_hierarchy_group_expanded(
        &self,
        layer: Option<LayerId>,
        value: bool,
        all: bool,
        c: &ReadControl<'_>,
    ) -> Result<(), BackingError> {
        let _ = self.normal_hierarchy_expanded_plan(layer, value, all, c)?;
        Ok(())
    }
    fn normal_hierarchy_expanded_plan(
        &self,
        layer: Option<LayerId>,
        value: bool,
        all: bool,
        c: &ReadControl<'_>,
    ) -> Result<(ModelCensus, Option<CopyKind<'static>>), BackingError> {
        let logical = measured_owner(self, c)?;
        if all {
            let index = self.checked_readonly_normal_hierarchy_current_index(c)?;
            let mut changed = false;
            for row in index.rows() {
                c.check(1)?;
                if let LayerContent::Group(g) = &row.layer.content {
                    changed |= g.expanded != value;
                }
            }
            return Ok((logical, changed.then_some(CopyKind::Expanded { target: None, value })));
        }
        let layer = layer.ok_or(BackingError::Bounds)?;
        let ordinal = target_ordinal(self.document(), layer, c)?;
        let target = layer_at(self.document(), ordinal, c)?;
        let LayerContent::Group(group) = &target.content else { return Err(BackingError::Unsupported) };
        c.check(1)?;
        Ok((logical, (group.expanded != value).then_some(CopyKind::Expanded { target: Some((ordinal, layer, group.expanded)), value })))
    }
    pub fn try_with_readonly_normal_hierarchy_group_expanded(
        &self,
        layer: Option<LayerId>,
        value: bool,
        all: bool,
        c: &ReadControl<'_>,
    ) -> Result<Self, BackingError> {
        let (logical, plan) = self.normal_hierarchy_expanded_plan(layer, value, all, c)?;
        let Some(plan) = plan else {
            c.check(1)?;
            return Ok(self.clone());
        };
        current_copy(self, logical, plan, c)
    }
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

pub(super) fn programme_selected_measured(doc: &Document, q: &BackingQuota, p: &SelectionPermit<'_>, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    let index = doc.checked_normal_hierarchy_mask_index_selected(NormalHierarchyLimits::STANDARD, q, c, p)?;
    shape_rows_selected(doc, q, c, TopologyPurpose::Current, index.rows(), true, Some(p))?;
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
#[path = "accounted_normal_hierarchy_tests.rs"]
// Test assertions are permitted by repository policy.
#[allow(clippy::unreachable)]
mod tests;

impl AccountedDocument {
    /// Source-only synchronous local callback; no callback is retained. The
    /// original prepaid native control and dynamic original Budget checks remain
    /// separate authorities, without a new counter or quota.
    pub fn new_readonly_normal_hierarchy_storage_with_local_work(
        document: Document,
        retention: Arc<dyn Any + Send + Sync>,
        quota: BackingQuota,
        c: &ReadControl<'_>,
        work: &dyn Fn(u64) -> Result<(), BackingError>,
    ) -> Result<Self, BackingError> {
        struct Local<'a, 'b> {
            base: &'a ReadControl<'b>,
            work: &'a dyn Fn(u64) -> Result<(), BackingError>,
        }
        impl ModelControl for Local<'_, '_> {
            fn check(&self, n: u64) -> Result<(), BackingError> {
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)?;
                (self.work)(n)?;
                self.base.interrupt.check().map_err(|_| BackingError::Cancelled)
            }
            fn sync_control(&self) -> &ReadControl<'_> {
                self.base
            }
        }
        struct Incoming {
            document: Document,
            retention: Arc<dyn Any + Send + Sync>,
        }
        let incoming = Incoming { document, retention };
        let local = Local { base: c, work };
        measured(&incoming.document, &quota, &local, TopologyPurpose::Source)?;
        let lease = quota.reserve(QuotaKind::Index, Self::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
        local.check(1)?;
        Ok(Self {
            owner: Arc::new(ModelOwner {
                _storage_class: StorageClass::ReadonlyNormalHierarchy,
                document: incoming.document,
                _selection: None,
                _selection_view: None,
                retention: incoming.retention,
                quota,
                _model: None,
                _allocation: Arc::new(lease),
            }),
        })
    }
}
