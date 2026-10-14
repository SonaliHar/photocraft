//! Complete saved-channel storage association, deliberately not rendering admission.
//! The result retains the entire decoded scene and real decoder reservations.
//! Public decoded fields are mutable upstream: span/metadata association here is
//! not an independent original-file/native-plane correspondence proof. Historical
//! source records and parsed configuration/graph ASTs are exposed unchanged.
use crate::{IoError, NativeCurrentCensus};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    FilterMask,
    item_identity::{ChannelId, FilterId, ItemRef, MaskId},
    xcf_state::{ItemLocks, ItemState, MaskState},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Budget, BudgetDecodedImport, BudgetKind, Drawable, Effect, EffectState, RawProperty, Reservation, Sample, SourceContainer, XcfFile};
use std::{collections::HashSet, mem::size_of};

#[derive(Clone, Copy, Debug)]
pub struct SceneChannelLimits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
    pub channels: usize,
    pub effects: usize,
    pub records: usize,
}
impl Default for SceneChannelLimits {
    fn default() -> Self {
        let standard = crate::NativeSaveLimits::default().preparation;
        Self {
            resident_bytes: standard.resident_bytes as u64,
            index_bytes: standard.index_bytes as u64,
            work_units: 32_000_000,
            channels: 10_000,
            effects: 100_000,
            records: 200_000,
        }
    }
}
/// Ordered physical channel records, including the selection-channel role.
/// IDs must already exist in the exact current-document census. No tattoo,
/// name or record offset is converted into a canonical identity.
#[derive(Clone, Copy, Debug)]
pub struct SceneChannelBinding {
    pub channel: ChannelId,
}
#[derive(Clone, Copy, Debug)]
pub struct SceneChannelRequirements {
    pub decoder_retained_bytes: u64,
    pub actual_decoded_file_bytes: u64,
    pub extra_decoder_reservation_bytes: u64,
    pub decoded_census_work_units: u64,
    pub source_retained_bytes: u64,
    pub current_retained_bytes: u64,
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct StoredSceneEffect {
    pub channel: ChannelId,
    pub filter: FilterId,
    pub mask: Option<MaskId>,
    pub channel_ordinal: usize,
    pub effect_ordinal: usize,
    pub composition_unresolved: bool,
}
#[derive(Debug)]
pub struct StoredSceneChannel {
    pub channel: ChannelId,
    pub source_ordinal: usize,
    pub selection: bool,
    pub active: bool,
    pub effects: Vec<StoredSceneEffect>,
}
/// Full ownership, rather than a selected projection or ImportResult. Every
/// unrelated layer/path/global/channel record remains reachable through file().
pub struct SceneChannelStorage<'s, 'd, 'b, 'a> {
    owner: BudgetDecodedImport<'b, 'a>,
    logical: &'s [u8],
    current: NativeCurrentCensus<'d>,
    channels: Vec<StoredSceneChannel>,
    masks: Vec<FilterMask>,
    mask_items: Vec<ItemState>,
    requirements: SceneChannelRequirements,
    _decoder_extra: Option<Reservation<'b, 'a>>,
    _output: Reservation<'b, 'a>,
    _index: Reservation<'b, 'a>,
}
impl SceneChannelStorage<'_, '_, '_, '_> {
    pub fn file(&self) -> &XcfFile {
        &self.owner.decoded.file
    }
    pub fn logical_source(&self) -> &[u8] {
        self.logical
    }
    pub fn current_document(&self) -> &photocraft_doc::Document {
        self.current.document()
    }
    pub fn channels(&self) -> &[StoredSceneChannel] {
        &self.channels
    }
    pub fn filter_masks(&self) -> &[FilterMask] {
        &self.masks
    }
    pub fn mask_items(&self) -> &[ItemState] {
        &self.mask_items
    }
    pub fn requirements(&self) -> SceneChannelRequirements {
        self.requirements
    }
    pub fn original_channel(&self, ordinal: usize) -> Option<&Drawable> {
        self.file().channels.get(ordinal)
    }
    pub fn original_effect(&self, descriptor: &StoredSceneEffect) -> Option<&Effect> {
        let channel = self.channels.get(descriptor.channel_ordinal)?;
        let actual = channel.effects.get(descriptor.effect_ordinal)?;
        if actual.filter != descriptor.filter || actual.channel != descriptor.channel || actual.mask != descriptor.mask || !descriptor.composition_unresolved {
            return None;
        }
        self.original_channel(descriptor.channel_ordinal)?.effects.get(descriptor.effect_ordinal)
    }
    /// No source operation version, config normalization, profile or enum
    /// meaning is guessed. Consumers read original_arguments/configuration/graph
    /// through original_effect and must supply their exact existing adapter.
    pub fn original_outer(&self, descriptor: &StoredSceneEffect) -> Option<&EffectState> {
        Some(&self.original_effect(descriptor)?.state)
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("XCF scene channel storage: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("accounting overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("accounting overflow"))
}
fn accounting(e: photocraft_xcf::XcfError) -> IoError {
    if e.kind == photocraft_xcf::ErrorKind::Cancelled { IoError::Cancelled } else { bad(&e.to_string()) }
}
struct Meter<'c, 'b, 'a, 'i> {
    limits: SceneChannelLimits,
    resident: u64,
    index: u64,
    work: u64,
    records: usize,
    shared: &'b Budget<'a>,
    ctl: &'c Interrupt<'i>,
}
impl Meter<'_, '_, '_, '_> {
    fn step(&mut self, n: u64) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)?;
        self.work = add(self.work, n)?;
        if self.work > self.limits.work_units {
            return Err(bad("work limit"));
        }
        self.shared.consume_work(n, "complete channel storage").map_err(accounting)
    }
    fn allocation(&mut self, n: u64, index: bool) -> Result<(), IoError> {
        self.resident = add(self.resident, n)?;
        if index {
            self.index = add(self.index, n)?;
        }
        if self.resident > self.limits.resident_bytes || self.index > self.limits.index_bytes {
            return Err(bad("aggregate capacity limit"));
        }
        Ok(())
    }
    fn vector<T>(&mut self, count: usize) -> Result<Vec<T>, IoError> {
        self.step(count as u64 + 1)?;
        let bytes = mul(count as u64, size_of::<T>() as u64)?;
        self.allocation(bytes, true)?;
        let mut out = Vec::new();
        out.try_reserve_exact(count).map_err(|_| bad("vector allocation"))?;
        self.allocation(mul(out.capacity().saturating_sub(count) as u64, size_of::<T>() as u64)?, true)?;
        Ok(out)
    }
    fn text(&mut self, text: &str) -> Result<String, IoError> {
        if text.len() > 65536 {
            return Err(bad("name limit"));
        }
        self.step(text.len() as u64 + 1)?;
        if text.contains('\0') {
            return Err(bad("name contains NUL"));
        }
        self.allocation(text.len() as u64, false)?;
        let mut out = String::new();
        out.try_reserve_exact(text.len()).map_err(|_| bad("name allocation"))?;
        self.allocation(out.capacity().saturating_sub(text.len()) as u64, false)?;
        out.push_str(text);
        Ok(out)
    }
    fn properties(&mut self, props: &[RawProperty], source: &[u8]) -> Result<(), IoError> {
        for p in props {
            self.step(1)?;
            self.records = self.records.checked_add(1).filter(|n| *n <= self.limits.records).ok_or_else(|| bad("record count"))?;
            if p.payload.len() != p.declared_bytes as usize {
                return Err(bad("raw declared length"));
            }
            let start = usize::try_from(p.offset).map_err(|_| bad("raw offset range"))?;
            let stop = start.checked_add(p.payload.len()).ok_or_else(|| bad("raw offset overflow"))?;
            let original = source.get(start..stop).ok_or_else(|| bad("raw property outside logical source"))?;
            for (a, b) in p.payload.chunks(4096).zip(original.chunks(4096)) {
                self.step(mul(a.len() as u64, 2)?)?;
                if a != b {
                    return Err(bad("raw property/source mismatch"));
                }
            }
        }
        Ok(())
    }
}
fn bounds(d: &Drawable) -> Result<Rect, IoError> {
    if d.width == 0 || d.height == 0 || d.width > 262144 || d.height > 262144 {
        return Err(bad("native extent"));
    }
    let x = d.offset.0.checked_add(i32::try_from(d.width).map_err(|_| bad("width range"))?).ok_or_else(|| bad("native extent overflow"))?;
    let y = d.offset.1.checked_add(i32::try_from(d.height).map_err(|_| bad("height range"))?).ok_or_else(|| bad("native extent overflow"))?;
    if [d.offset.0, d.offset.1, x, y].iter().any(|n| !(-(1 << 29)..=(1 << 29)).contains(n)) {
        return Err(bad("native offset limit"));
    }
    Ok(Rect::new(d.offset.0, d.offset.1, x, y))
}
fn sample(s: Sample) -> SampleType {
    match s {
        Sample::U8 => SampleType::U8,
        Sample::U16 => SampleType::U16,
        Sample::U32 => SampleType::U32,
        Sample::F16 | Sample::F32 => SampleType::F32,
        Sample::F64 => SampleType::F64,
    }
}
fn outer(effect: &Effect, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    let s = &effect.state;
    let mut seen = [false; 8];
    let mut argument_count = 0usize;
    for p in &effect.properties {
        m.step(1)?;
        if p.tag == 45 {
            argument_count = argument_count.checked_add(1).ok_or_else(|| bad("argument count overflow"))?;
            continue;
        }
        let (index, expected) = match p.tag {
            8 => (0, s.enabled.map(u32::from)),
            33 => (1, s.opacity_bits),
            7 => (2, s.mode.map(|n| n as u32)),
            35 => (3, s.composite_mode.map(|n| n as u32)),
            36 => (4, s.composite_space.map(|n| n as u32)),
            37 => (5, s.blend_space.map(|n| n as u32)),
            46 => (6, s.clipping),
            44 => (7, s.region),
            _ => return Err(bad("unmapped required outer property")),
        };
        let slot = seen.get_mut(index).ok_or_else(|| bad("outer property slot"))?;
        if *slot || p.payload.len() != 4 || expected.is_none() {
            return Err(bad("outer property presence/uniqueness"));
        }
        *slot = true;
        let bytes: [u8; 4] = p.payload.as_slice().try_into().map_err(|_| bad("outer word width"))?;
        if Some(u32::from_be_bytes(bytes)) != expected {
            return Err(bad("raw/current outer word mismatch"));
        }
    }
    if argument_count != effect.arguments.len() || s.opacity_u8.is_some() {
        return Err(bad("unmapped argument or legacy opacity syntax"));
    }
    // Absent fields are retained as absent, never assigned a resolved default.
    for (present, index) in [
        (s.enabled.is_some(), 0),
        (s.opacity_bits.is_some(), 1),
        (s.mode.is_some(), 2),
        (s.composite_mode.is_some(), 3),
        (s.composite_space.is_some(), 4),
        (s.blend_space.is_some(), 5),
        (s.clipping.is_some(), 6),
        (s.region.is_some(), 7),
    ] {
        if seen.get(index).copied() != Some(present) {
            return Err(bad("outer presence mismatch"));
        }
    }
    Ok(())
}

