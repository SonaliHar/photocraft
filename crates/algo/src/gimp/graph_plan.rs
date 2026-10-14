//! Immutable structural plans; operation admission and execution are separate.
//!
//! Callers validate nested operation/configuration schemas and their independent
//! depth-32 budget. This iterative DAG planner only bounds dependency depth,
//! explicitly chosen by its caller in 1..=4096. Ports are never renamed or aliased.
use photocraft_doc::{item_identity::GraphNodeId, xcf_filters::FilterGraph};
use photocraft_raster::Interrupt;
use std::{cmp::Reverse, collections::{BinaryHeap, HashMap, HashSet}, ops::Range};

const MAX_NODES: usize = 4096;
const MAX_EDGES: usize = 16384;
pub(super) const MAX_WORK: usize = 100_000;
const MAX_PORT_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphPlanError {
    CountLimit, WorkLimit, DepthLimit, PortLimit, InvalidPort, ZeroIdentity,
    DuplicateIdentity, MissingEndpoint, DuplicateInputBinding, Cycle,
    Allocation, Cancelled, InvalidIndex,
}
impl std::fmt::Display for GraphPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "invalid graph plan: {self:?}") }
}
impl std::error::Error for GraphPlanError {}
type Result<T> = std::result::Result<T, GraphPlanError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortDirection { Input, Output }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    ExternalInput(GraphNodeId),
    Node { id: GraphNodeId, index: usize },
}
#[derive(Clone, Debug)]
pub struct PlannedNode {
    id: GraphNodeId,
    index: usize,
    depth: usize,
    incoming: Range<usize>,
}
impl PlannedNode {
    pub fn id(&self) -> GraphNodeId { self.id }
    /// Original declared node index, independent of topological ordering.
    pub fn index(&self) -> usize { self.index }
    /// Number of operation nodes on the longest upstream dependency path.
    pub fn depth(&self) -> usize { self.depth }
}
pub struct GraphPlan<'a> {
    graph: &'a FilterGraph,
    nodes: Vec<PlannedNode>,
    incoming_edges: Vec<usize>,
    output: Endpoint,
}
impl<'a> GraphPlan<'a> {
    pub fn graph(&self) -> &'a FilterGraph { self.graph }
    pub fn nodes(&self) -> &[PlannedNode] { &self.nodes }
    pub fn output(&self) -> Endpoint { self.output }
    pub fn external_input(&self) -> Option<GraphNodeId> { self.graph.input }
    /// Original edge indices, in declared edge order, for a topological node.
    pub fn incoming_edge_indices(&self, plan_index: usize) -> Option<&[usize]> {
        let node = self.nodes.get(plan_index)?;
        self.incoming_edges.get(node.incoming.clone())
    }
}

fn poll(control: &Interrupt<'_>) -> Result<()> { control.check().map_err(|_| GraphPlanError::Cancelled) }
fn charge(work: &mut usize) -> Result<()> {
    *work = work.checked_add(1).filter(|n| *n <= MAX_WORK).ok_or(GraphPlanError::WorkLimit)?;
    Ok(())
}
fn filled(count: usize, value: usize) -> Result<Vec<usize>> {
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| GraphPlanError::Allocation)?;
    result.resize(count, value);
    Ok(result)
}
fn offsets(counts: &[usize], work: &mut usize, control: &Interrupt<'_>) -> Result<Vec<usize>> {
    let mut result = Vec::new();
    result.try_reserve_exact(counts.len().checked_add(1).ok_or(GraphPlanError::CountLimit)?).map_err(|_| GraphPlanError::Allocation)?;
    result.push(0);
    let mut total = 0usize;
    for &count in counts {
        poll(control)?;
        charge(work)?;
        total = total.checked_add(count).filter(|n| *n <= MAX_EDGES).ok_or(GraphPlanError::CountLimit)?;
        result.push(total);
    }
    Ok(result)
}
fn interval(offsets: &[usize], index: usize) -> Result<Range<usize>> {
    let start = *offsets.get(index).ok_or(GraphPlanError::InvalidIndex)?;
    let end = *offsets.get(index.checked_add(1).ok_or(GraphPlanError::InvalidIndex)?).ok_or(GraphPlanError::InvalidIndex)?;
    Ok(start..end)
}
fn increment(values: &mut [usize], index: usize) -> Result<()> {
    let value = values.get_mut(index).ok_or(GraphPlanError::InvalidIndex)?;
    *value = value.checked_add(1).ok_or(GraphPlanError::CountLimit)?;
    Ok(())
}
fn place(flat: &mut [usize], cursor: &mut [usize], index: usize, edge: usize) -> Result<()> {
    let position = cursor.get_mut(index).ok_or(GraphPlanError::InvalidIndex)?;
    *flat.get_mut(*position).ok_or(GraphPlanError::InvalidIndex)? = edge;
    *position = position.checked_add(1).ok_or(GraphPlanError::InvalidIndex)?;
    Ok(())
}

