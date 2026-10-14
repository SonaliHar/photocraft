//! Explicit owned-buffer/configured graph orchestration, not source admission.
//! Every held source, including pruned nodes, stays charged before planning.
//! No locator, ItemRef, inferred schema, profile alias or outer policy is resolved.
use crate::gimp_configured_color_graph_candidate::{self as configured, ProfileOwnedBinding};
use photocraft_algo::gimp::{
    graph_execute::{self, DomainAssignment, ExecutionDomain, ExecutionLimits, GraphExecuteError as E, GraphOutput},
    graph_pipeline::{self, GraphPipelineError as Error},
    graph_plan::PortDirection,
    graph_roi::GraphRoiError,
    resident_source_graph,
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterArgument, FilterGraph, FilterNode, FilterValue, GraphNode},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::mem::size_of;

/// Validator is a function pointer: it cannot hide owned captured buffers. Any
/// explicit caller context backing is supplied separately and stays charged.
pub struct OwnedSource<'a> {
    pub node: &'a GraphNode,
    pub surface: &'a Surface,
    pub extent: Rect,
    pub domain: ExecutionDomain,
    pub output_port: &'a str,
    pub validate_arguments: fn(&GraphNode) -> Result<(), E>,
    /// Caller-declared bounded validator work, prepaid even for pruned sources.
    pub validation_work: u64,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::ResidentLimit)
}
fn n(v: usize) -> Result<u64, E> {
    u64::try_from(v).map_err(|_| E::ResidentLimit)
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.contains('\0')
}
pub(crate) struct Census<'a, 'b> {
    bytes: u64,
    work: u64,
    visits: u64,
    limits: ExecutionLimits,
    control: &'a Interrupt<'b>,
}
impl<'a, 'b> Census<'a, 'b> {
    pub(crate) fn new(limits: ExecutionLimits, control: &'a Interrupt<'b>) -> Self {
        Self { bytes: 0, work: 0, visits: 0, limits, control }
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn work(&self) -> u64 {
        self.work
    }
    pub(crate) fn graph_storage(&mut self, graph: &FilterGraph, binding_count: usize, source_count: usize, assignment_count: usize) -> Result<u64, E> {
        let held_index = add(
            add(
                mul(n(graph.nodes.capacity())?, n(size_of::<GraphNode>())?)?,
                mul(n(graph.edges.capacity())?, n(size_of::<photocraft_doc::xcf_filters::GraphEdge>())?)?,
            )?,
            add(
                mul(n(assignment_count)?, n(size_of::<DomainAssignment>())?)?,
                add(mul(n(binding_count)?, n(size_of::<ProfileOwnedBinding<'_>>())?)?, mul(n(source_count)?, n(size_of::<OwnedSource<'_>>())?)?)?,
            )?,
        )?;
        if held_index > self.limits.index_bytes {
            return Err(E::IndexLimit);
        }
        // Linear lookup avoids a second allocating index; bound and precharge every
        // planner/footprint/evaluator lookup against the complete declared graph.
        let lookup_work = mul(add(n(graph.nodes.len())?, mul(n(graph.edges.len())?, 4)?)?, mul(add(n(binding_count)?, n(source_count)?)?, 16)?)?;
        self.charge(0, lookup_work)?;
        self.charge(
            add(
                mul(n(graph.nodes.capacity())?, n(size_of::<GraphNode>())?)?,
                mul(n(graph.edges.capacity())?, n(size_of::<photocraft_doc::xcf_filters::GraphEdge>())?)?,
            )?,
            1,
        )?;
        self.charge(
            add(
                mul(n(assignment_count)?, n(size_of::<DomainAssignment>())?)?,
                add(mul(n(binding_count)?, n(size_of::<ProfileOwnedBinding<'_>>())?)?, mul(n(source_count)?, n(size_of::<OwnedSource<'_>>())?)?)?,
            )?,
            1,
        )?;
        if let Some(s) = &graph.original_pipeline {
            self.charge(n(s.capacity())?, 1)?;
        }
        for edge in &graph.edges {
            self.charge(add(n(edge.input_port.capacity())?, n(edge.output_port.capacity())?)?, 1)?;
        }
        Ok(held_index)
    }

    pub(crate) fn charge(&mut self, bytes: u64, work: u64) -> Result<(), E> {
        poll(self.control)?;
        self.bytes = add(self.bytes, bytes)?;
        self.work = self.work.checked_add(work).ok_or(E::WorkLimit)?;
        if self.bytes > self.limits.resident_bytes {
            return Err(E::ResidentLimit);
        }
        if self.work > self.limits.work_units {
            return Err(E::WorkLimit);
        }
        Ok(())
    }
    fn args(&mut self, args: &Vec<FilterArgument>, depth: usize) -> Result<(), E> {
        if depth > 32 || args.len() > 4096 {
            return Err(E::InvalidPlan);
        }
        self.charge(mul(n(args.capacity())?, n(size_of::<FilterArgument>())?)?, n(args.len())?)?;
        for a in args {
            self.visits = self.visits.checked_add(1).ok_or(E::WorkLimit)?;
            if self.visits > 100_000 {
                return Err(E::WorkLimit);
            }
            self.charge(n(a.name.capacity())?, n(a.name.len())?)?;
            match &a.value {
                FilterValue::String(s) => self.charge(n(s.capacity())?, n(s.len())?)?,
                FilterValue::Enum { domain, .. } => self.charge(n(domain.capacity())?, n(domain.len())?)?,
                FilterValue::Color { components, .. } => {
                    self.charge(mul(n(components.capacity())?, n(size_of::<photocraft_doc::xcf_filters::ExactNumber>())?)?, n(components.len())?)?
                }
                FilterValue::Curve { points_f64_bits, point_types, samples_f64_bits, .. } => self.charge(
                    add(add(mul(n(points_f64_bits.capacity())?, 16)?, mul(n(point_types.capacity())?, 4)?)?, mul(n(samples_f64_bits.capacity())?, 8)?)?,
                    add(n(points_f64_bits.len())?, n(samples_f64_bits.len())?)?,
                )?,
                FilterValue::Config(children) => self.args(children, depth + 1)?,
                FilterValue::Graph(_) => return Err(E::Operation("nested graph bindings must be explicit")),
                FilterValue::Number(_) | FilterValue::Bool(_) | FilterValue::Resource(_) => {}
            }
        }
        Ok(())
    }
    pub(crate) fn node(&mut self, node: &GraphNode) -> Result<(), E> {
        self.charge(
            add(add(n(node.operation.name.capacity())?, n(node.operation.stored_version.capacity())?)?, n(node.operation.schema_epoch.capacity())?)?,
            1,
        )?;
        self.args(&node.arguments, 0)
    }
    pub(crate) fn filter(&mut self, node: &FilterNode) -> Result<(), E> {
        self.charge(n(size_of::<FilterNode>())?, 1)?;
        for s in [&node.operation.name, &node.operation.stored_version, &node.operation.schema_epoch] {
            self.charge(n(s.capacity())?, 1)?;
        }
        for s in [node.source_display_name.as_ref(), node.source_icon.as_ref()].into_iter().flatten() {
            self.charge(n(s.capacity())?, 1)?;
        }
        self.charge(mul(n(node.auxiliary.capacity())?, n(size_of::<(String, photocraft_doc::item_identity::ItemRef)>())?)?, n(node.auxiliary.len())?)?;
        for (s, _) in &node.auxiliary {
            self.charge(n(s.capacity())?, 1)?;
        }
        if let Some(composition) = &node.composition {
            self.charge(n(composition.compatibility_epoch.capacity())?, n(composition.compatibility_epoch.len())?)?;
        }
        self.args(&node.arguments, 0)
    }
    pub(crate) fn surface(&mut self, s: &Surface) -> Result<(), E> {
        // Cap an O(1) minimum before traversing hostile tile maps; exact payload
        // capacities and all map nodes are then counted with per-tile polling.
        self.charge(add(256, mul(n(s.tile_count())?, 256)?)?, n(s.tile_count())?)?;
        self.charge(n(s.default_bytes().len())?, 1)?;
        for (_, tile) in s.tiles() {
            self.charge(n(tile.bytes().len())?, 1)?;
        }
        Ok(())
    }
}
fn lookup<'a, 'b>(id: GraphNodeId, bindings: &'a [&'a ProfileOwnedBinding<'b>]) -> Option<&'a ProfileOwnedBinding<'b>> {
    bindings.iter().copied().find(|b| b.execution_parameters().node == id)
}
fn source<'a, 'b>(id: GraphNodeId, sources: &'a [OwnedSource<'b>]) -> Option<&'a OwnedSource<'b>> {
    sources.iter().find(|s| s.node.id == id)
}
fn resident<'a>(s: &'a OwnedSource<'_>) -> resident_source_graph::Binding<'a> {
    resident_source_graph::Binding {
        node: s.node,
        surface: s.surface,
        extent: s.extent,
        domain: s.domain,
        output_port: s.output_port,
        validate_arguments: &s.validate_arguments,
    }
}