fn quoted(raw: &[u8], expected: &[u8], m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    let body = raw.strip_prefix(b"\"").and_then(|s| s.strip_suffix(b"\"")).ok_or_else(|| bad("AST quoted envelope"))?;
    let mut at = 0usize;
    let mut out = 0usize;
    while let Some(mut byte) = body.get(at).copied() {
        m.step(1)?;
        at = at.checked_add(1).ok_or_else(|| bad("quote offset"))?;
        if byte == b'\\' {
            byte = body.get(at).copied().ok_or_else(|| bad("quote escape extent"))?;
            at = at.checked_add(1).ok_or_else(|| bad("quote offset"))?;
            byte = match byte {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'b' => 8,
                b'f' => 12,
                b'v' => 11,
                b'a' => 7,
                b'\\' => b'\\',
                b'"' => b'"',
                b'0'..=b'7' => {
                    let mut value = u16::from(byte - b'0');
                    for _ in 0..2 {
                        m.step(1)?;
                        if let Some(next @ b'0'..=b'7') = body.get(at).copied() {
                            value = value.checked_mul(8).and_then(|v| v.checked_add(u16::from(next - b'0'))).ok_or_else(|| bad("quote octal overflow"))?;
                            at = at.checked_add(1).ok_or_else(|| bad("quote offset"))?;
                        } else {
                            break;
                        }
                    }
                    u8::try_from(value).map_err(|_| bad("quote octal byte"))?
                }
                _ => return Err(bad("unknown required quote escape")),
            };
        } else if byte == 0 || byte == b'"' {
            return Err(bad("unescaped quoted delimiter"));
        }
        if expected.get(out).copied() != Some(byte) {
            return Err(bad("AST decoded quote/source mismatch"));
        }
        out = out.checked_add(1).ok_or_else(|| bad("quote byte count"))?;
    }
    if out != expected.len() {
        return Err(bad("AST decoded quote length"));
    }
    Ok(())
}
fn ast(nodes: &[photocraft_xcf::ConfigNode], text: &str, depth: usize, count: &mut usize, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    if depth > 32 {
        return Err(bad("configuration depth"));
    }
    for n in nodes {
        m.step(1)?;
        *count = count.checked_add(1).filter(|n| *n <= m.limits.records).ok_or_else(|| bad("aggregate AST count"))?;
        let start = usize::try_from(n.start).map_err(|_| bad("AST offset"))?;
        let end = usize::try_from(n.end).map_err(|_| bad("AST offset"))?;
        let span = text.get(start..end).ok_or_else(|| bad("AST UTF8 span"))?;
        match &n.value {
            photocraft_xcf::ConfigValue::Atom(atom) => {
                m.step(mul(span.len() as u64, 2)?)?;
                if span != atom {
                    return Err(bad("AST atom/source text mismatch"));
                }
            }
            photocraft_xcf::ConfigValue::Number { literal, bits } => {
                m.step(mul(span.len() as u64, 3)?)?;
                if span != literal || literal.parse::<f64>().ok().map(f64::to_bits) != Some(*bits) {
                    return Err(bad("AST exact number/source text mismatch"));
                }
            }
            photocraft_xcf::ConfigValue::Quoted(bytes) => {
                quoted(span.as_bytes(), bytes, m)?;
            }
            photocraft_xcf::ConfigValue::List(children) => {
                if !span.starts_with('(') || !span.ends_with(')') {
                    return Err(bad("AST list envelope"));
                }
                for child in children {
                    m.step(1)?;
                    if child.start < n.start || child.end > n.end {
                        return Err(bad("AST child outside parent"));
                    }
                }
                ast(children, text, depth + 1, count, m)?;
            }
        }
    }
    Ok(())
}
fn arguments(effect: &Effect, ast_count: &mut usize, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    use super::configured_source_candidate::{raw_argument, word};
    use photocraft_xcf::ArgumentValue as V;
    let mut indexes = m.vector::<usize>(effect.arguments.len())?;
    for argument in &effect.arguments {
        if argument.name.len() > 65536 {
            return Err(bad("argument name limit"));
        }
        m.step(mul(argument.name.len() as u64 + 1, 2)?)?;
        m.step(indexes.len() as u64)?;
        if indexes.contains(&argument.property_index) {
            return Err(bad("aliased argument property"));
        }
        indexes.push(argument.property_index);
        let (raw, at) = raw_argument(argument, &effect.properties)?;
        let kind = word(raw, at)?;
        let value_at = at.checked_add(4).ok_or_else(|| bad("argument overflow"))?;
        let data = raw.get(value_at..).ok_or_else(|| bad("argument extent"))?;
        let integer = |expected: u32| -> Result<(), IoError> {
            if data.len() != 4 || word(data, 0)? != expected {
                return Err(bad("argument exact word"));
            }
            Ok(())
        };
        match &argument.value {
            V::Integer(n) if kind == 1 => integer(*n as u32)?,
            V::Boolean(n) if kind == 2 => integer(u32::from(*n))?,
            V::Unsigned(n) if kind == 7 => integer(*n)?,
            V::Enum(n) if kind == 5 => integer(*n as u32)?,
            V::Float32Bits(n) if kind == 3 => integer(*n)?,
            V::Float64Bits(n) if kind == 3 => {
                let bytes: [u8; 8] = data.try_into().map_err(|_| bad("argument Float64 width"))?;
                if u64::from_be_bytes(bytes) != *n {
                    return Err(bad("argument Float64 bits"));
                }
            }
            V::String(text) | V::Config(text)
                if (matches!(&argument.value, V::String(_)) && kind == 4) || (matches!(&argument.value, V::Config(_)) && kind == 6) =>
            {
                let size = usize::try_from(word(data, 0)?).map_err(|_| bad("argument string width"))?;
                let end = 4usize.checked_add(size).ok_or_else(|| bad("argument string overflow"))?;
                m.step(mul(text.len() as u64 + 1, 2)?)?;
                if end != data.len() || data.get(4..end).and_then(|v| v.strip_suffix(&[0])) != Some(text.as_bytes()) {
                    return Err(bad("argument exact text"));
                }
                if let Some(config) = &argument.configuration {
                    if kind != 6 {
                        return Err(bad("configuration type"));
                    }
                    ast(&config.nodes, text, 0, ast_count, m)?;
                }
                if let Some(graph) = &argument.graph {
                    if kind != 4 {
                        return Err(bad("graph source type"));
                    }
                    graph_spans(&graph.entries, text, 0, ast_count, m)?;
                }
                // Graph remains borrowed; no schema/version/ports are invented.
            }
            V::NullColor if kind == 8 => integer(0)?,
            V::Color { encoding, samples, icc } if kind == 8 => {
                let mut offset = 0;
                let encoded = blob(data, &mut offset)?.strip_suffix(&[0]).ok_or_else(|| bad("color encoding terminator"))?;
                same_bytes(encoded, encoding.as_bytes(), m)?;
                same_bytes(blob(data, &mut offset)?, samples, m)?;
                same_bytes(blob(data, &mut offset)?, icc, m)?;
                if offset != data.len() {
                    return Err(bad("color trailing payload"));
                }
            }
            _ => return Err(bad("unknown required argument syntax")),
        }
        if matches!(&argument.value, V::Config(_)) && argument.configuration.is_none() {
            return Err(bad("required parsed configuration missing"));
        }
        if argument.configuration.is_some() && !matches!(&argument.value, V::Config(_)) {
            return Err(bad("configuration association"));
        }
        if argument.graph.is_some() && !matches!(&argument.value, V::String(_)) {
            return Err(bad("graph association"));
        }
    }
    Ok(())
}
fn graph_spans(entries: &[photocraft_xcf::GraphEntry], text: &str, depth: usize, count: &mut usize, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    if depth > 32 {
        return Err(bad("graph nesting limit"));
    }
    for entry in entries {
        m.step(1)?;
        *count = count.checked_add(1).filter(|n| *n <= m.limits.records).ok_or_else(|| bad("aggregate graph/config entries"))?;
        let (start, end) = match entry {
            photocraft_xcf::GraphEntry::Operation { start, end, .. } | photocraft_xcf::GraphEntry::Property { start, end, .. } => (*start, *end),
        };
        let span = text
            .get(usize::try_from(start).map_err(|_| bad("graph span"))?..usize::try_from(end).map_err(|_| bad("graph span"))?)
            .ok_or_else(|| bad("graph UTF8 span"))?;
        m.step(mul(span.len() as u64, 3)?)?;
        match entry {
            photocraft_xcf::GraphEntry::Operation { identity, .. } => {
                if span != identity {
                    return Err(bad("graph operation text association"));
                }
            }
            photocraft_xcf::GraphEntry::Property { name, value, .. } => {
                let remainder = span
                    .strip_prefix(name.as_str())
                    .map(str::trim_start)
                    .and_then(|s| s.strip_prefix('='))
                    .map(str::trim_start)
                    .ok_or_else(|| bad("graph property text association"))?;
                match value {
                    photocraft_xcf::GraphValue::Literal { source, numeric_bits } => {
                        if remainder != source {
                            return Err(bad("graph literal association"));
                        }
                        let parsed = match source.parse::<f64>() {
                            Ok(value) if value.is_finite() => Some(value.to_bits()),
                            Ok(_) => return Err(bad("nonfinite stored graph literal")),
                            Err(_) => None,
                        };
                        if *numeric_bits != parsed {
                            return Err(bad("graph exact None/Some numeric association"));
                        }
                    }
                    photocraft_xcf::GraphValue::Chain(children) => {
                        if !remainder.starts_with('[') || !remainder.ends_with(']') {
                            return Err(bad("graph chain envelope"));
                        }
                        for child in children {
                            m.step(1)?;
                            let (a, b) = match child {
                                photocraft_xcf::GraphEntry::Operation { start, end, .. } | photocraft_xcf::GraphEntry::Property { start, end, .. } => {
                                    (*start, *end)
                                }
                            };
                            if a < start || b > end {
                                return Err(bad("graph child span"));
                            }
                        }
                        graph_spans(children, text, depth + 1, count, m)?;
                    }
                }
            }
        }
    }
    Ok(())
}
fn current_arguments(args: &[photocraft_doc::xcf_filters::FilterArgument], max: &mut u64, depth: usize, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    if depth > 32 {
        return Err(bad("current graph/config namespace depth"));
    }
    for a in args {
        m.step(1)?;
        match &a.value {
            photocraft_doc::xcf_filters::FilterValue::Resource(id) => *max = (*max).max(id.0),
            photocraft_doc::xcf_filters::FilterValue::Color { profile: Some(id), .. } => *max = (*max).max(id.0),
            photocraft_doc::xcf_filters::FilterValue::Config(children) => current_arguments(children, max, depth + 1, m)?,
            photocraft_doc::xcf_filters::FilterValue::Graph(graph) => {
                if let Some(id) = graph.input {
                    *max = (*max).max(id.0);
                }
                *max = (*max).max(graph.output.0);
                for node in &graph.nodes {
                    m.step(1)?;
                    *max = (*max).max(node.id.0);
                    current_arguments(&node.arguments, max, depth + 1, m)?;
                }
                for edge in &graph.edges {
                    m.step(1)?;
                    *max = (*max).max(edge.from.0).max(edge.to.0);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn current_source(item: &photocraft_doc::xcf_items::SourceItem, max: &mut u64, m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    m.step(1)?;
    *max = (*max).max(item.owner.raw());
    let args = match &item.source {
        photocraft_doc::xcf_items::EditableSource::Text { font, configuration, .. } => {
            *max = (*max).max(font.0);
            configuration
        }
        photocraft_doc::xcf_items::EditableSource::Vector { path, configuration, .. } => {
            *max = (*max).max(path.0);
            configuration
        }
        photocraft_doc::xcf_items::EditableSource::Linked { resource, configuration, .. } => {
            *max = (*max).max(resource.0);
            configuration
        }
    };
    current_arguments(args, max, 0, m)
}
fn blob<'p>(data: &'p [u8], at: &mut usize) -> Result<&'p [u8], IoError> {
    let length = super::configured_source_candidate::word(data, *at)? as usize;
    let start = at.checked_add(4).ok_or_else(|| bad("color offset overflow"))?;
    let end = start.checked_add(length).ok_or_else(|| bad("color extent overflow"))?;
    let out = data.get(start..end).ok_or_else(|| bad("color blob extent"))?;
    *at = end;
    Ok(out)
}
fn same_bytes(a: &[u8], b: &[u8], m: &mut Meter<'_, '_, '_, '_>) -> Result<(), IoError> {
    if a.len() != b.len() {
        return Err(bad("color payload length association"));
    }
    for (a, b) in a.chunks(4096).zip(b.chunks(4096)) {
        m.step(mul(a.len() as u64, 2)?)?;
        if a != b {
            return Err(bad("color exact payload association"));
        }
    }
    Ok(())
}
fn mask_plan(mask: &Drawable, m: &mut Meter<'_, '_, '_, '_>, admit: bool) -> Result<(Rect, PixelFormat), IoError> {
    let r = bounds(mask)?;
    let plane = mask.plane.as_ref().ok_or_else(|| bad("native mask plane missing"))?;
    let format = PixelFormat::new(ColorMode::Grayscale, sample(plane.sample), false);
    if plane.width != mask.width || plane.height != mask.height || plane.channels != 1 {
        return Err(bad("native mask descriptor"));
    }
    let bytes = photocraft_raster::checked_region_byte_len(r, format).map_err(|e| bad(&e.to_string()))?;
    if bytes != plane.bytes.len() {
        return Err(bad("native mask byte count"));
    }
    let side = TILE_SIZE as i64;
    let cols = (i64::from(r.x1) - 1).div_euclid(side) - i64::from(r.x0).div_euclid(side) + 1;
    let rows = (i64::from(r.y1) - 1).div_euclid(side) - i64::from(r.y0).div_euclid(side) + 1;
    let tiles = mul(u64::try_from(cols).map_err(|_| bad("mask tile columns"))?, u64::try_from(rows).map_err(|_| bad("mask tile rows"))?)?;
    let tile = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, format.bytes_per_pixel() as u64)?;
    if admit {
        m.allocation(add(mul(tiles, add(tile, 512)?)?, tile)?, false)?;
        m.allocation(mul(tiles, 512)?, true)?;
        m.step(mul(add(bytes as u64, mul(tiles, tile)?)?, 8)?)?;
    }
    Ok((r, format))
}
fn fresh(next: &mut u64) -> Result<u64, IoError> {
    let id = *next;
    if id == 0 {
        return Err(bad("zero generated identity"));
    }
    *next = next.checked_add(1).ok_or_else(|| bad("identity namespace exhausted"))?;
    Ok(id)
}
#[allow(clippy::too_many_arguments)]
pub fn assemble_scene_channel_storage<'s, 'd, 'b, 'a>(
    owner: BudgetDecodedImport<'b, 'a>,
    logical: &'s [u8],
    current: NativeCurrentCensus<'d>,
    bindings: &[SceneChannelBinding],
    limits: SceneChannelLimits,
    shared: &'b Budget<'a>,
    ctl: &Interrupt<'_>,
) -> Result<SceneChannelStorage<'s, 'd, 'b, 'a>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let standard = SceneChannelLimits::default();
    if limits.resident_bytes > standard.resident_bytes
        || limits.index_bytes > standard.index_bytes
        || limits.work_units > standard.work_units
        || limits.channels > standard.channels
        || limits.effects > standard.effects
        || limits.records > standard.records
    {
        return Err(bad("limits must be lower-only"));
    }
    if owner.decoded.container != SourceContainer::Plain
        || !matches!(owner.decoded.file.version, 22 | 26)
        || logical.len() as u64 != owner.decoded.logical_bytes
        || !photocraft_xcf::is_xcf(logical)
    {
        return Err(bad("plain decoded full-scene binding required"));
    }
    // Real encoded/source leases are mandatory on this entry. CallerReserved
    // inputs need a separate genuine held-source receipt; no asserted byte total.
    if owner.reservations.source_bytes() < logical.len() as u64 || owner.reservations.encoded_bytes() < logical.len() as u64 {
        return Err(bad("complete decoder/source leases required"));
    }
    let header: &[u8] = match owner.decoded.file.version {
        22 => b"v022\0",
        26 => b"v026\0",
        _ => return Err(bad("source version")),
    };
    if logical.get(9..14) != Some(header) {
        return Err(bad("source version association"));
    }
    // Actual public fields may have grown after decoding. A historical charge
    // is never proof of current capacity. Borrow the exact file through an
    // allocation-free whole-file census; every structural visit debits shared
    // Work/Visits and the actual caller cancellation token before it proceeds.
    let base_held = add(owner.reservations.source_bytes(), current.retained_bytes() as u64)?;
    let census_resident = limits.resident_bytes.checked_sub(base_held).ok_or_else(|| bad("held source/current backing limit"))?;
    let poll = || {
        ctl.check().map_err(|_| photocraft_xcf::XcfError {
            kind: photocraft_xcf::ErrorKind::Cancelled,
            offset: 0,
            item: String::new(),
            message: "cancelled scene backing census".into(),
        })
    };
    let (actual_file, census_work) = {
        let standard = photocraft_xcf::OwnedBackingLimits::default();
        let lower = photocraft_xcf::OwnedBackingLimits {
            resident_bytes: census_resident.min(standard.resident_bytes).min(shared.limits().get(BudgetKind::Resident)),
            work_units: limits.work_units.min(standard.work_units).min(shared.limits().get(BudgetKind::Work).saturating_sub(shared.used(BudgetKind::Work))),
            visits: standard.visits.min(shared.limits().get(BudgetKind::Visits).saturating_sub(shared.used(BudgetKind::Visits))),
            depth: standard.depth.min(usize::try_from(shared.limits().get(BudgetKind::GroupDepth)).map_err(|_| bad("census depth range"))?),
        };
        let receipt = photocraft_xcf::census_owned_file(&owner.decoded.file, lower, shared, &poll).map_err(accounting)?;
        if !receipt.belongs_to(&owner.decoded.file) {
            return Err(bad("decoded backing receipt identity"));
        }
        (receipt.retained_bytes(), receipt.work_units())
    };
    // The lower receipt includes the XcfFile inline header, but not the enclosing
    // decoded-container/reservation header. Price that exact size separately.
    let owner_header = size_of::<BudgetDecodedImport<'b, 'a>>().checked_sub(size_of::<XcfFile>()).ok_or_else(|| bad("decoded owner header size"))? as u64;
    let actual_owner = add(actual_file, owner_header)?;
    let old = owner.reservations.retained_bytes();
    let extra_bytes = actual_owner.saturating_sub(old);
    let held = add(base_held, add(old, extra_bytes)?)?;
    if held > limits.resident_bytes {
        return Err(bad("complete actual retained backing limit"));
    }
    let decoder_extra =
        if extra_bytes == 0 { None } else { Some(shared.reserve(BudgetKind::Resident, extra_bytes, "actual full decoded backing delta").map_err(accounting)?) };
    let held_index = add(current.known_index_reservation_bytes() as u64, current.known_map_reservation_bytes() as u64)?;
    let output_limit = limits
        .resident_bytes
        .checked_sub(held)
        .ok_or_else(|| bad("complete retained backing limit"))?
        .min(shared.limits().get(BudgetKind::Resident).saturating_sub(shared.used(BudgetKind::Resident)));
    let index_limit = limits
        .index_bytes
        .checked_sub(held_index)
        .ok_or_else(|| bad("current namespace backing limit"))?
        .min(shared.limits().get(BudgetKind::Index).saturating_sub(shared.used(BudgetKind::Index)));
    // These additional leases cover every new allocation under one shared Meter.
    // Existing source/decoder/current backing is not released, refunded or copied.
    let output = shared.reserve(BudgetKind::Resident, output_limit, "scene channel output").map_err(accounting)?;
    let index = shared.reserve(BudgetKind::Index, index_limit, "scene channel index").map_err(accounting)?;
    let mut m = Meter {
        limits: SceneChannelLimits { resident_bytes: output_limit, index_bytes: index_limit, ..limits },
        resident: 0,
        index: 0,
        work: census_work, // helper already consumed shared Work: never debit twice.
        records: 0,
        shared,
        ctl,
    };
    m.allocation(size_of::<SceneChannelStorage<'s, 'd, 'b, 'a>>() as u64, false)?;
    let file = &owner.decoded.file;
    let doc = current.document();
    if file.channels.len() > limits.channels || bindings.len() != file.channels.len() {
        return Err(bad("complete ordered channel bindings required"));
    }
    let mut max = 0u64;
    // Maximum covers all current namespaces, including unrelated scene records.
    // Existing native/typed aliases describe the same owner; new IDs are strictly
    // above every retained namespace, so none can alias another family.
    photocraft_doc::resident_structure::visit_layers_with(doc, ctl, |layer| {
        m.step(1).map_err(|error| {
            if matches!(error, IoError::Cancelled) {
                photocraft_raster::RasterInputError::Cancelled
            } else {
                photocraft_raster::RasterInputError::LimitExceeded
            }
        })?;
        max = max.max(layer.id.0);
        Ok(true) // visit every descendant; false would truncate the namespace.
    })
    .map_err(|error| if ctl.cancelled() || error == photocraft_raster::RasterInputError::Cancelled { IoError::Cancelled } else { bad(&error.to_string()) })?;
    for channel in &doc.channels {
        m.step(1)?;
        max = max.max(channel.id.0);
    }
    for id in [doc.selection_channel_id, doc.quick_mask.as_ref().map(|c| c.id)].into_iter().flatten() {
        m.step(1)?;
        max = max.max(id.0);
    }
    for path in &doc.paths {
        m.step(1)?;
        max = max.max(path.id.0);
    }
    if let Some(state) = &doc.xcf_state {
        for item in &state.items {
            m.step(1)?;
            max = max.max(item.item.raw());
            if let Some(parent) = item.parent {
                max = max.max(parent.raw());
            }
            if let Some(mask) = &item.mask {
                max = max.max(mask.owner.raw());
            }
        }
        for item in &state.source_items {
            current_source(item, &mut max, &mut m)?;
        }
        for item in &state.editable_indexed {
            m.step(1)?;
            max = max.max(item.owner.0);
        }
        if let Some(floating) = &state.floating {
            m.step(1)?;
            max = max.max(floating.layer.0).max(floating.target.raw());
        }
        for item in state.selection.selected.iter().chain(&state.selection.selected_paths) {
            m.step(1)?;
            max = max.max(item.raw());
        }
        for item in [state.selection.active, state.selection.mask_target].into_iter().flatten() {
            m.step(1)?;
            max = max.max(item.raw());
        }
        for record in &state.opaque {
            m.step(1)?;
            if let Some(owner) = record.owner {
                max = max.max(owner.raw());
            }
        }
        for resource in &state.resources {
            m.step(1)?;
            max = max.max(resource.id.0);
        }
        for set in &state.item_sets {
            m.step(1)?;
            max = max.max(set.id.0);
            for member in &set.members {
                m.step(1)?;
                max = max.max(member.raw());
            }
        }
        for stack in &state.filter_stacks {
            m.step(1)?;
            max = max.max(stack.owner.raw());
            for node in &stack.nodes {
                m.step(1)?;
                max = max.max(node.id.0);
                current_arguments(&node.arguments, &mut max, 0, &mut m)?;
                if let Some(mask) = node.mask {
                    max = max.max(mask.raw());
                }
                for (_, owner) in &node.auxiliary {
                    m.step(1)?;
                    max = max.max(owner.raw());
                }
            }
        }
        for mask in &state.filter_masks {
            m.step(1)?;
            max = max.max(mask.id.0).max(mask.owner.0);
        }
    }
    let mut next = max.checked_add(1).ok_or_else(|| bad("current namespace exhausted"))?;
    let seen_bytes = mul(bindings.len() as u64, 128)?;
    m.allocation(seen_bytes, true)?;
    let mut seen = HashSet::new();
    seen.try_reserve(bindings.len()).map_err(|_| bad("channel namespace allocation"))?;
    m.allocation(mul(seen.capacity().saturating_sub(bindings.len()) as u64, 128)?, true)?;
    let mut channels = m.vector(file.channels.len())?;
    let mut total_effects = 0usize;
    let mut ast_count = 0usize;
    // First pass validates ALL channel/effect/argument/raw envelopes before any
    // native mask copies. No early selected-slice success can hide a late error.
    for (d, binding) in file.channels.iter().zip(bindings) {
        m.step(1)?;
        if binding.channel.0 == 0
            || !seen.insert(binding.channel)
            || d.model != photocraft_xcf::Model::Gray
            || d.alpha
            || d.group
            || d.mask.is_some()
            || d.floating_attachment.is_some()
            || d.link.is_some()
            || d.text.is_some()
            || d.vector.is_some()
            || d.transform.is_some()
        {
            return Err(bad("saved channel structural context"));
        }
        for reason in &d.unsupported {
            m.step(reason.len() as u64 + 1)?;
            if reason != "GEGL layer effects" {
                return Err(bad("unknown required channel syntax"));
            }
        }
        if doc.quick_mask.as_ref().is_some_and(|c| c.id == binding.channel) {
            return Err(bad("saved channel aliases Quick Mask"));
        }
        bounds(d)?;
        m.properties(&d.raw_properties, logical)?;
        let surface = if d.selection {
            if doc.selection_channel_id != Some(binding.channel) {
                return Err(bad("selection-channel role mismatch"));
            }
            doc.selection.as_ref().ok_or_else(|| bad("selection native storage missing"))?
        } else {
            let mut selected = None;
            for c in &doc.channels {
                m.step(1)?;
                if c.id == binding.channel && selected.replace(&c.surface).is_some() {
                    return Err(bad("duplicate current channel identity"));
                }
            }
            selected.ok_or_else(|| bad("current saved channel missing"))?
        };
        let plane = d.plane.as_ref().ok_or_else(|| bad("original channel plane missing"))?;
        if plane.width != d.width
            || plane.height != d.height
            || plane.channels != 1
            || surface.format() != PixelFormat::new(ColorMode::Grayscale, sample(plane.sample), false)
        {
            return Err(bad("current/native channel format association"));
        }
        let original_format = PixelFormat::new(ColorMode::Grayscale, sample(plane.sample), false);
        if plane.bytes.len() != photocraft_raster::checked_region_byte_len(bounds(d)?, original_format).map_err(|e| bad(&e.to_string()))? {
            return Err(bad("source channel native byte count"));
        }
        total_effects = total_effects.checked_add(d.effects.len()).filter(|n| *n <= limits.effects).ok_or_else(|| bad("aggregate effect count"))?;
        for effect in &d.effects {
            m.step(1)?;
            m.properties(&effect.properties, logical)?;
            outer(effect, &mut m)?;
            arguments(effect, &mut ast_count, &mut m)?;
            if let Some(mask) = effect.mask.as_deref() {
                bounds(mask)?;
                if mask.model != photocraft_xcf::Model::Gray
                    || mask.alpha
                    || mask.group
                    || mask.mask.is_some()
                    || !mask.effects.is_empty()
                    || !mask.unsupported.is_empty()
                {
                    return Err(bad("required native mask context"));
                }
                m.properties(&mask.raw_properties, logical)?;
                if mask.link.is_some() || mask.text.is_some() || mask.vector.is_some() || mask.transform.is_some() || mask.floating_attachment.is_some() {
                    return Err(bad("unsupported native mask source fields"));
                }
                m.step(mask.name.len() as u64 + 1)?;
                if mask.name.len() > 65536 || mask.name.contains('\0') {
                    return Err(bad("mask current name bound"));
                }
                mask_plan(mask, &mut m, true)?;
            }
        }
    }
    // Complete-role check: not only a channel subset. Quick Mask isn't serialized
    // as a saved source channel and remains an unrelated retained current role.
    let expected = doc.channels.len().checked_add(usize::from(doc.selection_channel_id.is_some())).ok_or_else(|| bad("current role count overflow"))?;
    if expected != bindings.len() {
        return Err(bad("current channel scene is not completely bound"));
    }
    let mut masks = m.vector(total_effects)?;
    let mut items = m.vector(total_effects)?;
    for (ordinal, (d, binding)) in file.channels.iter().zip(bindings).enumerate() {
        let mut effects = m.vector(d.effects.len())?;
        for (effect_ordinal, effect) in d.effects.iter().enumerate() {
            m.step(1)?;
            let filter = FilterId(fresh(&mut next)?);
            let mut mask_id = None;
            if let Some(mask) = effect.mask.as_deref() {
                let id = MaskId(fresh(&mut next)?);
                let (r, format) = mask_plan(mask, &mut m, false)?;
                let plane = mask.plane.as_ref().ok_or_else(|| bad("native mask plane disappeared"))?;
                let surface = photocraft_raster::Surface::try_from_interleaved_with(format, r, &plane.bytes, ctl)
                    .map_err(|e| if ctl.cancelled() { IoError::Cancelled } else { bad(&e.to_string()) })?;
                // Raw finite/over-one/F64 words are storage, never normalized to
                // coverage. No FilterMask::validate or renderer grade is granted.
                masks.push(FilterMask { id, owner: filter, surface, bounds: r });
                let name = m.text(&mask.name)?;
                items.push(ItemState {
                    item: ItemRef::Mask(id),
                    name,
                    custom_name: false,
                    visible: mask.visible,
                    locks: ItemLocks { content: mask.lock_content, position: mask.lock_position, alpha: mask.lock_alpha, visibility: mask.lock_visibility },
                    parent: None,
                    source_tattoo: mask.tattoo,
                    source_flags: mask.group_flags,
                    raster_bounds: Some(r),
                    mask: Some(MaskState {
                        owner: ItemRef::Channel(binding.channel),
                        bounds: r,
                        offset: [mask.offset.0, mask.offset.1],
                        outside_f64_bits: 0f64.to_bits(),
                        enabled: mask.mask_enabled,
                        shown: mask.mask_shown,
                        editing: mask.mask_editing,
                        linked: mask.legacy_linked,
                    }),
                    composition: None,
                });
                mask_id = Some(id);
            }
            effects.push(StoredSceneEffect {
                channel: binding.channel,
                filter,
                mask: mask_id,
                channel_ordinal: ordinal,
                effect_ordinal,
                composition_unresolved: true,
            });
        }
        channels.push(StoredSceneChannel { channel: binding.channel, source_ordinal: ordinal, selection: d.selection, active: d.active, effects });
    }
    m.step(1)?;
    let requirements = SceneChannelRequirements {
        decoder_retained_bytes: add(owner.reservations.retained_bytes(), extra_bytes)?,
        actual_decoded_file_bytes: actual_file,
        extra_decoder_reservation_bytes: extra_bytes,
        decoded_census_work_units: census_work,
        source_retained_bytes: owner.reservations.source_bytes(),
        current_retained_bytes: current.retained_bytes() as u64,
        output_bytes: m.resident,
        index_bytes: add(held_index, m.index)?,
        work_units: m.work,
    };
    Ok(SceneChannelStorage {
        owner,
        logical,
        current,
        channels,
        masks,
        mask_items: items,
        requirements,
        _decoder_extra: decoder_extra,
        _output: output,
        _index: index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> Budget<'static> {
        Budget::new(photocraft_xcf::ResourceLimits::for_policy(photocraft_xcf::ResourcePolicy::default()).unwrap(), None)
    }
    fn meter<'c, 'b, 'a, 'i>(shared: &'b Budget<'a>, ctl: &'c Interrupt<'i>) -> Meter<'c, 'b, 'a, 'i> {
        Meter { limits: SceneChannelLimits::default(), resident: 0, index: 0, work: 0, records: 0, shared, ctl }
    }
    #[test]
    fn complete_mask_plan_preserves_f64_overshoot_and_signed_words_without_unit_validation() {
        let values = [-0f64, 1.0000000361224046, 0.5f64];
        let bytes: Vec<u8> = values.into_iter().flat_map(f64::to_ne_bytes).collect();
        let mut mask = Drawable::default();
        mask.width = 3;
        mask.height = 1;
        mask.model = photocraft_xcf::Model::Gray;
        mask.plane = Some(photocraft_xcf::Plane { width: 3, height: 1, channels: 1, sample: Sample::F64, bytes: bytes.clone() });
        let shared = budget();
        let ctl = Interrupt::NONE;
        let mut m = meter(&shared, &ctl);
        let (r, f) = mask_plan(&mask, &mut m, true).unwrap();
        let out = photocraft_raster::Surface::try_from_interleaved_with(f, r, &bytes, &ctl).unwrap();
        assert_eq!(out.try_read_interleaved(r).unwrap(), bytes);
        assert!(m.resident > bytes.len() as u64);
        let before = mask.plane.as_ref().unwrap().bytes.clone();
        m.limits.resident_bytes = 0;
        assert!(mask_plan(&mask, &mut m, true).is_err());
        assert_eq!(mask.plane.as_ref().unwrap().bytes, before);
    }
    #[test]
    fn raw_source_offsets_and_late_outer_mismatch_refuse_before_native_copies() {
        let shared = budget();
        let ctl = Interrupt::NONE;
        let mut m = meter(&shared, &ctl);
        let property = RawProperty { tag: 45, offset: u64::MAX, declared_bytes: 4, payload: vec![0; 4] };
        assert!(m.properties(&[property], b"own source").is_err());
        assert_eq!(m.resident, 0);
        let mut effect = Effect {
            name: "Own unresolved".into(),
            icon: String::new(),
            operation: "gegl:unknown-storage-operation".into(),
            version: Some("0:0".into()),
            properties: vec![RawProperty { tag: 35, offset: 0, declared_bytes: 4, payload: 0xe67b8251u32.to_be_bytes().to_vec() }],
            arguments: Vec::new(),
            mask: None,
            state: Default::default(),
        };
        effect.state.composite_mode = Some(0xe67b8251u32 as i32);
        outer(&effect, &mut m).unwrap();
        effect.state.composite_mode = Some(0x6e2a8251u32 as i32);
        assert!(outer(&effect, &mut m).is_err());
        assert_eq!(m.resident, 0);
    }
    #[test]
    fn quoted_ast_and_graph_spans_retain_exact_text_and_refuse_aliasing() {
        let shared = budget();
        let ctl = Interrupt::NONE;
        let mut m = meter(&shared, &ctl);
        quoted(br#""own\n\001""#, b"own\n\x01", &mut m).unwrap();
        assert!(quoted(br#""own\n\001""#, b"own\n\x02", &mut m).is_err());
        let text = "exposure exposure=0.125";
        let entries = vec![
            photocraft_xcf::GraphEntry::Operation { identity: "exposure".into(), start: 0, end: 8 },
            photocraft_xcf::GraphEntry::Property {
                name: "exposure".into(),
                value: photocraft_xcf::GraphValue::Literal { source: "0.125".into(), numeric_bits: Some(0.125f64.to_bits()) },
                start: 9,
                end: text.len() as u64,
            },
        ];
        graph_spans(&entries, text, 0, &mut 0, &mut m).unwrap();
        assert!(graph_spans(&entries, "exposure exposure=0.250", 0, &mut 0, &mut m).is_err());
    }
    #[test]
    fn namespace_exhaustion_and_cancel_refuse_without_allocations() {
        let mut next = u64::MAX;
        assert!(fresh(&mut next).is_err());
        assert_eq!(next, u64::MAX);
        let shared = budget();
        let ctl = Interrupt::cancel_only(&|| true);
        let mut m = meter(&shared, &ctl);
        assert!(matches!(m.vector::<StoredSceneChannel>(2), Err(IoError::Cancelled)));
        assert_eq!(m.resident, 0);
    }
    #[test]
    fn editable_source_graph_and_direct_resource_id_are_in_fresh_namespace() {
        use photocraft_doc::{
            item_identity::{GraphNodeId, ResourceId},
            xcf_filters::{FilterArgument, FilterGraph, FilterValue},
            xcf_items::{EditableSource, SourceItem},
        };
        let source = SourceItem {
            owner: ItemRef::Layer(photocraft_doc::LayerId(3)),
            source: EditableSource::Linked {
                resource: ResourceId(90),
                last_verified_hash: None,
                current: None,
                configuration: vec![FilterArgument {
                    name: "own retained graph".into(),
                    value: FilterValue::Graph(FilterGraph {
                        input: Some(GraphNodeId(1000000)),
                        nodes: Vec::new(),
                        edges: Vec::new(),
                        output: GraphNodeId(2000000),
                        original_pipeline: None,
                    }),
                }],
            },
            cache: None,
            transform: None,
        };
        let shared = budget();
        let ctl = Interrupt::NONE;
        let mut m = meter(&shared, &ctl);
        let mut max = 0;
        current_source(&source, &mut max, &mut m).unwrap();
        assert_eq!(max, 2000000);
        let mut next = max + 1;
        assert!(fresh(&mut next).unwrap() > max);
    }
    #[test]
    fn graph_numeric_missing_some_is_not_an_alias_for_exact_literal() {
        let shared = budget();
        let ctl = Interrupt::NONE;
        let mut m = meter(&shared, &ctl);
        let text = "value=0.125";
        let entries = vec![photocraft_xcf::GraphEntry::Property {
            name: "value".into(),
            start: 0,
            end: text.len() as u64,
            value: photocraft_xcf::GraphValue::Literal { source: "0.125".into(), numeric_bits: None },
        }];
        assert!(graph_spans(&entries, text, 0, &mut 0, &mut m).is_err());
        assert_eq!(m.resident, 0);
    }
}
