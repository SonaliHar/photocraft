//! Borrowed full-current selected channel proof. Unrelated scene is retained,
//! never evaluated or qualified. No full-document XCF validator is called here.
use super::*;
use crate::native_save::NativeCurrentCensus;

#[derive(Clone, Copy, Debug)]
pub struct SelectedLimits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for SelectedLimits {
    fn default() -> Self {
        let p = crate::NativeSaveLimits::default().preparation;
        Self {
            resident_bytes: p.resident_bytes as u64,
            index_bytes: p.index_bytes as u64,
            work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 },
        }
    }
}
/// Explicit current interpretation policy, not authority inferred from an absent ICC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrentProfilePolicy {
    OriginalExact,
    MaterializedImplicit,
}
#[derive(Clone, Copy, Debug)]
pub struct SelectedProfileRequirements {
    pub resident_bytes: u64,
    pub work_units: u64,
}
fn cms_limits(limits: SelectedLimits) -> Result<photocraft_cms::control::Limits, IoError> {
    let maximum = photocraft_cms::control::Limits::default();
    Ok(photocraft_cms::control::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes.min(maximum.resident_bytes as u64)).map_err(|_| bad("selected CMS address space"))?,
        work_units: limits.work_units.min(maximum.work_units),
    })
}
fn implicit_fields<'d>(original: &Document, doc: &'d Document) -> Result<&'d [u8], IoError> {
    if doc.mode != original.mode
        || doc.depth != original.depth
        || doc.sample_encoding != original.sample_encoding
        || doc.source_color_context != original.source_color_context
        || !matches!(&doc.working_profile_interpretation,photocraft_doc::WorkingProfileInterpretation::ImplicitSrgb{icc_blake3} if icc_blake3.len()==64&&icc_blake3.bytes().all(|v|v.is_ascii_hexdigit()))
    {
        return Err(bad("selected materialized implicit context differs from original"));
    }
    let bytes = doc.icc_profile.as_ref().ok_or_else(|| bad("explicit implicit marker needs current ICC"))?;
    if bytes.is_empty() || bytes.len() > 16 << 20 {
        return Err(bad("selected materialized ICC bound"));
    }
    Ok(bytes.as_slice())
}
fn implicit_context<'d>(owner: &AssembledStoredChannel<'_, '_, '_>, doc: &'d Document) -> Result<&'d [u8], IoError> {
    // General CMS Gray or transfer support does not widen the closed assembly.
    if !owner.original.icc.is_empty() || owner.original.model != Model::Rgb || owner.original.encoding != Encoding::NonlinearProfile {
        return Err(bad("selected materialized source descriptor outside original domain"));
    }
    implicit_fields(&owner.document, doc)
}
/// Borrowed accounting only, never an interpretation proof. The engine may use
/// this before full-current census to prepay the eventual exact CMS verifier.
pub fn selected_profile_requirements(
    owner: &AssembledStoredChannel<'_, '_, '_>,
    doc: &Document,
    policy: CurrentProfilePolicy,
    limits: SelectedLimits,
    ctl: &Interrupt<'_>,
) -> Result<SelectedProfileRequirements, IoError> {
    tick(ctl)?;
    let maximum = SelectedLimits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.index_bytes > maximum.index_bytes || limits.work_units > maximum.work_units {
        return Err(bad("selected profile lower-only limits"));
    }
    if policy == CurrentProfilePolicy::OriginalExact {
        return Ok(SelectedProfileRequirements { resident_bytes: 0, work_units: 0 });
    }
    let bytes = implicit_context(owner, doc)?;
    let r = photocraft_cms::xcf_color::effective_implicit_profile_requirements_with_control(
        bytes,
        photocraft_cms::ColorSpace::Rgb,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        cms_limits(limits)?,
        &|| ctl.cancelled(),
    )
    .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
    let required = SelectedProfileRequirements {
        resident_bytes: r.peak_resident_bytes as u64,
        work_units: plus(times(r.work_units, 3)?, plus(bytes.len() as u64, 4096)?)?,
    };
    if required.resident_bytes > limits.resident_bytes || required.work_units > limits.work_units {
        return Err(bad("selected implicit profile envelope"));
    }
    tick(ctl)?;
    Ok(required)
}
#[derive(Clone, Copy, Debug)]
pub struct SelectedRequirements {
    pub current_retained_bytes: u64,
    pub current_index_bytes: u64,
    pub current_map_bytes: u64,
    pub complete_resident_bytes: u64,
    pub complete_index_bytes: u64,
    pub selection_work_units: u64,
    pub verification_work_units: u64,
    pub implicit_profile_resident_bytes: u64,
    pub implicit_profile_work_units: u64,
}
/// Private construction requires genuine same-document census receipts and a
/// bounded namespace/dependency scan. The census and immutable borrow stay live.
pub struct SelectedCurrentChannel<'d> {
    census: NativeCurrentCensus<'d>,
    original_owner_document: &'d Document,
    channel: &'d AlphaChannel,
    channel_item: &'d ItemState,
    mask: &'d FilterMask,
    mask_item: &'d ItemState,
    node: &'d FilterNode,
    ids: Ids,
    source_sha256: &'static str,
    source_blake3: blake3::Hash,
    requirements: SelectedRequirements,
    profile_policy: CurrentProfilePolicy,
    profile_limits: photocraft_cms::control::Limits,
}
impl<'d> SelectedCurrentChannel<'d> {
    pub fn document(&self) -> &'d Document {
        self.census.document()
    }
    pub fn channel_id(&self) -> ChannelId {
        self.ids.channel
    }
    pub fn mask_id(&self) -> MaskId {
        self.ids.mask
    }
    pub fn original_node(&self) -> &'d FilterNode {
        self.node
    }
    /// Already captured native stored pixels; no rescan or appearance authority.
    pub fn channel_surface(&self) -> &'d Surface {
        &self.channel.surface
    }
    /// Exact declared 32x32 extent checked while constructing this descriptor.
    pub fn channel_bounds(&self) -> Rect {
        Rect::new(0, 0, 32, 32)
    }
    pub fn filter_mask(&self) -> &'d FilterMask {
        self.mask
    }
    pub fn channel_item(&self) -> &'d ItemState {
        self.channel_item
    }
    pub fn mask_item(&self) -> &'d ItemState {
        self.mask_item
    }
    pub fn requirements(&self) -> SelectedRequirements {
        self.requirements
    }
}
fn plus(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("selected accounting overflow"))
}
fn times(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("selected accounting overflow"))
}
fn tick(ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}
fn selected_raw(ids: Ids, raw: u64) -> bool {
    raw == ids.channel.0 || raw == ids.mask.0 || raw == ids.filter.0
}
fn selected_ref(ids: Ids, r: ItemRef) -> bool {
    r == ItemRef::Channel(ids.channel) || r == ItemRef::Mask(ids.mask)
}
fn layers(layers: &[photocraft_doc::Layer], ids: Ids, depth: usize, visits: &mut usize, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    if depth > 256 {
        return Err(bad("selected layer hierarchy depth"));
    }
    for layer in layers {
        tick(ctl)?;
        *visits = visits.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| bad("selected layer scan count"))?;
        if selected_raw(ids, layer.id.0) {
            return Err(bad("selected identity aliases native layer"));
        }
        if let photocraft_doc::LayerContent::Group(group) = &layer.content {
            self::layers(&group.children, ids, depth + 1, visits, ctl)?;
        }
    }
    Ok(())
}
fn bounded_node(node: &FilterNode) -> Result<(), IoError> {
    if node.arguments.len() != 5
        || !node.auxiliary.is_empty()
        || node.operation.name.len() > 256
        || node.operation.stored_version.len() > 256
        || node.operation.schema_epoch.len() > 256
        || node.source_display_name.as_ref().is_some_and(|s| s.len() > 256)
        || node.source_icon.as_ref().is_some_and(|s| s.len() > 256)
    {
        return Err(bad("selected node metadata bound"));
    }
    for a in &node.arguments {
        if a.name.len() > 128 {
            return Err(bad("selected argument name bound"));
        }
        match &a.value {
            photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(_))
            | photocraft_doc::xcf_filters::FilterValue::Bool(_) => (),
            photocraft_doc::xcf_filters::FilterValue::Enum { domain, .. } if domain.len() <= 128 => (),
            _ => return Err(bad("selected scalar node domain")),
        }
    }
    Ok(())
}
/// The reserve callback must keep the entire lease alive until the descriptor
/// and proof are released. Work is prepaid before any namespace traversal.
/// Limits are remaining quotas, not asserted retention or permission grants.
pub fn select_current<'d>(
    census: NativeCurrentCensus<'d>,
    owner: &'d AssembledStoredChannel<'_, '_, '_>,
    limits: SelectedLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<SelectedCurrentChannel<'d>, IoError> {
    select_current_with_profile_policy(census, owner, CurrentProfilePolicy::OriginalExact, limits, ctl, reserve, charge)
}
/// Additive explicitly requested current-profile interpretation route.
#[allow(clippy::too_many_arguments)]
pub fn select_current_with_profile_policy<'d>(
    census: NativeCurrentCensus<'d>,
    owner: &'d AssembledStoredChannel<'_, '_, '_>,
    policy: CurrentProfilePolicy,
    limits: SelectedLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<SelectedCurrentChannel<'d>, IoError> {
    tick(ctl)?;
    let maximum = SelectedLimits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.index_bytes > maximum.index_bytes || limits.work_units > maximum.work_units {
        return Err(bad("selected lower-only limits"));
    }
    let doc = census.document();
    let ids = owner.ids;
    if ids.channel.0 == 0 || ids.mask.0 == 0 || ids.filter.0 == 0 || ids.channel.0 == ids.mask.0 || ids.channel.0 == ids.filter.0 || ids.mask.0 == ids.filter.0
    {
        return Err(bad("selected zero/aliased identities"));
    }
    let source = owner.correspondence_requirements()?;
    let profile = selected_profile_requirements(owner, doc, policy, limits, ctl)?;
    let current_retained = census.retained_bytes() as u64;
    let current_index = census.known_index_reservation_bytes() as u64;
    let current_map = census.known_map_reservation_bytes() as u64;
    // Includes the bounded borrowed recursion frames and descriptor, not an
    // allocated full-scene index. No selected-slice retention refund is made.
    let scratch = plus(256 * 256, std::mem::size_of::<SelectedCurrentChannel<'d>>() as u64)?;
    let complete_index = plus(plus(current_index, source.index_bytes)?, scratch)?;
    let complete_resident = plus(plus(plus(plus(current_retained, current_map)?, source.resident_bytes)?, complete_index)?, profile.resident_bytes)?;
    let visits = census.visit_scope();
    let outer = plus(plus(visits.identity_visits as u64, visits.layer_visits as u64)?, plus(visits.surface_visits as u64, visits.model_visits as u64)?)?;
    let selection_work = plus(times(census.model_retained_bytes() as u64, 16)?, plus(times(outer, 16)?, 65536)?)?;
    let verification_work = plus(plus(source.work_units, times(current_retained, 8)?)?, profile.work_units)?;
    if complete_resident > limits.resident_bytes || complete_index > limits.index_bytes || plus(selection_work, verification_work)? > limits.work_units {
        return Err(bad("complete selected owner envelope"));
    }
    reserve(complete_resident, complete_index)?;
    charge(selection_work)?;
    tick(ctl)?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("selected current source state missing"))?;
    if doc.channels.len() > 10_000
        || doc.paths.len() > 10_000
        || state.items.len() > 100_000
        || state.filter_masks.len() > 10_000
        || state.filter_stacks.len() > 10_000
        || state.source_items.len() > 10_000
        || state.opaque.len() > 100_000
        || state.resources.len() > 10_000
        || state.item_sets.len() > 10_000
    {
        return Err(bad("selected namespace bound"));
    }
    if doc.selection_channel_id.is_some_and(|id| selected_raw(ids, id.0))
        || doc.quick_mask.as_ref().is_some_and(|c| selected_raw(ids, c.id.0))
        || state.floating.as_ref().is_some_and(|f| selected_ref(ids, f.target) || selected_raw(ids, f.layer.0))
    {
        return Err(bad("selected identity aliases temporary/selection role"));
    }
    layers(&doc.layers, ids, 0, &mut 0, ctl)?;
    let mut channel = None;
    for c in &doc.channels {
        tick(ctl)?;
        if c.id == ids.channel {
            if channel.replace(c).is_some() {
                return Err(bad("duplicate selected native channel"));
            }
        } else if selected_raw(ids, c.id.0) {
            return Err(bad("selected identity aliases native channel"));
        }
    }
    for p in &doc.paths {
        tick(ctl)?;
        if selected_raw(ids, p.id.0) {
            return Err(bad("selected identity aliases path"));
        }
    }
    let mut channel_item = None;
    let mut mask_item = None;
    for i in &state.items {
        tick(ctl)?;
        if i.item == ItemRef::Channel(ids.channel) {
            if channel_item.replace(i).is_some() {
                return Err(bad("duplicate selected channel item"));
            }
        } else if i.item == ItemRef::Mask(ids.mask) {
            if mask_item.replace(i).is_some() {
                return Err(bad("duplicate selected mask item"));
            }
        } else if selected_raw(ids, i.item.raw())
            || i.parent.is_some_and(|p| selected_raw(ids, p.raw()))
            || i.mask.as_ref().is_some_and(|m| selected_raw(ids, m.owner.raw()))
        {
            return Err(bad("selected identity/owner dependency alias"));
        }
    }
    let channel = channel.ok_or_else(|| bad("selected native channel missing"))?;
    let channel_item = channel_item.ok_or_else(|| bad("selected channel item missing"))?;
    let mask_item = mask_item.ok_or_else(|| bad("selected mask item missing"))?;
    if channel.name.len() > 256
        || channel_item.name.len() > 256
        || mask_item.name.len() > 256
        || channel_item.parent.is_some()
        || channel_item.mask.is_some()
        || channel_item.raster_bounds != Some(Rect::new(0, 0, 32, 32))
        || channel.spot.is_some()
    {
        return Err(bad("selected channel metadata domain"));
    }
    let mut node = None;
    let mut node_count = 0usize;
    for stack in &state.filter_stacks {
        tick(ctl)?;
        node_count = node_count.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("selected filter scan bound"))?;
        if stack.owner == ItemRef::Channel(ids.channel) {
            if stack.nodes.len() != 1 || node.replace(stack.nodes.first().ok_or_else(|| bad("selected node missing"))?).is_some() {
                return Err(bad("selected stack ambiguous"));
            }
        } else if selected_raw(ids, stack.owner.raw()) {
            return Err(bad("selected stack owner alias"));
        }
        for n in &stack.nodes {
            tick(ctl)?;
            if n.auxiliary.len() > 100_000 {
                return Err(bad("selected dependency count"));
            }
            if stack.owner != ItemRef::Channel(ids.channel) && (selected_raw(ids, n.id.0) || n.mask.is_some_and(|r| selected_raw(ids, r.raw()))) {
                return Err(bad("selected filter dependency alias"));
            }
            for (_, r) in &n.auxiliary {
                tick(ctl)?;
                if selected_raw(ids, r.raw()) {
                    return Err(bad("selected auxiliary dependency"));
                }
            }
        }
    }
    let node = node.ok_or_else(|| bad("selected filter missing"))?;
    bounded_node(node)?;
    if node.id != ids.filter || node.mask != Some(ItemRef::Mask(ids.mask)) {
        return Err(bad("selected node/mask identities"));
    }
    let mut mask = None;
    for m in &state.filter_masks {
        tick(ctl)?;
        if m.id == ids.mask {
            if mask.replace(m).is_some() {
                return Err(bad("duplicate selected native mask"));
            }
        } else if selected_raw(ids, m.id.0) || m.owner == ids.filter {
            return Err(bad("selected native mask dependency alias"));
        }
    }
    let mask = mask.ok_or_else(|| bad("selected native mask missing"))?;
    if mask.owner != ids.filter
        || mask.bounds != Rect::new(0, 0, 96, 80)
        || mask_item.mask.as_ref().is_none_or(|m| m.owner != ItemRef::Channel(ids.channel) || m.bounds != mask.bounds)
    {
        return Err(bad("selected canonical mask association"));
    }
    for s in &state.source_items {
        tick(ctl)?;
        if selected_raw(ids, s.owner.raw()) {
            return Err(bad("selected editable source dependency"));
        }
    }
    for indexed in &state.editable_indexed {
        tick(ctl)?;
        if selected_raw(ids, indexed.owner.0) {
            return Err(bad("selected indexed owner alias"));
        }
    }
    for r in &state.resources {
        tick(ctl)?;
        if selected_raw(ids, r.id.0) {
            return Err(bad("selected resource identity alias"));
        }
    }
    for s in &state.item_sets {
        tick(ctl)?;
        if selected_raw(ids, s.id.0) {
            return Err(bad("selected set identity alias"));
        }
    }
    for r in &state.opaque {
        tick(ctl)?;
        if r.owner.is_some_and(|o| selected_raw(ids, o.raw()) && !selected_ref(ids, o)) {
            return Err(bad("selected opaque owner kind alias"));
        }
    }
    let requirements = SelectedRequirements {
        current_retained_bytes: current_retained,
        current_index_bytes: current_index,
        current_map_bytes: current_map,
        complete_resident_bytes: complete_resident,
        complete_index_bytes: complete_index,
        selection_work_units: selection_work,
        verification_work_units: verification_work,
        implicit_profile_resident_bytes: profile.resident_bytes,
        implicit_profile_work_units: profile.work_units,
    };
    tick(ctl)?;
    Ok(SelectedCurrentChannel {
        census,
        original_owner_document: owner.document(),
        channel,
        channel_item,
        mask,
        mask_item,
        node,
        ids,
        source_sha256: owner.source_sha256,
        source_blake3: owner.source_blake3,
        requirements,
        profile_policy: policy,
        profile_limits: if policy == CurrentProfilePolicy::MaterializedImplicit {
            photocraft_cms::control::Limits {
                resident_bytes: usize::try_from(profile.resident_bytes).map_err(|_| bad("selected implicit peak range"))?,
                work_units: profile.work_units,
            }
        } else {
            cms_limits(limits)?
        },
    })
}
impl AssembledStoredChannel<'_, '_, '_> {
    /// Independent original decode and exact selected current native planes only.
    /// charge may consume a caller prepaid receipt; it never resets a work meter.
    pub fn verify_selected_with_charge<'p>(
        &'p self,
        selected: &'p SelectedCurrentChannel<'_>,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
    ) -> Result<StoredChannelCorrespondence<'p>, IoError> {
        tick(ctl)?;
        if !std::ptr::eq(selected.original_owner_document, self.document())
            || selected.ids.channel != self.ids.channel
            || selected.ids.mask != self.ids.mask
            || selected.ids.filter != self.ids.filter
            || selected.source_sha256 != self.source_sha256
            || selected.source_blake3 != self.source_blake3
        {
            return Err(bad("selected descriptor belongs to another original"));
        }
        let mut remaining = selected.requirements.verification_work_units;
        while remaining != 0 {
            poll(self.shared, ctl)?;
            let n = remaining.min(4096);
            charge(n)?;
            remaining -= n;
        }
        let current = selected.document();
        let state = current.xcf_state.as_ref().ok_or_else(|| bad("selected state missing"))?;
        let baseline = self.document.xcf_state.as_ref().ok_or_else(|| bad("original mapped state missing"))?;
        if current.mode != self.document.mode
            || current.depth != self.document.depth
            || current.sample_encoding != self.document.sample_encoding
            || current.source_color_context != self.document.source_color_context
            || state.provenance.format != baseline.provenance.format
            || state.provenance.format_version != baseline.provenance.format_version
            || state.provenance.container != baseline.provenance.container
            || state.provenance.source_hash != baseline.provenance.source_hash
        {
            return Err(bad("selected original profile/provenance differs"));
        }
        match selected.profile_policy {
            CurrentProfilePolicy::OriginalExact => {
                if current.working_profile_interpretation != self.document.working_profile_interpretation || current.icc_profile != self.document.icc_profile {
                    return Err(bad("selected original working interpretation differs"));
                }
            }
            CurrentProfilePolicy::MaterializedImplicit => {
                let bytes = implicit_context(self, current)?;
                current
                    .working_profile_interpretation
                    .validate_with_interrupt(Some(bytes), current.mode, ctl)
                    .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
                // Temporary effective profile is dropped before issuing the proof.
                // Exact bytes, never same-color/profile-name aliases, bind authority.
                let effective = photocraft_cms::xcf_color::effective_implicit_profile_with_control(
                    bytes,
                    photocraft_cms::ColorSpace::Rgb,
                    photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    selected.profile_limits,
                    &|| ctl.cancelled(),
                )
                .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
                drop(effective);
            }
        }
        let original = self.original.channels.get(self.ordinal).ok_or_else(|| bad("original selected channel missing"))?;
        let effect = original.effects.first().ok_or_else(|| bad("original selected effect missing"))?;
        let original_mask = effect.mask.as_deref().ok_or_else(|| bad("original selected mask missing"))?;
        let expected_channel = self.document.channels.first().ok_or_else(|| bad("original mapped channel missing"))?;
        let expected_channel_item =
            baseline.items.iter().find(|i| i.item == ItemRef::Channel(self.ids.channel)).ok_or_else(|| bad("original channel item missing"))?;
        let expected_mask_item = baseline.items.iter().find(|i| i.item == ItemRef::Mask(self.ids.mask)).ok_or_else(|| bad("original mask item missing"))?;
        let expected_node = baseline.filter_stacks.first().and_then(|s| s.nodes.first()).ok_or_else(|| bad("original mapped node missing"))?;
        if selected.channel_item != expected_channel_item
            || selected.mask_item != expected_mask_item
            || selected.node != expected_node
            || selected.channel.name != expected_channel.name
            || selected.channel.spot != expected_channel.spot
            || selected.channel.color != expected_channel.color
            || selected.channel.opacity.to_bits() != expected_channel.opacity.to_bits()
            || selected.channel.indicates != expected_channel.indicates
        {
            return Err(bad("selected metadata/node differs from original"));
        }
        verify_plane(
            &selected.channel.surface,
            original.plane.as_ref().ok_or_else(|| bad("original channel plane missing"))?,
            bounds(original)?,
            self.shared,
            ctl,
        )?;
        verify_plane(
            &selected.mask.surface,
            original_mask.plane.as_ref().ok_or_else(|| bad("original mask plane missing"))?,
            bounds(original_mask)?,
            self.shared,
            ctl,
        )?;
        let related = |r: &&photocraft_doc::xcf_provenance::OpaqueRecord| r.owner.is_none() || r.owner.is_some_and(|o| selected_ref(self.ids, o));
        let mut originals = baseline.opaque.iter().filter(related);
        for a in &state.opaque {
            poll(self.shared, ctl)?;
            if !related(&a) {
                continue;
            }
            let b = originals.next().ok_or_else(|| bad("extra selected raw record"))?;
            if a.namespace.len() > 256
                || a.identity.len() > 256
                || a.blake3.len() != 64
                || a.owner != b.owner
                || a.namespace != b.namespace
                || a.identity != b.identity
                || a.source_version != b.source_version
                || a.order != b.order
                || a.flags != b.flags
                || a.blake3 != b.blake3
                || a.rendering_required != b.rendering_required
                || a.bytes.len() != b.bytes.len()
            {
                return Err(bad("selected raw record envelope differs"));
            }
            for (x, y) in a.bytes.chunks(4096).zip(b.bytes.chunks(4096)) {
                work(self.shared, ctl, x.len() as u64)?;
                if x != y {
                    return Err(bad("selected raw bytes differ"));
                }
            }
        }
        if originals.next().is_some() {
            return Err(bad("missing selected raw record"));
        }
        poll(self.shared, ctl)?;
        Ok(StoredChannelCorrespondence {
            document: current,
            bytes: self.source,
            node: selected.node,
            channel: self.ids.channel,
            mask: self.ids.mask,
            source_sha256: self.source_sha256,
            source_blake3: self.source_blake3,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_implicit_context_rejects_marker_model_encoding_and_source_aliases() {
        use photocraft_doc::WorkingProfileInterpretation;
        let mut original = Document::new("Original", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
        original.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        original.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 150 });
        let mut current = original.clone();
        let bytes = photocraft_cms::Builtin::Srgb.profile().to_bytes();
        current.icc_profile = Some(bytes.clone());
        current.working_profile_interpretation = WorkingProfileInterpretation::implicit_srgb(&bytes);
        assert_eq!(implicit_fields(&original, &current).unwrap(), bytes.as_slice());
        let good = current.clone();
        current.working_profile_interpretation = WorkingProfileInterpretation::Icc;
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.working_profile_interpretation = WorkingProfileInterpretation::implicit_sgray(&bytes);
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.icc_profile = None;
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.mode = ColorMode::Grayscale;
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.sample_encoding = Some(SampleEncoding::Linear);
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.source_color_context = Some(SourceColorContext { original_icc: Some(bytes.clone()), source_precision: 150 });
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: 151 });
        assert!(implicit_fields(&original, &current).is_err());
        current = good.clone();
        current.depth = SampleType::F64;
        assert!(implicit_fields(&original, &current).is_err());
        // Marker identity alone is not semantic verification: corrupt ICC still
        // requires the real CMS exact generated-definition check in proof.
        current = good;
        let mut corrupt = bytes.as_ref().clone();
        *corrupt.last_mut().unwrap() ^= 1;
        current.icc_profile = Some(Arc::new(corrupt));
        assert!(
            current
                .working_profile_interpretation
                .validate_with_interrupt(current.icc_profile.as_deref().map(Vec::as_slice), current.mode, &Interrupt::NONE)
                .is_err()
        );
    }
    #[test]
    fn native_layer_alias_and_cancel_refuse_without_changing_scene() {
        let ids = Ids { channel: ChannelId(41), filter: FilterId(42), mask: MaskId(43) };
        let mut layer = photocraft_doc::Layer::new("Unrelated", photocraft_doc::LayerContent::Raster(Surface::new(PixelFormat::RGBA8)));
        layer.id = photocraft_doc::LayerId(41);
        let layers_vec = vec![layer];
        let before = layers_vec.clone();
        assert!(layers(&layers_vec, ids, 0, &mut 0, &Interrupt::NONE).is_err());
        assert_eq!(layers_vec, before);
        assert!(matches!(layers(&layers_vec, ids, 0, &mut 0, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
    }
    #[test]
    fn selected_scalar_arguments_reject_nested_configs_before_equality() {
        use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity};
        let mut node = FilterNode {
            id: FilterId(42),
            operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "owned-source".into() },
            source_display_name: None,
            source_icon: None,
            arguments: (0..5).map(|_| FilterArgument { name: "scalar".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) }).collect(),
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(1),
            mask: Some(ItemRef::Mask(MaskId(43))),
            auxiliary: vec![],
            composition: None,
        };
        bounded_node(&node).unwrap();
        node.arguments[0].value = FilterValue::Config(vec![]);
        assert!(bounded_node(&node).is_err());
        node.arguments[0].value = FilterValue::Enum { domain: "x".repeat(129), value: 0 };
        assert!(bounded_node(&node).is_err());
    }
}
