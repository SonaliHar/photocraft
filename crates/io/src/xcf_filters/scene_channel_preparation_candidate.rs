//! Explicit INNER point-stage evaluation of retained saved-channel stacks.
//! This is not filter-stack rendering: unresolved composition, masks, opacity,
//! clipping, region and disabled effects are never silently interpreted. The
//! caller must supply a complete ordered graph and actual Gray Float working
//! source/profile/ports. The entire storage owner remains borrowed, including
//! unrelated/pruned backing. No source/default import admission is added.
use super::scene_channel_configured::ConfiguredSceneStacks;
use crate::IoError;
use photocraft_algo::gimp::{
    configured::Limits as ConfiguredLimits,
    graph_execute::{self, DomainAssignment, ExecutionDomain, ExecutionLimits, GraphExecuteError, GraphOutput},
    graph_pipeline,
    graph_plan::PortDirection,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_compose::{
    gimp_brightness_contrast_candidate as brightness, gimp_brightness_contrast_graph_candidate as brightness_graph,
    gimp_configured_color_candidate::profile_owned_color_requirements, gimp_configured_color_graph_candidate as configured_graph,
    gimp_configured_graph_preparation::ConfiguredWitness, gimp_render::PreparationBudget,
};
use photocraft_doc::{
    item_identity::{ChannelId, ItemRef},
    xcf_filters::{FilterArgument, FilterGraph, FilterNode, FilterStack, FilterValue, GraphNode},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::{
    mem::size_of,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OuterPolicy {
    /// Original outer state remains retained and explicitly UNEVALUATED.
    UnappliedInnerStage,
    ApplyOriginalOuter,
}
pub enum EffectWitness<'a> {
    CurvesOrLevels(ConfiguredWitness<'a>),
    BrightnessContrast { original: &'a FilterNode, node: &'a GraphNode, binding: brightness::SourceBinding<'a>, limits: brightness::Limits },
}
/// Exactly one supplied witness per stored effect, in original order. Supplied
/// graph identities are compared, never derived from FilterId or record offsets.
pub struct ChannelWitness<'a> {
    pub stack: &'a FilterStack,
    pub graph: &'a FilterGraph,
    pub effects: &'a [EffectWitness<'a>],
    pub working_source: &'a Surface,
    pub domain: ExecutionDomain,
    pub full_extent: Rect,
    pub assignments: &'a [DomainAssignment],
    pub outer: OuterPolicy,
    pub scheduler: ExecutionLimits,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES.min(1 << 30), index_bytes: 16 << 20, work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub preparation_work_units: u64,
    pub execution_work_units: u64,
    pub prepared_retained_bytes: u64,
    pub vector_bytes: u64,
}
enum PreparedEffect<'a> {
    Configured(configured_graph::ProfileOwnedBinding<'a>),
    Brightness(brightness::Prepared<'a, ()>),
}
#[derive(Clone, Copy)]
struct NodeQuota {
    peak: u64,
    retained: u64,
    work: u64,
}
struct PreparedChannel<'a> {
    witness: &'a ChannelWitness<'a>,
    effects: Vec<PreparedEffect<'a>>,
    work: AtomicU64,
}
/// Move-only private preparation. Output is an inner numerical candidate only.
/// Its lease holds the complete prepaid owner/constructor/scheduler envelope.
pub struct PreparedSceneChannels<'a, 'o, 'p, 's, 'd, 'b, 'r, L> {
    original: &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r>,
    channels: Vec<PreparedChannel<'a>>,
    requirements: Requirements,
    work: AtomicU64,
    ceiling: Limits,
    _lease: L,
}
fn bad(message: &str) -> IoError {
    IoError::Unsupported(format!("configured channel inner-stage: {message}"))
}
fn poll(c: &Interrupt<'_>) -> Result<(), IoError> {
    c.check().map_err(|_| IoError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_add(b).ok_or_else(|| bad("size/work overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64, IoError> {
    a.checked_mul(b).ok_or_else(|| bad("size/work overflow"))
}
fn bytes<T>(count: usize) -> Result<u64, IoError> {
    mul(count as u64, size_of::<T>() as u64)
}
fn raster(e: photocraft_raster::RasterInputError) -> IoError {
    if e == photocraft_raster::RasterInputError::Cancelled { IoError::Cancelled } else { bad(&e.to_string()) }
}
fn composition(e: photocraft_compose::gimp_composite::CompositionError) -> IoError {
    match e {
        photocraft_compose::gimp_composite::CompositionError::Raster(e) => raster(e),
        _ => bad(&e.to_string()),
    }
}
fn graph_error(e: GraphExecuteError) -> IoError {
    if matches!(e, GraphExecuteError::Cancelled) { IoError::Cancelled } else { bad(&e.to_string()) }
}
struct Census<'c, 'i> {
    resident: u64,
    index: u64,
    work: u64,
    visits: u64,
    limits: Limits,
    control: &'c Interrupt<'i>,
}
impl Census<'_, '_> {
    fn charge(&mut self, resident: u64, index: u64, work: u64) -> Result<(), IoError> {
        poll(self.control)?;
        self.resident = add(self.resident, resident)?;
        self.index = add(self.index, index)?;
        self.work = add(self.work, work)?;
        if self.resident > self.limits.resident_bytes || self.index > self.limits.index_bytes || self.work > self.limits.work_units {
            return Err(bad("lower aggregate envelope exhausted"));
        }
        Ok(())
    }
    fn string(&mut self, s: &String) -> Result<(), IoError> {
        self.charge(s.capacity() as u64, 0, add(s.len() as u64, 1)?)
    }
    fn arguments(&mut self, args: &Vec<FilterArgument>, depth: usize) -> Result<(), IoError> {
        if depth > 32 || args.len() > 4096 {
            return Err(bad("bounded configuration depth/count"));
        }
        self.charge(bytes::<FilterArgument>(args.capacity())?, bytes::<FilterArgument>(args.capacity())?, args.len() as u64)?;
        for a in args {
            self.visits = add(self.visits, 1)?;
            if self.visits > 100_000 {
                return Err(bad("configuration visit limit"));
            }
            self.string(&a.name)?;
            match &a.value {
                FilterValue::String(v) => self.string(v)?,
                FilterValue::Enum { domain, .. } => self.string(domain)?,
                FilterValue::Config(v) => self.arguments(v, depth + 1)?,
                FilterValue::Color { components, .. } => {
                    self.charge(bytes::<photocraft_doc::xcf_filters::ExactNumber>(components.capacity())?, 0, components.len() as u64)?
                }
                FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => {
                    self.charge(
                        add(
                            add(bytes::<[u64; 2]>(points_f64_bits.capacity())?, bytes::<i32>(point_types.capacity())?)?,
                            bytes::<u64>(samples_f64_bits.capacity())?,
                        )?,
                        0,
                        add(add(points_f64_bits.len() as u64, point_types.len() as u64)?, samples_f64_bits.len() as u64)?,
                    )?;
                }
                // Graph/resource-backed configuration is not part of these exact
                // three closed kernels. Refuse before any comparison/copy.
                FilterValue::Graph(_) | FilterValue::Resource(_) => return Err(bad("explicit nested graph/resource stage unsupported")),
                FilterValue::Number(_) | FilterValue::Bool(_) => {}
            }
        }
        Ok(())
    }
    fn filter(&mut self, node: &FilterNode) -> Result<(), IoError> {
        self.charge(size_of::<FilterNode>() as u64, 0, 1)?;
        self.string(&node.operation.name)?;
        self.string(&node.operation.stored_version)?;
        self.string(&node.operation.schema_epoch)?;
        for s in [node.source_display_name.as_ref(), node.source_icon.as_ref()].into_iter().flatten() {
            self.string(s)?;
        }
        self.charge(bytes::<(String, ItemRef)>(node.auxiliary.capacity())?, 0, node.auxiliary.len() as u64)?;
        for (name, _) in &node.auxiliary {
            self.string(name)?;
        }
        if let Some(c) = &node.composition {
            self.string(&c.compatibility_epoch)?;
        }
        self.arguments(&node.arguments, 0)
    }
    fn graph(&mut self, graph: &FilterGraph) -> Result<(), IoError> {
        if graph.nodes.len() > 4096 || graph.edges.len() > 4096 {
            return Err(bad("point chain count"));
        }
        let index = add(bytes::<GraphNode>(graph.nodes.capacity())?, bytes::<photocraft_doc::xcf_filters::GraphEdge>(graph.edges.capacity())?)?;
        self.charge(index, index, 1)?;
        if let Some(v) = &graph.original_pipeline {
            self.string(v)?;
        }
        for node in &graph.nodes {
            self.string(&node.operation.name)?;
            self.string(&node.operation.stored_version)?;
            self.string(&node.operation.schema_epoch)?;
            self.arguments(&node.arguments, 0)?;
        }
        for edge in &graph.edges {
            self.string(&edge.input_port)?;
            self.string(&edge.output_port)?;
        }
        Ok(())
    }
    fn surface(&mut self, s: &Surface) -> Result<(), IoError> {
        self.charge(add(size_of::<Surface>() as u64, mul(s.tile_count() as u64, 256)?)?, 0, add(s.tile_count() as u64, 1)?)?;
        self.charge(s.default_bytes().len() as u64, 0, 1)?;
        for (_, tile) in s.tiles() {
            self.charge(tile.bytes().len() as u64, 0, 1)?;
        }
        s.try_validate_tile_origins_with(self.control).map_err(raster)
    }
}
fn effect_nodes<'a>(e: &EffectWitness<'a>) -> (&'a FilterNode, &'a GraphNode) {
    match e {
        EffectWitness::CurvesOrLevels(v) => (v.source_filter, v.graph_node),
        EffectWitness::BrightnessContrast { original, node, .. } => (original, node),
    }
}
fn ports<'a>(e: &EffectWitness<'a>) -> (&'a str, &'a str, &'a str) {
    match e {
        EffectWitness::CurvesOrLevels(v) => (v.parameters.input_port, v.parameters.upstream_output_port, v.parameters.output_port),
        EffectWitness::BrightnessContrast { .. } => ("input", "output", "output"),
    }
}
fn text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.contains('\0')
}
/// Validates supplied topology without allocating a replacement graph/IDs. A
/// closed linear chain is deliberate: no disabled/pruned effect is discarded.
fn shape(w: &ChannelWitness<'_>, c: &Interrupt<'_>) -> Result<(), IoError> {
    poll(c)?;
    if w.outer != OuterPolicy::UnappliedInnerStage {
        return Err(bad("original outer mask/composition evaluation unsupported"));
    }
    if !matches!(w.stack.owner, ItemRef::Channel(_))
        || w.effects.len() > 4096
        || w.graph.nodes.len() != w.effects.len()
        || w.stack.nodes.len() != w.effects.len()
        || w.graph.edges.len() != w.effects.len()
        || w.assignments.len() != w.effects.len()
        || w.full_extent.is_empty()
        || w.domain.format.mode != ColorMode::Grayscale
        || w.domain.format.alpha
        || !matches!(w.domain.format.sample, SampleType::F32 | SampleType::F64)
        || w.domain.profile_fingerprint.is_none_or(|v| v == [0; 32])
        || w.working_source.format() != w.domain.format
    {
        return Err(bad("complete Gray Float point-chain working witness required"));
    }
    let mut previous = w.graph.input.ok_or_else(|| bad("explicit external input identity required"))?;
    if previous.0 == 0 {
        return Err(bad("zero external graph identity"));
    }
    for (i, effect) in w.effects.iter().enumerate() {
        poll(c)?;
        let (original, node) = effect_nodes(effect);
        let stored = w.stack.nodes.get(i).ok_or_else(|| bad("ordered stored node"))?;
        let declared = w.graph.nodes.get(i).ok_or_else(|| bad("ordered graph node"))?;
        if !std::ptr::eq(original, stored)
            || !std::ptr::eq(node, declared)
            || !original.enabled
            || !original.auxiliary.is_empty()
            || original.operation.name == "gimp:hue-saturation"
            || !matches!(original.operation.name.as_str(), "gimp:curves" | "gimp:levels" | "gimp:brightness-contrast")
            || node.id.0 == 0
            || node.id == previous
            || Some(node.id) == w.graph.input
        {
            return Err(bad("exact enabled ordered supported point-stage association"));
        }
        for prior in w.graph.nodes.iter().take(i) {
            poll(c)?;
            if prior.id == node.id {
                return Err(bad("duplicate graph identity"));
            }
        }
        let (input, upstream, output) = ports(effect);
        if ![input, upstream, output].into_iter().all(text) {
            return Err(bad("explicit bounded ports"));
        }
        let edge = w.graph.edges.get(i).ok_or_else(|| bad("declared ordered edge"))?;
        if edge.from != previous || edge.to != node.id || edge.input_port != input || edge.output_port != upstream {
            return Err(bad("exact declared linear input edge"));
        }
        if let Some(prior) = i.checked_sub(1).and_then(|p| w.effects.get(p)) {
            if ports(prior).2 != upstream {
                return Err(bad("upstream output port mismatch"));
            }
        }
        if w.assignments.get(i).is_none_or(|a| a.node != node.id || a.domain != w.domain) {
            return Err(bad("exact current per-node domain"));
        }
        if node.operation != original.operation || node.arguments != original.arguments {
            return Err(bad("retained operation/native arguments changed"));
        }
        match effect {
            EffectWitness::CurvesOrLevels(v) => {
                if !matches!(original.operation.name.as_str(), "gimp:curves" | "gimp:levels")
                    || v.parameters.node != node.id
                    || v.parameters.version_origin != node.version_origin
                    || v.parameters.source_domain != w.domain
                    || v.parameters.source_extent != w.full_extent
                {
                    return Err(bad("explicit configured/TRC/native domain witness"));
                }
            }
            EffectWitness::BrightnessContrast { binding, .. } => {
                if original.operation.name != "gimp:brightness-contrast"
                    || binding.context.encoding != w.domain.encoding
                    || Some(binding.context.profile_identity) != w.domain.profile_fingerprint
                {
                    return Err(bad("explicit brightness Float working context"));
                }
            }
        }
        previous = node.id;
    }
    if w.graph.output != previous {
        return Err(bad("complete ordered output, no silently pruned effects"));
    }
    Ok(())
}
fn configured_limits(w: &ChannelWitness<'_>, remaining: Limits) -> ConfiguredLimits {
    ConfiguredLimits {
        resident_bytes: w.scheduler.resident_bytes.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES).min(remaining.resident_bytes),
        work_units: w.scheduler.work_units.min(1_000_000_000).min(remaining.work_units),
    }
}
fn effect_quota(e: &EffectWitness<'_>, w: &ChannelWitness<'_>, remaining: Limits, c: &Interrupt<'_>) -> Result<(u64, u64, u64), IoError> {
    match e {
        EffectWitness::CurvesOrLevels(v) => {
            let q = profile_owned_color_requirements(
                v.source_filter,
                v.parameters.schema,
                v.profile_basis,
                v.current_encoding,
                v.current_context,
                configured_limits(w, remaining),
                c,
            )
            .map_err(raster)?;
            Ok((q.peak_resident_bytes, q.retained_bytes, q.work_units))
        }
        EffectWitness::BrightnessContrast { original, binding, limits, .. } => {
            let lower = brightness::Limits {
                resident_bytes: limits.resident_bytes.min(remaining.resident_bytes),
                work_units: limits.work_units.min(remaining.work_units),
            };
            let q = brightness::requirements(original, *binding, w.domain.format, lower, c).map_err(composition)?;
            Ok((q.retained_bytes, q.retained_bytes, q.preparation_work_units))
        }
    }
}
/// Allocation-free prospective census. Caller-provided graph/canonical config
/// storage is measured, not accepted through asserted byte counts. Decoder and
/// current-document owners use the genuine held scene receipts. Reservations
/// count conservatively per owner; no dedup/refund or process-RSS assertion.
pub fn requirements(
    scene: &ConfiguredSceneStacks<'_, '_, '_, '_, '_, '_>,
    witnesses: &[ChannelWitness<'_>],
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<Requirements, IoError> {
    requirements_each(scene, witnesses, limits, c, |_| Ok(()))
}
fn requirements_each(
    scene: &ConfiguredSceneStacks<'_, '_, '_, '_, '_, '_>,
    witnesses: &[ChannelWitness<'_>],
    limits: Limits,
    c: &Interrupt<'_>,
    mut record: impl FnMut(NodeQuota) -> Result<(), IoError>,
) -> Result<Requirements, IoError> {
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.index_bytes > max.index_bytes || limits.work_units > max.work_units {
        return Err(bad("lower-only scene limits"));
    }
    let mut m = Census { resident: 0, index: 0, work: 0, visits: 0, limits, control: c };
    m.charge(4096, 0, 1)?;
    if witnesses.len() != scene.stacks().len() || witnesses.len() > 10_000 {
        return Err(bad("one ordered witness per complete saved-channel stack"));
    }
    // Refuse policy/unsupported node shapes before any owned preparation.
    // Actual backing census bounds recursive equality before shape compares.
    let s = scene.scene().requirements();
    let mapping = scene.requirements();
    let held = add(add(s.decoder_retained_bytes, s.source_retained_bytes)?, add(s.current_retained_bytes, s.output_bytes)?)?;
    m.charge(held, s.index_bytes, 1)?;
    m.charge(mapping.charged_resident_bytes, mapping.charged_index_bytes, 1)?;
    m.charge(bytes::<ChannelWitness<'_>>(witnesses.len())?, bytes::<ChannelWitness<'_>>(witnesses.len())?, 1)?;
    let mut vectors = bytes::<PreparedChannel<'_>>(witnesses.len())?;
    let mut retained = 0;
    let mut execution = 0;
    for (w, stack) in witnesses.iter().zip(scene.stacks()) {
        if !std::ptr::eq(w.stack, stack) {
            return Err(bad("exact ordered scene stack borrow"));
        }
        m.charge(bytes::<EffectWitness<'_>>(w.effects.len())?, bytes::<EffectWitness<'_>>(w.effects.len())?, 1)?;
        if w.outer != OuterPolicy::UnappliedInnerStage {
            return Err(bad("original outer mask/composition evaluation unsupported"));
        }
        m.graph(w.graph)?;
        for n in &stack.nodes {
            m.filter(n)?;
        }
        m.surface(w.working_source)?;
        m.charge(bytes::<DomainAssignment>(w.assignments.len())?, bytes::<DomainAssignment>(w.assignments.len())?, 1)?;
        for e in w.effects {
            match e {
                EffectWitness::CurvesOrLevels(v) => {
                    // Borrowed caller fields remain held even when configuration
                    // factories derive their own profile models.
                    m.charge(v.parameters.working_default.len() as u64, 0, 1)?;
                    for s in [
                        v.parameters.input_port,
                        v.parameters.upstream_output_port,
                        v.parameters.output_port,
                        v.parameters.schema.expected_epoch,
                        v.parameters.schema.schema_sha256,
                    ] {
                        m.charge(s.len() as u64, 0, s.len() as u64)?;
                    }
                }
                EffectWitness::BrightnessContrast { binding, .. } => {
                    m.charge(
                        add(binding.expected_epoch.len() as u64, binding.schema_sha256.len() as u64)?,
                        0,
                        add(binding.expected_epoch.len() as u64, binding.schema_sha256.len() as u64)?,
                    )?;
                }
            }
        }
        // Charge all bounded linear identity/edge lookups before validation.
        m.charge(0, 0, mul(add(w.effects.len() as u64, 1)?, mul(add(w.effects.len() as u64, 1)?, 32)?)?)?;
        shape(w, c)?;
        if w.scheduler.resident_bytes == 0
            || w.scheduler.index_bytes == 0
            || w.scheduler.work_units == 0
            || w.scheduler.resident_bytes > limits.resident_bytes
            || w.scheduler.index_bytes > limits.index_bytes
            || w.scheduler.work_units > limits.work_units
            || w.scheduler.cache_bytes > w.scheduler.resident_bytes
        {
            return Err(bad("immutable lower scheduler partition"));
        }
        let plan = graph_execute::planning_requirements(w.graph.nodes.len(), w.graph.edges.len()).map_err(graph_error)?;
        m.charge(plan.index_bytes, plan.index_bytes, plan.work_units)?;
        for e in w.effects {
            let remaining = Limits {
                resident_bytes: limits.resident_bytes.saturating_sub(m.resident),
                index_bytes: limits.index_bytes.saturating_sub(m.index),
                work_units: limits.work_units.saturating_sub(m.work) / 3,
            };
            let (peak, keep, work) = effect_quota(e, w, remaining, c)?;
            m.charge(peak, 0, mul(work, 3)?)?;
            retained = add(retained, keep)?;
            record(NodeQuota { peak, retained: keep, work })?;
        }
        vectors = add(vectors, bytes::<PreparedEffect<'_>>(w.effects.len())?)?;
        vectors = add(vectors, bytes::<NodeQuota>(w.effects.len())?)?;
        m.charge(w.scheduler.resident_bytes, w.scheduler.index_bytes, 1)?;
        execution = add(execution, w.scheduler.work_units)?;
    }
    m.charge(vectors, vectors, 1)?;
    // The private preparation replays this allocation-free census to record the
    // exact prospective subquotas; that pass is also charged, without refunds.
    let preparation_work = mul(m.work, 2)?;
    if add(preparation_work, execution)? > limits.work_units {
        return Err(bad("constructor and cumulative scheduler work envelope"));
    }
    poll(c)?;
    Ok(Requirements {
        peak_resident_bytes: m.resident,
        index_bytes: m.index,
        preparation_work_units: preparation_work,
        execution_work_units: execution,
        prepared_retained_bytes: retained,
        vector_bytes: vectors,
    })
}

/// Child preparation receives a slice of the already-held aggregate reservation.
/// Unit leases cannot escape the aggregate owner and are never source authority.
struct ChildBudget {
    bytes: u64,
    work: u64,
}
impl PreparationBudget for ChildBudget {
    type Lease = ();
    fn reserve_prepared_bytes(&mut self, bytes: u64) -> photocraft_compose::gimp_composite::Result<()> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn consume_preparation_work(&mut self, work: u64) -> photocraft_compose::gimp_composite::Result<()> {
        self.work = self.work.checked_sub(work).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn copy_parameters<'a>(p: &configured_graph::ProfileOwnedParameters<'a>) -> configured_graph::ProfileOwnedParameters<'a> {
    configured_graph::ProfileOwnedParameters {
        node: p.node,
        version_origin: p.version_origin,
        schema: p.schema,
        source_domain: p.source_domain,
        source_extent: p.source_extent,
        working_default: p.working_default,
        input_port: p.input_port,
        upstream_output_port: p.upstream_output_port,
        output_port: p.output_port,
        output_policy: p.output_policy,
    }
}
pub fn prepare<'a, 'o, 'p, 's, 'd, 'b, 'r, B: PreparationBudget>(
    scene: &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r>,
    witnesses: &'a [ChannelWitness<'a>],
    limits: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<PreparedSceneChannels<'a, 'o, 'p, 's, 'd, 'b, 'r, B::Lease>, IoError> {
    poll(c)?;
    if size_of::<PreparedSceneChannels<'a, 'o, 'p, 's, 'd, 'b, 'r, B::Lease>>() > 4096 {
        return Err(bad("actual aggregate lease/header exceeds prepaid header"));
    }
    let q = requirements(scene, witnesses, limits, c)?;
    budget.consume_preparation_work(q.preparation_work_units).map_err(composition)?;
    let lease = budget.reserve_prepared_bytes(q.peak_resident_bytes).map_err(composition)?;
    let quota_count = witnesses.iter().try_fold(0usize, |n, w| n.checked_add(w.effects.len()).ok_or_else(|| bad("quota count overflow")))?;
    let mut quotas = Vec::new();
    quotas.try_reserve_exact(quota_count).map_err(|_| bad("quota allocation"))?;
    if quotas.capacity() != quota_count {
        return Err(bad("actual quota capacity exceeds prepaid vector"));
    }
    requirements_each(scene, witnesses, limits, c, |quota| {
        if quotas.len() >= quota_count {
            return Err(bad("quota association count"));
        }
        quotas.push(quota);
        Ok(())
    })?;
    let mut channels = Vec::new();
    channels.try_reserve_exact(witnesses.len()).map_err(|_| bad("channel allocation"))?;
    let mut actual_vectors = add(bytes::<PreparedChannel<'_>>(channels.capacity())?, bytes::<NodeQuota>(quotas.capacity())?)?;
    if actual_vectors > q.vector_bytes {
        return Err(bad("actual channel capacity"));
    }
    let mut actual_retained = 0;
    let mut position = 0usize;
    for w in witnesses {
        poll(c)?;
        let mut effects = Vec::new();
        effects.try_reserve_exact(w.effects.len()).map_err(|_| bad("effect allocation"))?;
        actual_vectors = add(actual_vectors, bytes::<PreparedEffect<'_>>(effects.capacity())?)?;
        if actual_vectors > q.vector_bytes {
            return Err(bad("actual effect capacity"));
        }
        for e in w.effects {
            poll(c)?;
            let quota = quotas.get(position).copied().ok_or_else(|| bad("exact recorded constructor quota"))?;
            position = position.checked_add(1).ok_or_else(|| bad("quota position overflow"))?;
            let (peak, keep, work) = (quota.peak, quota.retained, quota.work);
            let prepared = match e {
                EffectWitness::CurvesOrLevels(v) => {
                    let p = configured_graph::prepare_profile_owned_binding(
                        v.source_filter,
                        copy_parameters(&v.parameters),
                        v.profile_basis,
                        v.current_encoding,
                        v.current_context,
                        ConfiguredLimits { resident_bytes: peak, work_units: work },
                        c,
                    )
                    .map_err(raster)?;
                    let receipt = p.requirements_receipt();
                    if receipt.retained_bytes > keep || receipt.preparation_work_units > work {
                        return Err(bad("actual configured constructor exceeds admitted receipt"));
                    }
                    actual_retained = add(actual_retained, receipt.retained_bytes)?;
                    PreparedEffect::Configured(p)
                }
                EffectWitness::BrightnessContrast { original, binding, limits, .. } => {
                    let mut child = ChildBudget { bytes: keep, work };
                    let p = brightness::prepare(original, *binding, w.domain.format, *limits, &mut child, c).map_err(composition)?;
                    actual_retained = add(actual_retained, p.requirements().retained_bytes)?;
                    PreparedEffect::Brightness(p)
                }
            };
            if actual_retained > q.prepared_retained_bytes {
                return Err(bad("actual aggregate constructor retention"));
            }
            effects.push(prepared);
        }
        channels.push(PreparedChannel { witness: w, effects, work: AtomicU64::new(0) });
    }
    poll(c)?;
    Ok(PreparedSceneChannels { original: scene, channels, requirements: q, work: AtomicU64::new(q.preparation_work_units), ceiling: limits, _lease: lease })
}
impl<'a, 'o, 'p, 's, 'd, 'b, 'r, L> PreparedSceneChannels<'a, 'o, 'p, 's, 'd, 'b, 'r, L> {
    pub fn original(&self) -> &'a ConfiguredSceneStacks<'o, 'p, 's, 'd, 'b, 'r> {
        self.original
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    pub fn remaining_work(&self) -> u64 {
        self.ceiling.work_units.saturating_sub(self.work.load(Ordering::Relaxed))
    }
    /// Private numerical stage only. Each attempt prepays the complete immutable
    /// scheduler partition, including failed/cancelled attempts; no refund/reset.
    /// Returned outputs must stay within the enclosing aggregate lifetime/lease.
    pub fn execute_channel<'v>(&'v self, channel: ChannelId, roi: Rect, c: &Interrupt<'_>) -> Result<GraphOutput<'v>, IoError> {
        poll(c)?;
        let prepared =
            self.channels.iter().find(|p| p.witness.stack.owner == ItemRef::Channel(channel)).ok_or_else(|| bad("unknown exact channel identity"))?;
        let w = prepared.witness;
        // BrightnessContrast's existing callback requires full authoritative ROI;
        // this facade deliberately keeps that contract for every chain.
        if roi.is_empty() || roi != w.full_extent {
            return Err(bad("exact declared full-authoritative ROI required"));
        }
        let amount = w.scheduler.work_units;
        prepared
            .work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(amount).filter(|n| *n <= amount))
            .map_err(|_| bad("channel execution partition consumed"))?;
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(amount).filter(|n| *n <= self.ceiling.work_units))
            .map_err(|_| bad("aggregate cumulative execution work exhausted"))?;
        graph_pipeline::execute_pipeline_domains(
            w.graph,
            4096,
            Some(w.working_source),
            roi,
            w.domain,
            w.assignments,
            w.scheduler,
            c,
            |id, direction, port| {
                if Some(id) == w.graph.input {
                    return direction == PortDirection::Output && w.effects.first().is_some_and(|e| ports(e).1 == port);
                }
                w.effects.iter().find(|e| effect_nodes(e).1.id == id).is_some_and(|e| match direction {
                    PortDirection::Input => ports(e).0 == port,
                    PortDirection::Output => ports(e).2 == port,
                })
            },
            |request, node_budget| {
                let position = w.graph.nodes.iter().position(|n| std::ptr::eq(n, request.node)).ok_or(GraphExecuteError::InvalidPlan)?;
                let p = prepared.effects.get(position).ok_or(GraphExecuteError::InvalidPlan)?;
                match p {
                    PreparedEffect::Configured(p) => configured_graph::evaluate_profile_owned(p, request, node_budget),
                    PreparedEffect::Brightness(p) => {
                        let binding = brightness_graph::Binding {
                            node: request.node,
                            version_origin: request.node.version_origin,
                            prepared: p,
                            context: p.context(),
                            domain: w.domain,
                            full_extent: w.full_extent,
                        };
                        brightness_graph::evaluate(&binding, request, node_budget)
                    }
                }
            },
        )
        .map_err(|e| match e {
            graph_pipeline::GraphPipelineError::Execute(e) => graph_error(e),
            graph_pipeline::GraphPipelineError::Plan(photocraft_algo::gimp::graph_plan::GraphPlanError::Cancelled) => IoError::Cancelled,
            _ => bad(&e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        SampleEncoding,
        item_identity::{FilterId, GraphNodeId},
        xcf_filters::{ExactNumber, FilterGraph, GraphEdge, GraphNode, GraphVersionOrigin, OperationIdentity},
    };

    fn filter() -> FilterNode {
        FilterNode {
            id: FilterId(7),
            operation: OperationIdentity { name: "gimp:brightness-contrast".into(), stored_version: "0:0".into(), schema_epoch: "captured-epoch".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument {
                name: "config".into(),
                value: FilterValue::Config(
                    [("time", 0.0f64), ("brightness", 0.25), ("contrast", 0.0)]
                        .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(value.to_bits())) })
                        .into(),
                ),
            }],
            enabled: true,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(1),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(photocraft_doc::item_identity::MaskId(9))),
            auxiliary: vec![],
            composition: None,
        }
    }
    fn graph(original: &FilterNode) -> FilterGraph {
        FilterGraph {
            input: Some(GraphNodeId(81)),
            nodes: vec![GraphNode {
                id: GraphNodeId(82),
                operation: original.operation.clone(),
                version_origin: GraphVersionOrigin::Stored,
                arguments: original.arguments.clone(),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(81), to: GraphNodeId(82), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(82),
            original_pipeline: None,
        }
    }
    #[test]
    fn exact_inner_stage_retains_outer_words_but_outer_application_refuses() {
        let original = filter();
        let before = original.clone();
        let stack = FilterStack { owner: ItemRef::Channel(ChannelId(4)), nodes: vec![original], revision: 5 };
        let graph = graph(&stack.nodes[0]);
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::new(format);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([3; 32]) };
        let effects = [EffectWitness::BrightnessContrast {
            original: &stack.nodes[0],
            node: &graph.nodes[0],
            binding: brightness::SourceBinding {
                xcf_version: 22,
                expected_epoch: "captured-epoch",
                schema_sha256: brightness::SCHEMA,
                context: photocraft_algo::gimp::wide_point::WorkingContext { encoding: domain.encoding, profile_identity: [3; 32] },
                policy: brightness::NumericPolicy::DeclaredIecFloatHypothesis,
            },
            limits: brightness::Limits::default(),
        }];
        let assignments = [DomainAssignment { node: GraphNodeId(82), domain }];
        let mut witness = ChannelWitness {
            stack: &stack,
            graph: &graph,
            effects: &effects,
            working_source: &source,
            domain,
            full_extent: Rect::new(0, 0, 32, 32),
            assignments: &assignments,
            outer: OuterPolicy::UnappliedInnerStage,
            scheduler: ExecutionLimits::default(),
        };
        assert!(shape(&witness, &Interrupt::NONE).is_ok());
        assert_eq!(stack.nodes[0], before);
        witness.outer = OuterPolicy::ApplyOriginalOuter;
        assert!(shape(&witness, &Interrupt::NONE).is_err());
        assert_eq!(stack.nodes[0], before);
    }
    #[test]
    fn absent_float_domain_and_cancel_refuse_before_any_preparation() {
        let stack = FilterStack { owner: ItemRef::Channel(ChannelId(4)), nodes: vec![], revision: 0 };
        let graph = FilterGraph { input: Some(GraphNodeId(1)), nodes: vec![], edges: vec![], output: GraphNodeId(1), original_pipeline: None };
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let source = Surface::new(format);
        let witness = ChannelWitness {
            stack: &stack,
            graph: &graph,
            effects: &[],
            working_source: &source,
            domain: ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([1; 32]) },
            full_extent: Rect::new(0, 0, 1, 1),
            assignments: &[],
            outer: OuterPolicy::UnappliedInnerStage,
            scheduler: ExecutionLimits::default(),
        };
        assert!(shape(&witness, &Interrupt::NONE).is_err());
        let stop = || true;
        assert!(matches!(shape(&witness, &Interrupt::cancel_only(&stop)), Err(IoError::Cancelled)));
    }
    #[test]
    fn recursive_configuration_and_tiny_resident_are_bounded_without_copies() {
        let mut value = FilterValue::Bool(true);
        for _ in 0..34 {
            value = FilterValue::Config(vec![FilterArgument { name: "nested".into(), value }]);
        }
        let arguments = vec![FilterArgument { name: "config".into(), value }];
        let mut census = Census { resident: 0, index: 0, work: 0, visits: 0, limits: Limits::default(), control: &Interrupt::NONE };
        assert!(census.arguments(&arguments, 0).is_err());
        let mut tiny = Census {
            resident: 0,
            index: 0,
            work: 0,
            visits: 0,
            limits: Limits { resident_bytes: 0, index_bytes: 0, work_units: 1 },
            control: &Interrupt::NONE,
        };
        assert!(tiny.arguments(&arguments, 0).is_err());
        assert_eq!(tiny.visits, 0);
    }
    #[test]
    fn child_slice_has_no_independent_reset_or_unpaid_capacity() {
        let mut child = ChildBudget { bytes: 10, work: 7 };
        child.reserve_prepared_bytes(8).unwrap();
        child.consume_preparation_work(5).unwrap();
        assert!(child.reserve_prepared_bytes(3).is_err());
        assert!(child.consume_preparation_work(3).is_err());
        assert_eq!((child.bytes, child.work), (2, 2));
    }
}