/// Debits caller-owned graph/bindings/context and all source buffers before any
/// scheduler/planner allocations. Shared payloads are conservatively charged per
/// owner, never refunded. Callback accounting remains cooperative, not global RSS.
#[allow(clippy::too_many_arguments)]
pub fn execute<'s>(
    graph: &FilterGraph,
    bindings: &[&ProfileOwnedBinding<'_>],
    sources: &[OwnedSource<'_>],
    backing: &[&[u8]],
    external: Option<&'s Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    max_depth: usize,
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<GraphOutput<'s>, Error> {
    let run = || -> Result<_, E> {
        poll(control)?;
        let max = ExecutionLimits::default();
        if limits.resident_bytes > max.resident_bytes
            || limits.work_units > max.work_units
            || limits.index_bytes > max.index_bytes
            || limits.cache_bytes > max.cache_bytes
        {
            return Err(E::RaisedLimit);
        }
        if graph.nodes.len() > 4096
            || graph.edges.len() > 16384
            || bindings.len() > 4096
            || sources.len() > 4096
            || backing.len() > 4096
            || assignments.len() != graph.nodes.len()
            || bindings.len().checked_add(sources.len()) != Some(graph.nodes.len())
        {
            return Err(E::InvalidPlan);
        }
        let mut c = Census::new(limits, control);
        let held_index = c.graph_storage(graph, bindings.len(), sources.len(), assignments.len())?;
        for b in backing {
            c.charge(n(b.len())?, 1)?;
        }
        if let Some(s) = external {
            c.surface(s)?;
        }
        for b in bindings {
            poll(control)?;
            let p = b.execution_parameters();
            c.filter(b.execution_original())?;
            c.charge(
                add(b.execution_retained(), add(n(p.working_default.len())?, add(n(p.schema.expected_epoch.len())?, n(p.schema.schema_sha256.len())?)?)?)?,
                b.execution_preparation_work(),
            )?;
            for port in [p.input_port, p.upstream_output_port, p.output_port] {
                c.charge(n(port.len())?, 1)?;
            }
        }
        for s in sources {
            poll(control)?;
            if s.validation_work == 0 || !text(s.output_port) {
                return Err(E::InvalidPlan);
            }
            c.charge(n(s.output_port.len())?, s.validation_work)?;
            c.surface(s.surface)?;
        }
        for (node, assignment) in graph.nodes.iter().zip(assignments) {
            c.node(node)?;
            let b = lookup(node.id, bindings);
            let s = source(node.id, sources);
            if b.is_some() == s.is_some() || assignment.node != node.id {
                return Err(E::InvalidPlan);
            }
            // Exact one-to-one inventory also rejects extra/duplicate held bindings.
            if bindings.iter().filter(|b| b.execution_parameters().node == node.id).count() + sources.iter().filter(|s| s.node.id == node.id).count() != 1 {
                return Err(E::InvalidPlan);
            }
            if let Some(b) = b {
                let p = b.execution_parameters();
                let original = b.execution_original();
                if assignment.domain != p.source_domain
                    || node.version_origin != p.version_origin
                    || node.operation != original.operation
                    || node.arguments != original.arguments
                {
                    return Err(E::Domain);
                }
            }
            if let Some(s) = s {
                if !std::ptr::eq(s.node, node) || assignment.domain != s.domain {
                    return Err(E::Domain);
                }
                (s.validate_arguments)(node)?;
                resident_source_graph::requirements(&resident(s), s.extent, control)?;
            }
        }
        poll(control)?;
        let remaining = ExecutionLimits {
            resident_bytes: limits.resident_bytes.checked_sub(c.bytes()).ok_or(E::ResidentLimit)?,
            work_units: limits.work_units.checked_sub(c.work()).ok_or(E::WorkLimit)?,
            index_bytes: limits.index_bytes.checked_sub(held_index).ok_or(E::IndexLimit)?,
            ..limits
        };
        Ok(remaining)
    };
    let remaining = run().map_err(Error::Execute)?;
    graph_pipeline::execute_pipeline_with_roi_domains_available(
        graph,
        max_depth,
        external,
        roi,
        external_domain,
        assignments,
        remaining,
        control,
        |id, direction, port| {
            if let Some(b) = lookup(id, bindings) {
                configured::accepts_profile_owned_port(b, direction, port)
            } else if let Some(s) = source(id, sources) {
                direction == PortDirection::Output && port == s.output_port
            } else {
                Some(id) == graph.input && direction == PortDirection::Output && text(port)
            }
        },
        |request| {
            if let Some(b) = lookup(request.node, bindings) {
                configured::footprint_profile_owned(b, request)
            } else {
                Err(GraphRoiError::UnsupportedFootprint)
            }
        },
        |request, budget| {
            if let Some(b) = lookup(request.node.id, bindings) {
                configured::evaluate_profile_owned_available(b, request, budget)
            } else if let Some(s) = source(request.node.id, sources) {
                budget.charge_work(s.validation_work)?;
                graph_execute::evaluate_materialized(request, budget, |request, budget| resident_source_graph::evaluate(&resident(s), request, budget))
            } else {
                Err(E::InvalidPlan)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{SampleEncoding, xcf_filters::OperationIdentity};
    fn node(id: u64) -> GraphNode {
        GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "own:resident".into(), stored_version: "1".into(), schema_epoch: "explicit-owned".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        }
    }
    fn domain() -> ExecutionDomain {
        ExecutionDomain { format: PixelFormat::RGBA8, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([7; 32]) }
    }
    fn validate(node: &GraphNode) -> Result<(), E> {
        if node.arguments.is_empty() { Ok(()) } else { Err(E::InvalidPlan) }
    }
    #[test]
    fn owned_source_output_is_private_and_pruned_storage_remains_charged() {
        let graph = FilterGraph {
            input: None,
            nodes: vec![node(1), node(2)],
            edges: Vec::new(),
            output: GraphNodeId(1),
            original_pipeline: Some("original untouched".into()),
        };
        let d = domain();
        let one = Surface::try_with_default_bytes(d.format, &[7, 11, 13, 255]).unwrap();
        let two = Surface::try_with_default_bytes(d.format, &[1, 2, 3, 4]).unwrap();
        let before = one.clone();
        let roi = Rect::new(-1, 2, 1, 3);
        let sources = [
            OwnedSource {
                node: &graph.nodes[0],
                surface: &one,
                extent: roi,
                domain: d,
                output_port: "output",
                validate_arguments: validate,
                validation_work: 16,
            },
            OwnedSource {
                node: &graph.nodes[1],
                surface: &two,
                extent: roi,
                domain: d,
                output_port: "output",
                validate_arguments: validate,
                validation_work: 16,
            },
        ];
        let domains = [DomainAssignment { node: GraphNodeId(1), domain: d }, DomainAssignment { node: GraphNodeId(2), domain: d }];
        let output = execute(&graph, &[], &sources, &[], None, roi, d, &domains, 2, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), [7, 11, 13, 255, 7, 11, 13, 255]);
        assert_eq!(one, before);
        let held = vec![0u8; 4096];
        assert!(matches!(
            execute(
                &graph,
                &[],
                &sources,
                &[&held],
                None,
                roi,
                d,
                &domains,
                2,
                ExecutionLimits { resident_bytes: 4095, ..Default::default() },
                &Interrupt::NONE
            ),
            Err(Error::Execute(E::ResidentLimit))
        ));
        assert!(matches!(
            execute(&graph, &[], &sources, &[], None, roi, d, &domains, 2, ExecutionLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(Error::Execute(E::Cancelled))
        ));
    }
    #[test]
    fn duplicate_inventory_and_low_index_fail_before_any_validator() {
        fn forbidden(_: &GraphNode) -> Result<(), E> {
            panic!("validator must not be called before aggregate admission")
        }
        let graph = FilterGraph { input: None, nodes: vec![node(1)], edges: Vec::new(), output: GraphNodeId(1), original_pipeline: None };
        let d = domain();
        let surface = Surface::new(d.format);
        let roi = Rect::new(0, 0, 1, 1);
        let sources = [OwnedSource {
            node: &graph.nodes[0],
            surface: &surface,
            extent: roi,
            domain: d,
            output_port: "output",
            validate_arguments: forbidden,
            validation_work: 1,
        }];
        let assignments = [DomainAssignment { node: GraphNodeId(1), domain: d }];
        assert!(matches!(
            execute(&graph, &[], &sources, &[], None, roi, d, &assignments, 1, ExecutionLimits { index_bytes: 0, ..Default::default() }, &Interrupt::NONE),
            Err(Error::Execute(E::IndexLimit))
        ));
        assert!(matches!(
            execute(&graph, &[], &sources, &[], None, roi, d, &[], 1, ExecutionLimits::default(), &Interrupt::NONE),
            Err(Error::Execute(E::InvalidPlan))
        ));
    }
}
