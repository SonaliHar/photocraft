//! Candidate metadata for the observed stored-channel selection consumer.
//!
//! An explicit binding hashes one of the four observed original projects and
//! borrows its original typed node. Stored reads fence current namespace and
//! original node equality without interpreting opaque outer words. They never
//! authorize selection publication, appearance evaluation or bake. Default/raw
//! Load admission is unchanged. A thumbnail is never native sample authority.
use crate::{EngineError, Result};
use photocraft_doc::{
    Document, SampleEncoding, SampleType,
    item_identity::{ChannelId, ItemRef},
    xcf_filters::{ExactNumber, FilterNode, FilterValue},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelConsumer {
    StoredSelectionRead,
    FilteredAppearance,
    Bake,
}
/// Borrowed structural candidate. Its operation includes the actual current
/// epoch/version and composition includes the actual stored tuple, if present;
/// neither is compared to an invented research binding here.
pub struct StoredSelectionCandidate<'a> {
    pub channel: ChannelId,
    pub bounds: Rect,
    pub format: photocraft_color::PixelFormat,
    pub filter: &'a FilterNode,
}
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.loadSelection".into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
/// Inspect only the measured 32x32 U8/F64 nonlinear saved-channel shape.
/// Unknown or different shapes return None. Malformed identity/count/cancel
/// errors stay explicit. No sample scans, cloning, renderer work or publication.
pub fn inspect_candidate<'a>(
    document: &'a Document,
    id: ChannelId,
    consumer: ChannelConsumer,
    control: &Interrupt<'_>,
) -> Result<Option<StoredSelectionCandidate<'a>>> {
    poll(control)?;
    if consumer != ChannelConsumer::StoredSelectionRead {
        return Err(bad("filtered channel appearance and bake need a qualified native evaluator"));
    }
    if id.0 == 0 {
        return Err(bad("zero saved channel identity"));
    }
    let Some(state) = &document.xcf_state else {
        return Ok(None);
    };
    if document.channels.len() > 10_000
        || state.items.len() > 10_000
        || state.filter_stacks.len() > 10_000
        || state.source_items.len() > 10_000
        || state.resources.len() > 10_000
        || state.filter_masks.len() > 10_000
        || state.opaque.len() > 100_000
    {
        return Err(bad("channel candidate count limit"));
    }
    if document.sample_encoding != Some(SampleEncoding::NonlinearProfile)
        || !matches!(document.depth, SampleType::U8 | SampleType::F64)
        || document.size.width != 32
        || document.size.height != 32
        || state.floating.is_some()
    {
        return Ok(None);
    }
    if document.selection_channel_id == Some(id) || document.quick_mask.as_ref().is_some_and(|channel| channel.id == id) {
        return Err(bad("saved channel aliases a selection role"));
    }
    let mut channel = None;
    for value in &document.channels {
        poll(control)?;
        if value.id == id && channel.replace(value).is_some() {
            return Err(bad("duplicate saved channel identity"));
        }
    }
    let Some(channel) = channel else {
        return Err(bad("missing saved channel identity"));
    };
    let format = channel.surface.format();
    if channel.spot.is_some() || format.mode != photocraft_color::ColorMode::Grayscale || format.alpha || format.sample != document.depth {
        return Ok(None);
    }
    let reference = ItemRef::Channel(id);
    let mut item = None;
    for value in &state.items {
        poll(control)?;
        if value.item == reference && item.replace(value).is_some() {
            return Err(bad("duplicate typed channel owner"));
        }
    }
    let Some(item) = item else {
        return Err(bad("missing typed channel owner"));
    };
    let bounds = Rect::new(0, 0, 32, 32);
    if item.raster_bounds != Some(bounds)
        || item.source_flags != 0
        || item.parent.is_some()
        || item.mask.is_some()
        || item.composition.is_some()
        || item.visible
    {
        return Ok(None);
    }
    for source in &state.source_items {
        poll(control)?;
        if source.owner == reference {
            return Ok(None);
        }
    }
    for record in &state.opaque {
        poll(control)?;
        if record.rendering_required && (record.owner.is_none() || record.owner == Some(reference)) {
            return Ok(None);
        }
    }
    let mut count = 0usize;
    let mut selected = None;
    for stack in &state.filter_stacks {
        poll(control)?;
        count = count.checked_add(stack.nodes.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("channel candidate work limit"))?;
        for node in &stack.nodes {
            poll(control)?;
            count = count.checked_add(node.auxiliary.len()).filter(|n| *n <= 100_000).ok_or_else(|| bad("channel dependency work limit"))?;
        }
        if stack.owner == reference && selected.replace(stack).is_some() {
            return Err(bad("duplicate channel filter stack"));
        }
    }
    let Some(stack) = selected else {
        return Ok(None);
    };
    let Some(filter) = stack.nodes.first().filter(|_| stack.nodes.len() == 1) else {
        return Ok(None);
    };
    if filter.id.0 == 0 || filter.operation.name != "gegl:gaussian-blur" || !filter.enabled || !filter.auxiliary.is_empty() || filter.arguments.len() > 16 {
        return Ok(None);
    }
    if filter.operation.stored_version.len() > 256 || filter.operation.schema_epoch.len() > 256 {
        return Err(bad("channel source identity length limit"));
    }
    poll(control)?;
    Ok(Some(StoredSelectionCandidate { channel: id, bounds, format, filter }))
}

