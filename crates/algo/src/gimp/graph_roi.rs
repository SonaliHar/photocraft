//! Backward rectangular requirements over a validated DAG, not an evaluator.
//! Caller footprints qualify each exact input port independently. Bounding-box
//! unions may over-read; no port aliases, operation defaults or profile guesses.
use super::{
    graph_execute::{DomainAssignment, ExecutionDomain, validate_domains},
    graph_plan::{Endpoint, GraphPlan},
    roi::{Abyss, Halo, RoiBudget, RoiLimits, RoiPlan, RoiUsage},
};
use photocraft_color::PixelFormat;
use photocraft_doc::{item_identity::GraphNodeId, xcf_filters::GraphEdge};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, checked_region_byte_len};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphRoiError {
    InvalidPlan,
    UnsupportedFootprint,
    Allocation,
    WorkLimit,
    Cancelled,
    Bounds,
}
impl std::fmt::Display for GraphRoiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "graph ROI: {self:?}")
    }
}
impl std::error::Error for GraphRoiError {}
impl From<RasterInputError> for GraphRoiError {
    fn from(error: RasterInputError) -> Self {
        match error {
            RasterInputError::Cancelled => Self::Cancelled,
            _ => Self::Bounds,
        }
    }
}
type Result<T> = std::result::Result<T, GraphRoiError>;

#[derive(Clone, Copy, Debug)]
pub struct InputFootprint {
    /// Exact caller-supplied output-to-input coordinate mapping, before halo.
    pub mapped_output: Rect,
    pub source_extent: Rect,
    pub halo: Halo,
    pub abyss: Abyss,
    /// Conservative arithmetic/read work per sample; zero is refused.
    pub work_per_sample: u64,
}
pub struct FootprintRequest<'a> {
    pub node: GraphNodeId,
    pub declared_index: usize,
    pub edge_index: usize,
    pub edge: &'a GraphEdge,
    pub output: Rect,
}
pub struct GraphRoiPlan {
    node_outputs: Vec<Option<Rect>>,
    edge_inputs: Vec<Option<RoiPlan>>,
    external_input: Option<Rect>,
    usage: RoiUsage,
    index_bytes: u64,
}
impl GraphRoiPlan {
    /// Original declared node/edge indices. None means no output dependency.
    pub fn node_outputs(&self) -> &[Option<Rect>] {
        &self.node_outputs
    }
    pub fn edge_inputs(&self) -> &[Option<RoiPlan>] {
        &self.edge_inputs
    }
    pub fn external_input(&self) -> Option<Rect> {
        self.external_input
    }
    pub fn usage(&self) -> RoiUsage {
        self.usage
    }
    /// Separate conservative index reservation, already deducted from limits.
    pub fn index_bytes(&self) -> u64 {
        self.index_bytes
    }
}
fn poll(control: &Interrupt<'_>, work: &mut usize) -> Result<()> {
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    *work = work.checked_add(1).filter(|value| *value <= 100_000).ok_or(GraphRoiError::WorkLimit)?;
    Ok(())
}
fn slots<T: Clone>(count: usize, value: T) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| GraphRoiError::Allocation)?;
    values.resize(count, value);
    Ok(values)
}
fn union(target: &mut Option<Rect>, incoming: Rect, format: PixelFormat) -> Result<()> {
    checked_region_byte_len(incoming, format)?;
    let combined = match *target {
        None => incoming,
        Some(old) if old.is_empty() => incoming,
        Some(old) if incoming.is_empty() => old,
        Some(old) => Rect::new(old.x0.min(incoming.x0), old.y0.min(incoming.y0), old.x1.max(incoming.x1), old.y1.max(incoming.y1)),
    };
    checked_region_byte_len(combined, format)?;
    *target = Some(combined);
    Ok(())
}

