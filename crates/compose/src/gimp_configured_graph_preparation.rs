//! Explicit retained GraphNode/FilterNode preparation witnesses.
//! No relation, port, source schema, auxiliary serialization or authority is
//! synthesized. Default renderer/engine qualification is deliberately separate.
use crate::{
    gimp_configured_candidate::retained_source_transfer,
    gimp_configured_color_candidate::{ProfileOwnedColorRequirements, profile_owned_color_requirements},
    gimp_configured_color_graph_candidate::{ProfileOwnedBinding, ProfileOwnedParameters, prepare_profile_owned_binding},
    gimp_configured_graph_execution::{Census, OwnedSource},
};
use photocraft_algo::gimp::resident_source_graph;
use photocraft_algo::gimp::{
    configured::{Limits as NodeLimits, Transfer, WorkingContext},
    graph_execute::{DomainAssignment, ExecutionDomain, ExecutionLimits, GraphExecuteError as E, GraphOutput},
    graph_pipeline::GraphPipelineError,
    graph_plan::{self, PortDirection},
};
use photocraft_cms::{Profile, xcf_color::Encoding};
use photocraft_doc::xcf_filters::{FilterGraph, FilterNode, GraphNode};
use photocraft_raster::{Interrupt, Surface};
use std::mem::size_of;