/// Explicit source witness; construction hashes the supplied original bytes.
/// Borrowing the original node does not decode, reinterpret or evaluate it.
pub struct SourceBinding<'a> {
    channel: ChannelId,
    original: &'a FilterNode,
    source_blake3: blake3::Hash,
    source_sha256: &'static str,
    depth: SampleType,
    mask_record_offset: u64,
}
impl SourceBinding<'_> {
    pub fn source_sha256(&self) -> &str {
        self.source_sha256
    }
    /// Opaque source record offset, not a current mask ID or an abyss policy.
    pub fn mask_record_offset(&self) -> u64 {
        self.mask_record_offset
    }
    /// Original mask extent is independent of the 32x32 saved channel. Stored
    /// read validates this relationship but neither samples nor applies it.
    pub fn mask_bounds(&self) -> Rect {
        Rect::new(0, 0, 96, 80)
    }
}
fn scalar_schema(node: &FilterNode, x: u32) -> bool {
    let expected = ["std-dev-x", "std-dev-y", "filter", "abyss-policy", "clip-extent"];
    if node.arguments.len() != expected.len() {
        return false;
    }
    for (argument, name) in node.arguments.iter().zip(expected) {
        if argument.name != name {
            return false;
        }
        let valid = match name {
            "std-dev-x" => argument.value == FilterValue::Number(ExactNumber::Float32Bits(x)),
            "std-dev-y" => argument.value == FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())),
            "filter" | "abyss-policy" => matches!(&argument.value, FilterValue::Enum {domain,value}
                if !domain.is_empty() && domain.len()<=128 && *value==if name=="filter" {0} else {1}),
            "clip-extent" => argument.value == FilterValue::Bool(true),
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    true
}
/// This binds exact observed source bytes and the caller's borrowed original
/// typed mapping. The opaque mode/space words are compared, never resolved.
pub fn bind_source<'a>(bytes: &[u8], channel: ChannelId, original: &'a FilterNode, control: &Interrupt<'_>) -> Result<SourceBinding<'a>> {
    poll(control)?;
    if bytes.is_empty() || bytes.len() > 16_384 || channel.0 == 0 {
        return Err(bad("channel source binding size/identity limit"));
    }
    let sha = format!("{:x}", Sha256::digest(bytes));
    let (source_sha256, depth, x, mask_record_offset) = match sha.as_str() {
        "55c49a2d78207ff67fb354e8f2506703a0570318cb07472614cf978cbc0de3f6" => {
            ("55c49a2d78207ff67fb354e8f2506703a0570318cb07472614cf978cbc0de3f6", SampleType::U8, 1f32.to_bits(), 4289)
        }
        "f4bd99ae659d3062e8fa23762cbc6b78d4d4622ba9e4969a62c87f09d81adfe6" => {
            ("f4bd99ae659d3062e8fa23762cbc6b78d4d4622ba9e4969a62c87f09d81adfe6", SampleType::U8, 3.75f32.to_bits(), 4289)
        }
        "b8291cf6e7a8dedcf38c01a039eed4353c70054914a704f60834a684d9d02eab" => {
            ("b8291cf6e7a8dedcf38c01a039eed4353c70054914a704f60834a684d9d02eab", SampleType::F64, 1f32.to_bits(), 8700)
        }
        "f3477ab5c4f24d9cfb8e69ee9df8f591ca35ae29601753b3abfe8517fb0b7b40" => {
            ("f3477ab5c4f24d9cfb8e69ee9df8f591ca35ae29601753b3abfe8517fb0b7b40", SampleType::F64, 3.75f32.to_bits(), 8700)
        }
        _ => return Err(bad("channel source bytes lack an observed binding")),
    };
    let tuple = original.composition.as_ref().ok_or_else(|| bad("missing original channel outer tuple"))?;
    if original.id.0 == 0
        || original.operation.name != "gegl:gaussian-blur"
        || original.operation.stored_version != "0:0"
        || original.operation.schema_epoch.is_empty()
        || original.operation.schema_epoch.len() > 256
        || !original.enabled
        || original.opacity_f64_bits != 1f64.to_bits()
        || original.source_clipping != Some(0)
        || original.source_region != Some(1)
        || !matches!(original.mask,Some(ItemRef::Mask(id)) if id.0!=0)
        || !original.auxiliary.is_empty()
        || !scalar_schema(original, x)
        || tuple.source_mode != 62
        || tuple.stored_blend_space != 239108689
        || tuple.stored_composite_space != 239108689
        || tuple.stored_operator != 239108689
        || tuple.compatibility_epoch.is_empty()
        || tuple.compatibility_epoch.len() > 256
    {
        return Err(bad("original typed channel node differs from observed source fields"));
    }
    let source_blake3 = blake3::hash(bytes);
    poll(control)?;
    Ok(SourceBinding { channel, original, source_blake3, source_sha256, depth, mask_record_offset })
}
/// Stored Gray bytes only. The caller owns any later selection transaction;
/// this immutable view supplies neither a rendered output nor mutation grant.
pub struct StoredGrayRead<'a> {
    pub channel: ChannelId,
    pub bounds: Rect,
    pub surface: &'a Surface,
}
pub fn read_stored<'a>(document: &'a Document, binding: &SourceBinding<'_>, control: &Interrupt<'_>) -> Result<StoredGrayRead<'a>> {
    let candidate = inspect_candidate(document, binding.channel, ChannelConsumer::StoredSelectionRead, control)?
        .ok_or_else(|| bad("current channel is outside the observed stored-read shape"))?;
    let state = document.xcf_state.as_ref().ok_or_else(|| bad("missing channel source namespace"))?;
    let source_hash = state.provenance.source_hash.as_deref().ok_or_else(|| bad("missing original channel source digest"))?;
    if state.provenance.format != "xcf"
        || state.provenance.format_version != 26
        || state.provenance.container != "plain"
        || source_hash.len() != 64
        || !source_hash.bytes().all(|b| b.is_ascii_hexdigit())
        || !source_hash.eq_ignore_ascii_case(binding.source_blake3.to_hex().as_str())
        || document.depth != binding.depth
        || candidate.filter != binding.original
    {
        return Err(bad("current channel/source node no longer matches the original binding"));
    }
    let ItemRef::Mask(mask_id) = binding.original.mask.ok_or_else(|| bad("missing original typed mask reference"))? else {
        return Err(bad("original channel mask is not a typed mask"));
    };
    if state.filter_masks.len() > 10_000 {
        return Err(bad("filter mask namespace limit"));
    }
    let mut mask = None;
    for value in &state.filter_masks {
        poll(control)?;
        if value.id == mask_id && mask.replace(value).is_some() {
            return Err(bad("duplicate current filter mask"));
        }
    }
    let mask = mask.ok_or_else(|| bad("original channel mask mapping is unavailable"))?;
    if mask.owner != candidate.filter.id || mask.bounds != binding.mask_bounds() {
        return Err(bad("current filter mask owner/bounds changed"));
    }
    let mut metadata = None;
    for value in &state.items {
        poll(control)?;
        if value.item == ItemRef::Mask(mask_id) && metadata.replace(value).is_some() {
            return Err(bad("duplicate current mask namespace"));
        }
    }
    let metadata = metadata.ok_or_else(|| bad("missing current mask metadata"))?;
    let association = metadata.mask.as_ref().ok_or_else(|| bad("missing current mask association"))?;
    if association.owner != ItemRef::Channel(binding.channel)
        || association.bounds != binding.mask_bounds()
        || metadata.raster_bounds != Some(binding.mask_bounds())
    {
        return Err(bad("current mask association/bounds changed"));
    }
    let channel = document.channels.iter().find(|value| value.id == binding.channel).ok_or_else(|| bad("stored channel disappeared"))?;
    let surface = &channel.surface;
    if surface.tile_count() > 1 || surface.default_bytes().len() != candidate.format.bytes_per_pixel() {
        return Err(bad("stored channel tile/default shape limit"));
    }
    let tile_bytes = usize::try_from(photocraft_geom::TILE_SIZE)
        .ok()
        .and_then(|n| n.checked_mul(n))
        .and_then(|n| n.checked_mul(candidate.format.bytes_per_pixel()))
        .ok_or_else(|| bad("stored tile size overflow"))?;
    for (coordinate, tile) in surface.tiles() {
        poll(control)?;
        if coordinate.tx != 0 || coordinate.ty != 0 || tile.bytes().len() != tile_bytes {
            return Err(bad("stored channel tile origin/length mismatch"));
        }
    }
    // The shared typed validator owns global same-kind namespace/relationship
    // checks. This read does not resolve the stored opaque composition tuple.
    document.validate_xcf_state_with(control).map_err(|error| if control.cancelled() { EngineError::Cancelled } else { bad(&error.to_string()) })?;
    poll(control)?;
    Ok(StoredGrayRead { channel: binding.channel, bounds: candidate.bounds, surface })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appearance_is_not_a_stored_selection_candidate() {
        let document = Document::new("Own candidate", photocraft_doc::Size::new(32, 32), photocraft_doc::ColorMode::Rgb, SampleType::U8);
        for consumer in [ChannelConsumer::FilteredAppearance, ChannelConsumer::Bake] {
            assert!(inspect_candidate(&document, ChannelId(1), consumer, &Interrupt::NONE).is_err());
        }
        assert!(inspect_candidate(&document, ChannelId(1), ChannelConsumer::StoredSelectionRead, &Interrupt::NONE).unwrap().is_none());
        assert!(inspect_candidate(&document, ChannelId(0), ChannelConsumer::StoredSelectionRead, &Interrupt::NONE).is_err());
    }
    #[test]
    fn unobserved_source_cannot_become_a_read_binding() {
        use photocraft_doc::{item_identity::FilterId, xcf_filters::OperationIdentity};
        let node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "unproven".into() },
            source_display_name: None,
            source_icon: None,
            arguments: Vec::new(),
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(1),
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        };
        assert!(bind_source(b"not an observed original", ChannelId(1), &node, &Interrupt::NONE).is_err());
        assert!(bind_source(&[], ChannelId(1), &node, &Interrupt::NONE).is_err());
        assert!(bind_source(b"x", ChannelId(0), &node, &Interrupt::NONE).is_err());
    }
    #[test]
    fn scalar_binding_does_not_narrow_numbers_or_guess_enum_values() {
        use photocraft_doc::{
            item_identity::FilterId,
            xcf_filters::{FilterArgument, OperationIdentity},
        };
        let mut node = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "caller-original-mapping".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                FilterArgument { name: "std-dev-x".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) },
                FilterArgument { name: "std-dev-y".into(), value: FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits())) },
                FilterArgument { name: "filter".into(), value: FilterValue::Enum { domain: "caller-original-filter-domain".into(), value: 0 } },
                FilterArgument { name: "abyss-policy".into(), value: FilterValue::Enum { domain: "caller-original-abyss-domain".into(), value: 1 } },
                FilterArgument { name: "clip-extent".into(), value: FilterValue::Bool(true) },
            ],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(1),
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        };
        assert!(scalar_schema(&node, 1f32.to_bits()));
        node.arguments[0].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits()));
        assert!(!scalar_schema(&node, 1f32.to_bits()));
        node.arguments[0].value = FilterValue::Number(ExactNumber::Float32Bits(1f32.to_bits()));
        node.arguments.swap(2, 3);
        assert!(!scalar_schema(&node, 1f32.to_bits()));
    }
    #[test]
    fn original_mask_extent_is_not_channel_extent_or_current_mask_identity() {
        use photocraft_doc::{item_identity::FilterId, xcf_filters::OperationIdentity};
        let original = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gegl:gaussian-blur".into(), stored_version: "0:0".into(), schema_epoch: "unresolved".into() },
            source_display_name: None,
            source_icon: None,
            arguments: Vec::new(),
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(1),
            mask: Some(ItemRef::Mask(photocraft_doc::item_identity::MaskId(9))),
            auxiliary: Vec::new(),
            composition: None,
        };
        let binding = SourceBinding {
            channel: ChannelId(7),
            original: &original,
            source_blake3: blake3::hash(b"unit draft"),
            source_sha256: "draft",
            depth: SampleType::U8,
            mask_record_offset: 4289,
        };
        // Isolated post-binding geometry regression; original-byte binding
        // remains separately enforced by bind_source (this unit does not admit
        // synthetic source bytes through that public constructor).
        let mut document = Document::new("Own distinct mask geometry", photocraft_doc::Size::new(32, 32), photocraft_doc::ColorMode::Rgb, SampleType::U8);
        document.sample_encoding = Some(SampleEncoding::NonlinearProfile);
        let mut channel = photocraft_doc::AlphaChannel::new("Own saved", Surface::new(photocraft_color::PixelFormat::GRAY8));
        channel.id = ChannelId(7);
        document.channels.push(channel);
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(&document).unwrap();
        let item = state.items.iter_mut().find(|item| item.item == ItemRef::Channel(ChannelId(7))).unwrap();
        item.visible = false;
        item.raster_bounds = Some(Rect::new(0, 0, 32, 32));
        let mut mask_item = item.clone();
        mask_item.item = ItemRef::Mask(photocraft_doc::item_identity::MaskId(9));
        mask_item.raster_bounds = Some(binding.mask_bounds());
        mask_item.mask = Some(photocraft_doc::xcf_state::MaskState {
            owner: ItemRef::Channel(ChannelId(7)),
            bounds: binding.mask_bounds(),
            offset: [0, 0],
            outside_f64_bits: 0f64.to_bits(),
            enabled: true,
            shown: false,
            editing: false,
            linked: true,
        });
        state.items.push(mask_item);
        state.filter_masks.push(
            photocraft_doc::FilterMask::try_new(
                photocraft_doc::item_identity::MaskId(9),
                FilterId(1),
                Surface::new(photocraft_color::PixelFormat::GRAY8),
                binding.mask_bounds(),
            )
            .unwrap(),
        );
        state.filter_stacks.push(photocraft_doc::xcf_filters::FilterStack {
            owner: ItemRef::Channel(ChannelId(7)),
            nodes: vec![original.clone()],
            revision: 0,
        });
        state.provenance.format = "xcf".into();
        state.provenance.format_version = 26;
        state.provenance.container = "plain".into();
        state.provenance.source_hash = Some(binding.source_blake3.to_hex().to_string());
        document.xcf_state = Some(state);
        let before = document.clone();
        let read = read_stored(&document, &binding, &Interrupt::NONE).unwrap();
        assert_eq!(read.bounds, Rect::new(0, 0, 32, 32));
        assert!(std::ptr::eq(read.surface, &document.channels[0].surface));
        assert_eq!(document, before);
        // Accidentally resizing only the filter mask to channel bounds must fail.
        document.xcf_state.as_mut().unwrap().filter_masks[0].bounds = Rect::new(0, 0, 32, 32);
        let before = document.clone();
        assert!(read_stored(&document, &binding, &Interrupt::NONE).is_err());
        assert_eq!(document, before);
        assert_ne!(binding.mask_record_offset(), 9);
    }
    #[test]
    fn cancellation_does_not_return_candidate_metadata() {
        let document = Document::new("Own cancel", photocraft_doc::Size::new(32, 32), photocraft_doc::ColorMode::Rgb, SampleType::F64);
        let stop = || true;
        let control = Interrupt::cancel_only(&stop);
        assert!(matches!(inspect_candidate(&document, ChannelId(1), ChannelConsumer::StoredSelectionRead, &control), Err(EngineError::Cancelled)));
    }
}