/// Plan all declared nodes without evaluating, disabling or normalizing any operation.
/// The required predicate must accept only exact independently qualified schema ports;
/// this planner cannot infer aliases or operation-specific port names from opaque text.
/// Node IDs, ports, argument values and original pipeline bytes remain borrowed unchanged.
pub fn plan_graph<'a>(
    graph: &'a FilterGraph, max_depth: usize, control: &Interrupt<'_>,
    mut port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
) -> Result<GraphPlan<'a>> {
    poll(control)?;
    if !(1..=MAX_NODES).contains(&max_depth) { return Err(GraphPlanError::DepthLimit); }
    let n = graph.nodes.len();
    if n > MAX_NODES || graph.edges.len() > MAX_EDGES { return Err(GraphPlanError::CountLimit); }
    if graph.output.0 == 0 { return Err(GraphPlanError::ZeroIdentity); }
    let slots = n.checked_add(usize::from(graph.input.is_some())).ok_or(GraphPlanError::CountLimit)?;
    let mut identities = HashMap::new();
    identities.try_reserve(slots).map_err(|_| GraphPlanError::Allocation)?;
    let mut work = 0usize;
    for (index, node) in graph.nodes.iter().enumerate() {
        poll(control)?;
        charge(&mut work)?;
        if node.id.0 == 0 { return Err(GraphPlanError::ZeroIdentity); }
        if identities.insert(node.id, index).is_some() { return Err(GraphPlanError::DuplicateIdentity); }
    }
    if let Some(input) = graph.input {
        if input.0 == 0 { return Err(GraphPlanError::ZeroIdentity); }
        if identities.insert(input, n).is_some() { return Err(GraphPlanError::DuplicateIdentity); }
    }
    let output_index = *identities.get(&graph.output).ok_or(GraphPlanError::MissingEndpoint)?;
    let output = if Some(graph.output) == graph.input { Endpoint::ExternalInput(graph.output) } else { Endpoint::Node { id: graph.output, index: output_index } };
    let mut indegree = filled(n, 0)?;
    let mut out_count = filled(slots, 0)?;
    let mut ports = HashSet::new();
    ports.try_reserve(graph.edges.len()).map_err(|_| GraphPlanError::Allocation)?;
    let mut port_bytes = 0usize;
    for edge in &graph.edges {
        poll(control)?;
        charge(&mut work)?;
        if edge.from.0 == 0 || edge.to.0 == 0 { return Err(GraphPlanError::ZeroIdentity); }
        let from = *identities.get(&edge.from).ok_or(GraphPlanError::MissingEndpoint)?;
        let to = *identities.get(&edge.to).ok_or(GraphPlanError::MissingEndpoint)?;
        if to >= n { return Err(GraphPlanError::MissingEndpoint); }
        for (id, direction, port) in [(edge.from, PortDirection::Output, edge.output_port.as_str()), (edge.to, PortDirection::Input, edge.input_port.as_str())] {
            if port.is_empty() || port.contains('\0') { return Err(GraphPlanError::InvalidPort); }
            port_bytes = port_bytes.checked_add(port.len()).filter(|v| *v <= MAX_PORT_BYTES).ok_or(GraphPlanError::PortLimit)?;
            if !port_allowed(id, direction, port) { return Err(GraphPlanError::InvalidPort); }
            poll(control)?;
        }
        if !ports.insert((edge.to, edge.input_port.as_str())) { return Err(GraphPlanError::DuplicateInputBinding); }
        increment(&mut indegree, to)?;
        increment(&mut out_count, from)?;
    }
    let outgoing = offsets(&out_count, &mut work, control)?;
    let incoming = offsets(&indegree, &mut work, control)?;
    let mut outgoing_cursor = filled(slots, 0)?;
    outgoing_cursor.copy_from_slice(outgoing.get(..slots).ok_or(GraphPlanError::InvalidIndex)?);
    let mut incoming_cursor = filled(n, 0)?;
    incoming_cursor.copy_from_slice(incoming.get(..n).ok_or(GraphPlanError::InvalidIndex)?);
    let mut outgoing_edges = filled(graph.edges.len(), 0)?;
    let mut incoming_edges = filled(graph.edges.len(), 0)?;
    for (index, edge) in graph.edges.iter().enumerate() {
        poll(control)?;
        charge(&mut work)?;
        let from = *identities.get(&edge.from).ok_or(GraphPlanError::MissingEndpoint)?;
        let to = *identities.get(&edge.to).ok_or(GraphPlanError::MissingEndpoint)?;
        place(&mut outgoing_edges, &mut outgoing_cursor, from, index)?;
        place(&mut incoming_edges, &mut incoming_cursor, to, index)?;
    }
    let mut depth = filled(n, 1)?;
    // The caller-image endpoint is ready before operation nodes, never an edge destination.
    if graph.input.is_some() {
        for &edge_index in outgoing_edges.get(interval(&outgoing, n)?).ok_or(GraphPlanError::InvalidIndex)? {
            poll(control)?;
            charge(&mut work)?;
            let edge = graph.edges.get(edge_index).ok_or(GraphPlanError::InvalidIndex)?;
            let to = *identities.get(&edge.to).ok_or(GraphPlanError::MissingEndpoint)?;
            let degree = indegree.get_mut(to).ok_or(GraphPlanError::InvalidIndex)?;
            *degree = degree.checked_sub(1).ok_or(GraphPlanError::InvalidIndex)?;
        }
    }
    let mut ready = BinaryHeap::new();
    ready.try_reserve_exact(n).map_err(|_| GraphPlanError::Allocation)?;
    for (index, &degree) in indegree.iter().enumerate() {
        poll(control)?;
        charge(&mut work)?;
        if degree == 0 { ready.push(Reverse(index)); }
    }
    let mut planned = Vec::new();
    planned.try_reserve_exact(n).map_err(|_| GraphPlanError::Allocation)?;
    while let Some(Reverse(index)) = ready.pop() {
        poll(control)?;
        charge(&mut work)?;
        let node = graph.nodes.get(index).ok_or(GraphPlanError::InvalidIndex)?;
        let node_depth = *depth.get(index).ok_or(GraphPlanError::InvalidIndex)?;
        if node_depth > max_depth { return Err(GraphPlanError::DepthLimit); }
        planned.push(PlannedNode { id: node.id, index, depth: node_depth, incoming: interval(&incoming, index)? });
        for &edge_index in outgoing_edges.get(interval(&outgoing, index)?).ok_or(GraphPlanError::InvalidIndex)? {
            poll(control)?;
            charge(&mut work)?;
            let edge = graph.edges.get(edge_index).ok_or(GraphPlanError::InvalidIndex)?;
            let to = *identities.get(&edge.to).ok_or(GraphPlanError::MissingEndpoint)?;
            let downstream = depth.get_mut(to).ok_or(GraphPlanError::InvalidIndex)?;
            *downstream = (*downstream).max(node_depth.checked_add(1).ok_or(GraphPlanError::DepthLimit)?);
            let degree = indegree.get_mut(to).ok_or(GraphPlanError::InvalidIndex)?;
            *degree = degree.checked_sub(1).ok_or(GraphPlanError::InvalidIndex)?;
            if *degree == 0 { ready.push(Reverse(to)); }
        }
    }
    if planned.len() != n { return Err(GraphPlanError::Cycle); }
    poll(control)?;
    Ok(GraphPlan { graph, nodes: planned, incoming_edges, output })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::xcf_filters::{GraphEdge, GraphNode, GraphVersionOrigin, OperationIdentity};
    fn node(raw: u64) -> GraphNode {
        GraphNode {
            id: GraphNodeId(raw),
            operation: OperationIdentity { name: "Own structural fixture".into(), stored_version: "0:0".into(), schema_epoch: "own-topology-only".into() },
            version_origin: GraphVersionOrigin::Stored, arguments: Vec::new(),
        }
    }
    fn edge(from: u64, to: u64, port: &str) -> GraphEdge {
        GraphEdge { from: GraphNodeId(from), output_port: "output".into(), to: GraphNodeId(to), input_port: port.into() }
    }
    fn ports(_: GraphNodeId, direction: PortDirection, name: &str) -> bool {
        matches!((direction, name), (PortDirection::Input, "input" | "aux") | (PortDirection::Output, "output"))
    }
    #[test]
    fn declared_order_breaks_ready_ties_and_original_edges_remain_borrowed() {
        let graph = FilterGraph {
            input: Some(GraphNodeId(99)), nodes: vec![node(40), node(10), node(20), node(30)],
            edges: vec![edge(99, 10, "input"), edge(10, 40, "input"), edge(20, 40, "aux")],
            output: GraphNodeId(40), original_pipeline: Some("retained opaque text".into()),
        };
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, ports).unwrap();
        assert!(std::ptr::eq(plan.graph(), &graph));
        assert_eq!(plan.nodes().iter().map(PlannedNode::index).collect::<Vec<_>>(), vec![1, 2, 0, 3]);
        assert_eq!(plan.nodes().get(2).unwrap().depth(), 2);
        assert_eq!(plan.incoming_edge_indices(2), Some([1, 2].as_slice()));
        assert_eq!(plan.output(), Endpoint::Node { id: GraphNodeId(40), index: 0 });
        assert_eq!(plan.external_input(), Some(GraphNodeId(99)));
        assert!(plan.incoming_edge_indices(usize::MAX).is_none());
        assert!(matches!(plan_graph(&graph, 1, &Interrupt::NONE, ports), Err(GraphPlanError::DepthLimit)));
    }
    #[test]
    fn hostile_identity_binding_alias_cycle_and_cancel_do_not_publish_a_plan() {
        let base = FilterGraph { input: None, nodes: vec![node(1), node(2)], edges: vec![edge(1, 2, "input")], output: GraphNodeId(2), original_pipeline: None };
        let mut graph = base.clone();
        graph.nodes.get_mut(0).unwrap().id = GraphNodeId(0);
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::ZeroIdentity)));
        graph = base.clone();
        graph.nodes.get_mut(0).unwrap().id = GraphNodeId(2);
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::DuplicateIdentity)));
        graph = base.clone();
        graph.edges.push(edge(1, 2, "input"));
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::DuplicateInputBinding)));
        graph = base.clone();
        graph.edges.get_mut(0).unwrap().input_port = "INPUT".into();
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::InvalidPort)));
        graph = base.clone();
        graph.edges.push(edge(2, 1, "input"));
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::Cycle)));
        graph = base.clone();
        graph.output = GraphNodeId(3);
        assert!(matches!(plan_graph(&graph, 2, &Interrupt::NONE, ports), Err(GraphPlanError::MissingEndpoint)));
        assert!(matches!(plan_graph(&base, 2, &Interrupt::cancel_only(&|| true), ports), Err(GraphPlanError::Cancelled)));
        let passthrough = FilterGraph { input: Some(GraphNodeId(9)), nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(9), original_pipeline: None };
        assert_eq!(plan_graph(&passthrough, 1, &Interrupt::NONE, ports).unwrap().output(), Endpoint::ExternalInput(GraphNodeId(9)));
    }
}