#[derive(Clone, Copy, Debug)]
pub struct BatchRequirements {
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
    pub prepared_retained_bytes: u64,
    pub vector_bytes: u64,
}
/// Immutable phase partition. The execution reserve is retained alongside all
/// constructor owners and is never refreshed to the enclosing full ceiling.
pub struct PreparedGraph<'a> {
    graph: &'a FilterGraph,
    bindings: Vec<ProfileOwnedBinding<'a>>,
    sources: &'a [OwnedSource<'a>],
    backing: &'a [&'a [u8]],
    external: Option<&'a Surface>,
    external_domain: ExecutionDomain,
    assignments: &'a [DomainAssignment],
    max_depth: usize,
    scheduler_reserve: ExecutionLimits,
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::ResidentLimit)
}
fn mult(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::ResidentLimit)
}
fn size(n: usize, width: usize) -> Result<u64, E> {
    u64::try_from(n.checked_mul(width).ok_or(E::ResidentLimit)?).map_err(|_| E::ResidentLimit)
}
fn node_limits(limits: ExecutionLimits) -> NodeLimits {
    NodeLimits { resident_bytes: limits.resident_bytes.min(photocraft_raster::MAX_CHECKED_RASTER_BYTES), work_units: limits.work_units.min(1_000_000_000) }
}
fn quota(w: &ConfiguredWitness<'_>, limits: ExecutionLimits, c: &Interrupt<'_>) -> Result<ProfileOwnedColorRequirements, E> {
    profile_owned_color_requirements(w.source_filter, w.parameters.schema, w.profile_basis, w.current_encoding, w.current_context, node_limits(limits), c)
        .map_err(raster)
}
fn plan_error(error: graph_plan::GraphPlanError) -> E {
    match error {
        graph_plan::GraphPlanError::Cancelled => E::Cancelled,
        graph_plan::GraphPlanError::Allocation => E::Allocation,
        graph_plan::GraphPlanError::WorkLimit => E::WorkLimit,
        _ => E::InvalidPlan,
    }
}
/// Uses original declared edges and caller ports; no graph/auxiliary relation is
/// synthesized. Every configured node needs exactly its one declared input,
/// including nodes later pruned from execution.
fn validate_topology(
    graph: &FilterGraph,
    witnesses: &[ConfiguredWitness<'_>],
    sources: &[OwnedSource<'_>],
    external_domain: ExecutionDomain,
    max_depth: usize,
    control: &Interrupt<'_>,
) -> Result<(), E> {
    for witness in witnesses {
        poll(control)?;
        let p = &witness.parameters;
        let mut count = 0usize;
        for edge in &graph.edges {
            poll(control)?;
            if edge.to == p.node {
                count = count.checked_add(1).ok_or(E::InvalidPlan)?;
                if edge.input_port != p.input_port || edge.output_port != p.upstream_output_port {
                    return Err(E::InvalidPlan);
                }
                let upstream = if Some(edge.from) == graph.input {
                    Some(external_domain)
                } else {
                    witnesses
                        .iter()
                        .find(|w| w.parameters.node == edge.from)
                        .map(|w| w.parameters.source_domain)
                        .or_else(|| sources.iter().find(|s| s.node.id == edge.from).map(|s| s.domain))
                };
                if upstream.is_some_and(|domain| domain != p.source_domain) {
                    return Err(E::Domain);
                }
            }
        }
        if count != 1 {
            return Err(E::MissingInput);
        }
    }
    let plan = graph_plan::plan_graph(graph, max_depth, control, |id, direction, port| {
        if let Some(w) = witnesses.iter().find(|w| w.parameters.node == id) {
            let p = &w.parameters;
            return match direction {
                PortDirection::Input => port == p.input_port,
                PortDirection::Output => port == p.output_port,
            };
        }
        if let Some(s) = sources.iter().find(|s| s.node.id == id) {
            return direction == PortDirection::Output && port == s.output_port;
        }
        Some(id) == graph.input && direction == PortDirection::Output && witnesses.iter().any(|w| w.parameters.upstream_output_port == port)
    })
    .map_err(plan_error)?;
    poll(control)?;
    drop(plan);
    Ok(())
}
fn phase_caps(limits: ExecutionLimits, reserve: ExecutionLimits) -> Result<(), E> {
    if reserve.resident_bytes == 0
        || reserve.index_bytes == 0
        || reserve.work_units == 0
        || reserve.resident_bytes > limits.resident_bytes
        || reserve.index_bytes > limits.index_bytes
        || reserve.cache_bytes > limits.cache_bytes
        || reserve.work_units > limits.work_units
    {
        return Err(E::RaisedLimit);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn requirements(
    graph: &FilterGraph,
    witnesses: &[ConfiguredWitness<'_>],
    sources: &[OwnedSource<'_>],
    backing: &[&[u8]],
    external: Option<&Surface>,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    limits: ExecutionLimits,
    scheduler_reserve: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<BatchRequirements, E> {
    phase_caps(limits, scheduler_reserve)?;
    let (held, index, visits) = borrowed_census(graph, witnesses, sources, backing, external, external_domain, assignments, limits, control)?;
    let planner = photocraft_algo::gimp::graph_execute::planning_requirements(graph.nodes.len(), graph.edges.len())?;
    let mut peak = add(held, planner.index_bytes)?;
    let mut retained = 0;
    let mut work = add(visits, planner.work_units)?;
    for witness in witnesses {
        poll(control)?;
        let q = quota(witness, limits, control)?;
        peak = add(peak, q.peak_resident_bytes)?;
        retained = add(retained, q.retained_bytes)?;
        work = add(work, mult(q.work_units, 3)?)?;
        if peak > limits.resident_bytes {
            return Err(E::ResidentLimit);
        }
        if work > limits.work_units {
            return Err(E::WorkLimit);
        }
    }
    let vector_bytes = add(
        size(witnesses.len(), size_of::<ProfileOwnedBinding<'_>>())?,
        add(size(witnesses.len(), size_of::<ProfileOwnedColorRequirements>())?, size(witnesses.len(), size_of::<&ProfileOwnedBinding<'_>>())?)?,
    )?;
    peak = add(add(peak, vector_bytes)?, scheduler_reserve.resident_bytes)?;
    let index_bytes = add(add(add(index, vector_bytes)?, planner.index_bytes)?, scheduler_reserve.index_bytes)?;
    work = add(work, scheduler_reserve.work_units)?;
    if peak > limits.resident_bytes {
        return Err(E::ResidentLimit);
    }
    if index_bytes > limits.index_bytes {
        return Err(E::IndexLimit);
    }
    if work > limits.work_units {
        return Err(E::WorkLimit);
    }
    poll(control)?;
    Ok(BatchRequirements { peak_resident_bytes: peak, index_bytes, work_units: work, prepared_retained_bytes: retained, vector_bytes })
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_graph<'a>(
    graph: &'a FilterGraph,
    witnesses: &[ConfiguredWitness<'a>],
    sources: &'a [OwnedSource<'a>],
    backing: &'a [&'a [u8]],
    external: Option<&'a Surface>,
    external_domain: ExecutionDomain,
    assignments: &'a [DomainAssignment],
    max_depth: usize,
    limits: ExecutionLimits,
    scheduler_reserve: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<PreparedGraph<'a>, E> {
    if !(1..=4096).contains(&max_depth) {
        return Err(E::InvalidPlan);
    }
    let admitted = requirements(graph, witnesses, sources, backing, external, external_domain, assignments, limits, scheduler_reserve, control)?;
    validate_topology(graph, witnesses, sources, external_domain, max_depth, control)?;
    // The full batch was admitted before either vector reserve or CMS constructor.
    let mut quotas = Vec::new();
    quotas.try_reserve_exact(witnesses.len()).map_err(|_| E::Allocation)?;
    let mut bindings = Vec::new();
    bindings.try_reserve_exact(witnesses.len()).map_err(|_| E::Allocation)?;
    let mut actual_vectors =
        add(size(quotas.capacity(), size_of::<ProfileOwnedColorRequirements>())?, size(bindings.capacity(), size_of::<ProfileOwnedBinding<'_>>())?)?;
    actual_vectors = add(actual_vectors, size(witnesses.len(), size_of::<&ProfileOwnedBinding<'_>>())?)?;
    if actual_vectors > admitted.vector_bytes {
        return Err(E::IndexLimit);
    }
    for w in witnesses {
        poll(control)?;
        quotas.push(quota(w, limits, control)?);
    }
    let mut retained = 0;
    for (w, q) in witnesses.iter().zip(&quotas) {
        poll(control)?;
        let p = &w.parameters;
        let parameters = ProfileOwnedParameters {
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
        };
        let binding = prepare_profile_owned_binding(
            w.source_filter,
            parameters,
            w.profile_basis,
            w.current_encoding,
            w.current_context,
            NodeLimits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units },
            control,
        )
        .map_err(raster)?;
        if binding.execution_retained() > q.retained_bytes || binding.execution_preparation_work() > q.work_units {
            return Err(E::ResidentLimit);
        }
        retained = add(retained, binding.execution_retained())?;
        bindings.push(binding);
    }
    if retained > admitted.prepared_retained_bytes {
        return Err(E::ResidentLimit);
    }
    poll(control)?;
    Ok(PreparedGraph { graph, bindings, sources, backing, external, external_domain, assignments, max_depth, scheduler_reserve })
}
impl<'a> PreparedGraph<'a> {
    pub fn original_graph(&self) -> &'a FilterGraph {
        self.graph
    }
    pub fn prepared_bindings(&self) -> &[ProfileOwnedBinding<'a>] {
        &self.bindings
    }
    /// Uses the retained execution partition; no caller may raise construction or
    /// execution caps. Output stays private, with no document/engine publication.
    pub fn execute(&self, roi: photocraft_geom::Rect, control: &Interrupt<'_>) -> Result<GraphOutput<'a>, GraphPipelineError> {
        poll(control).map_err(GraphPipelineError::Execute)?;
        let mut refs = Vec::new();
        refs.try_reserve_exact(self.bindings.len()).map_err(|_| GraphPipelineError::Execute(E::Allocation))?;
        if refs.capacity() != self.bindings.len() {
            return Err(GraphPipelineError::Execute(E::IndexLimit));
        }
        for b in &self.bindings {
            poll(control).map_err(GraphPipelineError::Execute)?;
            refs.push(b);
        }
        crate::gimp_configured_graph_execution::execute(
            self.graph,
            &refs,
            self.sources,
            self.backing,
            self.external,
            roi,
            self.external_domain,
            self.assignments,
            self.max_depth,
            self.scheduler_reserve,
            control,
        )
    }
}

/// Exact borrowed inventory; graph origin is compared with the explicit
/// parameters, while retained operation/version/arguments match the source DTO.
pub struct ConfiguredWitness<'a> {
    pub graph_node: &'a GraphNode,
    pub source_filter: &'a FilterNode,
    pub parameters: ProfileOwnedParameters<'a>,
    pub profile_basis: &'a Profile,
    pub current_encoding: Encoding,
    pub current_context: WorkingContext,
}

fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn raster(e: photocraft_raster::RasterInputError) -> E {
    if e == photocraft_raster::RasterInputError::Cancelled { E::Cancelled } else { E::Raster(e) }
}
fn parameters_valid(w: &ConfiguredWitness<'_>, c: &Interrupt<'_>) -> Result<(), E> {
    poll(c)?;
    let p = &w.parameters;
    let actual = w.current_context;
    let encoding = match actual.transfer {
        Transfer::Linear => photocraft_doc::SampleEncoding::Linear,
        Transfer::NonlinearProfile => photocraft_doc::SampleEncoding::NonlinearProfile,
        Transfer::Perceptual => photocraft_doc::SampleEncoding::Perceptual,
    };
    if p.source_extent.is_empty()
        || !matches!(p.source_domain.format.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        || actual.profile_identity == [0; 32]
        || p.source_domain.profile_fingerprint != Some(actual.profile_identity)
        || p.source_domain.encoding != encoding
        || p.source_domain.format.mode.color_channels() != actual.color_channels
        || p.source_domain.format.alpha != actual.alpha
    {
        return Err(E::Domain);
    }
    for port in [p.input_port, p.upstream_output_port, p.output_port] {
        if port.is_empty() || port.len() > 256 || port.contains('\0') {
            return Err(E::InvalidPlan);
        }
    }
    let mode = match actual.color_channels {
        1 => photocraft_color::ColorMode::Grayscale,
        3 => photocraft_color::ColorMode::Rgb,
        _ => return Err(E::Domain),
    };
    let format = photocraft_color::PixelFormat::new(mode, photocraft_color::SampleType::F64, actual.alpha);
    if p.working_default.len() != format.bytes_per_pixel() {
        return Err(E::Domain);
    }
    photocraft_raster::checked_region_byte_len(p.source_extent, p.source_domain.format).map_err(raster)?;
    for channel in 0..format.channels() {
        poll(c)?;
        let value = photocraft_color::try_read_sample_f64(p.working_default, format.sample, channel).map_err(|_| E::Domain)?;
        if !value.is_finite() || (actual.alpha && channel == actual.color_channels && !(0.0..=1.0).contains(&value)) {
            return Err(E::Domain);
        }
    }
    poll(c)
}
/// Shared bounded graph/source/config census; no CMS construction or allocations.
/// Caller context backing stays explicit and no source locator is read.
pub fn validate_witnesses(
    graph: &FilterGraph,
    witnesses: &[ConfiguredWitness<'_>],
    sources: &[OwnedSource<'_>],
    backing: &[&[u8]],
    external: Option<&Surface>,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<(), E> {
    borrowed_census(graph, witnesses, sources, backing, external, external_domain, assignments, limits, control).map(|_| ())
}
fn borrowed_census(
    graph: &FilterGraph,
    witnesses: &[ConfiguredWitness<'_>],
    sources: &[OwnedSource<'_>],
    backing: &[&[u8]],
    external: Option<&Surface>,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<(u64, u64, u64), E> {
    poll(control)?;
    let max = ExecutionLimits::default();
    if limits.resident_bytes > max.resident_bytes
        || limits.index_bytes > max.index_bytes
        || limits.work_units > max.work_units
        || limits.cache_bytes > max.cache_bytes
    {
        return Err(E::RaisedLimit);
    }
    if graph.nodes.len() > 4096
        || graph.edges.len() > 16384
        || witnesses.len() > 4096
        || sources.len() > 4096
        || backing.len() > 4096
        || assignments.len() != graph.nodes.len()
        || witnesses.len().checked_add(sources.len()) != Some(graph.nodes.len())
        || graph.input.is_some() != external.is_some()
    {
        return Err(E::InvalidPlan);
    }
    if let Some(s) = external {
        if s.format() != external_domain.format {
            return Err(E::Domain);
        }
    }
    let mut c = Census::new(limits, control);
    let index = c.graph_storage(graph, witnesses.len(), sources.len(), assignments.len())?;
    c.charge(size(witnesses.len(), size_of::<ConfiguredWitness<'_>>())?, 1)?;
    for bytes in backing {
        c.charge(u64::try_from(bytes.len()).map_err(|_| E::ResidentLimit)?, 1)?;
    }
    if let Some(s) = external {
        c.surface(s)?;
    }
    for s in sources {
        poll(control)?;
        if s.validation_work == 0 {
            return Err(E::InvalidPlan);
        }
        c.surface(s.surface)?;
        c.charge(u64::try_from(s.output_port.len()).map_err(|_| E::ResidentLimit)?, s.validation_work)?;
        (s.validate_arguments)(s.node)?;
        let binding = resident_source_graph::Binding {
            node: s.node,
            surface: s.surface,
            extent: s.extent,
            domain: s.domain,
            output_port: s.output_port,
            validate_arguments: &s.validate_arguments,
        };
        resident_source_graph::requirements(&binding, s.extent, control)?;
    }
    for witness in witnesses {
        poll(control)?;
        let node = witness.graph_node;
        let p = &witness.parameters;
        c.charge(0, 512)?;
        parameters_valid(witness, control)?;
        if !graph.nodes.iter().any(|n| std::ptr::eq(n, node)) || node.id.0 == 0 || p.node != node.id || p.version_origin != node.version_origin {
            return Err(E::InvalidPlan);
        }
        // Existing V22 five-selector validator bounds nested configuration before
        // exact equality, and rejects unknown operation/version/schema domains.
        c.charge(0, 16384)?;
        retained_source_transfer(witness.source_filter, p.schema, control).map_err(raster)?;
        if node.operation != witness.source_filter.operation || node.arguments != witness.source_filter.arguments {
            return Err(E::InvalidPlan);
        }
        c.filter(witness.source_filter)?;
        for bytes in [
            p.working_default,
            p.schema.expected_epoch.as_bytes(),
            p.schema.schema_sha256.as_bytes(),
            p.input_port.as_bytes(),
            p.upstream_output_port.as_bytes(),
            p.output_port.as_bytes(),
        ] {
            c.charge(u64::try_from(bytes.len()).map_err(|_| E::ResidentLimit)?, 1)?;
        }
    }
    for (node, assignment) in graph.nodes.iter().zip(assignments) {
        poll(control)?;
        c.node(node)?;
        if assignment.node != node.id {
            return Err(E::InvalidPlan);
        }
        let mut matches = 0usize;
        for witness in witnesses {
            poll(control)?;
            if witness.parameters.node == node.id {
                matches = matches.checked_add(1).ok_or(E::InvalidPlan)?;
                if assignment.domain != witness.parameters.source_domain {
                    return Err(E::Domain);
                }
            }
        }
        for s in sources {
            poll(control)?;
            if s.node.id == node.id {
                matches = matches.checked_add(1).ok_or(E::InvalidPlan)?;
                if !std::ptr::eq(s.node, node) || assignment.domain != s.domain {
                    return Err(E::Domain);
                }
            }
        }
        if matches != 1 {
            return Err(E::InvalidPlan);
        }
    }
    poll(control)?;
    Ok((c.bytes(), index, c.work()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{SampleEncoding, item_identity::GraphNodeId, xcf_filters::OperationIdentity};
    use photocraft_geom::Rect;
    fn domain() -> ExecutionDomain {
        ExecutionDomain { format: PixelFormat::RGBA8, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([7; 32]) }
    }
    #[test]
    fn early_plan_rejects_cycles_missing_endpoints_and_port_mismatch() {
        use crate::gimp_configured_candidate::{LEVELS_SCHEMA, RetainedSchemaIdentity};
        use crate::gimp_configured_render_candidate::{CoveragePolicy, OutputPolicy};
        use photocraft_color::checked_arithmetic::{Domain, Policy, Rounding, UnitOverflow};
        use photocraft_doc::{item_identity::FilterId, xcf_filters::GraphEdge};
        let op = OperationIdentity { name: "gimp:levels".into(), stored_version: "0:0".into(), schema_epoch: "owned-topology".into() };
        let source = FilterNode {
            id: FilterId(1),
            operation: op.clone(),
            source_display_name: None,
            source_icon: None,
            arguments: Vec::new(),
            enabled: true,
            opacity_f64_bits: 1.0_f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: Vec::new(),
            composition: None,
        };
        let make = |id| GraphNode { id: GraphNodeId(id), operation: op.clone(), version_origin: Default::default(), arguments: Vec::new() };
        let graph = FilterGraph {
            input: None,
            nodes: vec![make(1), make(2)],
            edges: vec![
                GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), output_port: "output".into(), input_port: "input".into() },
                GraphEdge { from: GraphNodeId(2), to: GraphNodeId(1), output_port: "output".into(), input_port: "input".into() },
            ],
            output: GraphNodeId(2),
            original_pipeline: None,
        };
        let policy = Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest };
        let basis = photocraft_cms::Builtin::AdobeRgbCompat.profile();
        let default = [0u8; 32];
        let witnesses: Vec<_> = graph
            .nodes
            .iter()
            .map(|node| ConfiguredWitness {
                graph_node: node,
                source_filter: &source,
                profile_basis: basis,
                current_encoding: Encoding::NonlinearProfile,
                current_context: WorkingContext { transfer: Transfer::NonlinearProfile, profile_identity: [7; 32], color_channels: 3, alpha: true },
                parameters: ProfileOwnedParameters {
                    node: node.id,
                    version_origin: node.version_origin,
                    schema: RetainedSchemaIdentity { xcf_version: 22, expected_epoch: "owned-topology", schema_sha256: LEVELS_SCHEMA },
                    source_domain: domain(),
                    source_extent: Rect::new(0, 0, 1, 1),
                    working_default: &default,
                    input_port: "input",
                    upstream_output_port: "output",
                    output_port: "output",
                    output_policy: OutputPolicy { colors: policy, alpha: policy, coverage: CoveragePolicy::EvaluatedRejectUnit },
                },
            })
            .collect();
        // Structural-only helper deliberately never constructs the dummy source
        // configuration/profile. Both input witnesses are exact borrowed nodes.
        assert_eq!(validate_topology(&graph, &witnesses, &[], domain(), 2, &Interrupt::NONE), Err(E::InvalidPlan));
        let mut changed = graph.clone();
        changed.edges[0].input_port = "other".into();
        assert_eq!(validate_topology(&changed, &witnesses, &[], domain(), 2, &Interrupt::NONE), Err(E::InvalidPlan));
        changed = graph.clone();
        changed.edges[0].from = GraphNodeId(99);
        assert_eq!(validate_topology(&changed, &witnesses, &[], domain(), 2, &Interrupt::NONE), Err(E::InvalidPlan));
        changed = graph.clone();
        changed.edges.push(changed.edges[0].clone());
        assert_eq!(validate_topology(&changed, &witnesses, &[], domain(), 2, &Interrupt::NONE), Err(E::MissingInput));
        assert_eq!(validate_topology(&graph, &witnesses, &[], domain(), 2, &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
    }
    #[test]
    fn prepared_passthrough_keeps_partition_and_raw_borrowed_source() {
        let graph = FilterGraph {
            input: Some(GraphNodeId(1)),
            nodes: Vec::new(),
            edges: Vec::new(),
            output: GraphNodeId(1),
            original_pipeline: Some("owned passthrough".into()),
        };
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[7, 11, 13, 255]).unwrap();
        let before = source.clone();
        let reserve = ExecutionLimits { resident_bytes: 16 << 20, index_bytes: 1 << 20, cache_bytes: 1 << 20, work_units: 1_000_000 };
        let prepared = prepare_graph(&graph, &[], &[], &[], Some(&source), domain(), &[], 1, ExecutionLimits::default(), reserve, &Interrupt::NONE).unwrap();
        assert!(std::ptr::eq(prepared.original_graph(), &graph));
        assert!(prepared.prepared_bindings().is_empty());
        let output = prepared.execute(Rect::new(-2, 7, -1, 8), &Interrupt::NONE).unwrap();
        assert_eq!(output.surface().try_read_interleaved(Rect::new(-2, 7, -1, 8)).unwrap(), [7, 11, 13, 255]);
        assert_eq!(source, before);
        assert!(matches!(prepared.execute(Rect::new(0, 0, 1, 1), &Interrupt::cancel_only(&|| true)), Err(GraphPipelineError::Execute(E::Cancelled))));
        assert!(matches!(
            prepare_graph(
                &graph,
                &[],
                &[],
                &[],
                Some(&source),
                domain(),
                &[],
                1,
                ExecutionLimits { resident_bytes: reserve.resident_bytes - 1, ..Default::default() },
                reserve,
                &Interrupt::NONE
            ),
            Err(E::RaisedLimit)
        ));
    }
    #[test]
    fn backing_caps_and_cancellation_precede_any_profile_construction() {
        let graph = FilterGraph { input: Some(GraphNodeId(1)), nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(1), original_pipeline: None };
        let source = Surface::new(PixelFormat::RGBA8);
        let backing = vec![0u8; 4096];
        assert_eq!(
            validate_witnesses(
                &graph,
                &[],
                &[],
                &[&backing],
                Some(&source),
                domain(),
                &[],
                ExecutionLimits { resident_bytes: 4095, ..Default::default() },
                &Interrupt::NONE
            ),
            Err(E::ResidentLimit)
        );
        assert_eq!(
            validate_witnesses(&graph, &[], &[], &[], Some(&source), domain(), &[], ExecutionLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(E::Cancelled)
        );
        assert!(validate_witnesses(&graph, &[], &[], &[], Some(&source), domain(), &[], ExecutionLimits::default(), &Interrupt::NONE).is_ok());
    }
    #[test]
    fn owned_source_clone_is_not_a_pointer_inventory_witness() {
        fn args(_: &GraphNode) -> Result<(), E> {
            Ok(())
        }
        let node = GraphNode {
            id: GraphNodeId(2),
            operation: OperationIdentity { name: "own:buffer".into(), stored_version: "1".into(), schema_epoch: "own".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        };
        let copied = node.clone();
        let graph = FilterGraph { input: None, nodes: vec![node], edges: Vec::new(), output: GraphNodeId(2), original_pipeline: None };
        let d = domain();
        let source = Surface::new(d.format);
        let sources = [OwnedSource {
            node: &copied,
            surface: &source,
            extent: Rect::new(0, 0, 1, 1),
            domain: d,
            output_port: "output",
            validate_arguments: args,
            validation_work: 1,
        }];
        let assignments = [DomainAssignment { node: GraphNodeId(2), domain: d }];
        assert_eq!(validate_witnesses(&graph, &[], &sources, &[], None, d, &assignments, ExecutionLimits::default(), &Interrupt::NONE), Err(E::Domain));
    }
}
