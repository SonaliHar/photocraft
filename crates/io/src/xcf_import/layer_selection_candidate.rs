//! Private source-only projection of the observed v22 Gray layer selection.
//! No default import, evaluator epoch, working-space or mask-coverage authority.
//! The caller retains and precharges the complete decoder/container backing;
//! selected borrowed floors here never replace or refund that enclosing lease.
use super::*;
use photocraft_doc::{
    LayerId,
    item_identity::{FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use std::collections::HashSet;
pub const LAYER_SELECTION_EPOCH: &str = "xcf22-layer-selection-gaussian-unresolved-v1";
/// Structural retention does not resolve any space/operator word or qualify rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerOuterPolicy {
    ObservedLiteral,
    RetainUnresolved,
}
#[derive(Clone, Copy, Debug)]
pub struct LayerSelectionIds {
    pub layer: LayerId,
    pub filter: FilterId,
    pub mask: MaskId,
}
pub struct LayerSelectionNamespace<'a> {
    pub layers: &'a [LayerId],
    pub filters: &'a [FilterId],
    pub masks: &'a [MaskId],
}
#[derive(Clone, Copy, Debug)]
pub struct LayerSelectionLimits {
    pub resident_bytes: u64,
    pub metadata_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
    pub namespace_items: usize,
}
impl Default for LayerSelectionLimits {
    fn default() -> Self {
        Self { resident_bytes: 16 << 20, metadata_bytes: 1 << 20, index_bytes: 1 << 20, work_units: 8_000_000, namespace_items: 10_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LayerSelectionRequirements {
    pub reserved_resident_bytes: u64,
    pub reserved_index_bytes: u64,
    pub reserved_work_units: u64,
    pub known_resident_bytes: u64,
    pub known_index_bytes: u64,
    pub work_units: u64,
}
pub struct LayerSelectionCandidate<'a> {
    pub source_surface: Surface,
    pub source_bounds: Rect,
    pub state: photocraft_doc::XcfState,
    pub original_layer: &'a Drawable,
    pub original_effect: &'a photocraft_xcf::Effect,
    pub original_mask: &'a Drawable,
    pub requirements: LayerSelectionRequirements,
}
impl LayerSelectionCandidate<'_> {
    pub fn original_layer_properties(&self) -> &[photocraft_xcf::RawProperty] {
        &self.original_layer.raw_properties
    }
    pub fn original_effect_properties(&self) -> &[photocraft_xcf::RawProperty] {
        &self.original_effect.properties
    }
    pub fn original_mask_properties(&self) -> &[photocraft_xcf::RawProperty] {
        &self.original_mask.raw_properties
    }
}
fn fail(s: &str) -> IoError {
    IoError::Unsupported(format!("XCF private layer selection: {s}"))
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| fail("accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| fail("accounting overflow"))
}
struct Local<'a, 'i> {
    limits: LayerSelectionLimits,
    metadata: u64,
    index: u64,
    work: u64,
    ctl: &'a Interrupt<'i>,
}
impl Local<'_, '_> {
    fn step(&mut self, n: u64) -> Result<(), IoError> {
        poll(self.ctl)?;
        self.work = add(self.work, n)?;
        if self.work > self.limits.work_units {
            return Err(fail("work limit"));
        }
        Ok(())
    }
    fn metadata(&mut self, n: u64, b: &mut Budget<'_>) -> Result<(), IoError> {
        self.metadata = add(self.metadata, n)?;
        if self.metadata > self.limits.metadata_bytes {
            return Err(fail("metadata limit"));
        }
        b.charge(usize::try_from(n).map_err(|_| fail("metadata address space"))?)
    }
    fn index(&mut self, n: u64, b: &mut Budget<'_>) -> Result<(), IoError> {
        self.index = add(self.index, n)?;
        if self.index > self.limits.index_bytes {
            return Err(fail("index limit"));
        }
        b.charge(usize::try_from(n).map_err(|_| fail("index address space"))?)
    }
    fn text(&mut self, s: &str, b: &mut Budget<'_>) -> Result<String, IoError> {
        self.step(s.len() as u64 + 1)?;
        if s.len() > 256 || s.contains('\0') {
            return Err(fail("text bound"));
        }
        let mut out = String::new();
        self.metadata(s.len() as u64, b)?;
        out.try_reserve_exact(s.len()).map_err(|_| fail("text allocation"))?;
        self.metadata(out.capacity().saturating_sub(s.len()) as u64, b)?;
        out.push_str(s);
        Ok(out)
    }
    fn vec<T>(&mut self, n: usize, b: &mut Budget<'_>) -> Result<Vec<T>, IoError> {
        self.metadata(mul(n as u64, std::mem::size_of::<T>() as u64)?, b)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n).map_err(|_| fail("vector allocation"))?;
        self.metadata(mul(v.capacity().saturating_sub(n) as u64, std::mem::size_of::<T>() as u64)?, b)?;
        Ok(v)
    }
}
fn namespace(ids: LayerSelectionIds, n: LayerSelectionNamespace<'_>, l: &mut Local<'_, '_>, b: &mut Budget<'_>) -> Result<(), IoError> {
    poll(l.ctl)?;
    if ids.layer.0 == 0 || ids.filter.0 == 0 || ids.mask.0 == 0 || ids.layer.0 == ids.filter.0 || ids.layer.0 == ids.mask.0 || ids.filter.0 == ids.mask.0 {
        return Err(fail("zero or aliased requested identities"));
    }
    let count = n
        .layers
        .len()
        .checked_add(n.filters.len())
        .and_then(|x| x.checked_add(n.masks.len()))
        .filter(|x| *x <= l.limits.namespace_items)
        .ok_or_else(|| fail("namespace count"))?;
    let prospective = mul(count as u64, 64)?;
    l.index(prospective, b)?;
    let mut seen = HashSet::new();
    seen.try_reserve(count).map_err(|_| fail("namespace allocation"))?;
    l.index(mul(seen.capacity() as u64, 64)?.saturating_sub(prospective), b)?;
    let mut matches = 0usize;
    for (raw, is_layer) in n.layers.iter().map(|v| (v.0, true)).chain(n.filters.iter().map(|v| (v.0, false))).chain(n.masks.iter().map(|v| (v.0, false))) {
        l.step(1)?;
        if raw == 0 || !seen.insert(raw) || raw == ids.filter.0 || raw == ids.mask.0 {
            return Err(fail("existing identity duplicate/collision"));
        }
        if raw == ids.layer.0 {
            if !is_layer {
                return Err(fail("layer identity wrong kind"));
            }
            matches += 1;
        }
    }
    if matches != 1 {
        return Err(fail("layer must have one explicit declaration"));
    }
    Ok(())
}
fn word(bytes: &[u8], at: usize) -> Result<u32, IoError> {
    let end = at.checked_add(4).ok_or_else(|| fail("raw word overflow"))?;
    let x: [u8; 4] = bytes.get(at..end).ok_or_else(|| fail("short raw word"))?.try_into().map_err(|_| fail("word width"))?;
    Ok(u32::from_be_bytes(x))
}
fn outer_words(blend: Option<i32>, space: Option<i32>, mode: Option<i32>, policy: LayerOuterPolicy) -> Result<(i32, i32, i32), IoError> {
    let words = (
        blend.ok_or_else(|| fail("missing blend space"))?,
        space.ok_or_else(|| fail("missing composite space"))?,
        mode.ok_or_else(|| fail("missing composite mode"))?,
    );
    if policy == LayerOuterPolicy::ObservedLiteral && words != (1848279633, 1848279633, 1848279633) {
        return Err(fail("observed source outer tuple"));
    }
    Ok(words)
}
fn argument(a: &photocraft_xcf::Argument, e: &photocraft_xcf::Effect, l: &mut Local<'_, '_>, b: &mut Budget<'_>) -> Result<FilterValue, IoError> {
    l.step(1)?;
    if a.configuration.is_some() || a.graph.is_some() {
        return Err(fail("nested argument"));
    }
    let p = e.properties.get(a.property_index).filter(|p| p.tag == 45).ok_or_else(|| fail("raw argument association"))?;
    let end = 4usize.checked_add(usize::try_from(word(&p.payload, 0)?).map_err(|_| fail("name width"))?).ok_or_else(|| fail("name overflow"))?;
    let name = p.payload.get(4..end).and_then(|v| v.strip_suffix(&[0])).ok_or_else(|| fail("name envelope"))?;
    if name != a.name.as_bytes() || p.payload.len() != end.checked_add(8).ok_or_else(|| fail("argument overflow"))? {
        return Err(fail("raw argument mismatch"));
    }
    let (kind, value) = (word(&p.payload, end)?, word(&p.payload, end.checked_add(4).ok_or_else(|| fail("argument overflow"))?)?);
    match (&a.value, kind) {
        (photocraft_xcf::ArgumentValue::Float32Bits(x), 3) if *x == value => Ok(FilterValue::Number(ExactNumber::Float32Bits(*x))),
        (photocraft_xcf::ArgumentValue::Boolean(x), 2) if value == u32::from(*x) => Ok(FilterValue::Bool(*x)),
        (photocraft_xcf::ArgumentValue::Enum(x), 5) if *x == value as i32 => {
            Ok(FilterValue::Enum { domain: l.text(crate::xcf_channel_candidate::RAW_ENUM_DOMAIN, b)?, value: *x })
        }
        _ => Err(fail("raw argument kind/value mismatch")),
    }
}
fn raw(
    properties: &[photocraft_xcf::RawProperty],
    owner: ItemRef,
    state: &mut photocraft_doc::XcfState,
    l: &mut Local<'_, '_>,
    b: &mut Budget<'_>,
) -> Result<(), IoError> {
    if properties.len() > 32 {
        return Err(fail("raw property count"));
    }
    let mut copied = l.vec::<photocraft_xcf::RawProperty>(properties.len(), b)?;
    l.metadata(mul(properties.len() as u64, std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>() as u64)?, b)?;
    for p in properties {
        l.step(1)?;
        if p.payload.len() > 64 << 10 || usize::try_from(p.declared_bytes).ok() != Some(p.payload.len()) {
            return Err(fail("raw property bound"));
        }
        let mut payload = l.vec::<u8>(p.payload.len(), b)?;
        for c in p.payload.chunks(4096) {
            l.step(mul(c.len() as u64, 2)?)?;
            payload.extend_from_slice(c);
        }
        copied.push(photocraft_xcf::RawProperty { tag: p.tag, offset: p.offset, declared_bytes: p.declared_bytes, payload });
    }
    // Reuses normal import's exact raw-record naming/hash/Arc transfer. Bounded
    // properties and copy/hash work were prepaid; no decoder or semantics fork.
    // The shared raw-record helper reserves 512 bytes per record for its text
    // and Arc/header owners; include that in this lane's metadata quota too.
    l.metadata(mul(properties.len() as u64, 512)?, b)?;
    let previous = state.opaque.capacity();
    preserve_properties(state, copied, Some(owner), 22, b)?;
    let extra = state.opaque.capacity().saturating_sub(previous.saturating_add(properties.len()));
    l.metadata(mul(extra as u64, std::mem::size_of::<photocraft_doc::xcf_provenance::OpaqueRecord>() as u64)?, b)?;
    poll(l.ctl)
}
fn item(d: &Drawable, reference: ItemRef, mask: Option<MaskState>, l: &mut Local<'_, '_>, b: &mut Budget<'_>) -> Result<ItemState, IoError> {
    Ok(ItemState {
        item: reference,
        name: l.text(&d.name, b)?,
        custom_name: false,
        visible: d.visible,
        locks: ItemLocks { content: d.lock_content, position: d.lock_position, alpha: d.lock_alpha, visibility: d.lock_visibility },
        parent: None,
        source_tattoo: d.tattoo,
        source_flags: d.group_flags,
        raster_bounds: Some(bounds(d)?),
        mask,
        composition: None,
    })
}
/// Private mapping only. Both callbacks must debit the same enclosing ledger;
/// its complete decoded backing lease is separate and remains held throughout.
/// reserve takes resident/index quotas; charge prepays local work without refund.
#[allow(clippy::too_many_arguments)]
pub fn map_layer_selection<'a>(
    file: &'a XcfFile,
    ordinal: usize,
    ids: LayerSelectionIds,
    existing: LayerSelectionNamespace<'_>,
    limits: LayerSelectionLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<LayerSelectionCandidate<'a>, IoError> {
    map_layer_selection_with_outer_policy(file, ordinal, ids, LayerOuterPolicy::ObservedLiteral, existing, limits, ctl, reserve, charge)
}
/// Explicit source-only structural tuple retention. Old entry remains literal.
#[allow(clippy::too_many_arguments)]
pub fn map_layer_selection_with_outer_policy<'a>(
    file: &'a XcfFile,
    ordinal: usize,
    ids: LayerSelectionIds,
    outer_policy: LayerOuterPolicy,
    existing: LayerSelectionNamespace<'_>,
    limits: LayerSelectionLimits,
    ctl: &Interrupt<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<LayerSelectionCandidate<'a>, IoError> {
    poll(ctl)?;
    let maximum = LayerSelectionLimits::default();
    if limits.resident_bytes > maximum.resident_bytes
        || limits.metadata_bytes > maximum.metadata_bytes
        || limits.index_bytes > maximum.index_bytes
        || limits.work_units > maximum.work_units
        || limits.namespace_items > maximum.namespace_items
    {
        return Err(fail("lower-only limits"));
    }
    if file.version != 22 || file.model != Model::Gray || file.encoding != Encoding::NonlinearProfile || !matches!(file.sample, Sample::U8 | Sample::F64) {
        return Err(fail("closed source descriptor"));
    }
    let layer = file.layers.get(ordinal).ok_or_else(|| fail("layer ordinal"))?;
    if layer.width != 32
        || layer.height != 32
        || layer.offset != (0, 0)
        || layer.model != Model::Gray
        || layer.alpha
        || layer.group
        || layer.mask.is_some()
        || layer.link.is_some()
        || layer.text.is_some()
        || layer.vector.is_some()
        || layer.transform.is_some()
        || layer.floating_attachment.is_some()
        || layer.effects.len() != 1
        || layer.path.len() > 1
        || layer.unsupported.len() > 1
        || layer.unsupported.iter().any(|s| s != "GEGL layer effects")
    {
        return Err(fail("selected layer shape"));
    }
    let effect = layer.effects.first().ok_or_else(|| fail("layer effect"))?;
    let mask = effect.mask.as_deref().ok_or_else(|| fail("retained effect mask"))?;
    outer_words(effect.state.blend_space, effect.state.composite_space, effect.state.composite_mode, outer_policy)?;
    if effect.operation != "gegl:gaussian-blur"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 5
        || effect.properties.len() != 13
        || effect.state.enabled != Some(true)
        || effect.state.opacity_bits != Some(1f32.to_bits())
        || effect.state.opacity_u8.is_some()
        || effect.state.mode != Some(62)
        || effect.state.region != Some(0)
        || effect.state.clipping != Some(0)
    {
        return Err(fail("observed source effect tuple"));
    }
    if mask.width != 32
        || mask.height != 32
        || mask.model != Model::Gray
        || mask.alpha
        || mask.group
        || mask.mask.is_some()
        || !mask.effects.is_empty()
        || !mask.unsupported.is_empty()
        || mask.link.is_some()
        || mask.text.is_some()
        || mask.vector.is_some()
        || mask.transform.is_some()
    {
        return Err(fail("mask descriptor"));
    }
    let source_plane = layer.plane.as_ref().ok_or_else(|| fail("layer native plane"))?;
    let mask_plane = mask.plane.as_ref().ok_or_else(|| fail("mask native plane"))?;
    for p in [source_plane, mask_plane] {
        if p.width != 32
            || p.height != 32
            || p.channels != 1
            || p.sample != file.sample
            || p.bytes.len()
                != 32usize
                    .checked_mul(32)
                    .and_then(|n| n.checked_mul(if file.sample == Sample::F64 { 8 } else { 1 }))
                    .ok_or_else(|| fail("native bytes overflow"))?
        {
            return Err(fail("native plane descriptor"));
        }
    }
    reserve(limits.resident_bytes, limits.index_bytes)?;
    charge(limits.work_units)?;
    let mut b = Budget { used: 0, max: usize::try_from(limits.resident_bytes).map_err(|_| fail("resident address space"))?, peak: 0, shared: None };
    let mut l = Local { limits, metadata: 0, index: 0, work: 0, ctl };
    l.metadata(std::mem::size_of::<LayerSelectionCandidate<'a>>() as u64, &mut b)?;
    namespace(ids, existing, &mut l, &mut b)?;
    // Count and validate every raw state word before allocating canonical node.
    let mut tags = 0u64;
    let mut argument_count = 0usize;
    for p in &effect.properties {
        l.step(1)?;
        if p.tag == 45 {
            argument_count += 1;
            continue;
        }
        let slot = match p.tag {
            8 => 0,
            33 => 1,
            7 => 2,
            37 => 3,
            36 => 4,
            35 => 5,
            46 => 6,
            44 => 7,
            _ => return Err(fail("unobserved outer property")),
        };
        let bit = 1u64 << slot;
        if tags & bit != 0 || p.payload.len() != 4 {
            return Err(fail("outer property duplicate/size"));
        }
        tags |= bit;
        let expected = match p.tag {
            8 => 1,
            33 => 1f32.to_bits(),
            7 => 62,
            35 => effect.state.composite_mode.ok_or_else(|| fail("missing raw operator"))? as u32,
            36 => effect.state.composite_space.ok_or_else(|| fail("missing raw composite space"))? as u32,
            37 => effect.state.blend_space.ok_or_else(|| fail("missing raw blend space"))? as u32,
            44 | 46 => 0,
            _ => return Err(fail("outer property")),
        };
        if word(&p.payload, 0)? != expected {
            return Err(fail("raw/decoded tuple mismatch"));
        }
    }
    if tags != 255 || argument_count != 5 {
        return Err(fail("outer property completeness"));
    }
    let mut args = l.vec::<FilterArgument>(5, &mut b)?;
    for (a, name) in effect.arguments.iter().zip(["std-dev-x", "std-dev-y", "filter", "abyss-policy", "clip-extent"]) {
        if a.name != name {
            return Err(fail("stored argument order"));
        }
        let value = argument(a, effect, &mut l, &mut b)?;
        let accepted = match (&value, name) {
            (FilterValue::Number(ExactNumber::Float32Bits(x)), "std-dev-x") => [1f32.to_bits(), 3.75f32.to_bits()].contains(x),
            (FilterValue::Number(ExactNumber::Float32Bits(x)), "std-dev-y") => *x == 1f32.to_bits(),
            (FilterValue::Enum { value, .. }, "filter") => *value == 0,
            (FilterValue::Enum { value, .. }, "abyss-policy") => *value == 1,
            (FilterValue::Bool(true), "clip-extent") => true,
            _ => false,
        };
        if !accepted {
            return Err(fail("unobserved literal argument"));
        }
        args.push(FilterArgument { name: l.text(name, &mut b)?, value });
    }
    let r = bounds(layer)?;
    let mr = bounds(mask)?;
    // Exact native ingestion: no unit clipping, alpha conversion or F32 narrowing.
    l.step(mul(add(source_plane.bytes.len() as u64, mask_plane.bytes.len() as u64)?, 8)?)?;
    let source_surface = surface(&mut b, source_plane, r, ColorMode::Grayscale, false, &source_plane.bytes, ctl)?;
    let mask_surface = surface(&mut b, mask_plane, mr, ColorMode::Grayscale, false, &mask_plane.bytes, ctl)?;
    let node = FilterNode {
        id: ids.filter,
        operation: OperationIdentity {
            name: l.text(&effect.operation, &mut b)?,
            stored_version: l.text("0:0", &mut b)?,
            schema_epoch: l.text(LAYER_SELECTION_EPOCH, &mut b)?,
        },
        source_display_name: Some(l.text(&effect.name, &mut b)?),
        source_icon: Some(l.text(&effect.icon, &mut b)?),
        arguments: args,
        enabled: true,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: effect.state.clipping,
        source_region: effect.state.region,
        mask: Some(ItemRef::Mask(ids.mask)),
        auxiliary: Vec::new(),
        composition: Some(photocraft_doc::import_compositing::GimpComposition {
            compatibility_epoch: l.text(LAYER_SELECTION_EPOCH, &mut b)?,
            source_mode: 62,
            legacy: false,
            stored_blend_space: effect.state.blend_space.ok_or_else(|| fail("missing stored blend space"))?,
            stored_composite_space: effect.state.composite_space.ok_or_else(|| fail("missing stored composite space"))?,
            stored_operator: effect.state.composite_mode.ok_or_else(|| fail("missing stored operator"))?,
            resolved: None,
            pass_through: false,
        }),
    };
    let mut state =
        photocraft_doc::XcfState { items: l.vec(2, &mut b)?, filter_stacks: l.vec(1, &mut b)?, filter_masks: l.vec(1, &mut b)?, ..Default::default() };
    state.items.push(item(layer, ItemRef::Layer(ids.layer), None, &mut l, &mut b)?);
    state.items.push(item(
        mask,
        ItemRef::Mask(ids.mask),
        Some(MaskState {
            owner: ItemRef::Layer(ids.layer),
            bounds: mr,
            offset: [mask.offset.0, mask.offset.1],
            outside_f64_bits: 0f64.to_bits(),
            enabled: mask.mask_enabled,
            shown: mask.mask_shown,
            editing: mask.mask_editing,
            linked: mask.legacy_linked,
        }),
        &mut l,
        &mut b,
    )?);
    let mut nodes = l.vec(1, &mut b)?;
    nodes.push(node);
    state.filter_stacks.push(FilterStack { owner: ItemRef::Layer(ids.layer), nodes, revision: 0 });
    state.filter_masks.push(photocraft_doc::FilterMask { id: ids.mask, owner: ids.filter, surface: mask_surface, bounds: mr });
    raw(&layer.raw_properties, ItemRef::Layer(ids.layer), &mut state, &mut l, &mut b)?;
    raw(&effect.properties, ItemRef::Layer(ids.layer), &mut state, &mut l, &mut b)?;
    raw(&mask.raw_properties, ItemRef::Mask(ids.mask), &mut state, &mut l, &mut b)?;
    poll(ctl)?;
    let requirements = LayerSelectionRequirements {
        reserved_resident_bytes: limits.resident_bytes,
        reserved_index_bytes: limits.index_bytes,
        reserved_work_units: limits.work_units,
        known_resident_bytes: b.peak as u64,
        known_index_bytes: l.index,
        work_units: l.work,
    };
    Ok(LayerSelectionCandidate { source_surface, source_bounds: r, state, original_layer: layer, original_effect: effect, original_mask: mask, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_outer_policy_retains_signed_words_without_literal_aliasing() {
        let successor = 0xe67b8251u32 as i32;
        assert_eq!(outer_words(Some(successor), Some(i32::MIN), Some(i32::MAX), LayerOuterPolicy::RetainUnresolved).unwrap(), (successor, i32::MIN, i32::MAX));
        assert!(outer_words(Some(successor), Some(successor), Some(successor), LayerOuterPolicy::ObservedLiteral).is_err());
        assert!(outer_words(None, Some(successor), Some(successor), LayerOuterPolicy::RetainUnresolved).is_err());
        assert_eq!(word(&successor.to_be_bytes(), 0).unwrap(), successor as u32);
        assert!(word(&successor.to_be_bytes()[..3], 0).is_err());
    }
    #[test]
    fn f64_native_ingestion_preserves_nonunit_signed_zero_and_subnormal_words() {
        let words = [1.0000000361224046f64.to_bits(), (-0f64).to_bits(), 1u64];
        let bytes = words.into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let plane = Plane { width: 3, height: 1, channels: 1, sample: Sample::F64, bytes: bytes.clone() };
        let mut b = Budget { used: 0, max: 16 << 20, peak: 0, shared: None };
        let out = surface(&mut b, &plane, Rect::new(0, 0, 3, 1), ColorMode::Grayscale, false, &bytes, &Interrupt::NONE).unwrap();
        for (x, expected) in words.into_iter().enumerate() {
            let mut raw = [0u8; 8];
            out.try_read_pixel_bytes(x as i32, 0, &mut raw).unwrap();
            assert_eq!(u64::from_ne_bytes(raw), expected);
        }
    }
    #[test]
    fn malformed_raw_property_declared_length_is_refused() {
        let limits = LayerSelectionLimits::default();
        let mut l = Local { limits, metadata: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        let mut b = Budget { used: 0, max: limits.resident_bytes as usize, peak: 0, shared: None };
        let mut state = photocraft_doc::XcfState::default();
        let property = photocraft_xcf::RawProperty { tag: 8, offset: 10, declared_bytes: 4, payload: vec![0, 1] };
        assert!(raw(&[property], ItemRef::Layer(LayerId(1)), &mut state, &mut l, &mut b).is_err());
        assert!(state.opaque.is_empty());
    }
    #[test]
    fn identity_alias_and_namespace_overflow_are_refused() {
        let limits = LayerSelectionLimits::default();
        let mut l = Local { limits, metadata: 0, index: 0, work: 0, ctl: &Interrupt::NONE };
        let mut b = Budget { used: 0, max: limits.resident_bytes as usize, peak: 0, shared: None };
        assert!(
            namespace(
                LayerSelectionIds { layer: LayerId(1), filter: FilterId(1), mask: MaskId(3) },
                LayerSelectionNamespace { layers: &[LayerId(1)], filters: &[], masks: &[] },
                &mut l,
                &mut b
            )
            .is_err()
        );
        assert!(mul(u64::MAX, 64).is_err());
    }
    #[test]
    fn cancellation_precedes_namespace_allocation() {
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        let limits = LayerSelectionLimits::default();
        let mut l = Local { limits, metadata: 0, index: 0, work: 0, ctl: &ctl };
        let mut b = Budget { used: 0, max: limits.resident_bytes as usize, peak: 0, shared: None };
        assert!(matches!(
            namespace(
                LayerSelectionIds { layer: LayerId(1), filter: FilterId(2), mask: MaskId(3) },
                LayerSelectionNamespace { layers: &[LayerId(1)], filters: &[], masks: &[] },
                &mut l,
                &mut b
            ),
            Err(IoError::Cancelled)
        ));
    }
}