/// Plan first, allocate pixels later. Limits cover conservative output/support
/// declarations plus index reservation, not actual execution/RSS. All buffers are
/// private metadata; cancellation/errors publish no partial plan. The supplied
/// GraphPlan already proves cycles/ports; upstream order is defensively checked.
/// This first lane uses one exact format throughout, with no sample conversion.
pub fn propagate_roi(
    plan: &GraphPlan<'_>,
    requested: Rect,
    format: PixelFormat,
    limits: RoiLimits,
    control: &Interrupt<'_>,
    footprint: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint>,
) -> Result<GraphRoiPlan> {
    propagate_roi_impl(plan, requested, format, format, None, limits, control, footprint)
}
/// Opt-in native domains: every output union is charged in its producer format,
/// every input footprint in its actual upstream format. No conversion occurs.
#[allow(clippy::too_many_arguments)]
pub fn propagate_roi_domains(
    plan: &GraphPlan<'_>,
    requested: Rect,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    limits: RoiLimits,
    control: &Interrupt<'_>,
    footprint: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint>,
) -> Result<GraphRoiPlan> {
    let output = validate_domains(plan.graph(), external_domain, assignments, control).map_err(|error| match error {
        super::graph_execute::GraphExecuteError::Cancelled => GraphRoiError::Cancelled,
        _ => GraphRoiError::InvalidPlan,
    })?;
    propagate_roi_impl(plan, requested, output.format, external_domain.format, Some(assignments), limits, control, footprint)
}
#[allow(clippy::too_many_arguments)]
fn propagate_roi_impl(
    plan: &GraphPlan<'_>,
    requested: Rect,
    format: PixelFormat,
    external_format: PixelFormat,
    assignments: Option<&[DomainAssignment]>,
    mut limits: RoiLimits,
    control: &Interrupt<'_>,
    mut footprint: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint>,
) -> Result<GraphRoiPlan> {
    let mut work = 0;
    poll(control, &mut work)?;
    checked_region_byte_len(requested, format)?;
    let graph = plan.graph();
    let count = graph.nodes.len();
    if count > 4096 || graph.edges.len() > 16384 || plan.nodes().len() != count {
        return Err(GraphRoiError::InvalidPlan);
    }
    // Includes hash entries, both vectors and temporary mapping overhead.
    let index_bytes = (count as u64)
        .checked_mul(256)
        .and_then(|bytes| (graph.edges.len() as u64).checked_mul(192).and_then(|edges| bytes.checked_add(edges)))
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or(GraphRoiError::Bounds)?;
    let index_cap = if cfg!(target_arch = "wasm32") { 16 << 20 } else { 64 << 20 };
    if index_bytes > index_cap || limits.retained_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
        return Err(GraphRoiError::Bounds);
    }
    limits.retained_bytes = limits.retained_bytes.checked_sub(index_bytes).filter(|bytes| *bytes > 0).ok_or(GraphRoiError::Bounds)?;
    let mut budget = RoiBudget::new(limits)?;
    let mut identities = HashMap::new();
    identities.try_reserve(count).map_err(|_| GraphRoiError::Allocation)?;
    for (order, node) in plan.nodes().iter().enumerate() {
        poll(control, &mut work)?;
        if node.index() >= count
            || graph.nodes.get(node.index()).is_none_or(|source| source.id != node.id())
            || identities.insert(node.id(), (node.index(), order)).is_some()
        {
            return Err(GraphRoiError::InvalidPlan);
        }
    }
    let node_format = |index: usize, id: GraphNodeId| -> Result<PixelFormat> {
        match assignments {
            None => Ok(format),
            Some(domains) => domains.get(index).filter(|entry| entry.node == id).map(|entry| entry.domain.format).ok_or(GraphRoiError::InvalidPlan),
        }
    };
    let mut node_outputs = slots(count, None)?;
    let mut edge_inputs = slots(graph.edges.len(), None)?;
    let mut external_input = None;
    match plan.output() {
        Endpoint::ExternalInput(id) if Some(id) == plan.external_input() => union(&mut external_input, requested, external_format)?,
        Endpoint::Node { id, index } if identities.get(&id).is_some_and(|(actual, _)| *actual == index) => {
            union(node_outputs.get_mut(index).ok_or(GraphRoiError::InvalidPlan)?, requested, node_format(index, id)?)?
        }
        _ => return Err(GraphRoiError::InvalidPlan),
    }
    // Charge final output also for a pure external-input passthrough.
    let final_output = RoiPlan::new(requested, requested, Halo::default(), Abyss::None, format, control)?;
    budget.charge(&final_output, 1, 0, control)?;
    for (order, node) in plan.nodes().iter().enumerate().rev() {
        poll(control, &mut work)?;
        let Some(output) = *node_outputs.get(node.index()).ok_or(GraphRoiError::InvalidPlan)? else { continue };
        // A fan-out union can contain gaps absent from its individual consumers.
        // Charge the complete bounding box before any future output allocation.
        let merged = RoiPlan::new(output, output, Halo::default(), Abyss::None, node_format(node.index(), node.id())?, control)?;
        budget.charge(&merged, 1, 0, control)?;
        for &edge_index in plan.incoming_edge_indices(order).ok_or(GraphRoiError::InvalidPlan)? {
            poll(control, &mut work)?;
            let edge = graph.edges.get(edge_index).filter(|edge| edge.to == node.id()).ok_or(GraphRoiError::InvalidPlan)?;
            let supplied = footprint(FootprintRequest { node: node.id(), declared_index: node.index(), edge_index, edge, output })?;
            poll(control, &mut work)?;
            let upstream_format = if Some(edge.from) == plan.external_input() {
                external_format
            } else {
                let &(index, _) = identities.get(&edge.from).ok_or(GraphRoiError::InvalidPlan)?;
                node_format(index, edge.from)?
            };
            let input = RoiPlan::new(supplied.mapped_output, supplied.source_extent, supplied.halo, supplied.abyss, upstream_format, control)?;
            budget.charge(&input, supplied.work_per_sample, 0, control)?;
            if Some(edge.from) == plan.external_input() {
                if !input.source_read().is_empty() {
                    union(&mut external_input, input.source_read(), external_format)?;
                }
            } else {
                let &(index, upstream_order) = identities.get(&edge.from).ok_or(GraphRoiError::InvalidPlan)?;
                if upstream_order >= order {
                    return Err(GraphRoiError::InvalidPlan);
                }
                if !input.source_read().is_empty() {
                    union(node_outputs.get_mut(index).ok_or(GraphRoiError::InvalidPlan)?, input.source_read(), upstream_format)?;
                }
            }
            let slot = edge_inputs.get_mut(edge_index).ok_or(GraphRoiError::InvalidPlan)?;
            if slot.is_some() {
                return Err(GraphRoiError::InvalidPlan);
            }
            *slot = Some(input);
        }
    }
    if let Some(external) = external_input {
        let merged = RoiPlan::new(external, external, Halo::default(), Abyss::None, external_format, control)?;
        budget.charge(&merged, 1, 0, control)?;
    }
    poll(control, &mut work)?;
    Ok(GraphRoiPlan { node_outputs, edge_inputs, external_input, usage: budget.used(), index_bytes })
}

