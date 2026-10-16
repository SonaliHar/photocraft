//! Explicit immutable isolated small-Auto Gaussian group custody. The runtime
//! class is not a source, profile, evaluator or generic-edit grant.
use super::*;
use crate::import_compositing::{GimpOperator, GimpSpace};
use crate::xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity};
use crate::{ColorMode, PixelFormat, SampleEncoding, SampleType, item_identity::ItemRef};

const FILTER_EPOCH: &str = "xcf22-isolated-small-auto-v1";
#[path = "accounted_gaussian_group_readonly_enabled.rs"]
mod enabled;
pub use enabled::GaussianGroupEnabledPatch;
#[path = "accounted_gaussian_group_readonly_patch.rs"]
mod patch;
pub use patch::GaussianGroupPatch;
#[path = "accounted_gaussian_group_readonly_metadata.rs"]
pub(super) mod metadata;
fn fail() -> BackingError {
    BackingError::Unsupported
}
fn exact(value: &str, expected: &str, c: &ReadControl<'_>) -> Result<bool, BackingError> {
    if value.len() > 4096 {
        return Err(BackingError::Length);
    }
    c.check(value.len() as u64 + 1)?;
    Ok(value == expected)
}
// The former global XcfState validator checked these inert metadata fields.
// Retain its text/hash invariants without its allocating sets or unpaid scans.
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
fn hash_syntax(value: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if value.len() != 64 {
        return Err(BackingError::Integrity);
    }
    c.check(64)?;
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackingError::Integrity);
    }
    Ok(())
}
fn payload_digest(bytes: &[u8], expected: &str, c: &ReadControl<'_>) -> Result<(), BackingError> {
    if bytes.len() > 64 << 20 {
        return Err(BackingError::Length);
    }
    hash_syntax(expected, c)?;
    c.check(std::mem::size_of::<blake3::Hasher>() as u64 + 128)?;
    let mut hash = blake3::Hasher::new();
    for chunk in bytes.chunks(32 << 10) {
        c.check(chunk.len() as u64 + 1)?;
        hash.update(chunk);
    }
    c.check(128)?;
    if hash.finalize().to_hex().as_str() != expected {
        return Err(BackingError::Integrity);
    }
    c.check(1)
}
fn node_shape(node: &FilterNode, extent: Rect, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    for value in [&node.source_display_name, &node.source_icon].into_iter().flatten() {
        text(value, c)?;
    }
    if node.id.0 == 0
        || !exact(&node.operation.name, "gegl:gaussian-blur", c)?
        || !exact(&node.operation.stored_version, "0:0", c)?
        || !exact(&node.operation.schema_epoch, FILTER_EPOCH, c)?
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(if extent.y0 == -1 { 0 } else { 1 })
        || !node.auxiliary.is_empty()
        || node.arguments.len() != 5
    {
        return Err(fail());
    }
    let policy = node.composition.as_ref().ok_or_else(fail)?;
    let raw = [policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator];
    if !exact(&policy.compatibility_epoch, FILTER_EPOCH, c)?
        || policy.source_mode != 62
        || policy.legacy
        || policy.pass_through
        || policy.resolved.is_some()
        || !(raw == [-643996976; 3] || (!node.enabled && raw == [-1474592175; 3]))
    {
        return Err(fail());
    }
    let mut seen = [false; 5];
    for arg in &node.arguments {
        c.check(1)?;
        if arg.name.len() > 64 {
            return Err(BackingError::Length);
        }
        c.check(arg.name.len() as u64)?;
        let index = match (arg.name.as_str(), &arg.value) {
            ("std-dev-x", FilterValue::Number(ExactNumber::Float32Bits(bits))) if patch::valid_axis(*bits) => 0,
            ("std-dev-y", FilterValue::Number(ExactNumber::Float32Bits(bits))) if patch::valid_axis(*bits) => 1,
            ("filter", FilterValue::Enum { domain, value: 0 }) if exact(domain, "gegl:gaussian-blur::filter", c)? => 2,
            ("abyss-policy", FilterValue::Enum { domain, value: 1 }) if exact(domain, "gegl:gaussian-blur::abyss-policy", c)? => 3,
            ("clip-extent", FilterValue::Bool(true)) => 4,
            _ => return Err(fail()),
        };
        let slot = seen.get_mut(index).ok_or(BackingError::Integrity)?;
        if *slot {
            return Err(BackingError::Integrity);
        }
        *slot = true;
    }
    Ok(())
}
pub(super) fn shape(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<(), BackingError> {
    c.check(1)?;
    let state = doc.xcf_state.as_ref().ok_or_else(fail)?;
    let source = doc.source_color_context.as_ref().ok_or_else(fail)?;
    if doc.size != crate::Size::new(16, 12)
        || doc.mode != ColorMode::Rgb
        || doc.depth != SampleType::F32
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(doc.working_profile_interpretation, WorkingProfileInterpretation::ImplicitSrgb { .. })
        || doc.icc_profile.as_ref().is_none_or(|p| !(132..=65536).contains(&p.len()))
        || source.original_icc.is_some()
        || source.source_precision != 650
        || doc.layers.len() != 2
        || !doc.channels.is_empty()
        || doc.selection.is_some()
        || doc.selection_channel_id.is_some()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !exact(&state.provenance.format, "xcf", c)?
        || state.provenance.format_version != 22
        || state.items.len() != 6
        || state.current_filter_output_precision != crate::FilterOutputPrecision::Canonical
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || state.floating.is_some()
        || !state.resources.is_empty()
        || state.opaque.len() > 4096
        || !state.item_sets.is_empty()
        || state.selection.selected.len() > 1
        || !state.selection.selected_paths.is_empty()
        || state.selection.mask_target.is_some()
        || state.provenance.protected_source_fingerprints.len() > 4096
    {
        return Err(fail());
    }
    let provenance = &state.provenance;
    for value in [&provenance.format, &provenance.container, &provenance.source_label] {
        text(value, c)?;
    }
    for value in [&provenance.producer, &provenance.release_id, &provenance.original_locator].into_iter().flatten() {
        text(value, c)?;
    }
    if let Some(hash) = &provenance.source_hash {
        hash_syntax(hash, c)?;
    }
    for hash in &provenance.protected_source_fingerprints {
        c.check(1)?;
        hash_syntax(hash, c)?;
    }
    let base = doc.layers.first().ok_or_else(fail)?;
    let group = doc.layers.get(1).ok_or_else(fail)?;
    let LayerContent::Group(tree) = &group.content else { return Err(fail()) };
    if tree.children.len() != 2 || tree.artboard.is_some() {
        return Err(fail());
    }
    let multiply = tree.children.first().ok_or_else(fail)?;
    let normal = tree.children.get(1).ok_or_else(fail)?;
    let layers = [base, group, multiply, normal];
    let mut extents = [doc.bounds(); 4];
    for (index, layer) in layers.iter().enumerate() {
        c.check(1)?;
        if layer.id.0 == 0
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.fill_opacity.to_bits() != 1f32.to_bits()
            || layer.clipped
            || layer.excluded_channels != 0
            || layer.blend != if index == 2 { crate::BlendMode::Multiply } else { crate::BlendMode::Normal }
            || layer.blend_if != Default::default()
            || layer.advanced != Default::default()
            || layer.indexed_source.is_some()
            || layer.vector_mask.is_some()
            || !layer.effects.items.is_empty()
            || layer.effects.psd_raw.is_some()
            || !layer.psd_blocks.is_empty()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.import_compositing
                != Some(crate::ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false })
            || (index != 1 && layer.mask.is_some())
        {
            return Err(fail());
        }
        for previous in layers.get(..index).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous.id == layer.id {
                return Err(BackingError::Integrity);
            }
        }
        let mut found = None;
        for item in &state.items {
            c.check(1)?;
            if item.item == ItemRef::Layer(layer.id) {
                if found.is_some() {
                    return Err(BackingError::Integrity);
                }
                found = Some(item);
            }
        }
        let item = found.ok_or(BackingError::Integrity)?;
        let extent = item.raster_bounds.ok_or_else(fail)?;
        *extents.get_mut(index).ok_or(BackingError::Length)? = extent;
        if item.parent != if index < 2 { None } else { Some(ItemRef::Layer(group.id)) } || item.mask.is_some() || item.visible != layer.visible {
            return Err(fail());
        }
        let policy = item.composition.as_ref().ok_or_else(fail)?;
        let epoch = if index == 1 {
            crate::import_compositing::XCF13_ISOLATED_MULTIPLY_SEED_EPOCH
        } else if index == 2 {
            "xcf-22-authored-blend-auto"
        } else {
            "xcf-22-normal"
        };
        if !exact(&policy.compatibility_epoch, epoch, c)?
            || policy.source_mode != if index == 2 { 30 } else { 28 }
            || policy.legacy
            || policy.pass_through
            || (policy.stored_blend_space, policy.stored_composite_space, policy.stored_operator) != if index == 2 { (-1, -1, -2) } else { (0, -1, -1) }
        {
            return Err(fail());
        }
        let resolved = policy.checked_resolved().map_err(|_| BackingError::Integrity)?;
        if resolved.blend_space != GimpSpace::LinearRgb
            || resolved.composite_space != GimpSpace::LinearRgb
            || resolved.operator != if index == 2 { GimpOperator::ClipToBackdrop } else { GimpOperator::Union }
        {
            return Err(fail());
        }
        if index != 1 {
            let LayerContent::BackedRaster(backed) = &layer.content else { return Err(fail()) };
            if backed.format() != PixelFormat::new(ColorMode::Rgb, SampleType::F32, true)
                || !q.shares_ledger(backed.quota())
                || backed.operative_bounds() != extent
            {
                return Err(BackingError::Integrity);
            }
        }
    }
    let extent = extents[1];
    if extents[0] != doc.bounds()
        || extents[3] != Rect::from_xywh(0, 0, 11, 8)
        || !((extent == Rect::from_xywh(0, -1, 12, 9) && extents[2] == Rect::from_xywh(2, -1, 10, 9))
            || (extent == Rect::from_xywh(-1, 0, 12, 11) && extents[2] == Rect::from_xywh(-1, 2, 10, 9)))
    {
        return Err(fail());
    }
    let physical = group.mask.as_ref().ok_or_else(fail)?;
    if physical.surface.format() != PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false)
        || physical.surface.tile_count() > 4
        || !physical.linked
        || physical.density.to_bits() != 1f32.to_bits()
        || physical.feather.to_bits() != 0f32.to_bits()
    {
        return Err(fail());
    }
    validate_shared_mask(physical, &mut 0, c)?;
    let stack = state.filter_stacks.first().ok_or_else(fail)?;
    if stack.owner != ItemRef::Layer(group.id) || stack.nodes.len() != 1 {
        return Err(fail());
    }
    let node = stack.nodes.first().ok_or_else(fail)?;
    node_shape(node, extent, c)?;
    let filter = state.filter_masks.first().ok_or_else(fail)?;
    if filter.id.0 == 0
        || node.mask != Some(ItemRef::Mask(filter.id))
        || filter.owner != node.id
        || filter.bounds != doc.bounds()
        || filter.surface.format() != PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false)
    {
        return Err(fail());
    }
    filter_readonly::MaskCopyPlan::new(&filter.surface, filter.bounds, c)?;
    let mut physical_id = None;
    for item in &state.items {
        c.check(1)?;
        if let ItemRef::Mask(id) = item.item {
            let m = item.mask.as_ref().ok_or_else(fail)?;
            let is_filter = id == filter.id;
            if id.0 == 0
                || item.parent.is_some()
                || item.composition.is_some()
                || !item.visible
                || m.owner != ItemRef::Layer(group.id)
                || m.enabled != if is_filter { true } else { physical.enabled }
                || m.shown
                || m.bounds != if is_filter { doc.bounds() } else { extent }
                || m.offset != if is_filter { [0, 0] } else { [extent.x0, extent.y0] }
                || m.outside_f64_bits != 0f64.to_bits()
                || m.linked == is_filter
                || m.editing == is_filter
                || item.raster_bounds != Some(m.bounds)
            {
                return Err(fail());
            }
            if !is_filter {
                if physical_id.replace(id).is_some() {
                    return Err(BackingError::Integrity);
                }
            }
        }
    }
    if physical_id.is_none() {
        return Err(BackingError::Integrity);
    }
    // Canonical references remain bounded and may not point to absent objects.
    for (i, item) in state.items.iter().enumerate() {
        text(&item.name, c)?;
        for previous in state.items.get(..i).ok_or(BackingError::Length)? {
            c.check(1)?;
            if previous.item == item.item {
                return Err(BackingError::Integrity);
            }
        }
    }
    let present = |id| -> Result<(), BackingError> {
        for item in &state.items {
            c.check(1)?;
            if item.item == id {
                return Ok(());
            }
        }
        Err(BackingError::Integrity)
    };
    for id in &state.selection.selected {
        present(*id)?;
    }
    if let Some(id) = state.selection.active {
        present(id)?;
        c.check(1)?;
        if state.selection.selected.first() != Some(&id) {
            return Err(BackingError::Integrity);
        }
    }
    let mut opaque_bytes = 0usize;
    for raw in &state.opaque {
        c.check(1)?;
        if raw.rendering_required {
            return Err(fail());
        }
        if let Some(id) = raw.owner {
            present(id)?;
        }
        text(&raw.namespace, c)?;
        text(&raw.identity, c)?;
        opaque_bytes = opaque_bytes.checked_add(raw.bytes.len()).ok_or(BackingError::Length)?;
        if opaque_bytes > 256 << 20 {
            return Err(BackingError::Length);
        }
        payload_digest(&raw.bytes, &raw.blake3, c)?;
    }
    for set in &state.item_sets {
        if set.members.len() > 6 {
            return Err(BackingError::Length);
        }
        for id in &set.members {
            present(*id)?;
        }
    }
    c.check(1)?;
    Ok(())
}
fn measured(doc: &Document, q: &BackingQuota, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    shape(doc, q, c)?;
    census_inner(doc, true, true, None, q, c, true)
}
impl AccountedDocument {
    /// Consuming trusted codec handoff. The guard must already retain the real
    /// model, immutable masks/native planes and profile/source charges on q.
    /// This constructor charges only its additional owner/Arc metadata.
    pub fn from_owned_readonly_gaussian_group(
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
                _storage_class: StorageClass::ReadonlyGaussianGroup,
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
    pub fn is_readonly_gaussian_group_storage(&self) -> bool {
        self.owner._storage_class == StorageClass::ReadonlyGaussianGroup
    }
    pub fn validate_gaussian_group_storage(&self, c: &ReadControl<'_>) -> Result<(), BackingError> {
        if !self.is_readonly_gaussian_group_storage() {
            return Err(fail());
        }
        measured(self.document(), self.quota(), c)?;
        Ok(())
    }
}

struct OriginalRetention {
    original: AccountedDocument,
}
fn copy_node(node: &FilterNode, c: &ReadControl<'_>) -> Result<FilterNode, BackingError> {
    let mut arguments = Vec::new();
    arguments.try_reserve_exact(5).map_err(|_| BackingError::Allocation)?;
    if arguments.capacity() > 10 {
        return Err(BackingError::Allocation);
    }
    for arg in &node.arguments {
        c.check(1)?;
        let value = match &arg.value {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => FilterValue::Number(ExactNumber::Float32Bits(*bits)),
            FilterValue::Bool(v) => FilterValue::Bool(*v),
            FilterValue::Enum { domain, value } => FilterValue::Enum { domain: imported_edits::copy_name(domain, c)?, value: *value },
            _ => return Err(fail()),
        };
        arguments.push(FilterArgument { name: imported_edits::copy_name(&arg.name, c)?, value });
    }
    let old = node.composition.as_ref().ok_or_else(fail)?;
    let composition = crate::import_compositing::GimpComposition {
        compatibility_epoch: imported_edits::copy_name(&old.compatibility_epoch, c)?,
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
            name: imported_edits::copy_name(&node.operation.name, c)?,
            stored_version: imported_edits::copy_name(&node.operation.stored_version, c)?,
            schema_epoch: imported_edits::copy_name(&node.operation.schema_epoch, c)?,
        },
        source_display_name: node.source_display_name.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
        source_icon: node.source_icon.as_deref().map(|v| imported_edits::copy_name(v, c)).transpose()?,
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
    let logical = measured(owner.document(), owner.quota(), c)?;
    if imported_edits::names_equal(&owner.document().name, name, c)? {
        c.check(1)?;
        return Ok(owner.clone());
    }
    current_copy(owner, logical, CopyKind::Title(name), c)
}
#[derive(Clone, Copy)]
enum CopyKind<'a> {
    ProgrammeNative(programme_native::Plan<'a>),
    Title(&'a str),
    Filter(patch::Plan),
    Metadata(metadata::Plan<'a>),
}
fn current_copy(owner: &AccountedDocument, logical: ModelCensus, kind: CopyKind<'_>, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    let name_bytes = match kind {
        CopyKind::ProgrammeNative(_) => 0,
        CopyKind::Title(name) => name.len() as u64,
        CopyKind::Filter(_) => 0,
        CopyKind::Metadata(plan) => plan.replacement_name_bytes()?,
    };
    let wrapper = std::mem::size_of::<OriginalRetention>() as u64 + 128;
    let envelope = logical
        .index
        .checked_mul(2)
        .and_then(|n| n.checked_add(name_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(4096 + wrapper))
        .ok_or(BackingError::Length)?;
    let allocation = owner.quota().reserve(QuotaKind::Index, AccountedDocument::owned_model_reservation_bytes()).map_err(|_| BackingError::Quota)?;
    let mut index = owner.quota().reserve(QuotaKind::Index, envelope).map_err(|_| BackingError::Quota)?;
    let payload_envelope = logical.payload.checked_mul(2).ok_or(BackingError::Length)?;
    let mut payload = owner.quota().reserve(QuotaKind::Resident, payload_envelope).map_err(|_| BackingError::Quota)?;
    c.check(logical.total()?)?;
    if let CopyKind::ProgrammeNative(plan) = kind { c.check(plan.copy_work())?; }
    let document = imported_selection::copy_gaussian_group(owner.document(), owner.quota(), c)?;
    let mut staged = ModelStaging { document, selection_view: None };
    let source = owner.document().xcf_state.as_ref().ok_or_else(fail)?;
    let target = staged.document.xcf_state.as_mut().ok_or_else(fail)?;
    let stack = source.filter_stacks.first().ok_or_else(fail)?;
    target.filter_stacks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    target.filter_masks.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(1).map_err(|_| BackingError::Allocation)?;
    if target.filter_stacks.capacity() > 2 || target.filter_masks.capacity() > 2 || nodes.capacity() > 2 {
        return Err(BackingError::Allocation);
    }
    nodes.push(copy_node(stack.nodes.first().ok_or_else(fail)?, c)?);
    target.filter_stacks.push(FilterStack { owner: stack.owner, revision: stack.revision, nodes });
    let mask = source.filter_masks.first().ok_or_else(fail)?;
    let plan = filter_readonly::MaskCopyPlan::new(&mask.surface, mask.bounds, c)?;
    target.filter_masks.push(crate::FilterMask { id: mask.id, owner: mask.owner, bounds: mask.bounds, surface: plan.copy(&mask.surface, c)? });
    match kind {
        CopyKind::ProgrammeNative(plan) => programme_native::apply(&mut staged.document, plan, c)?,
        CopyKind::Title(name) => staged.document.name = imported_edits::copy_name(name, c)?,
        CopyKind::Filter(plan) => patch::apply(&mut staged.document, plan, c)?,
        CopyKind::Metadata(plan) => metadata::apply(&mut staged.document, plan, c)?,
    }
    let actual = measured(&staged.document, owner.quota(), c)?;
    let actual_index = actual.index.checked_add(wrapper).ok_or(BackingError::Length)?;
    if actual_index > envelope || actual.payload > payload_envelope {
        return Err(BackingError::Allocation);
    }
    index.reconcile_capacity(actual_index).map_err(|_| BackingError::Quota)?;
    payload.reconcile_capacity(actual.payload).map_err(|_| BackingError::Quota)?;
    c.check(1)?;
    let original = owner.owner.retention.downcast_ref::<OriginalRetention>().map_or_else(|| owner.clone(), |r| r.original.clone());
    let retention = Arc::new(OriginalRetention { original });
    Ok(AccountedDocument {
        owner: Arc::new(ModelOwner {
            _storage_class: StorageClass::ReadonlyGaussianGroup,
            document: staged.document,
            _selection: None,
            _selection_view: None,
            retention,
            quota: owner.quota().clone(),
            _model: Some(ModelCharges { _index: index, _payload: payload }),
            _allocation: Arc::new(allocation),
        }),
    })
}

// Private additive programme-native bridge. Existing entrypoints and copier
// schedules retain their own class-specific custody and final census.
pub(super) fn programme_measured(owner: &AccountedDocument, c: &ReadControl<'_>) -> Result<ModelCensus, BackingError> {
    measured(owner.document(), owner.quota(), c)
}
pub(super) fn programme_copy(owner: &AccountedDocument, logical: ModelCensus, plan: programme_native::Plan<'_>, c: &ReadControl<'_>) -> Result<AccountedDocument, BackingError> {
    current_copy(owner, logical, CopyKind::ProgrammeNative(plan), c)
}

#[cfg(test)]
#[path = "accounted_gaussian_group_readonly_tests.rs"]
mod tests;
