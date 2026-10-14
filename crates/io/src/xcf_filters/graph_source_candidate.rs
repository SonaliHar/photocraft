//! Bound exceptional GEGL graph effect envelope. Not stock/default admission.
//! GIMP's development-flag reopen preserves this source; default-profile reopen
//! discards it. A caller policy here does not enroll fallback execution or grants.
use super::configured_source_candidate::{CurvesMappingLimits, Meter, raw_argument, word};
use crate::{
    IoError,
    xcf_graph_candidate::{self, GraphCandidate},
    xcf_graphs,
};
use photocraft_algo::gimp::{graph_execute::ExecutionDomain, typed_schema::PreparedSchemaIndex};
use photocraft_doc::{
    item_identity::{FilterId, GraphNodeId, MaskId},
    xcf_filters::{GraphVersionOrigin, OperationIdentity},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{Argument, ArgumentValue, Effect, GraphEntry, GraphValue, RawProperty};
use std::mem::size_of;

#[derive(Clone, Copy, Debug)]
pub enum ExceptionalGraphProvenance {
    RetainedDevelopmentFlagDefaultDiscard,
}
#[derive(Clone, Copy)]
pub struct GraphSourceAuthority<'a> {
    pub outer_identity: &'a OperationIdentity,
    pub outer_schema_sha256: &'a str,
    /// Chain nodes have no stored version; each exact schema resolver supplies it.
    pub nested_version_origin: GraphVersionOrigin,
    pub external_domain: ExecutionDomain,
    pub provenance: ExceptionalGraphProvenance,
}
#[derive(Clone, Copy, Debug)]
pub struct GraphSourceLimits {
    pub envelope: CurvesMappingLimits,
    pub graph: xcf_graph_candidate::Limits,
}
impl Default for GraphSourceLimits {
    fn default() -> Self {
        Self { envelope: Default::default(), graph: Default::default() }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct GraphSourceRequirements {
    pub reserved_resident_bytes: u64,
    pub reserved_index_bytes: u64,
    pub reserved_work_units: u64,
    pub selected_input_bytes: u64,
    pub envelope_work_units: u64,
    pub bridge_charged_bytes: u64,
    pub bridge_index_bytes: u64,
    pub bridge_index_preparation_work: u64,
    pub requires_provider_index_admission: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct GraphOuterMetadata {
    pub filter: FilterId,
    pub mask: Option<MaskId>,
    pub enabled: bool,
    pub opacity_f32_bits: u32,
    pub mode: i32,
    pub blend_space: i32,
    pub composite_space: i32,
    pub composite_mode: i32,
    pub clipping: u32,
    pub region: u32,
}
pub struct MappedGraphSource<'a, P> {
    original: &'a Effect<P>,
    source: &'a [u8],
    error: &'a str,
    authority: GraphSourceAuthority<'a>,
    outer: GraphOuterMetadata,
    graph: GraphCandidate<'a>,
    requirements: GraphSourceRequirements,
}
impl<'a, P> MappedGraphSource<'a, P> {
    pub fn original_effect(&self) -> &'a Effect<P> {
        self.original
    }
    pub fn original_source(&self) -> &'a [u8] {
        self.source
    }
    pub fn original_error(&self) -> &'a str {
        self.error
    }
    pub fn authority(&self) -> GraphSourceAuthority<'a> {
        self.authority
    }
    pub fn outer(&self) -> GraphOuterMetadata {
        self.outer
    }
    pub fn candidate(&self) -> &GraphCandidate<'a> {
        &self.graph
    }
    pub fn requirements(&self) -> GraphSourceRequirements {
        self.requirements
    }
}
fn bad(s: &str) -> IoError {
    IoError::Unsupported(format!("exceptional graph source: {s}"))
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size overflow"))
}
fn metadata<P>(effect: &Effect<P>, filter: FilterId, mask: Option<MaskId>) -> Result<GraphOuterMetadata, IoError> {
    if filter.0 == 0 || mask.is_some_and(|v| v.0 == 0 || v.0 == filter.0) || mask.is_some() != effect.mask.is_some() {
        return Err(bad("canonical mask association"));
    }
    let state = &effect.state;
    let opacity = state.opacity_bits.ok_or_else(|| bad("Float32 opacity absent"))?;
    if state.opacity_u8.is_some() || !f32::from_bits(opacity).is_finite() || !(0.0..=1.0).contains(&f32::from_bits(opacity)) {
        return Err(bad("Float32 opacity"));
    }
    Ok(GraphOuterMetadata {
        filter,
        mask,
        enabled: state.enabled.ok_or_else(|| bad("enabled absent"))?,
        opacity_f32_bits: opacity,
        mode: state.mode.ok_or_else(|| bad("mode absent"))?,
        blend_space: state.blend_space.ok_or_else(|| bad("blend space absent"))?,
        composite_space: state.composite_space.ok_or_else(|| bad("composite space absent"))?,
        composite_mode: state.composite_mode.ok_or_else(|| bad("operator absent"))?,
        clipping: state.clipping.ok_or_else(|| bad("clipping absent"))?,
        region: state.region.ok_or_else(|| bad("region absent"))?,
    })
}
fn graph_input(entries: &Vec<GraphEntry>, depth: usize, count: &mut usize, limits: GraphSourceLimits, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    if depth > limits.graph.mapping.depth {
        return Err(bad("graph depth"));
    }
    m.input(mul(entries.capacity() as u64, size_of::<GraphEntry>() as u64)?)?;
    for entry in entries {
        m.step(1)?;
        *count = count.checked_add(1).filter(|v| *v <= limits.graph.mapping.entries).ok_or_else(|| bad("graph entries"))?;
        match entry {
            GraphEntry::Operation { identity, .. } => m.input_text(identity)?,
            GraphEntry::Property { name, value, .. } => {
                m.input_text(name)?;
                match value {
                    GraphValue::Literal { source, .. } => m.input_text(source)?,
                    GraphValue::Chain(children) => graph_input(children, depth + 1, count, limits, m)?,
                }
            }
        }
    }
    Ok(())
}
fn outer_words<P>(effect: &Effect<P>, outer: GraphOuterMetadata, m: &mut Meter<'_, '_>) -> Result<(), IoError> {
    let mut seen = 0u16;
    let mut arguments = 0usize;
    for p in &effect.properties {
        m.step(8)?;
        if p.tag == 45 {
            arguments = arguments.checked_add(1).ok_or_else(|| bad("argument count overflow"))?;
            continue;
        }
        let (slot, expected) = match p.tag {
            8 => (0, u32::from(outer.enabled)),
            33 => (1, outer.opacity_f32_bits),
            7 => (2, outer.mode as u32),
            37 => (3, outer.blend_space as u32),
            36 => (4, outer.composite_space as u32),
            35 => (5, outer.composite_mode as u32),
            46 => (6, outer.clipping),
            44 => (7, outer.region),
            _ => return Err(bad("unknown outer property")),
        };
        let bit = 1u16 << slot;
        if seen & bit != 0 || p.payload.len() != 4 || word(&p.payload, 0)? != expected {
            return Err(bad("outer raw/state mismatch or duplicate"));
        }
        seen |= bit;
    }
    if seen != 255 || arguments != 2 {
        return Err(bad("complete outer/argument property envelope"));
    }
    Ok(())
}
/// Exact two stored type4 strings (`string`, `error`), including the known empty
/// error field. Complete plain source/decoder/mask and opaque validator captures
/// remain caller-held; selected source storage is also conservatively debited here.
/// The aggregate reserves the actual AST-derived mapper/planner index envelope
/// plus the source envelope before invoking any bridge allocator. Opaque resolver
/// argument/index allocations remain a separately admitted provider boundary.
/// Ports, resource/aux authority, profile domains and schema-derived node versions
/// are explicit. Neither this adapter nor its plan resolves or executes locators.
#[allow(clippy::too_many_arguments)]
pub fn map_exceptional_graph_source<'a, P>(
    source: &'a [u8],
    effect: &'a Effect<P>,
    filter: FilterId,
    mask: Option<MaskId>,
    authority: GraphSourceAuthority<'a>,
    schemas: &PreparedSchemaIndex<'_, '_>,
    limits: GraphSourceLimits,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> GraphNodeId,
    resolve: &mut xcf_graphs::GraphSchemaResolver<'_>,
    ports: &mut xcf_graph_candidate::PortValidator<'_>,
    auxiliary: &mut xcf_graph_candidate::AuxiliaryValidator<'_>,
    resources: &mut xcf_graph_candidate::ResourceValidator<'_>,
    reserve: &mut dyn FnMut(u64, u64) -> Result<(), IoError>,
    charge: &mut dyn FnMut(u64) -> Result<(), IoError>,
) -> Result<MappedGraphSource<'a, P>, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let max = GraphSourceLimits::default();
    let e = limits.envelope;
    let g = limits.graph;
    let gm = g.mapping;
    let d = max.graph.mapping;
    if e.resident_bytes > max.envelope.resident_bytes
        || e.index_bytes > max.envelope.index_bytes
        || e.work_units > max.envelope.work_units
        || gm.nodes > d.nodes
        || gm.edges > d.edges
        || gm.depth > d.depth
        || gm.entries > d.entries
        || gm.resident_bytes > d.resident_bytes
        || g.work_units > max.graph.work_units
        || g.dependency_depth == 0
        || g.dependency_depth > max.graph.dependency_depth
    {
        return Err(bad("lower-only limits"));
    }
    if effect.operation != "gegl:gegl"
        || effect.version.as_deref() != Some(authority.outer_identity.stored_version.as_str())
        || authority.outer_identity.name != effect.operation
        || authority.outer_identity.stored_version != "0:0"
        || authority.outer_identity.schema_epoch.is_empty()
        || authority.outer_identity.schema_epoch.len() > 128
        || authority.outer_identity.schema_epoch.contains('\0')
        || authority.outer_schema_sha256.len() != 64
        || !authority.outer_schema_sha256.bytes().all(|v| v.is_ascii_hexdigit())
        || authority.nested_version_origin != GraphVersionOrigin::SchemaResolved
        || authority.external_domain.profile_fingerprint.is_none_or(|hash| hash == [0; 32])
        || effect.arguments.len() != 2
        || effect.properties.len() > 32
        || !photocraft_xcf::is_xcf(source)
    {
        return Err(bad("explicit exceptional source/schema/version/domain binding"));
    }
    let outer = metadata(effect, filter, mask)?;
    let resident = add(e.resident_bytes, gm.resident_bytes as u64)?;
    let work = add(e.work_units, g.work_units)?;
    charge(work)?;
    let stored = effect.arguments.iter().find(|a| a.name == "string").and_then(|a| a.graph.as_ref()).ok_or_else(|| bad("retained graph absent"))?;
    let index_requirements = xcf_graph_candidate::prospective_index_requirements(stored, schemas, g, ctl)?;
    let index = add(e.index_bytes, index_requirements.known_index_bytes)?;
    reserve(resident, index)?;
    let bridge_limits = xcf_graph_candidate::Limits {
        work_units: g.work_units.checked_sub(index_requirements.work_units).ok_or_else(|| bad("index preparation work slice"))?,
        ..g
    };
    let mut m = Meter { limits: e, input: 0, copied: size_of::<MappedGraphSource<'a, P>>() as u64, index: 0, work: 0, ctl };
    m.check()?;
    m.input(size_of::<Effect<P>>() as u64)?;
    for v in [&effect.name, &effect.icon, &effect.operation] {
        m.input_text(v)?;
    }
    if let Some(v) = &effect.version {
        m.input_text(v)?;
    }
    m.input(mul(effect.properties.capacity() as u64, size_of::<RawProperty>() as u64)?)?;
    for p in &effect.properties {
        m.input(p.payload.capacity() as u64)?;
        if p.payload.len() > 1 << 20 || usize::try_from(p.declared_bytes).ok() != Some(p.payload.len()) {
            return Err(bad("property bound"));
        }
        let start = usize::try_from(p.offset).map_err(|_| bad("source offset"))?;
        let end = start.checked_add(p.payload.len()).ok_or_else(|| bad("source extent"))?;
        let actual = source.get(start..end).ok_or_else(|| bad("source property extent"))?;
        for (a, b) in actual.chunks(4096).zip(p.payload.chunks(4096)) {
            m.step(mul(a.len() as u64, 2)?)?;
            if a != b {
                return Err(bad("raw/source mismatch"));
            }
        }
    }
    outer_words(effect, outer, &mut m)?;
    m.input(mul(effect.arguments.capacity() as u64, size_of::<Argument>() as u64)?)?;
    let mut string = None;
    let mut error = None;
    for a in &effect.arguments {
        if !matches!(a.name.as_str(), "string" | "error") || a.configuration.is_some() {
            return Err(bad("known graph string fields required"));
        }
        m.input_text(&a.name)?;
        m.step(add(mul(a.name.len() as u64, 2)?, 1)?)?;
        let (raw, end) = raw_argument(a, &effect.properties)?;
        let ArgumentValue::String(value) = &a.value else {
            return Err(bad("type4 string required"));
        };
        m.input_text(value)?;
        if word(raw, end)? != 4 {
            return Err(bad("raw type4 string"));
        }
        let at = end.checked_add(4).ok_or_else(|| bad("string offset"))?;
        let len = usize::try_from(word(raw, at)?).map_err(|_| bad("string length"))?;
        let start = at.checked_add(4).ok_or_else(|| bad("string start"))?;
        let stop = start.checked_add(len).ok_or_else(|| bad("string extent"))?;
        m.step(add(mul(value.len() as u64, 2)?, 1)?)?;
        if stop != raw.len() || raw.get(start..stop).and_then(|v| v.strip_suffix(&[0])) != Some(value.as_bytes()) {
            return Err(bad("raw argument string mismatch"));
        }
        match a.name.as_str() {
            "string" if string.is_none() => {
                let graph = a.graph.as_ref().ok_or_else(|| bad("retained graph absent"))?;
                if graph.nodes > gm.nodes as u64 || graph.edges > gm.edges as u64 {
                    return Err(bad("declared graph size"));
                }
                m.input(size_of::<photocraft_xcf::StoredGraph>() as u64)?;
                graph_input(&graph.entries, 0, &mut 0, limits, &mut m)?;
                string = Some((value.as_str(), graph));
            }
            "error" if error.is_none() && a.graph.is_none() && value.is_empty() => error = Some(value.as_str()),
            _ => return Err(bad("duplicate field or nonempty retained error")),
        }
    }
    let (original, graph) = string.ok_or_else(|| bad("string absent"))?;
    let error = error.ok_or_else(|| bad("error absent"))?;
    // Existing bridge owns mapping/schema/planner allocations and exact f64
    // literals. Its full quota was prepaid above; callback admits every debit
    // against that same held slice without reset/refund or a second allocation.
    let mut bridge_bytes = 0;
    let mapped =
        xcf_graph_candidate::map_with_schemas(graph, original, schemas, bridge_limits, ctl, fresh, resolve, ports, auxiliary, resources, &mut |bytes| {
            ctl.check().map_err(|_| IoError::Cancelled)?;
            bridge_bytes = add(bridge_bytes, bytes as u64)?;
            if bridge_bytes > gm.resident_bytes as u64 {
                return Err(bad("bridge resident slice"));
            }
            Ok(())
        })?;
    for node in &mapped.graph().nodes {
        m.step(1)?;
        if node.version_origin != authority.nested_version_origin {
            return Err(bad("nested version origin differs"));
        }
    }
    m.step(1)?;
    let requirements = GraphSourceRequirements {
        reserved_resident_bytes: resident,
        reserved_index_bytes: index,
        reserved_work_units: work,
        selected_input_bytes: m.input,
        envelope_work_units: m.work,
        bridge_charged_bytes: bridge_bytes,
        bridge_index_bytes: index_requirements.known_index_bytes,
        bridge_index_preparation_work: index_requirements.work_units,
        requires_provider_index_admission: index_requirements.requires_provider_index_admission,
    };
    Ok(MappedGraphSource { original: effect, source, error, authority, outer, graph: mapped, requirements })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outer_words_and_float_opacity_are_not_resolved_or_quantized() {
        let mut e: Effect<photocraft_xcf::Plane> = Effect {
            name: String::new(),
            icon: String::new(),
            operation: "gegl:gegl".into(),
            version: Some("0:0".into()),
            properties: Vec::new(),
            arguments: Vec::new(),
            mask: None,
            state: Default::default(),
        };
        e.state = photocraft_xcf::EffectState {
            enabled: Some(true),
            opacity_bits: Some((-0f32).to_bits()),
            mode: Some(62),
            blend_space: Some(0xccf68251u32 as i32),
            composite_space: Some(i32::MIN),
            composite_mode: Some(i32::MAX),
            clipping: Some(0),
            region: Some(0),
            ..Default::default()
        };
        let m = metadata(&e, FilterId(2), None).unwrap();
        assert_eq!(m.opacity_f32_bits, (-0f32).to_bits());
        assert_eq!(m.blend_space, 0xccf68251u32 as i32);
        assert!(metadata(&e, FilterId(2), Some(MaskId(3))).is_err());
    }
    #[test]
    fn retained_graph_census_polls_before_visits_and_rejects_overflow() {
        let stop = || true;
        let ctl = Interrupt::cancel_only(&stop);
        let mut m = Meter { limits: Default::default(), input: 0, copied: 0, index: 0, work: 0, ctl: &ctl };
        assert!(matches!(graph_input(&Vec::new(), 0, &mut 0, Default::default(), &mut m), Err(IoError::Cancelled)));
        assert!(add(u64::MAX, 1).is_err());
    }
}
