//! Opt-in stored-channel metadata mapping, not appearance/evaluator admission.
//! Default import never calls this module. The decoded originals remain borrowed
//! intact so an explicit owner can retain channel/effect properties, arg45 bytes
//! and the actual decoded mask without guessing a record-offset identity.
//!
//! ENCLOSING OWNER RULE: before map(), the caller must charge ALL backing of
//! the borrowed Drawable/Effect/mask/namespace inputs to its shared ledger,
//! including every container capacity, nested metadata/resource and generic
//! plane/provider. Keep that charge alive for the candidate's entire lifetime.
//! This module acquires no borrowed ownership and does not census that backing.
//! Its separate ledger covers new output/index allocations plus selected
//! metadata and scratch charges only; selected borrowed charges deliberately
//! remain conservative additions, never refunds of the enclosing reservation.
use crate::IoError;
use photocraft_doc::{
    SampleEncoding, SampleType,
    import_compositing::GimpComposition,
    item_identity::{ChannelId, FilterId, ItemRef, MaskId},
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterStack, FilterValue, OperationIdentity},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Drawable, Effect, RawProperty};
use std::collections::HashSet;

/// Source-only structural epoch. No registry/evaluator accepts this as rendered.
pub const STORED_CHANNEL_EPOCH: &str = "xcf26-stored-channel-gaussian-unresolved-v1";
/// Physical serialized argument type 5; deliberately not a GEGL enum domain.
pub const RAW_ENUM_DOMAIN: &str = "xcf/raw-argument-type-5";
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Remaining output/index + selected metadata/scratch allowance AFTER the
    /// enclosing owner has reserved all borrowed input backing separately.
    pub metadata_bytes: u64,
    pub work_units: u64,
    pub namespace_items: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { metadata_bytes: 1 << 20, work_units: 100_000, namespace_items: 10_000 }
    }
}
#[derive(Clone, Copy)]
pub struct Ids {
    pub channel: ChannelId,
    pub filter: FilterId,
    pub mask: MaskId,
}
/// Channel has already been declared once; new same-kind filter/mask IDs must
/// be absent. No record offset/name is ever converted into a canonical ID here.
pub struct Namespace<'a> {
    pub channels: &'a [ChannelId],
    pub filters: &'a [FilterId],
    pub masks: &'a [MaskId],
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    /// Reserved by charge() in the enclosing shared ledger. This is additional
    /// to its mandatory reservation of all decoded input backing.
    pub reserved_metadata_bytes: u64,
    /// Cumulative conservative charges for selected borrowed metadata,
    /// output capacities and private index/scratch. Not complete input backing,
    /// distinct live bytes, allocator usage, RSS or newly acquired ownership.
    pub known_metadata_bytes: u64,
    pub work_units: u64,
}
/// Owns only the mapped stack. The originals are borrowed; their complete
/// backing reservation must remain held by the enclosing owner, including
/// capacities beyond slices exposed by these metadata accessors.
pub struct StoredChannelCandidate<'a, P> {
    pub stack: FilterStack,
    pub channel: &'a Drawable<P>,
    pub effect: &'a Effect<P>,
    pub mask: &'a Drawable<P>,
    pub requirements: Requirements,
}
impl<P> StoredChannelCandidate<'_, P> {
    /// Original borrowed view; never transfers or refunds its backing charge.
    pub fn channel_properties(&self) -> &[RawProperty] {
        &self.channel.raw_properties
    }
    /// Original borrowed view covered by the enclosing-owner rule.
    pub fn effect_properties(&self) -> &[RawProperty] {
        &self.effect.properties
    }
    /// Includes original unresolved typed values/envelopes; backing stays owned
    /// and fully charged by the caller, regardless of this slice's length.
    pub fn original_arguments(&self) -> &[Argument] {
        &self.effect.arguments
    }
    /// Does not grant mask evaluation or acquire original mask ownership.
    pub fn mask_properties(&self) -> &[RawProperty] {
        &self.mask.raw_properties
    }
    /// The original 96x80 mask is not resized to the 32x32 channel. Its record
    /// offset/opacity are inert source metadata; caller IDs stay independent.
    pub fn original_mask_descriptor(&self) -> (u32, u32, (i32, i32), u32, u64) {
        (self.mask.width, self.mask.height, self.mask.offset, self.mask.opacity.to_bits(), self.mask.record_offset)
    }
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("XCF stored-channel candidate: {message}"))
}
struct Budget<'a, 'i> {
    limits: Limits,
    bytes: u64,
    work: u64,
    control: &'a Interrupt<'i>,
}
impl Budget<'_, '_> {
    fn step(&mut self, n: u64) -> Result<(), IoError> {
        self.control.check().map_err(|_| IoError::Cancelled)?;
        self.work = self.work.checked_add(n).filter(|n| *n <= self.limits.work_units).ok_or_else(|| bad("work limit"))?;
        Ok(())
    }
    fn add(&mut self, n: u64) -> Result<(), IoError> {
        self.bytes = self.bytes.checked_add(n).filter(|n| *n <= self.limits.metadata_bytes).ok_or_else(|| bad("metadata limit"))?;
        Ok(())
    }
    fn text(&mut self, value: &str) -> Result<String, IoError> {
        self.step(value.len() as u64 + 1)?;
        if value.len() > 256 || value.contains('\0') {
            return Err(bad("string bound"));
        }
        self.add(value.len() as u64)?;
        let mut output = String::new();
        output.try_reserve_exact(value.len()).map_err(|_| bad("string allocation"))?;
        self.add(output.capacity().saturating_sub(value.len()) as u64)?;
        output.push_str(value);
        Ok(output)
    }
    fn originals(&mut self, properties: &Vec<RawProperty>) -> Result<(), IoError> {
        if properties.len() > 32 {
            return Err(bad("raw property count"));
        }
        self.add((properties.capacity() as u64).checked_mul(std::mem::size_of::<RawProperty>() as u64).ok_or_else(|| bad("property capacity overflow"))?)?;
        for property in properties {
            self.step(1)?;
            if property.payload.len() > 64 * 1024 || property.declared_bytes as usize != property.payload.len() {
                return Err(bad("raw property envelope"));
            }
            self.add(property.payload.capacity() as u64)?;
        }
        Ok(())
    }
}
fn namespace(ids: Ids, source: &Namespace<'_>, budget: &mut Budget<'_, '_>) -> Result<(), IoError> {
    if ids.channel.0 == 0 || ids.filter.0 == 0 || ids.mask.0 == 0 {
        return Err(bad("zero caller identity"));
    }
    if source.channels.len() > budget.limits.namespace_items
        || source.filters.len() > budget.limits.namespace_items
        || source.masks.len() > budget.limits.namespace_items
    {
        return Err(bad("namespace count"));
    }
    let maximum = source.channels.len().max(source.filters.len()).max(source.masks.len());
    let prospective = (maximum as u64).checked_mul(64).ok_or_else(|| bad("namespace capacity overflow"))?;
    // Scratch is charged before reserve and reconciled to the actual capacity.
    budget.add(prospective)?;
    let mut seen = HashSet::new();
    seen.try_reserve(maximum).map_err(|_| bad("namespace allocation"))?;
    let actual = (seen.capacity() as u64).checked_mul(64).ok_or_else(|| bad("namespace capacity overflow"))?;
    budget.add(actual.saturating_sub(prospective))?;
    for family in 0..3 {
        seen.clear();
        let length = match family {
            0 => source.channels.len(),
            1 => source.filters.len(),
            _ => source.masks.len(),
        };
        let expected = match family {
            0 => ids.channel.0,
            1 => ids.filter.0,
            _ => ids.mask.0,
        };
        let mut matches = 0usize;
        for index in 0..length {
            budget.step(1)?;
            let id = match family {
                0 => source.channels.get(index).map(|id| id.0),
                1 => source.filters.get(index).map(|id| id.0),
                _ => source.masks.get(index).map(|id| id.0),
            }
            .ok_or_else(|| bad("namespace index"))?;
            if id == 0 || !seen.insert(id) {
                return Err(bad("duplicate/zero same-kind namespace"));
            }
            if id == expected {
                matches += 1;
            }
        }
        if (family == 0 && matches != 1) || (family != 0 && matches != 0) {
            return Err(bad("caller identity declaration/collision"));
        }
    }
    Ok(())
}
fn property_tags(count: usize, budget: &mut Budget<'_, '_>) -> Result<HashSet<u32>, IoError> {
    if count != 13 {
        return Err(bad("observed outer property count"));
    }
    let prospective = (count as u64).checked_mul(64).ok_or_else(|| bad("property namespace overflow"))?;
    budget.add(prospective)?;
    let mut tags = HashSet::new();
    tags.try_reserve(count).map_err(|_| bad("property namespace allocation"))?;
    let actual = (tags.capacity() as u64).checked_mul(64).ok_or_else(|| bad("property namespace overflow"))?;
    budget.add(actual.saturating_sub(prospective))?;
    Ok(tags)
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, IoError> {
    let end = offset.checked_add(4).ok_or_else(|| bad("argument offset overflow"))?;
    let slice = bytes.get(offset..end).ok_or_else(|| bad("short raw argument"))?;
    let array: [u8; 4] = slice.try_into().map_err(|_| bad("raw word width"))?;
    Ok(u32::from_be_bytes(array))
}
fn scalar(argument: &Argument, effect: &Effect<impl Sized>, budget: &mut Budget<'_, '_>) -> Result<FilterValue, IoError> {
    budget.step(1)?;
    if argument.configuration.is_some() || argument.graph.is_some() {
        return Err(bad("nested argument is not the observed scalar schema"));
    }
    let property = effect.properties.get(argument.property_index).ok_or_else(|| bad("argument property identity"))?;
    if property.tag != 45 {
        return Err(bad("argument does not reference raw tag45"));
    }
    let bytes = &property.payload;
    let name_len = usize::try_from(word(bytes, 0)?).map_err(|_| bad("argument name width"))?;
    let name_end = 4usize.checked_add(name_len).ok_or_else(|| bad("argument name overflow"))?;
    let name = bytes.get(4..name_end).and_then(|name| name.strip_suffix(&[0])).ok_or_else(|| bad("argument name envelope"))?;
    let expected_end = name_end.checked_add(8).ok_or_else(|| bad("argument size overflow"))?;
    if name != argument.name.as_bytes() || bytes.len() != expected_end {
        return Err(bad("argument raw/typed identity mismatch"));
    }
    let kind = word(bytes, name_end)?;
    let value = word(bytes, name_end + 4)?;
    match (&argument.value, kind) {
        (ArgumentValue::Float32Bits(bits), 3) if *bits == value => Ok(FilterValue::Number(ExactNumber::Float32Bits(*bits))),
        (ArgumentValue::Boolean(flag), 2) if value == u32::from(*flag) => Ok(FilterValue::Bool(*flag)),
        (ArgumentValue::Enum(number), 5) if *number == value as i32 => Ok(FilterValue::Enum { domain: budget.text(RAW_ENUM_DOMAIN)?, value: *number }),
        _ => Err(bad("argument numeric type/width differs from raw bytes")),
    }
}
/// Mapping is explicit and source-only. Charge reserves the entire lower caller
/// envelope before allocations; requirements reconcile known retained metadata.
/// Before this call the enclosing owner MUST precharge all borrowed input
/// backing (all Drawable/Effect/mask containers, nested metadata/resources,
/// namespace backing and generic plane/provider). Limits apply only to remaining
/// headroom. charge() must use that SAME shared ledger and retain its separate
/// output reservation alongside the input reservation; no borrowed reservation
/// is refunded. Requirements are the local selected slice, not a full census.
/// No source hashes, file paths, defaults or runtime registry admission.
#[allow(clippy::too_many_arguments)]
pub fn map<'a, P>(
    channel: &'a Drawable<P>,
    version: u32,
    depth: SampleType,
    encoding: SampleEncoding,
    ids: Ids,
    existing: Namespace<'_>,
    limits: Limits,
    control: &Interrupt<'_>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<StoredChannelCandidate<'a, P>, IoError> {
    control.check().map_err(|_| IoError::Cancelled)?;
    let standard = Limits::default();
    if limits.metadata_bytes > standard.metadata_bytes || limits.work_units > standard.work_units || limits.namespace_items > standard.namespace_items {
        return Err(bad("limits must be lower-only"));
    }
    if version != 26
        || !matches!(depth, SampleType::U8 | SampleType::F64)
        || encoding != SampleEncoding::NonlinearProfile
        || channel.width != 32
        || channel.height != 32
        || channel.offset != (0, 0)
        || channel.alpha
        || channel.model != photocraft_xcf::Model::Gray
        || channel.visible
        || channel.selection
        || channel.group
        || channel.plane.is_none()
        || channel.mask.is_some()
        || channel.link.is_some()
        || channel.text.is_some()
        || channel.vector.is_some()
        || channel.transform.is_some()
        || channel.effects.len() != 1
        || channel.unsupported.len() != 1
        || channel.unsupported.first().is_none_or(|value| value != "GEGL layer effects")
    {
        return Err(bad("outside observed stored saved-channel envelope"));
    }
    let effect = channel.effects.first().ok_or_else(|| bad("missing stored effect"))?;
    let mask = effect.mask.as_deref().ok_or_else(|| bad("actual decoded effect mask is required"))?;
    let state = &effect.state;
    if effect.operation != "gegl:gaussian-blur"
        || effect.version.as_deref() != Some("0:0")
        || effect.arguments.len() != 5
        || effect.properties.len() != 13
        || state.enabled != Some(true)
        || state.opacity_bits != Some(1f32.to_bits())
        || state.opacity_u8.is_some()
        || state.mode != Some(62)
        || state.blend_space != Some(239108689)
        || state.composite_space != Some(239108689)
        || state.composite_mode != Some(239108689)
        || state.clipping != Some(0)
        || state.region != Some(1)
        || mask.record_offset == 0
        || mask.width != 96
        || mask.height != 80
        || mask.model != photocraft_xcf::Model::Gray
        || mask.alpha
        || mask.plane.is_none()
        || !mask.effects.is_empty()
        || !mask.unsupported.is_empty()
    {
        return Err(bad("outside observed effect/mask record envelope"));
    }
    charge(limits.metadata_bytes)?;
    let mut budget = Budget { limits, bytes: 0, work: 0, control };
    // Selected decoded capacities are conservative local additions. ALL
    // borrowed backing (not just P) was separately reserved by the caller;
    // this partial slice never replaces or refunds that enclosing reservation.
    for text in [&channel.name, &effect.name, &effect.icon, &effect.operation, &mask.name] {
        if text.len() > 256 {
            return Err(bad("original metadata string bound"));
        }
        budget.step(text.len() as u64 + 1)?;
        budget.add(text.capacity() as u64)?;
    }
    if let Some(version) = &effect.version {
        budget.add(version.capacity() as u64)?;
    }
    budget.add(
        (effect.arguments.capacity() as u64).checked_mul(std::mem::size_of::<Argument>() as u64).ok_or_else(|| bad("original argument capacity overflow"))?,
    )?;
    for argument in &effect.arguments {
        budget.step(1)?;
        budget.add(argument.name.capacity() as u64)?;
    }
    namespace(ids, &existing, &mut budget)?;
    budget.originals(&channel.raw_properties)?;
    budget.originals(&effect.properties)?;
    budget.originals(&mask.raw_properties)?;
    let mut tags = property_tags(effect.properties.len(), &mut budget)?;
    let mut arguments_seen = 0usize;
    for property in &effect.properties {
        budget.step(1)?;
        if property.tag == 45 {
            arguments_seen += 1;
            continue;
        }
        if !tags.insert(property.tag) || property.payload.len() != 4 {
            return Err(bad("duplicate/invalid outer effect property"));
        }
        let expected = match property.tag {
            8 | 44 => 1,
            33 => 1f32.to_bits(),
            7 => 62,
            35 | 36 | 37 => 239108689,
            46 => 0,
            _ => return Err(bad("unknown outer effect property")),
        };
        if word(&property.payload, 0)? != expected {
            return Err(bad("raw outer tuple differs from decoded observed fields"));
        }
    }
    if arguments_seen != 5 {
        return Err(bad("unmapped raw argument properties"));
    }
    let mut arguments = Vec::new();
    budget.add(5 * std::mem::size_of::<FilterArgument>() as u64)?;
    arguments.try_reserve_exact(5).map_err(|_| bad("argument allocation"))?;
    budget.add(
        (arguments.capacity().saturating_sub(5) as u64)
            .checked_mul(std::mem::size_of::<FilterArgument>() as u64)
            .ok_or_else(|| bad("argument capacity overflow"))?,
    )?;
    for (source, name) in effect.arguments.iter().zip(["std-dev-x", "std-dev-y", "filter", "abyss-policy", "clip-extent"]) {
        if source.name != name {
            return Err(bad("source scalar declaration order"));
        }
        let value = scalar(source, effect, &mut budget)?;
        let observed = match (&value, name) {
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), "std-dev-x") => [1f32.to_bits(), 3.75f32.to_bits()].contains(bits),
            (FilterValue::Number(ExactNumber::Float32Bits(bits)), "std-dev-y") => *bits == 1f32.to_bits(),
            (FilterValue::Enum { value, .. }, "filter") => *value == 0,
            (FilterValue::Enum { value, .. }, "abyss-policy") => *value == 1,
            (FilterValue::Bool(true), "clip-extent") => true,
            _ => false,
        };
        if !observed {
            return Err(bad("outside observed literal scalar values"));
        }
        arguments.push(FilterArgument { name: budget.text(name)?, value });
    }
    let operation =
        OperationIdentity { name: budget.text(&effect.operation)?, stored_version: budget.text("0:0")?, schema_epoch: budget.text(STORED_CHANNEL_EPOCH)? };
    let node = FilterNode {
        id: ids.filter,
        operation,
        source_display_name: Some(budget.text(&effect.name)?),
        source_icon: Some(budget.text(&effect.icon)?),
        arguments,
        enabled: true,
        opacity_f64_bits: 1f64.to_bits(),
        source_clipping: state.clipping,
        source_region: state.region,
        mask: Some(ItemRef::Mask(ids.mask)),
        auxiliary: Vec::new(),
        composition: Some(GimpComposition {
            compatibility_epoch: budget.text(STORED_CHANNEL_EPOCH)?,
            source_mode: 62,
            legacy: false,
            stored_blend_space: 239108689,
            stored_composite_space: 239108689,
            stored_operator: 239108689,
            resolved: None,
            pass_through: false,
        }),
    };
    let mut nodes = Vec::new();
    budget.add(std::mem::size_of::<FilterNode>() as u64)?;
    nodes.try_reserve_exact(1).map_err(|_| bad("node allocation"))?;
    budget
        .add((nodes.capacity().saturating_sub(1) as u64).checked_mul(std::mem::size_of::<FilterNode>() as u64).ok_or_else(|| bad("node capacity overflow"))?)?;
    nodes.push(node);
    budget.step(1)?;
    Ok(StoredChannelCandidate {
        stack: FilterStack { owner: ItemRef::Channel(ids.channel), nodes, revision: 0 },
        channel,
        effect,
        mask,
        requirements: Requirements { reserved_metadata_bytes: limits.metadata_bytes, known_metadata_bytes: budget.bytes, work_units: budget.work },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tag_index_lower_metadata_refuses_before_reserve_and_reconciles_capacity() {
        let limits = Limits { metadata_bytes: 13 * 64 - 1, ..Limits::default() };
        let mut budget = Budget { limits, bytes: 0, work: 0, control: &Interrupt::NONE };
        assert!(property_tags(13, &mut budget).is_err());
        assert_eq!(budget.bytes, 0);
        let mut budget = Budget { limits: Limits::default(), bytes: 0, work: 0, control: &Interrupt::NONE };
        assert!(property_tags(14, &mut budget).is_err());
        assert_eq!(budget.bytes, 0);
        let tags = property_tags(13, &mut budget).unwrap();
        assert!(tags.is_empty());
        assert_eq!(budget.bytes, tags.capacity() as u64 * 64);
    }
    #[test]
    fn namespace_refuses_collisions_and_cross_kind_is_not_an_alias() {
        let control = Interrupt::NONE;
        let mut budget = Budget { limits: Limits::default(), bytes: 0, work: 0, control: &control };
        let ids = Ids { channel: ChannelId(7), filter: FilterId(7), mask: MaskId(7) };
        namespace(ids, &Namespace { channels: &[ChannelId(7)], filters: &[], masks: &[] }, &mut budget).unwrap();
        assert!(namespace(ids, &Namespace { channels: &[ChannelId(7)], filters: &[FilterId(7)], masks: &[] }, &mut budget).is_err());
        assert!(namespace(ids, &Namespace { channels: &[ChannelId(7), ChannelId(7)], filters: &[], masks: &[] }, &mut budget).is_err());
    }
    #[test]
    fn raw_scalar_width_and_name_envelope_are_not_guessed() {
        let name = b"std-dev-x\0";
        let mut payload = Vec::new();
        payload.extend_from_slice(&(name.len() as u32).to_be_bytes());
        payload.extend_from_slice(name);
        payload.extend_from_slice(&3u32.to_be_bytes());
        payload.extend_from_slice(&1f32.to_bits().to_be_bytes());
        let mut effect = Effect::<()> {
            name: "Own".into(),
            icon: "".into(),
            operation: "gegl:gaussian-blur".into(),
            version: Some("0:0".into()),
            properties: vec![RawProperty { tag: 45, offset: 1, declared_bytes: payload.len() as u32, payload }],
            arguments: Vec::new(),
            mask: None,
            state: Default::default(),
        };
        let argument =
            Argument { name: "std-dev-x".into(), property_index: 0, value: ArgumentValue::Float32Bits(1f32.to_bits()), configuration: None, graph: None };
        let mut budget = Budget { limits: Limits::default(), bytes: 0, work: 0, control: &Interrupt::NONE };
        assert_eq!(scalar(&argument, &effect, &mut budget).unwrap(), FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())));
        effect.properties[0].payload[0] = 0xff;
        assert!(scalar(&argument, &effect, &mut budget).is_err());
    }
}
