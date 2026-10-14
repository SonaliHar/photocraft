//! Opt-in private stored-channel assembly and raw original-plane correspondence.
//! Never called by normal import, never an appearance/source evaluator qualifier.
//! This selective Document contains the selected channel/filter mask only; the
//! complete decoded original scene remains retained in the proof owner.
#[path = "stored_channel_selected.rs"]
mod selected;
pub use selected::{CurrentProfilePolicy, SelectedCurrentChannel, SelectedLimits, SelectedProfileRequirements, SelectedRequirements, select_current, select_current_with_profile_policy, selected_profile_requirements};
use super::*;
use crate::xcf_channel_candidate::{self as mapper, Ids, Namespace};
use photocraft_doc::{
    item_identity::{ChannelId, FilterId, ItemRef, MaskId},
    xcf_filter_masks::FilterMask,
    xcf_filters::FilterNode,
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_xcf::{BudgetKind, DecodeReservations, Reservation, SourceAccounting, SourceContainer};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
/// Complete caller-held namespace backing; a slice length is only a local floor.
/// No default or inferred ownership. This reservation is an addition to decoder
/// backing and mapper selected metadata, never a refund of another owner.
pub struct NamespaceBacking<'a> {
    pub namespace: Namespace<'a>,
    pub retained_bytes: u64,
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("XCF stored-channel assembly: {message}"))
}
fn poll(shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    shared.poll("stored-channel candidate").map_err(accounting_error)
}
fn work(shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>, n: u64) -> Result<(), IoError> {
    poll(shared, ctl)?;
    shared.consume_work(n, "stored-channel candidate").map_err(accounting_error)
}
fn add(a: usize, b: usize) -> Result<usize, IoError> {
    a.checked_add(b).ok_or_else(|| bad("capacity overflow"))
}
fn mul(a: usize, b: usize) -> Result<usize, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("capacity overflow"))
}
fn text(value: &str, b: &mut Budget<'_>, shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>) -> Result<String, IoError> {
    if value.len() > 256 || value.contains('\0') {
        return Err(bad("bounded source string"));
    }
    work(shared, ctl, value.len() as u64 + 1)?;
    b.charge(value.len())?;
    let mut s = String::new();
    s.try_reserve_exact(value.len()).map_err(|_| bad("string allocation"))?;
    b.charge(s.capacity().saturating_sub(value.len()))?;
    s.push_str(value);
    Ok(s)
}
fn reserve<T>(v: &mut Vec<T>, count: usize, b: &mut Budget<'_>) -> Result<(), IoError> {
    let old = v.capacity();
    b.charge(mul(count, std::mem::size_of::<T>())?)?;
    v.try_reserve_exact(count).map_err(|_| bad("vector allocation"))?;
    b.charge(mul(v.capacity().saturating_sub(old.saturating_add(count)), std::mem::size_of::<T>())?)?;
    Ok(())
}
fn hashes(bytes: &[u8], shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>) -> Result<(&'static str, blake3::Hash), IoError> {
    if bytes.is_empty() || bytes.len() > 16_384 || !photocraft_xcf::is_xcf(bytes) {
        return Err(bad("outside observed plain source size"));
    }
    let mut sha = Sha256::new();
    let mut blake = blake3::Hasher::new();
    for chunk in bytes.chunks(4096) {
        work(shared, ctl, chunk.len() as u64)?;
        sha.update(chunk);
        blake.update(chunk);
    }
    let mut hex = [0u8; 64];
    for (value, pair) in sha.finalize().iter().zip(hex.chunks_exact_mut(2)) {
        let alphabet = b"0123456789abcdef";
        *pair.get_mut(0).ok_or_else(|| bad("digest width"))? = *alphabet.get(usize::from(value >> 4)).ok_or_else(|| bad("digest digit"))?;
        *pair.get_mut(1).ok_or_else(|| bad("digest width"))? = *alphabet.get(usize::from(value & 15)).ok_or_else(|| bad("digest digit"))?;
    }
    let actual = std::str::from_utf8(&hex).map_err(|_| bad("digest encoding"))?;
    let sha = match actual {
        "55c49a2d78207ff67fb354e8f2506703a0570318cb07472614cf978cbc0de3f6" => "55c49a2d78207ff67fb354e8f2506703a0570318cb07472614cf978cbc0de3f6",
        "f4bd99ae659d3062e8fa23762cbc6b78d4d4622ba9e4969a62c87f09d81adfe6" => "f4bd99ae659d3062e8fa23762cbc6b78d4d4622ba9e4969a62c87f09d81adfe6",
        "b8291cf6e7a8dedcf38c01a039eed4353c70054914a704f60834a684d9d02eab" => "b8291cf6e7a8dedcf38c01a039eed4353c70054914a704f60834a684d9d02eab",
        "f3477ab5c4f24d9cfb8e69ee9df8f591ca35ae29601753b3abfe8517fb0b7b40" => "f3477ab5c4f24d9cfb8e69ee9df8f591ca35ae29601753b3abfe8517fb0b7b40",
        _ => return Err(bad("source lacks an observed original binding")),
    };
    poll(shared, ctl)?;
    Ok((sha, blake.finalize()))
}
fn properties(
    state: &mut photocraft_doc::XcfState,
    original: &[photocraft_xcf::RawProperty],
    owner: Option<ItemRef>,
    b: &mut Budget<'_>,
    shared: &photocraft_xcf::Budget<'_>,
    ctl: &Interrupt<'_>,
) -> Result<(), IoError> {
    if original.len() > 32 {
        return Err(bad("source property count"));
    }
    let old = state.opaque.len();
    reserve(&mut state.opaque, original.len(), b)?;
    let mut copied = Vec::new();
    reserve(&mut copied, original.len(), b)?;
    for property in original {
        poll(shared, ctl)?;
        if property.payload.len() > 64 << 10 || property.declared_bytes as usize != property.payload.len() {
            return Err(bad("property envelope"));
        }
        let mut bytes = Vec::new();
        reserve(&mut bytes, property.payload.len(), b)?;
        for chunk in property.payload.chunks(4096) {
            work(shared, ctl, chunk.len() as u64)?;
            bytes.extend_from_slice(chunk);
        }
        copied.push(photocraft_xcf::RawProperty { tag: property.tag, offset: property.offset, declared_bytes: property.declared_bytes, payload: bytes });
    }
    // Parent hashes individually bounded selected records; no codec/math fork.
    work(shared, ctl, original.iter().try_fold(0u64, |n, p| n.checked_add(p.payload.len() as u64).ok_or_else(|| bad("property work overflow")))?)?;
    preserve_properties(state, copied, owner, 26, b)?;
    if state.opaque.len() != old.checked_add(original.len()).ok_or_else(|| bad("property output count"))? {
        return Err(bad("property mapping count"));
    }
    poll(shared, ctl)
}
fn item(d: &Drawable, owner: ItemRef, r: Rect, name: String, mask: Option<MaskState>) -> ItemState {
    ItemState {
        item: owner,
        name,
        custom_name: false,
        visible: d.visible,
        locks: ItemLocks { content: d.lock_content, position: d.lock_position, alpha: d.lock_alpha, visibility: d.lock_visibility },
        parent: None,
        source_tattoo: d.tattoo,
        source_flags: d.group_flags,
        raster_bounds: Some(r),
        mask,
        composition: None,
    }
}
#[derive(Clone, Copy, Debug)]
pub struct CorrespondenceRequirements {
    /// Full conservative proof-owner retention plus the already-held verification
    /// scratch. No selected-slice refund; not a process RSS measurement.
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
/// Constant/count-only gate before any full current-document validator or
/// proof census. This deliberately selective seam cannot verify arbitrary scenes.
fn selective(current: &Document, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let state = current.xcf_state.as_ref().ok_or_else(|| bad("missing selective state"))?;
    if !current.layers.is_empty()
        || !current.paths.is_empty()
        || current.work_path.is_some()
        || current.clipping_path.is_some()
        || current.selection.is_some()
        || current.selection_channel_id.is_some()
        || current.quick_mask.is_some()
        || current.quick_mask_unselected
        || current.channels.len() != 1
        || state.items.len() != 2
        || state.filter_stacks.len() != 1
        || state.filter_masks.len() != 1
        || !state.resources.is_empty()
        || !state.source_items.is_empty()
        || !state.editable_indexed.is_empty()
        || !state.item_sets.is_empty()
        || state.floating.is_some()
        || !state.selection.selected.is_empty()
        || !state.selection.selected_paths.is_empty()
        || state.selection.active.is_some()
        || state.selection.mask_target.is_some()
    {
        return Err(bad("correspondence requires the selective channel/mask-only structure"));
    }
    Ok(())
}
/// Holds all original decoder backing and mapped output reservations. Borrowed
/// bytes and canonical IDs never become locator/read permissions.
pub struct AssembledStoredChannel<'s, 'b, 'a> {
    document: Document,
    original: XcfFile,
    source: &'s [u8],
    source_sha256: &'static str,
    source_blake3: blake3::Hash,
    ordinal: usize,
    ids: Ids,
    shared: &'b photocraft_xcf::Budget<'a>,
    _decoder: DecodeReservations<'b, 'a>,
    _output: Reservation<'b, 'a>,
    _namespace: Reservation<'b, 'a>,
    _index: Reservation<'b, 'a>,
}
impl AssembledStoredChannel<'_, '_, '_> {
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.source
    }
    pub fn source_sha256(&self) -> &'static str {
        self.source_sha256
    }
    pub fn channel_id(&self) -> ChannelId {
        self.ids.channel
    }
    /// Conservative complete live owner reservations, including full decoded
    /// scene and namespace, not just the selected source slices. CallerReserved
    /// source storage must remain externally held and is also priced here.
    pub fn retained_envelope_bytes(&self) -> Result<u64, IoError> {
        self._decoder
            .retained_bytes()
            .checked_add(self.source.len() as u64)
            .and_then(|n| n.checked_add(self._output.bytes()))
            .and_then(|n| n.checked_add(self._namespace.bytes()))
            .and_then(|n| n.checked_add(self._index.bytes()))
            .ok_or_else(|| bad("owner envelope overflow"))
    }
    pub fn correspondence_requirements(&self) -> Result<CorrespondenceRequirements, IoError> {
        let resident_bytes = self.retained_envelope_bytes()?;
        let work_units = resident_bytes
            .checked_mul(8)
            .and_then(|n| n.checked_add((photocraft_geom::TILE_SIZE as u64).checked_mul(photocraft_geom::TILE_SIZE as u64)?.checked_mul(128)?))
            .ok_or_else(|| bad("verification work overflow"))?;
        Ok(CorrespondenceRequirements { resident_bytes, index_bytes: self._index.bytes(), work_units })
    }
    /// Precharge under another caller's admitted worker ledger before any proof
    /// traversal. The original owner reservations stay live independently.
    pub fn verify_document_with_charge<'p>(
        &'p self,
        current: &'p Document,
        ctl: &Interrupt<'_>,
        charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
    ) -> Result<StoredChannelCorrespondence<'p>, IoError> {
        selective(current, ctl)?;
        let mut upper = self.correspondence_requirements()?.work_units;
        while upper != 0 {
            poll(self.shared, ctl)?;
            let n = upper.min(4096);
            charge(n)?;
            upper -= n;
        }
        self.verify_document(current, ctl)
    }
    /// A proof borrows the verified current document, so it cannot mutate while
    /// the proof is in use. This does not prove numerical Gaussian/mask behavior.
    pub fn verify_document<'p>(&'p self, current: &'p Document, ctl: &Interrupt<'_>) -> Result<StoredChannelCorrespondence<'p>, IoError> {
        poll(self.shared, ctl)?;
        selective(current, ctl)?;
        // Price complete bounded proof traversal before any current metadata or
        // original/current byte comparisons; actual row charges add conservatively.
        let mut upper = self.correspondence_requirements()?.work_units;
        while upper != 0 {
            let n = upper.min(4096);
            work(self.shared, ctl, n)?;
            upper -= n;
        }

        let original = self.original.channels.get(self.ordinal).ok_or_else(|| bad("original channel disappeared"))?;
        let effect = original.effects.first().ok_or_else(|| bad("original effect disappeared"))?;
        let mask = effect.mask.as_deref().ok_or_else(|| bad("original mask disappeared"))?;
        let state = current.xcf_state.as_ref().ok_or_else(|| bad("missing candidate state"))?;
        let baseline = self.document.xcf_state.as_ref().ok_or_else(|| bad("missing original mapped state"))?;
        if current.working_profile_interpretation != self.document.working_profile_interpretation
            || current.physical_resolution != self.document.physical_resolution
            || current.resolution_dpi.to_bits() != self.document.resolution_dpi.to_bits()
            || current.mode != self.document.mode
            || current.icc_profile != self.document.icc_profile
            || current.source_color_context != self.document.source_color_context
            || !state.resources.is_empty()
            || !state.source_items.is_empty()
            || !state.editable_indexed.is_empty()
            || !state.item_sets.is_empty()
            || state.floating.is_some()
            || current.depth != depth(self.original.sample)
            || current.sample_encoding != Some(SampleEncoding::NonlinearProfile)
            || current.size != Size::new(32, 32)
            || current.channels.len() != 1
            || state.items.len() != 2
            || state.filter_stacks.len() != 1
            || state.filter_masks.len() != 1
            || state.opaque.len() != baseline.opaque.len()
            || state.provenance != baseline.provenance
            || state.items != baseline.items
            || state.filter_stacks != baseline.filter_stacks
        {
            return Err(bad("current source metadata/node correspondence differs"));
        }
        let channel = current.channels.first().filter(|v| v.id == self.ids.channel).ok_or_else(|| bad("current channel identity"))?;
        let mapped = self.document.channels.first().ok_or_else(|| bad("mapped channel missing"))?;
        if channel.name != mapped.name
            || channel.spot != mapped.spot
            || channel.color != mapped.color
            || channel.opacity.to_bits() != mapped.opacity.to_bits()
            || channel.indicates != mapped.indicates
        {
            return Err(bad("current channel presentation differs"));
        }
        let current_mask = state
            .filter_masks
            .first()
            .filter(|m| m.id == self.ids.mask && m.owner == self.ids.filter && m.bounds == Rect::new(0, 0, 96, 80))
            .ok_or_else(|| bad("current filter mask association"))?;
        verify_plane(&channel.surface, original.plane.as_ref().ok_or_else(|| bad("original channel plane"))?, bounds(original)?, self.shared, ctl)?;
        verify_plane(&current_mask.surface, mask.plane.as_ref().ok_or_else(|| bad("original mask plane"))?, bounds(mask)?, self.shared, ctl)?;
        for (a, b) in state.opaque.iter().zip(&baseline.opaque) {
            if a.owner != b.owner
                || a.namespace != b.namespace
                || a.identity != b.identity
                || a.source_version != b.source_version
                || a.order != b.order
                || a.flags != b.flags
                || a.blake3 != b.blake3
                || a.rendering_required != b.rendering_required
                || a.bytes.len() != b.bytes.len()
            {
                return Err(bad("current original opaque envelope differs"));
            }
            for (x, y) in a.bytes.chunks(4096).zip(b.bytes.chunks(4096)) {
                work(self.shared, ctl, x.len() as u64)?;
                if x != y {
                    return Err(bad("current original opaque bytes differ"));
                }
            }
        }
        work(self.shared, ctl, self.source.len() as u64 + 4096)?;
        current.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { IoError::Cancelled } else { bad("invalid verified candidate state") })?;
        let node = state.filter_stacks.first().and_then(|s| s.nodes.first()).ok_or_else(|| bad("mapped node missing"))?;
        poll(self.shared, ctl)?;
        Ok(StoredChannelCorrespondence {
            document: current,
            bytes: self.source,
            node,
            channel: self.ids.channel,
            mask: self.ids.mask,
            source_sha256: self.source_sha256,
            source_blake3: self.source_blake3,
        })
    }
}
/// Private construction: unlike a fingerprint-only assertion, this value follows
/// original-byte decode and exact native plane + mapped node/opaque comparison.
pub struct StoredChannelCorrespondence<'p> {
    document: &'p Document,
    bytes: &'p [u8],
    node: &'p FilterNode,
    channel: ChannelId,
    mask: MaskId,
    source_sha256: &'static str,
    source_blake3: blake3::Hash,
}
impl StoredChannelCorrespondence<'_> {
    pub fn document(&self) -> &Document {
        self.document
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.bytes
    }
    pub fn original_node(&self) -> &FilterNode {
        self.node
    }
    pub fn channel_id(&self) -> ChannelId {
        self.channel
    }
    pub fn mask_id(&self) -> MaskId {
        self.mask
    }
    pub fn source_sha256(&self) -> &'static str {
        self.source_sha256
    }
    pub fn source_blake3(&self) -> blake3::Hash {
        self.source_blake3
    }
}
fn verify_plane(s: &Surface, p: &Plane, r: Rect, shared: &photocraft_xcf::Budget<'_>, ctl: &Interrupt<'_>) -> Result<(), IoError> {
    let format = PixelFormat::new(ColorMode::Grayscale, depth(p.sample), false);
    let stride = format.bytes_per_pixel();
    if p.channels != 1
        || p.width != r.width()
        || p.height != r.height()
        || !matches!(p.sample, Sample::U8 | Sample::F64)
        || s.format() != format
        || r.x0 != 0
        || r.y0 != 0
        || p.bytes.len() != mul(mul(p.width as usize, p.height as usize)?, stride)?
        || s.tile_count() > 1
        || s.default_bytes() != [0u8; 8].get(..stride).ok_or_else(|| bad("default sample width"))?
    {
        return Err(bad("original native plane/default descriptor differs"));
    }
    let tile_len = mul(mul(photocraft_geom::TILE_SIZE as usize, photocraft_geom::TILE_SIZE as usize)?, stride)?;
    for (coordinate, tile) in s.tiles() {
        poll(shared, ctl)?;
        if coordinate.tx != 0 || coordinate.ty != 0 || tile.bytes().len() != tile_len {
            return Err(bad("native plane tile extent differs"));
        }
    }
    // Compare the complete single native tile, not just sparse content bounds.
    // Defaults/tails are explicitly zero; missing tiles can only match a zero plane.
    let mut pixel = [0u8; 8];
    for y in 0..photocraft_geom::TILE_SIZE {
        work(shared, ctl, (photocraft_geom::TILE_SIZE as u64).checked_mul(stride as u64).ok_or_else(|| bad("row work overflow"))?)?;
        for x in 0..photocraft_geom::TILE_SIZE {
            let actual = pixel.get_mut(..stride).ok_or_else(|| bad("native sample width"))?;
            s.try_read_pixel_bytes(x, y, actual).map_err(|_| bad("native sample read"))?;
            let expected = if x < r.x1 && y < r.y1 {
                let at = mul(add(mul(y as usize, p.width as usize)?, x as usize)?, stride)?;
                p.bytes.get(at..add(at, stride)?).ok_or_else(|| bad("original native row range"))?
            } else {
                [0u8; 8].get(..stride).ok_or_else(|| bad("native default width"))?
            };
            if actual != expected {
                return Err(bad("current plane raw bytes differ from original decoded plane"));
            }
        }
    }
    poll(shared, ctl)
}
#[allow(clippy::too_many_arguments)]
pub fn assemble_owned_stored_channel<'s, 'b, 'a>(
    bytes: &'s [u8],
    channel_ordinal: usize,
    ids: Ids,
    namespace: NamespaceBacking<'_>,
    mapping: mapper::Limits,
    options: &DecodeOptions<'_>,
    shared: &'b photocraft_xcf::Budget<'a>,
    accounting: SourceAccounting,
    ctl: &Interrupt<'_>,
) -> Result<AssembledStoredChannel<'s, 'b, 'a>, IoError> {
    let (sha, hash) = hashes(bytes, shared, ctl)?;
    let floor = namespace
        .namespace
        .channels
        .len()
        .checked_mul(std::mem::size_of::<ChannelId>())
        .and_then(|n| n.checked_add(namespace.namespace.filters.len().checked_mul(std::mem::size_of::<FilterId>())?))
        .and_then(|n| n.checked_add(namespace.namespace.masks.len().checked_mul(std::mem::size_of::<MaskId>())?))
        .ok_or_else(|| bad("namespace floor overflow"))?;
    if namespace.retained_bytes < floor as u64 {
        return Err(bad("namespace backing not admitted"));
    }
    let namespace_lease = shared.reserve(BudgetKind::Resident, namespace.retained_bytes, "stored namespace backing").map_err(accounting_error)?;
    // Mapper's namespace/index upper is reserved before its HashSet allocation.
    let index = shared.reserve(BudgetKind::Index, mapping.metadata_bytes, "stored channel mapping index").map_err(accounting_error)?;
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
        return Err(bad("wrapper not in observed source domain"));
    }
    let original = decoded.decoded.file;
    if original.version != 26
        || original.width != 32
        || original.height != 32
        || original.model != Model::Rgb
        || original.encoding != Encoding::NonlinearProfile
        || !matches!(original.sample, Sample::U8 | Sample::F64)
    {
        return Err(bad("outside observed source descriptor"));
    }
    let channel = original.channels.get(channel_ordinal).ok_or_else(|| bad("original channel ordinal"))?;
    let output_lease = RefCell::new(shared.reserve(BudgetKind::Resident, 0, "stored candidate output").map_err(accounting_error)?);
    let accounted = Cell::new(0usize);
    let account = |used: usize| -> Result<(), IoError> {
        if used > accounted.get() {
            work(shared, ctl, (used - accounted.get()) as u64)?;
        }
        output_lease
            .try_borrow_mut()
            .map_err(|_| bad("output accounting already borrowed"))?
            .resize(used as u64, "stored candidate output")
            .map_err(accounting_error)?;
        accounted.set(used);
        Ok(())
    };
    let mut b = Budget { used: 0, max: options.limits.resident_bytes, peak: 0, shared: Some(&account) };
    // Canonical validator's tiny closed-namespace temporary maps remain
    // prospectively retained alongside source/output until proof owner drops.
    b.charge(16 << 10)?;
    b.charge(std::mem::size_of::<Document>())?;
    let candidate = mapper::map(channel, 26, depth(original.sample), encoding(original.encoding), ids, namespace.namespace, mapping, ctl, &mut |n| {
        b.charge(usize::try_from(n).map_err(|_| bad("mapper charge address space"))?)
    })?;
    work(shared, ctl, candidate.requirements.work_units)?;
    let mask = candidate.mask;
    let mask_plane = mask.plane.as_ref().ok_or_else(|| bad("original mask plane"))?;
    let channel_plane = channel.plane.as_ref().ok_or_else(|| bad("original channel plane"))?;
    if bounds(mask)? != Rect::new(0, 0, 96, 80) || mask_plane.sample != original.sample || channel_plane.sample != original.sample {
        return Err(bad("mask/source native correspondence"));
    }
    let mut doc = Document::new("", Size::new(32, 32), ColorMode::Rgb, depth(original.sample));
    doc.name = text("Stored channel candidate", &mut b, shared, ctl)?;
    doc.sample_encoding = Some(SampleEncoding::NonlinearProfile);
    if !original.icc.is_empty() {
        let mut icc = Vec::new();
        reserve(&mut icc, original.icc.len(), &mut b)?;
        for chunk in original.icc.chunks(4096) {
            work(shared, ctl, chunk.len() as u64)?;
            icc.extend_from_slice(chunk);
        }
        let icc = Arc::new(icc);
        doc.icc_profile = Some(icc.clone());
        doc.source_color_context = Some(SourceColorContext { original_icc: Some(icc), source_precision: original.precision });
    } else {
        doc.icc_profile = None;
        doc.source_color_context = Some(SourceColorContext { original_icc: None, source_precision: original.precision });
    }
    if !original.dpi.0.is_finite()
        || !original.dpi.1.is_finite()
        || original.dpi.0 <= 0.
        || original.dpi.1 <= 0.
        || !matches!(original.unit, photocraft_xcf::Unit::Inch)
    {
        return Err(bad("unported source physical unit"));
    }
    doc.set_physical_resolution(photocraft_doc::PhysicalResolution {
        x_dpi: f64::from(original.dpi.0),
        y_dpi: f64::from(original.dpi.1),
        unit: photocraft_doc::PhysicalUnit::Inch,
    })
    .map_err(|_| bad("source physical resolution"))?;
    let channel_surface = surface(&mut b, channel_plane, bounds(channel)?, ColorMode::Grayscale, false, &channel_plane.bytes, ctl)?;
    let mask_surface = surface(&mut b, mask_plane, bounds(mask)?, ColorMode::Grayscale, false, &mask_plane.bytes, ctl)?;
    reserve(&mut doc.channels, 1, &mut b)?;
    let color = channel.channel_color.map_or(AlphaChannel::DEFAULT_COLOR, |c| photocraft_color::Color::rgb(c[0], c[1], c[2]));
    doc.channels.push(AlphaChannel {
        id: ids.channel,
        name: text(&channel.name, &mut b, shared, ctl)?,
        surface: channel_surface,
        spot: None,
        color,
        opacity: channel.opacity,
        indicates: if channel.show_masked { photocraft_doc::ColorIndicates::MaskedAreas } else { photocraft_doc::ColorIndicates::SelectedAreas },
    });
    let mut state = photocraft_doc::XcfState::default();
    state.provenance = photocraft_doc::xcf_provenance::ProjectProvenance {
        format: text("xcf", &mut b, shared, ctl)?,
        format_version: 26,
        container: text("plain", &mut b, shared, ctl)?,
        source_label: text("Private stored channel", &mut b, shared, ctl)?,
        source_hash: Some(text(hash.to_hex().as_str(), &mut b, shared, ctl)?),
        ..Default::default()
    };
    reserve(&mut state.items, 2, &mut b)?;
    state.items.push(item(channel, ItemRef::Channel(ids.channel), bounds(channel)?, text(&channel.name, &mut b, shared, ctl)?, None));
    state.items.push(item(
        mask,
        ItemRef::Mask(ids.mask),
        bounds(mask)?,
        text(&mask.name, &mut b, shared, ctl)?,
        Some(MaskState {
            owner: ItemRef::Channel(ids.channel),
            bounds: bounds(mask)?,
            offset: [mask.offset.0, mask.offset.1],
            outside_f64_bits: 0f64.to_bits(),
            enabled: mask.mask_enabled,
            shown: mask.mask_shown,
            editing: mask.mask_editing,
            linked: mask.legacy_linked,
        }),
    ));
    reserve(&mut state.filter_stacks, 1, &mut b)?;
    state.filter_stacks.push(candidate.stack);
    reserve(&mut state.filter_masks, 1, &mut b)?;
    state.filter_masks.push(FilterMask { id: ids.mask, owner: ids.filter, surface: mask_surface, bounds: bounds(mask)? });
    properties(&mut state, &original.raw_properties, None, &mut b, shared, ctl)?;
    properties(&mut state, &channel.raw_properties, Some(ItemRef::Channel(ids.channel)), &mut b, shared, ctl)?;
    properties(&mut state, &candidate.effect.properties, Some(ItemRef::Channel(ids.channel)), &mut b, shared, ctl)?;
    properties(&mut state, &mask.raw_properties, Some(ItemRef::Mask(ids.mask)), &mut b, shared, ctl)?;
    doc.xcf_state = Some(state);
    work(shared, ctl, mapping.metadata_bytes)?;
    doc.validate_xcf_state_with(ctl).map_err(|e| if e.code == "cancelled" { IoError::Cancelled } else { bad("invalid private mapped state") })?;
    // Retain all decoder backing; no source/plane refund based on selected slices.
    drop(b);
    let assembled = AssembledStoredChannel {
        document: doc,
        original,
        source: bytes,
        source_sha256: sha,
        source_blake3: hash,
        ordinal: channel_ordinal,
        ids,
        shared,
        _decoder: decoded.reservations,
        _output: output_lease.into_inner(),
        _namespace: namespace_lease,
        _index: index,
    };
    assembled.verify_document(assembled.document(), ctl)?;
    poll(shared, ctl)?;
    Ok(assembled)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_gate_refuses_extra_scene_and_persisted_selection_before_traversal() {
        let mut doc = Document::new("Selective", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
        let s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        doc.channels.push(AlphaChannel::new("Stored", s));
        let mut state = photocraft_doc::XcfState::default();
        // Only lengths matter to the early guard; relationship validation is later.
        let descriptor = Drawable::<Plane>::default();
        state.items.push(item(&descriptor, ItemRef::Channel(ChannelId(1)), Rect::new(0, 0, 32, 32), String::new(), None));
        state.items.push(item(&descriptor, ItemRef::Mask(MaskId(1)), Rect::new(0, 0, 96, 80), String::new(), None));
        state.filter_stacks.push(photocraft_doc::xcf_filters::FilterStack { owner: ItemRef::Channel(ChannelId(1)), nodes: vec![], revision: 0 });
        state.filter_masks.push(FilterMask {
            id: MaskId(1),
            owner: FilterId(1),
            surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)),
            bounds: Rect::new(0, 0, 96, 80),
        });
        doc.xcf_state = Some(state);
        selective(&doc, &Interrupt::NONE).unwrap();
        doc.layers.push(Layer::new("Foreign", LayerContent::Raster(Surface::new(doc.pixel_format()))));
        assert!(selective(&doc, &Interrupt::NONE).is_err());
        doc.layers.clear();
        doc.xcf_state.as_mut().unwrap().selection.selected.push(ItemRef::Channel(ChannelId(1)));
        assert!(selective(&doc, &Interrupt::NONE).is_err());
        assert!(matches!(selective(&doc, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
    }
    // Unrun raw F64 signed-zero/subnormal and padded-tail correspondence draft;
    // it is native mapping evidence only, not Gaussian/render qualification.
    #[test]
    fn raw_f64_and_zero_tail_correspondence_is_not_float_equality() {
        let values = [(-0f64).to_bits(), 1u64, 0.75f64.to_bits(), 1f64.to_bits()];
        let raw = values.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let plane = Plane { width: 2, height: 2, channels: 1, sample: Sample::F64, bytes: raw.clone() };
        let limits = photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default()).unwrap();
        let shared = photocraft_xcf::Budget::new(limits, None);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let r = Rect::new(0, 0, 2, 2);
        let mut s = Surface::try_from_interleaved_with(format, r, &raw, &Interrupt::NONE).unwrap();
        verify_plane(&s, &plane, r, &shared, &Interrupt::NONE).unwrap();
        s.try_write_interleaved(Rect::new(0, 0, 1, 1), &0f64.to_ne_bytes()).unwrap();
        assert!(verify_plane(&s, &plane, r, &shared, &Interrupt::NONE).is_err());
        assert!(matches!(verify_plane(&s, &plane, r, &shared, &Interrupt::cancel_only(&|| true)), Err(IoError::Cancelled)));
    }
}
