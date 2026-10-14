//! Independent whole-decoder owner for a private v22 layer projection.
//! Exact original native planes/records only; never filter appearance, profile
//! semantics, selection evaluation, renderer admission or serialized grants.
use super::layer_selection_candidate::{self as mapper, LayerOuterPolicy, LayerSelectionIds, LayerSelectionLimits, LayerSelectionNamespace};
use super::*;
use crate::native_save::NativeCurrentCensus;
use photocraft_doc::{
    LayerId,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::FilterNode,
    xcf_state::ItemState,
};
use photocraft_xcf::{BudgetKind, DecodeReservations, Reservation, SourceAccounting, SourceContainer};
use sha2::{Digest, Sha256};

pub struct AssembledLayerSelection<'s, 'b, 'a> {
    original: XcfFile,
    source: &'s [u8],
    source_sha256: String,
    source_blake3: blake3::Hash,
    ordinal: usize,
    ids: LayerSelectionIds,
    source_surface: Surface,
    source_bounds: Rect,
    state: photocraft_doc::XcfState,
    shared: &'b photocraft_xcf::Budget<'a>,
    _decoder: DecodeReservations<'b, 'a>,
    _output: Reservation<'b, 'a>,
    _index: Reservation<'b, 'a>,
    _header: Reservation<'b, 'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct LayerCorrespondenceRequirements {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct LayerCurrentLimits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for LayerCurrentLimits {
    fn default() -> Self {
        let p = crate::NativeSaveLimits::default().preparation;
        Self {
            resident_bytes: p.resident_bytes as u64,
            index_bytes: p.index_bytes as u64,
            work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 },
        }
    }
}
/// Explicit current Gray profile policy. No authority is inferred from source absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerProfilePolicy {
    OriginalExact,
    MaterializedImplicitSGray,
}
#[derive(Clone, Copy, Debug)]
pub struct LayerProfileRequirements {
    pub resident_bytes: u64,
    pub work_units: u64,
}
fn profile_limits(limits: LayerCurrentLimits) -> Result<photocraft_cms::control::Limits, IoError> {
    let maximum = photocraft_cms::control::Limits::default();
    Ok(photocraft_cms::control::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes.min(maximum.resident_bytes as u64)).map_err(|_| bad("Gray CMS address space"))?,
        work_units: limits.work_units.min(maximum.work_units),
    })
}
fn materialized_gray<'d>(owner: &AssembledLayerSelection<'_, '_, '_>, doc: &'d Document) -> Result<&'d [u8], IoError> {
    if owner.original.model != Model::Gray || owner.original.encoding != Encoding::NonlinearProfile || !owner.original.icc.is_empty() {
        return Err(bad("original implicit Gray source mismatch"));
    }
    current_gray_metadata(doc, depth(owner.original.sample), owner.original.precision)
}
fn current_gray_metadata(doc: &Document, original_depth: SampleType, original_precision: u32) -> Result<&[u8], IoError> {
    if doc.mode != ColorMode::Grayscale
        || doc.depth != original_depth
        || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || doc.source_color_context.as_ref().is_none_or(|s| s.original_icc.is_some() || s.source_precision != original_precision)
        || !matches!(&doc.working_profile_interpretation,photocraft_doc::WorkingProfileInterpretation::ImplicitSGray{icc_blake3} if icc_blake3.len()==64&&icc_blake3.bytes().all(|v|v.is_ascii_hexdigit()))
    {
        return Err(bad("explicit current implicit Gray/source context mismatch"));
    }
    let bytes = doc.icc_profile.as_ref().ok_or_else(|| bad("explicit Gray marker requires current ICC"))?;
    if bytes.is_empty() || bytes.len() > 16 << 20 {
        return Err(bad("current Gray ICC bound"));
    }
    Ok(bytes.as_slice())
}
/// Borrowed prospective recipe only, not hash/equality proof. Use before full
/// census to prepay exact CMS validation and retain its temporary owner envelope.
pub fn layer_profile_requirements(
    owner: &AssembledLayerSelection<'_, '_, '_>,
    doc: &Document,
    policy: LayerProfilePolicy,
    limits: LayerCurrentLimits,
    ctl: &Interrupt<'_>,
) -> Result<LayerProfileRequirements, IoError> {
    tick(ctl)?;
    let maximum = LayerCurrentLimits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.index_bytes > maximum.index_bytes || limits.work_units > maximum.work_units {
        return Err(bad("Gray profile lower-only limits"));
    }
    if policy == LayerProfilePolicy::OriginalExact {
        return Ok(LayerProfileRequirements { resident_bytes: 0, work_units: 0 });
    }
    let bytes = materialized_gray(owner, doc)?;
    let r = photocraft_cms::xcf_color::effective_implicit_profile_requirements_with_control(
        bytes,
        photocraft_cms::ColorSpace::Gray,
        photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        profile_limits(limits)?,
        &|| ctl.cancelled(),
    )
    .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
    // Conservative complete schedule, not a count of three helper invocations:
    // Four possible recipe evaluations (caller, selection, verifier and builder)
    // fit the fixed Gray recipe's conservative validation floor; the multiplier
    // also holds clone/encoding checks, marker hash and exact byte comparison.
    let required =
        LayerProfileRequirements { resident_bytes: r.peak_resident_bytes as u64, work_units: add(mul(r.work_units, 3)?, add(bytes.len() as u64, 4096)?)? };
    if required.resident_bytes > limits.resident_bytes || required.work_units > limits.work_units {
        return Err(bad("Gray profile prospective envelope"));
    }
    tick(ctl)?;
    Ok(required)
}
#[derive(Clone, Copy, Debug)]
pub struct LayerCurrentRequirements {
    pub current_retained_bytes: u64,
    pub complete_resident_bytes: u64,
    pub complete_index_bytes: u64,
    pub selection_work_units: u64,
    pub verification_work_units: u64,
    pub implicit_profile_peak_bytes: u64,
    pub implicit_profile_work_units: u64,
}
pub struct SelectedCurrentLayer<'d> {
    census: NativeCurrentCensus<'d>,
    owner_surface: &'d Surface,
    ids: LayerSelectionIds,
    layer: &'d Layer,
    surface: &'d Surface,
    item: &'d ItemState,
    node: &'d FilterNode,
    mask: &'d photocraft_doc::FilterMask,
    mask_item: &'d ItemState,
    requirements: LayerCurrentRequirements,
    profile_policy: LayerProfilePolicy,
    profile_limits: photocraft_cms::control::Limits,
}
impl<'d> SelectedCurrentLayer<'d> {
    pub fn document(&self) -> &'d Document {
        self.census.document()
    }
    pub fn layer_id(&self) -> LayerId {
        self.ids.layer
    }
    pub fn filter_id(&self) -> FilterId {
        self.ids.filter
    }
    pub fn mask_id(&self) -> MaskId {
        self.ids.mask
    }
    pub fn layer(&self) -> &'d Layer {
        self.layer
    }
    pub fn source_surface(&self) -> &'d Surface {
        self.surface
    }
    pub fn source_bounds(&self) -> Rect {
        Rect::new(0, 0, 32, 32)
    }
    pub fn layer_item(&self) -> &'d ItemState {
        self.item
    }
    pub fn original_node(&self) -> &'d FilterNode {
        self.node
    }
    pub fn filter_mask(&self) -> &'d photocraft_doc::FilterMask {
        self.mask
    }
    pub fn mask_item(&self) -> &'d ItemState {
        self.mask_item
    }
    pub fn requirements(&self) -> LayerCurrentRequirements {
        self.requirements
    }
}
/// Construction is private and follows independent whole original decode and
/// exact selected native comparison. No output or working-space credential.
pub struct LayerOriginalPlanesProof<'p> {
    document: &'p Document,
    source: &'p [u8],
    source_sha256: &'p str,
    source_blake3: blake3::Hash,
    node: &'p FilterNode,
    ids: LayerSelectionIds,
}
impl LayerOriginalPlanesProof<'_> {
    pub fn document(&self) -> &Document {
        self.document
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.source
    }
    pub fn source_sha256(&self) -> &str {
        self.source_sha256
    }
    pub fn source_blake3(&self) -> blake3::Hash {
        self.source_blake3
    }
    pub fn original_node(&self) -> &FilterNode {
        self.node
    }
    pub fn layer_id(&self) -> LayerId {
        self.ids.layer
    }
    pub fn filter_id(&self) -> FilterId {
        self.ids.filter
    }
    pub fn mask_id(&self) -> MaskId {
        self.ids.mask
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("XCF private layer owner: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("accounting overflow"))
}
fn tick(ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}
fn poll(shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    tick(ctl)?;
    shared.poll("layer original owner").map_err(accounting_error)
}
fn work(shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>, n: u64) -> Result<(), IoError> {
    poll(shared, ctl)?;
    shared.consume_work(n, "layer original owner").map_err(accounting_error)
}
impl AssembledLayerSelection<'_, '_, '_> {
    /// Immutable independently decoded backing, not a rendering or read grant.
    pub fn original_file(&self) -> &XcfFile {
        &self.original
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.source
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn source_blake3(&self) -> blake3::Hash {
        self.source_blake3
    }
    pub fn layer_id(&self) -> LayerId {
        self.ids.layer
    }
    pub fn filter_id(&self) -> FilterId {
        self.ids.filter
    }
    pub fn mask_id(&self) -> MaskId {
        self.ids.mask
    }
    pub fn source_surface(&self) -> &Surface {
        &self.source_surface
    }
    pub fn source_bounds(&self) -> Rect {
        self.source_bounds
    }
    pub fn state(&self) -> &photocraft_doc::XcfState {
        &self.state
    }
    pub fn original_node(&self) -> Result<&FilterNode, IoError> {
        self.state.filter_stacks.first().and_then(|s| s.nodes.first()).ok_or_else(|| bad("private node missing"))
    }
    pub fn original_layer(&self) -> Result<&Drawable, IoError> {
        self.original.layers.get(self.ordinal).ok_or_else(|| bad("private original layer missing"))
    }
    pub fn correspondence_requirements(&self) -> Result<LayerCorrespondenceRequirements, IoError> {
        let resident =
            add(add(add(add(self._decoder.retained_bytes(), self.source.len() as u64)?, self._output.bytes())?, self._index.bytes())?, self._header.bytes())?;
        let units = add(mul(resident, 8)?, mul(mul(photocraft_geom::TILE_SIZE as u64, photocraft_geom::TILE_SIZE as u64)?, 128)?)?;
        Ok(LayerCorrespondenceRequirements { resident_bytes: resident, index_bytes: self._index.bytes(), work_units: units })
    }
}
fn copy_global(
    properties: &[photocraft_xcf::RawProperty],
    state: &mut photocraft_doc::XcfState,
    maximum: usize,
    shared: &photocraft_xcf::Budget<'_>,
    ctl: &Interrupt<'_>,
) -> Result<usize, IoError> {
    if properties.len() > 32 {
        return Err(bad("global original property count"));
    }
    let mut b = Budget { used: 0, max: maximum, peak: 0, shared: None };
    let mut copied = Vec::new();
    b.charge(properties.len().checked_mul(std::mem::size_of::<photocraft_xcf::RawProperty>()).ok_or_else(|| bad("global property index"))?)?;
    copied.try_reserve_exact(properties.len()).map_err(|_| bad("global property allocation"))?;
    b.charge(
        copied
            .capacity()
            .saturating_sub(properties.len())
            .checked_mul(std::mem::size_of::<photocraft_xcf::RawProperty>())
            .ok_or_else(|| bad("global property capacity"))?,
    )?;
    for p in properties {
        poll(shared, ctl)?;
        if p.payload.len() > 64 << 10 || usize::try_from(p.declared_bytes).ok() != Some(p.payload.len()) {
            return Err(bad("global raw envelope"));
        }
        let mut payload = Vec::new();
        b.charge(p.payload.len())?;
        payload.try_reserve_exact(p.payload.len()).map_err(|_| bad("global payload allocation"))?;
        b.charge(payload.capacity().saturating_sub(p.payload.len()))?;
        for c in p.payload.chunks(4096) {
            work(shared, ctl, mul(c.len() as u64, 2)?)?;
            payload.extend_from_slice(c);
        }
        copied.push(photocraft_xcf::RawProperty { tag: p.tag, offset: p.offset, declared_bytes: p.declared_bytes, payload });
    }
    b.charge(properties.len().checked_mul(std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>()).ok_or_else(|| bad("global opaque index"))?)?;
    preserve_properties(state, copied, None, 22, &mut b)?;
    b.charge(
        state
            .opaque
            .capacity()
            .saturating_sub(properties.len())
            .checked_mul(std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>())
            .ok_or_else(|| bad("actual global opaque capacity"))?,
    )?;
    poll(shared, ctl)?;
    Ok(b.peak)
}
/// Takes one independent full eager decode under the held source/container
/// budget. SourceAccounting retains its existing caller/container contract.
/// This returns a PRIVATE projection; source bytes are never format credentials.
#[allow(clippy::too_many_arguments)]
pub fn assemble_owned_layer_selection<'s, 'b, 'a>(
    bytes: &'s [u8],
    ordinal: usize,
    ids: LayerSelectionIds,
    limits: LayerSelectionLimits,
    options: &DecodeOptions<'_>,
    shared: &'b photocraft_xcf::Budget<'a>,
    accounting: SourceAccounting,
    ctl: &Interrupt<'_>,
) -> Result<AssembledLayerSelection<'s, 'b, 'a>, IoError> {
    assemble_owned_layer_selection_with_outer_policy(bytes, ordinal, ids, LayerOuterPolicy::ObservedLiteral, limits, options, shared, accounting, ctl)
}
/// Explicit unresolved outer-word retention; source interpretation remains closed.
#[allow(clippy::too_many_arguments)]
pub fn assemble_owned_layer_selection_with_outer_policy<'s, 'b, 'a>(
    bytes: &'s [u8],
    ordinal: usize,
    ids: LayerSelectionIds,
    outer_policy: LayerOuterPolicy,
    limits: LayerSelectionLimits,
    options: &DecodeOptions<'_>,
    shared: &'b photocraft_xcf::Budget<'a>,
    accounting: SourceAccounting,
    ctl: &Interrupt<'_>,
) -> Result<AssembledLayerSelection<'s, 'b, 'a>, IoError> {
    poll(shared, ctl)?;
    if bytes.is_empty() || bytes.len() > 16_384 || !photocraft_xcf::is_xcf(bytes) {
        return Err(bad("plain bounded layer source"));
    }
    let standard = LayerSelectionLimits::default();
    if limits.resident_bytes > standard.resident_bytes
        || limits.metadata_bytes > standard.metadata_bytes
        || limits.index_bytes > standard.index_bytes
        || limits.work_units > standard.work_units
        || limits.namespace_items > standard.namespace_items
    {
        return Err(bad("lower-only projection limits"));
    }
    if ids.layer.0 == 0 || ids.filter.0 == 0 || ids.mask.0 == 0 || ids.layer.0 == ids.filter.0 || ids.layer.0 == ids.mask.0 || ids.filter.0 == ids.mask.0 {
        return Err(bad("zero/aliased requested layer identities"));
    }
    let header = shared
        .reserve(BudgetKind::Resident, add(std::mem::size_of::<AssembledLayerSelection<'s, 'b, 'a>>() as u64, 128)?, "layer owner/header digest")
        .map_err(accounting_error)?;
    let mut sha = Sha256::new();
    let mut b3 = blake3::Hasher::new();
    for c in bytes.chunks(4096) {
        work(shared, ctl, c.len() as u64)?;
        sha.update(c);
        b3.update(c);
    }
    let mut source_sha256 = String::new();
    source_sha256.try_reserve_exact(64).map_err(|_| bad("source digest allocation"))?;
    if source_sha256.capacity() > 128 {
        return Err(bad("source digest capacity"));
    }
    for x in sha.finalize() {
        let alphabet = b"0123456789abcdef";
        source_sha256.push(char::from(*alphabet.get(usize::from(x >> 4)).ok_or_else(|| bad("digest nibble"))?));
        source_sha256.push(char::from(*alphabet.get(usize::from(x & 15)).ok_or_else(|| bad("digest nibble"))?));
    }
    let source_blake3 = b3.finalize();
    let cancelled = || ctl.cancelled();
    let effective = DecodeOptions {
        limits: options.limits.clone(),
        cancelled: Some(&cancelled),
        progress: options.progress,
        allow_finite_drawable_alpha: options.allow_finite_drawable_alpha,
    };
    let decoded =
        photocraft_xcf::decode_candidate_with_budget(bytes, &effective, shared, accounting).map_err(accounting_error)?.ok_or_else(|| bad("not a plain XCF"))?;
    if decoded.decoded.container != SourceContainer::Plain {
        return Err(bad("wrapped source not admitted"));
    }
    let original = decoded.decoded.file;
    if original.version != 22
        || original.model != Model::Gray
        || original.encoding != Encoding::NonlinearProfile
        || !matches!(original.sample, Sample::U8 | Sample::F64)
        || !original.icc.is_empty()
        || original.width != 32
        || original.height != 32
        || original.layers.len() != 1
        || original.channels.len() > 1
        || !original.paths.is_empty()
        || ordinal != 0
    {
        return Err(bad("closed private original scene descriptor"));
    }
    let decoded_layer = original.layers.get(ordinal).ok_or_else(|| bad("selected original layer missing"))?;
    let decoded_mask = decoded_layer.effects.first().and_then(|e| e.mask.as_deref()).ok_or_else(|| bad("selected original mask missing"))?;
    if decoded_layer.offset != (0, 0) || decoded_mask.offset != (0, 0) {
        return Err(bad("initial raw correspondence requires observed origin-zero planes"));
    }
    // Derive this tiny namespace from actual owned canonical IDs, never a
    // caller-asserted retained namespace byte total. No extra borrowed namespace.
    let output = shared.reserve(BudgetKind::Resident, limits.resident_bytes, "layer projection output").map_err(accounting_error)?;
    let index = shared.reserve(BudgetKind::Index, limits.index_bytes, "layer projection index").map_err(accounting_error)?;
    let global_upper = original.raw_properties.iter().try_fold(4096usize, |n, p| {
        poll(shared, ctl)?;
        if p.payload.len() > 64 << 10 || original.raw_properties.len() > 32 {
            return Err(bad("global original metadata bound"));
        }
        n.checked_add(p.payload.capacity()).and_then(|x| x.checked_add(1024)).ok_or_else(|| bad("global original metadata overflow"))
    })?;
    if global_upper as u64 >= limits.resident_bytes || global_upper as u64 >= limits.metadata_bytes {
        return Err(bad("global original quota"));
    }
    let mapping = LayerSelectionLimits {
        resident_bytes: limits.resident_bytes - global_upper as u64,
        metadata_bytes: limits.metadata_bytes - global_upper as u64,
        ..limits
    };
    let candidate = mapper::map_layer_selection_with_outer_policy(
        &original,
        ordinal,
        ids,
        outer_policy,
        LayerSelectionNamespace { layers: &[ids.layer], filters: &[], masks: &[] },
        mapping,
        ctl,
        &mut |resident, index_bytes| {
            if resident > output.bytes() || index_bytes > index.bytes() { Err(bad("projection reservation mismatch")) } else { Ok(()) }
        },
        &mut |units| work(shared, ctl, units),
    )?;
    let known = candidate.requirements.known_resident_bytes;
    let source_surface = candidate.source_surface;
    let source_bounds = candidate.source_bounds;
    let mut mapped_state = candidate.state;
    // Borrowed mapper references end before moving original into this owner.
    let mut state = photocraft_doc::XcfState::default();
    let global = copy_global(&original.raw_properties, &mut state, global_upper, shared, ctl)?;
    // Keep both old vectors charged while the merge may allocate a new body.
    // Admit the full prospective final body BEFORE reserve; then reconcile its
    // actual capacity without relying on allocator growth or refunding owners.
    let prospective = admit_merge_peak(known, global as u64, state.opaque.len(), state.opaque.capacity(), mapped_state.opaque.len(), limits.resident_bytes)?;
    poll(shared, ctl)?;
    state.opaque.try_reserve_exact(mapped_state.opaque.len()).map_err(|_| bad("projection opaque merge allocation"))?;
    let extra = state
        .opaque
        .capacity()
        .checked_mul(std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>())
        .ok_or_else(|| bad("projection opaque merge capacity"))?;
    let excess = (extra as u64).saturating_sub(prospective);
    if add(add(known, global as u64)?, add(prospective, excess)?)? > limits.resident_bytes {
        return Err(bad("actual projection combined retention"));
    }
    state.opaque.append(&mut mapped_state.opaque);
    mapped_state.opaque = state.opaque;
    state = mapped_state;
    poll(shared, ctl)?;
    Ok(AssembledLayerSelection {
        original,
        source: bytes,
        source_sha256,
        source_blake3,
        ordinal,
        ids,
        source_surface,
        source_bounds,
        state,
        shared,
        _decoder: decoded.reservations,
        _output: output,
        _index: index,
        _header: header,
    })
}
fn admit_merge_peak(known: u64, global: u64, len: usize, capacity: usize, additional: usize, limit: u64) -> Result<u64, IoError> {
    let slots = len.checked_add(additional).ok_or_else(|| bad("merged opaque slot overflow"))?.max(capacity);
    let bytes = slots.checked_mul(std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>()).ok_or_else(|| bad("merged opaque body overflow"))?;
    let bytes = u64::try_from(bytes).map_err(|_| bad("merged opaque byte range"))?;
    if add(add(known, global)?, bytes)? > limit {
        return Err(bad("prospective projection combined retention"));
    }
    Ok(bytes)
}
fn raw_id(ids: LayerSelectionIds, n: u64) -> bool {
    n == ids.layer.0 || n == ids.filter.0 || n == ids.mask.0
}
fn reference(ids: LayerSelectionIds, r: ItemRef) -> bool {
    r == ItemRef::Layer(ids.layer) || r == ItemRef::Mask(ids.mask)
}
fn visit_layers<'d>(
    values: &'d [Layer],
    ids: LayerSelectionIds,
    depth: usize,
    count: &mut usize,
    found: &mut Option<&'d Layer>,
    ctl: &Interrupt<'_>,
) -> Result<(), IoError> {
    if depth > 256 {
        return Err(bad("current hierarchy depth"));
    }
    for l in values {
        tick(ctl)?;
        *count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| bad("current hierarchy count"))?;
        if l.id == ids.layer {
            if found.replace(l).is_some() {
                return Err(bad("duplicate current native layer"));
            }
        } else if raw_id(ids, l.id.0) {
            return Err(bad("current native layer ID alias"));
        }
        if let LayerContent::Group(g) = &l.content {
            visit_layers(&g.children, ids, depth + 1, count, found, ctl)?;
        }
    }
    Ok(())
}
/// Current retention comes ONLY from an authentic immutable census. The real
/// callbacks must retain the complete lease and prepay traversal; no refunds.
#[allow(clippy::too_many_arguments)]
pub fn select_layer_current<'d>(
    census: NativeCurrentCensus<'d>,
    owner: &'d AssembledLayerSelection<'_, '_, '_>,
    limits: LayerCurrentLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<SelectedCurrentLayer<'d>, IoError> {
    select_layer_current_with_profile_policy(census, owner, LayerProfilePolicy::OriginalExact, limits, ctl, reserve, charge)
}
/// Explicit selected current-profile opt-in, independent of opaque tuple policy.
#[allow(clippy::too_many_arguments)]
pub fn select_layer_current_with_profile_policy<'d>(
    census: NativeCurrentCensus<'d>,
    owner: &'d AssembledLayerSelection<'_, '_, '_>,
    policy: LayerProfilePolicy,
    limits: LayerCurrentLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<SelectedCurrentLayer<'d>, IoError> {
    tick(ctl)?;
    let maximum = LayerCurrentLimits::default();
    if limits.resident_bytes > maximum.resident_bytes || limits.index_bytes > maximum.index_bytes || limits.work_units > maximum.work_units {
        return Err(bad("current lower-only limits"));
    }
    let profile = layer_profile_requirements(owner, census.document(), policy, limits, ctl)?;
    let current = census.retained_bytes() as u64;
    let source = owner.correspondence_requirements()?;
    let scratch = add(256 * 256, std::mem::size_of::<SelectedCurrentLayer<'d>>() as u64)?;
    let index = add(add(census.known_index_reservation_bytes() as u64, source.index_bytes)?, scratch)?;
    let resident = add(add(add(add(current, census.known_map_reservation_bytes() as u64)?, source.resident_bytes)?, index)?, profile.resident_bytes)?;
    let selection_work = add(mul(census.model_retained_bytes() as u64, 16)?, 65536)?;
    let verification_work = add(add(source.work_units, mul(current, 8)?)?, profile.work_units)?;
    if resident > limits.resident_bytes || index > limits.index_bytes || add(selection_work, verification_work)? > limits.work_units {
        return Err(bad("complete layer correspondence envelope"));
    }
    reserve(resident, index)?;
    charge(selection_work)?;
    let doc = census.document();
    let ids = owner.ids;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("current source state missing"))?;
    if state.items.len() > 100_000
        || state.filter_stacks.len() > 10_000
        || state.filter_masks.len() > 10_000
        || state.opaque.len() > 100_000
        || state.source_items.len() > 10_000
        || state.resources.len() > 10_000
        || state.item_sets.len() > 10_000
        || state.editable_indexed.len() > 10_000
        || doc.paths.len() > 10_000
        || doc.channels.len() > 10_000
    {
        return Err(bad("current namespace count"));
    }
    let mut found = None;
    visit_layers(&doc.layers, ids, 0, &mut 0, &mut found, ctl)?;
    let layer = found.ok_or_else(|| bad("current selected layer missing"))?;
    let LayerContent::Raster(surface) = &layer.content else { return Err(bad("current selected native raster required")) };
    let mut item = None;
    let mut mask_item = None;
    for i in &state.items {
        tick(ctl)?;
        if i.item == ItemRef::Layer(ids.layer) {
            if item.replace(i).is_some() {
                return Err(bad("duplicate current layer item"));
            }
        } else if i.item == ItemRef::Mask(ids.mask) {
            if mask_item.replace(i).is_some() {
                return Err(bad("duplicate current mask item"));
            }
        } else if raw_id(ids, i.item.raw()) || i.parent.is_some_and(|r| reference(ids, r)) || i.mask.as_ref().is_some_and(|m| reference(ids, m.owner)) {
            return Err(bad("current selected item alias/dependency"));
        }
    }
    let item = item.ok_or_else(|| bad("current layer item missing"))?;
    let mask_item = mask_item.ok_or_else(|| bad("current mask item missing"))?;
    let mut node = None;
    let mut count = 0usize;
    for stack in &state.filter_stacks {
        tick(ctl)?;
        count = count.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("current filter count"))?;
        if stack.owner == ItemRef::Layer(ids.layer) {
            if stack.nodes.len() != 1 || node.replace(stack.nodes.first().ok_or_else(|| bad("current node missing"))?).is_some() {
                return Err(bad("current selected stack ambiguity"));
            }
        } else if raw_id(ids, stack.owner.raw()) {
            return Err(bad("current selected stack alias"));
        }
        for n in &stack.nodes {
            tick(ctl)?;
            if n.auxiliary.len() > 100_000 {
                return Err(bad("current dependency count"));
            }
            if stack.owner != ItemRef::Layer(ids.layer) && (raw_id(ids, n.id.0) || n.mask.is_some_and(|r| raw_id(ids, r.raw()))) {
                return Err(bad("current filter alias"));
            }
            for (_, r) in &n.auxiliary {
                tick(ctl)?;
                if raw_id(ids, r.raw()) {
                    return Err(bad("current auxiliary dependency"));
                }
            }
        }
    }
    let node = node.ok_or_else(|| bad("current selected node missing"))?;
    if node.id != ids.filter || node.mask != Some(ItemRef::Mask(ids.mask)) || node.arguments.len() != 5 {
        return Err(bad("current selected node identities"));
    }
    let mut mask = None;
    for m in &state.filter_masks {
        tick(ctl)?;
        if m.id == ids.mask {
            if mask.replace(m).is_some() {
                return Err(bad("duplicate native current mask"));
            }
        } else if raw_id(ids, m.id.0) || m.owner == ids.filter {
            return Err(bad("current native mask alias"));
        }
    }
    let mask = mask.ok_or_else(|| bad("current native mask missing"))?;
    if mask.owner != ids.filter || mask.bounds != Rect::new(0, 0, 32, 32) {
        return Err(bad("current mask association"));
    }
    for c in &doc.channels {
        tick(ctl)?;
        if raw_id(ids, c.id.0) {
            return Err(bad("current channel alias"));
        }
    }
    for p in &doc.paths {
        tick(ctl)?;
        if raw_id(ids, p.id.0) {
            return Err(bad("current path alias"));
        }
    }
    for s in &state.source_items {
        tick(ctl)?;
        if raw_id(ids, s.owner.raw()) {
            return Err(bad("current editable source dependency"));
        }
    }
    for r in &state.resources {
        tick(ctl)?;
        if raw_id(ids, r.id.0) {
            return Err(bad("current resource alias"));
        }
    }
    for e in &state.editable_indexed {
        tick(ctl)?;
        if raw_id(ids, e.owner.0) {
            return Err(bad("current indexed owner alias"));
        }
    }
    for set in &state.item_sets {
        tick(ctl)?;
        if raw_id(ids, set.id.0) {
            return Err(bad("current set identity alias"));
        }
    }
    for record in &state.opaque {
        tick(ctl)?;
        if record.owner.is_some_and(|o| raw_id(ids, o.raw()) && !reference(ids, o)) {
            return Err(bad("current raw owner kind alias"));
        }
    }
    if doc.quick_mask.as_ref().is_some_and(|c| raw_id(ids, c.id.0))
        || doc.selection_channel_id.is_some_and(|id| raw_id(ids, id.0))
        || state.floating.as_ref().is_some_and(|f| raw_id(ids, f.layer.0) || raw_id(ids, f.target.raw()))
    {
        return Err(bad("current temporary identity alias"));
    }
    tick(ctl)?;
    Ok(SelectedCurrentLayer {
        census,
        owner_surface: &owner.source_surface,
        ids,
        layer,
        surface,
        item,
        node,
        mask,
        mask_item,
        requirements: LayerCurrentRequirements {
            current_retained_bytes: current,
            complete_resident_bytes: resident,
            complete_index_bytes: index,
            selection_work_units: selection_work,
            verification_work_units: verification_work,
            implicit_profile_peak_bytes: profile.resident_bytes,
            implicit_profile_work_units: profile.work_units,
        },
        profile_policy: policy,
        profile_limits: if policy == LayerProfilePolicy::MaterializedImplicitSGray {
            photocraft_cms::control::Limits {
                resident_bytes: usize::try_from(profile.resident_bytes).map_err(|_| bad("Gray profile peak range"))?,
                work_units: profile.work_units,
            }
        } else {
            profile_limits(limits)?
        },
    })
}
impl AssembledLayerSelection<'_, '_, '_> {
    pub fn verify_selected_layer_with_charge<'p>(
        &'p self,
        selected: &'p SelectedCurrentLayer<'_>,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
    ) -> Result<LayerOriginalPlanesProof<'p>, IoError> {
        poll(self.shared, ctl)?;
        if !std::ptr::eq(selected.owner_surface, &self.source_surface)
            || selected.ids.layer != self.ids.layer
            || selected.ids.filter != self.ids.filter
            || selected.ids.mask != self.ids.mask
        {
            return Err(bad("selected layer descriptor belongs to another source"));
        }
        let mut units = selected.requirements.verification_work_units;
        while units != 0 {
            poll(self.shared, ctl)?;
            let n = units.min(4096);
            charge(n)?;
            units -= n;
        }
        let doc = selected.document();
        let state = doc.xcf_state.as_ref().ok_or_else(|| bad("verified source state missing"))?;
        if doc.mode != ColorMode::Grayscale
            || doc.depth != depth(self.original.sample)
            || doc.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            || doc.source_color_context.as_ref().is_none_or(|s| s.original_icc.is_some() || s.source_precision != self.original.precision)
            || state.provenance.format != "xcf"
            || state.provenance.format_version != 22
            || state.provenance.container != "plain"
            || state.provenance.source_hash.as_deref() != Some(self.source_blake3.to_hex().as_str())
        {
            return Err(bad("original exact Gray profile/source context differs"));
        }
        match selected.profile_policy {
            LayerProfilePolicy::OriginalExact => {
                if doc.icc_profile.is_some() || doc.working_profile_interpretation != photocraft_doc::WorkingProfileInterpretation::Icc {
                    return Err(bad("original exact current Gray interpretation differs"));
                }
            }
            LayerProfilePolicy::MaterializedImplicitSGray => {
                let bytes = materialized_gray(self, doc)?;
                doc.working_profile_interpretation
                    .validate_with_interrupt(Some(bytes), doc.mode, ctl)
                    .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
                let effective = photocraft_cms::xcf_color::effective_implicit_profile_with_control(
                    bytes,
                    photocraft_cms::ColorSpace::Gray,
                    photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                    selected.profile_limits,
                    &|| ctl.cancelled(),
                )
                .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
                drop(effective);
            }
        }
        let expected_item = self.state.items.iter().find(|i| i.item == ItemRef::Layer(self.ids.layer)).ok_or_else(|| bad("original layer item missing"))?;
        let expected_mask = self.state.items.iter().find(|i| i.item == ItemRef::Mask(self.ids.mask)).ok_or_else(|| bad("original mask item missing"))?;
        if selected.item != expected_item
            || selected.mask_item != expected_mask
            || selected.node != self.original_node()?
            || selected.layer.name != expected_item.name
        {
            return Err(bad("selected current source node/item metadata differs"));
        }
        let original = self.original_layer()?;
        let mask = original.effects.first().and_then(|e| e.mask.as_deref()).ok_or_else(|| bad("original mask missing"))?;
        // Existing comparator: exact native bytes, zero default and complete
        // padded single tiles. This initial lane explicitly keeps origin zero.
        super::stored_channel_candidate::verify_plane(
            selected.surface,
            original.plane.as_ref().ok_or_else(|| bad("original layer plane"))?,
            bounds(original)?,
            self.shared,
            ctl,
        )?;
        super::stored_channel_candidate::verify_plane(
            &selected.mask.surface,
            mask.plane.as_ref().ok_or_else(|| bad("original mask plane"))?,
            bounds(mask)?,
            self.shared,
            ctl,
        )?;
        let related = |r: &&photocraft_doc::xcf_provenance::OpaqueRecord| {
            r.owner.is_none() || r.owner == Some(ItemRef::Layer(self.ids.layer)) || r.owner == Some(ItemRef::Mask(self.ids.mask))
        };
        let mut expected = self.state.opaque.iter().filter(related);
        for a in &state.opaque {
            poll(self.shared, ctl)?;
            if !related(&a) {
                continue;
            }
            let b = expected.next().ok_or_else(|| bad("extra related source record"))?;
            if a.namespace.len() > 256
                || a.identity.len() > 256
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
                return Err(bad("related source record envelope differs"));
            }
            for (x, y) in a.bytes.chunks(4096).zip(b.bytes.chunks(4096)) {
                work(self.shared, ctl, x.len() as u64)?;
                if x != y {
                    return Err(bad("related raw source bytes differ"));
                }
            }
        }
        if expected.next().is_some() {
            return Err(bad("related source record missing"));
        }
        poll(self.shared, ctl)?;
        Ok(LayerOriginalPlanesProof {
            document: doc,
            source: self.source,
            source_sha256: &self.source_sha256,
            source_blake3: self.source_blake3,
            node: selected.node,
            ids: self.ids,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_gray_metadata_never_infers_authority_from_absent_source_icc() {
        let mut doc = Document::new("Metadata witness", Size::new(32, 32), ColorMode::Grayscale, SampleType::F64);
        let bytes = b"metadata witness only, not a valid implicit profile".to_vec();
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.source_color_context = Some(photocraft_doc::SourceColorContext { original_icc: None, source_precision: 750 });
        doc.icc_profile = Some(bytes.clone());
        assert!(current_gray_metadata(&doc, SampleType::F64, 750).is_err());
        doc.working_profile_interpretation = photocraft_doc::WorkingProfileInterpretation::implicit_sgray(&bytes);
        assert_eq!(current_gray_metadata(&doc, SampleType::F64, 750).unwrap(), bytes);
        assert!(
            photocraft_cms::xcf_color::effective_implicit_profile_with_control(
                &bytes,
                photocraft_cms::ColorSpace::Gray,
                photocraft_cms::xcf_color::Encoding::NonlinearProfile,
                photocraft_cms::control::Limits::default(),
                &|| false,
            )
            .is_err()
        );
        let stop = || true;
        assert!(matches!(tick(&Interrupt::cancel_only(&stop)), Err(IoError::Cancelled)));
        assert!(doc.working_profile_interpretation.validate_with_interrupt(Some(b"changed"), doc.mode, &Interrupt::NONE).is_err());
        assert!(current_gray_metadata(&doc, SampleType::F64, 751).is_err());
        assert!(current_gray_metadata(&doc, SampleType::U8, 750).is_err());
        doc.sample_encoding = Some(SampleEncoding::Linear);
        assert!(current_gray_metadata(&doc, SampleType::F64, 750).is_err());
        doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        doc.mode = ColorMode::Rgb;
        assert!(current_gray_metadata(&doc, SampleType::F64, 750).is_err());
        doc.mode = ColorMode::Grayscale;
        doc.icc_profile = None;
        assert!(current_gray_metadata(&doc, SampleType::F64, 750).is_err());
        doc.icc_profile = Some(bytes);
        doc.source_color_context.as_mut().unwrap().original_icc = Some(std::sync::Arc::new(vec![1]));
        assert!(current_gray_metadata(&doc, SampleType::F64, 750).is_err());
    }
    #[test]
    fn opaque_merge_admits_simultaneous_bodies_before_reserve_and_checks_overflow() {
        let bytes = 3 * std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>() as u64;
        let limit = 100 + 200 + bytes;
        assert!(admit_merge_peak(100, 200, 1, 1, 2, limit - 1).is_err());
        assert_eq!(admit_merge_peak(100, 200, 1, 1, 2, limit).unwrap(), bytes);
        assert!(admit_merge_peak(0, 0, usize::MAX, 0, 1, u64::MAX).is_err());
        assert!(admit_merge_peak(u64::MAX, 1, 0, 0, 0, u64::MAX).is_err());
    }
    #[test]
    fn alias_namespace_and_overflow_refuse_before_current_traversal() {
        assert!(add(u64::MAX, 1).is_err());
        assert!(mul(u64::MAX, 8).is_err());
        let ids = LayerSelectionIds { layer: LayerId(1), filter: FilterId(2), mask: MaskId(3) };
        let mut layer = Layer::new("Alias", LayerContent::Raster(Surface::new(PixelFormat::GRAY8)));
        layer.id = LayerId(2);
        assert!(visit_layers(&[layer], ids, 0, &mut 0, &mut None, &Interrupt::NONE).is_err());
    }
    #[test]
    fn cancelled_current_scan_does_not_select_or_modify_native_source() {
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        let layer = Layer::new("Source", LayerContent::Raster(Surface::new(PixelFormat::GRAY8)));
        let before = layer.clone();
        let mut found = None;
        assert!(matches!(
            visit_layers(
                std::slice::from_ref(&layer),
                LayerSelectionIds { layer: layer.id, filter: FilterId(2), mask: MaskId(3) },
                0,
                &mut 0,
                &mut found,
                &ctl
            ),
            Err(IoError::Cancelled)
        ));
        assert!(found.is_none());
        assert_eq!(layer, before);
    }
}