#[cfg(test)]
mod tests {
    use super::super::graph_plan::plan_graph;
    use super::*;
    use photocraft_doc::xcf_filters::{FilterGraph, GraphNode, OperationIdentity};
    fn limits() -> RoiLimits {
        RoiLimits { area: 1_000_000, samples: 4_000_000, work: 4_000_000, retained_bytes: 16 << 20 }
    }
    fn node(id: u64) -> GraphNode {
        GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "Own ROI fixture".into(), stored_version: "0:0".into(), schema_epoch: "own-explicit-footprints".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        }
    }
    fn edge(from: u64, to: u64, port: &str) -> GraphEdge {
        GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: port.into(), output_port: "output".into() }
    }
    #[test]
    fn exact_input_ports_have_distinct_halos_and_union_before_upstream_callback() {
        let graph = FilterGraph {
            input: Some(GraphNodeId(99)),
            nodes: vec![node(1), node(2)],
            edges: vec![edge(99, 1, "input"), edge(1, 2, "input"), edge(1, 2, "aux")],
            output: GraphNodeId(2),
            original_pipeline: None,
        };
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, _, _| true).unwrap();
        let requested = Rect::new(4, 4, 6, 6);
        let mut visits = Vec::new();
        let result = propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::NONE, |request| {
            visits.push((request.node, request.edge.input_port.clone(), request.output));
            let halo = if request.edge.input_port == "aux" { Halo { left: 3, ..Default::default() } } else { Halo { right: 2, ..Default::default() } };
            Ok(InputFootprint { mapped_output: request.output, source_extent: Rect::new(0, 0, 20, 20), halo, abyss: Abyss::None, work_per_sample: 1 })
        })
        .unwrap();
        assert_eq!(result.node_outputs()[0], Some(Rect::new(1, 4, 8, 6)));
        assert_eq!(result.edge_inputs()[1].unwrap().source_read(), Rect::new(4, 4, 8, 6));
        assert_eq!(result.edge_inputs()[2].unwrap().source_read(), Rect::new(1, 4, 6, 6));
        assert_eq!(visits.last().unwrap().2, Rect::new(1, 4, 8, 6));
        assert_eq!(result.external_input(), Some(Rect::new(1, 4, 10, 6)));
    }
    #[test]
    fn clamp_passthrough_unsupported_overflow_budget_and_cancel_are_explicit() {
        let graph = FilterGraph {
            input: Some(GraphNodeId(99)),
            nodes: vec![node(1)],
            edges: vec![edge(99, 1, "input")],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, _, _| true).unwrap();
        let requested = Rect::new(-8, -8, -6, -6);
        let make = |halo| InputFootprint { mapped_output: requested, source_extent: Rect::new(1, 2, 4, 5), halo, abyss: Abyss::Clamp, work_per_sample: 1 };
        let result = propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::NONE, |_| Ok(make(Halo::default()))).unwrap();
        assert_eq!(result.external_input(), Some(Rect::new(1, 2, 2, 3)));
        assert!(propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::NONE, |_| Err(GraphRoiError::UnsupportedFootprint)).is_err());
        assert!(
            propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::NONE, |_| Ok(make(Halo { left: u32::MAX, ..Default::default() })))
                .is_err()
        );
        assert!(
            propagate_roi(&plan, requested, PixelFormat::RGBA16, RoiLimits { work: 0, ..limits() }, &Interrupt::NONE, |_| Ok(make(Halo::default()))).is_err()
        );
        assert!(propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::cancel_only(&|| true), |_| Ok(make(Halo::default()))).is_err());
        let graph = FilterGraph { input: Some(GraphNodeId(99)), nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(99), original_pipeline: None };
        let plan = plan_graph(&graph, 1, &Interrupt::NONE, |_, _, _| true).unwrap();
        let result = propagate_roi(&plan, requested, PixelFormat::RGBA16, limits(), &Interrupt::NONE, |_| Err(GraphRoiError::UnsupportedFootprint)).unwrap();
        assert_eq!(result.external_input(), Some(requested));
    }
}
