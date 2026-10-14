//! Generic resident ROI execution; no stock operation/profile admission is implied.
//!
//! Callers qualify operation schemas, exact ports, ICC/encoding and same-ROI
//! dependency semantics before execution. A missing profile fingerprint is opaque,
//! never an sRGB assumption. Trusted callbacks must reserve before allocations and
//! charge primitive sample visits; this cooperative ledger cannot constrain arbitrary
//! Rust callback allocations or process RSS. Enclosing transactions debit other
//! retained document/history allocations before providing these lower ceilings.
//! Callback scratch/index allocations must be dropped before returning an output;
//! persistent callback-owned buffers must instead be debited by the enclosing owner.
//! Private output tiles may cover the requested ROI boundary but may not retain
//! unrelated outside-ROI tiles. This is an initial fixed-domain ROI contract, not
//! a declaration that arbitrary neighborhood/halo/profile graphs are qualified.
use super::graph_plan::{Endpoint, GraphPlan};
use super::{graph_roi::GraphRoiPlan, roi::RoiPlan};
use photocraft_color::PixelFormat;
use photocraft_doc::{item_identity::GraphNodeId, sample_encoding::SampleEncoding, xcf_filters::GraphNode};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, RasterInputError, Surface};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionDomain {
    pub format: PixelFormat,
    pub encoding: SampleEncoding,
    pub profile_fingerprint: Option<[u8; 32]>,
}
/// Explicit opt-in output domain for one original declared graph node.
/// Slice order must match graph.nodes exactly; metadata never requests conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainAssignment {
    pub node: GraphNodeId,
    pub domain: ExecutionDomain,
}
pub(crate) fn validate_domains(
    graph: &photocraft_doc::xcf_filters::FilterGraph,
    external: ExecutionDomain,
    assignments: &[DomainAssignment],
    control: &Interrupt<'_>,
) -> Result<ExecutionDomain> {
    poll(control)?;
    if graph.nodes.len() > 4096 || assignments.len() != graph.nodes.len() {
        return Err(GraphExecuteError::InvalidPlan);
    }
    for (node, assignment) in graph.nodes.iter().zip(assignments) {
        poll(control)?;
        if assignment.node != node.id || node.id.0 == 0 {
            return Err(GraphExecuteError::InvalidPlan);
        }
    }
    if graph.input == Some(graph.output) {
        return Ok(external);
    }
    assignments.iter().find(|assignment| assignment.node == graph.output).map(|assignment| assignment.domain).ok_or(GraphExecuteError::InvalidPlan)
}
fn selected_domain(assignments: Option<&[DomainAssignment]>, index: usize, id: GraphNodeId, uniform: ExecutionDomain) -> Result<ExecutionDomain> {
    match assignments {
        None => Ok(uniform),
        Some(domains) => domains.get(index).filter(|entry| entry.node == id).map(|entry| entry.domain).ok_or(GraphExecuteError::InvalidPlan),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutionLimits {
    pub resident_bytes: u64,
    pub cache_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for ExecutionLimits {
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self { resident_bytes: 512 << 20, cache_bytes: 64 << 20, index_bytes: 16 << 20, work_units: 1 << 36 }
        } else {
            Self { resident_bytes: 2 << 30, cache_bytes: 256 << 20, index_bytes: 64 << 20, work_units: 1 << 40 }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphExecuteError {
    RaisedLimit,
    ResidentLimit,
    CacheLimit,
    IndexLimit,
    WorkLimit,
    Allocation,
    Cancelled,
    Domain,
    Roi,
    MissingInput,
    InvalidPlan,
    Reservation,
    Raster(RasterInputError),
    Operation(&'static str),
}
impl std::fmt::Display for GraphExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "graph execution refused: {self:?}")
    }
}
impl std::error::Error for GraphExecuteError {}
type Result<T> = std::result::Result<T, GraphExecuteError>;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| GraphExecuteError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn count(n: usize) -> Result<u64> {
    u64::try_from(n).map_err(|_| GraphExecuteError::IndexLimit)
}

pub struct InputBinding<'a> {
    pub input_port: &'a str,
    pub output_port: &'a str,
    pub endpoint: Endpoint,
    pub surface: &'a Surface,
    /// Actual producer domain, never inferred from this node's output domain.
    pub domain: ExecutionDomain,
    /// Exact input read region; upstream producer may cover a larger union.
    pub roi: Rect,
    /// Adaptive caller-qualified halo/abyss mapping; None in the fixed-ROI API.
    pub footprint: Option<RoiPlan>,
}
/// Pixel availability is distinct from native default/abyss pixel values.
#[derive(Clone, Copy)]
pub enum InputPixels<'a> {
    Materialized(&'a Surface),
    /// The checked adaptive footprint requires no reads from this endpoint.
    NoPixelsRequired,
}
pub struct AvailableInputBinding<'a> {
    pub input_port: &'a str,
    pub output_port: &'a str,
    pub endpoint: Endpoint,
    pub pixels: InputPixels<'a>,
    pub domain: ExecutionDomain,
    pub roi: Rect,
    pub footprint: Option<RoiPlan>,
}
pub struct AvailableNodeRequest<'a> {
    pub node: &'a GraphNode,
    pub inputs: &'a [AvailableInputBinding<'a>],
    pub roi: Rect,
    pub domain: ExecutionDomain,
}
/// Checked compatibility bridge. It never fabricates pixels for a legacy callback.
pub fn evaluate_materialized(
    request: AvailableNodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
    mut evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<NodeOutput> {
    let bytes = request.inputs.len().checked_mul(std::mem::size_of::<InputBinding<'_>>()).ok_or(GraphExecuteError::IndexLimit)?;
    budget.reserve_scratch(count(bytes)?)?;
    budget.reserve_index(count(bytes)?)?;
    budget.charge_work(count(request.inputs.len())?)?;
    let mut inputs = Vec::new();
    inputs.try_reserve_exact(request.inputs.len()).map_err(|_| GraphExecuteError::Allocation)?;
    let actual = inputs.capacity().checked_mul(std::mem::size_of::<InputBinding<'_>>()).ok_or(GraphExecuteError::IndexLimit)?;
    if actual > bytes { budget.reserve_scratch(count(actual - bytes)?)?; budget.reserve_index(count(actual - bytes)?)?; }
    for input in request.inputs {
        budget.poll()?;
        let InputPixels::Materialized(surface) = input.pixels else {
            return Err(GraphExecuteError::Operation("callback requires materialized input pixels"));
        };
        inputs.push(InputBinding { input_port: input.input_port, output_port: input.output_port, endpoint: input.endpoint,
            surface, domain: input.domain, roi: input.roi, footprint: input.footprint });
    }
    evaluate(NodeRequest { node: request.node, inputs: &inputs, roi: request.roi, domain: request.domain }, budget)
}
pub struct NodeRequest<'a> {
    pub node: &'a GraphNode,
    pub inputs: &'a [InputBinding<'a>],
    pub roi: Rect,
    pub domain: ExecutionDomain,
}
#[derive(Clone, Copy, Debug)]
pub struct RemainingBudget {
    pub resident_bytes: u64,
    pub cache_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
pub struct NodeBudget<'a, 'b> {
    control: &'a Interrupt<'b>,
    remaining: RemainingBudget,
    node: GraphNodeId,
    roi: Rect,
    domain: ExecutionDomain,
    output_reserved: Option<u64>,
    work_used: u64,
}
pub struct NodeOutput {
    surface: Surface,
    node: GraphNodeId,
    roi: Rect,
    domain: ExecutionDomain,
    resident: u64,
}
impl<'a, 'b> NodeBudget<'a, 'b> {
    /// The same cancellation/deadline witness used by scheduler and budget checks.
    pub fn interrupt(&self) -> &'a Interrupt<'b> {
        self.control
    }
    pub fn remaining(&self) -> RemainingBudget {
        self.remaining
    }
    pub fn poll(&self) -> Result<()> {
        poll(self.control)
    }
    /// Reserve callback-owned memory before allocation. Reservations remain charged
    /// through the callback; reuse already-reserved buffers instead of charging twice.
    pub fn reserve_scratch(&mut self, bytes: u64) -> Result<()> {
        self.poll()?;
        self.remaining.resident_bytes = self.remaining.resident_bytes.checked_sub(bytes).ok_or(GraphExecuteError::ResidentLimit)?;
        Ok(())
    }
    pub fn reserve_index(&mut self, bytes: u64) -> Result<()> {
        self.poll()?;
        let index = self.remaining.index_bytes.checked_sub(bytes).ok_or(GraphExecuteError::IndexLimit)?;
        let resident = self.remaining.resident_bytes.checked_sub(bytes).ok_or(GraphExecuteError::ResidentLimit)?;
        self.remaining.index_bytes = index;
        self.remaining.resident_bytes = resident;
        Ok(())
    }
    /// Exactly one output reservation, before output/COW tile allocation. Capacity
    /// for all live upstream owners is already deducted by the executor.
    pub fn reserve_output(&mut self, bytes: u64) -> Result<()> {
        self.poll()?;
        if self.output_reserved.is_some() || bytes == 0 {
            return Err(GraphExecuteError::Reservation);
        }
        let cache = self.remaining.cache_bytes.checked_sub(bytes).ok_or(GraphExecuteError::CacheLimit)?;
        let resident = self.remaining.resident_bytes.checked_sub(bytes).ok_or(GraphExecuteError::ResidentLimit)?;
        self.remaining.cache_bytes = cache;
        self.remaining.resident_bytes = resident;
        self.output_reserved = Some(bytes);
        Ok(())
    }
    /// Primitive channel-sample visits, including taps, auxiliary inputs and repeats.
    /// Performed work is never returned when an intermediate is freed.
    pub fn charge_work(&mut self, units: u64) -> Result<()> {
        self.poll()?;
        let remaining = self.remaining.work_units.checked_sub(units).ok_or(GraphExecuteError::WorkLimit)?;
        let used = self.work_used.checked_add(units).ok_or(GraphExecuteError::WorkLimit)?;
        self.remaining.work_units = remaining;
        self.work_used = used;
        Ok(())
    }
    /// Adopt a checked output only after pre-reservation. Domain metadata describes
    /// the caller-qualified context; echoing it is not independent profile evidence.
    pub fn finish_output(&mut self, surface: Surface, roi: Rect, domain: ExecutionDomain) -> Result<NodeOutput> {
        self.poll()?;
        if roi != self.roi {
            return Err(GraphExecuteError::Roi);
        }
        if domain != self.domain {
            return Err(GraphExecuteError::Domain);
        }
        let reserved = self.output_reserved.take().ok_or(GraphExecuteError::Reservation)?;
        let resident = surface_charge(&surface, domain.format, reserved, Some(roi), self.control)?;
        self.poll()?;
        Ok(NodeOutput { surface, node: self.node, roi, domain, resident })
    }
}

enum OutputStorage<'a> {
    Borrowed(&'a Surface),
    Owned(Surface),
}
pub struct GraphOutput<'a> {
    storage: OutputStorage<'a>,
    roi: Rect,
    domain: ExecutionDomain,
}
impl GraphOutput<'_> {
    pub fn surface(&self) -> &Surface {
        match &self.storage {
            OutputStorage::Borrowed(surface) => surface,
            OutputStorage::Owned(surface) => surface,
        }
    }
    pub fn roi(&self) -> Rect {
        self.roi
    }
    pub fn domain(&self) -> ExecutionDomain {
        self.domain
    }
    /// Passthrough does not silently clone/allocate the caller's external input.
    pub fn into_owned(self) -> Option<Surface> {
        match self.storage {
            OutputStorage::Borrowed(_) => None,
            OutputStorage::Owned(surface) => Some(surface),
        }
    }
}

fn surface_charge(surface: &Surface, format: PixelFormat, ceiling: u64, roi: Option<Rect>, control: &Interrupt<'_>) -> Result<u64> {
    if surface.format() != format || surface.default_bytes().len() != format.bytes_per_pixel() {
        return Err(GraphExecuteError::Domain);
    }
    let tile_bytes = (TILE_SIZE as u64)
        .checked_mul(TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64))
        .ok_or(GraphExecuteError::ResidentLimit)?;
    // Bound scans before walking any hostile sparse map. COW sharing is deliberately
    // charged in every simultaneously live surface, including borrowed source tiles.
    let tiles = count(surface.tile_count())?;
    let expected = add(add(256, surface.default_bytes().len() as u64)?, tiles.checked_mul(add(tile_bytes, 256)?).ok_or(GraphExecuteError::ResidentLimit)?)?;
    if expected > ceiling {
        return Err(GraphExecuteError::ResidentLimit);
    }
    surface.try_validate_tile_origins_with(control).map_err(GraphExecuteError::Raster)?;
    for (coordinate, tile) in surface.tiles() {
        poll(control)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(GraphExecuteError::Domain);
        }
        if let Some(bounds) = roi {
            let x = i64::from(coordinate.tx).checked_mul(i64::from(TILE_SIZE)).ok_or(GraphExecuteError::Roi)?;
            let y = i64::from(coordinate.ty).checked_mul(i64::from(TILE_SIZE)).ok_or(GraphExecuteError::Roi)?;
            let right = x.checked_add(i64::from(TILE_SIZE)).ok_or(GraphExecuteError::Roi)?;
            let bottom = y.checked_add(i64::from(TILE_SIZE)).ok_or(GraphExecuteError::Roi)?;
            if x >= i64::from(bounds.x1) || right <= i64::from(bounds.x0) || y >= i64::from(bounds.y1) || bottom <= i64::from(bounds.y0) {
                return Err(GraphExecuteError::Roi);
            }
        }
    }
    Ok(expected)
}

/// Execute the already checked structural plan. There is no operation fallback:
/// every declared node reaches the required callback in deterministic plan order.
/// Intermediate surfaces are private; errors/cancellation drop them without publication.
pub fn execute_graph<'a>(
    plan: &GraphPlan<'_>,
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    poll(control)?;
    if plan.external_input().is_some() != external.is_some() {
        return Err(GraphExecuteError::MissingInput);
    }
    let graph = plan.graph();
    let reservation = reserve_execution(external, roi, domain, limits, graph.nodes.len(), graph.edges.len(), control)?;
    execute_reserved(plan, reservation, control, evaluate)
}

/// Explicit per-node-domain execution. No format/profile conversion or domain
/// alias is introduced; every callback must handle its exact upstream metadata.
#[allow(clippy::too_many_arguments)]
pub fn execute_graph_domains<'a>(
    plan: &GraphPlan<'_>,
    external: Option<&'a Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    assignments: &[DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    if plan.external_input().is_some() != external.is_some() {
        return Err(GraphExecuteError::MissingInput);
    }
    let final_domain = validate_domains(plan.graph(), external_domain, assignments, control)?;
    let graph = plan.graph();
    let reservation = reserve_execution_domains(external, roi, external_domain, final_domain, limits, graph.nodes.len(), graph.edges.len(), control)?;
    execute_reserved_rois_domains(plan, reservation, None, assignments, control, evaluate)
}

/// Internal one-transaction witness shared by the planning facade and executor.
/// Source and joint planner/scheduler allowance are charged before plan allocation.
pub(super) struct ExecutionReservation<'a> {
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    external_domain: ExecutionDomain,
    limits: ExecutionLimits,
    nodes: usize,
    edges: usize,
    index_charge: u64,
    fixed_resident: u64,
    sample_units: u64,
}
impl ExecutionReservation<'_> {
    /// Reserve held ROI witness and propagation scratch before allocation.
    pub(super) fn reserve_roi_witness(&mut self) -> Result<(u64, u64, u64)> {
        let bytes = add(
            add(
                count(self.nodes)?.checked_mul(256).ok_or(GraphExecuteError::IndexLimit)?,
                count(self.edges)?.checked_mul(192).ok_or(GraphExecuteError::IndexLimit)?,
            )?,
            4096,
        )?;
        let index = add(self.index_charge, bytes)?;
        let resident = add(self.fixed_resident, bytes)?;
        if index > self.limits.index_bytes {
            return Err(GraphExecuteError::IndexLimit);
        }
        if resident > self.limits.resident_bytes {
            return Err(GraphExecuteError::ResidentLimit);
        }
        // Conservative structural visit allowance shared across planning/propagation.
        self.limits.work_units = self.limits.work_units.checked_sub(200_000).ok_or(GraphExecuteError::WorkLimit)?;
        self.index_charge = index;
        self.fixed_resident = resident;
        Ok((self.limits.resident_bytes.checked_sub(resident).ok_or(GraphExecuteError::ResidentLimit)?, self.limits.work_units, bytes))
    }
    pub(super) fn charge_roi_work(&mut self, work: u64) -> Result<()> {
        self.limits.work_units = self.limits.work_units.checked_sub(work).ok_or(GraphExecuteError::WorkLimit)?;
        Ok(())
    }
}
pub(super) fn reserve_execution<'a>(
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    nodes: usize,
    edges: usize,
    control: &Interrupt<'_>,
) -> Result<ExecutionReservation<'a>> {
    reserve_execution_domains(external, roi, domain, domain, limits, nodes, edges, control)
}
/// Allocation-free envelope shared by the executor and callers that need an
/// additional structural plan before preparing operation-owned data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanningRequirements {
    pub index_bytes: u64,
    pub work_units: u64,
}
pub fn planning_requirements(nodes: usize, edges: usize) -> Result<PlanningRequirements> {
    if nodes > 4096 || edges > 16384 {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let index_bytes = add(
        add(count(nodes)?.checked_mul(512).ok_or(GraphExecuteError::IndexLimit)?, count(edges)?.checked_mul(256).ok_or(GraphExecuteError::IndexLimit)?)?,
        4096,
    )?;
    Ok(PlanningRequirements {
        index_bytes,
        work_units: u64::try_from(super::graph_plan::MAX_WORK).map_err(|_| GraphExecuteError::WorkLimit)?,
    })
}
#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_execution_domains<'a>(
    external: Option<&'a Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    nodes: usize,
    edges: usize,
    control: &Interrupt<'_>,
) -> Result<ExecutionReservation<'a>> {
    poll(control)?;
    if nodes > 4096 || edges > 16384 {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let standard = ExecutionLimits::default();
    if limits.resident_bytes > standard.resident_bytes
        || limits.cache_bytes > standard.cache_bytes
        || limits.index_bytes > standard.index_bytes
        || limits.work_units > standard.work_units
    {
        return Err(GraphExecuteError::RaisedLimit);
    }
    if roi.is_empty() {
        return Err(GraphExecuteError::Roi);
    }
    let roi_bytes = photocraft_raster::checked_region_byte_len(roi, domain.format).map_err(GraphExecuteError::Raster)?;
    let sample_units = count(roi_bytes)?.checked_div(domain.format.sample.bytes() as u64).ok_or(GraphExecuteError::WorkLimit)?;
    // One conservative allowance covers retained plan indices plus peak planner
    // scratch and executor metadata/borrow bindings. Planner scratch is freed
    // before execution; a second plan is never constructed or retained.
    let index_charge = planning_requirements(nodes, edges)?.index_bytes;
    if index_charge > limits.index_bytes || index_charge > limits.resident_bytes {
        return Err(GraphExecuteError::IndexLimit);
    }
    let source_charge = match external {
        Some(surface) => surface_charge(
            surface,
            external_domain.format,
            limits.resident_bytes.checked_sub(index_charge).ok_or(GraphExecuteError::ResidentLimit)?,
            None,
            control,
        )?,
        None => 0,
    };
    let fixed_resident = add(index_charge, source_charge)?;
    poll(control)?;
    Ok(ExecutionReservation { external, roi, domain, external_domain, limits, nodes, edges, index_charge, fixed_resident, sample_units })
}
pub(super) fn execute_reserved<'a>(
    plan: &GraphPlan<'_>,
    reservation: ExecutionReservation<'a>,
    control: &Interrupt<'_>,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    execute_reserved_rois(plan, reservation, None, control, evaluate)
}
pub(super) fn execute_reserved_rois<'a>(
    plan: &GraphPlan<'_>,
    reservation: ExecutionReservation<'a>,
    rois: Option<&GraphRoiPlan>,
    control: &Interrupt<'_>,
    mut evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    execute_reserved_rois_impl(plan, reservation, rois, None, control, |request, budget| evaluate_materialized(request, budget, &mut evaluate))
}
pub(super) fn execute_reserved_rois_domains<'a>(
    plan: &GraphPlan<'_>,
    reservation: ExecutionReservation<'a>,
    rois: Option<&GraphRoiPlan>,
    assignments: &[DomainAssignment],
    control: &Interrupt<'_>,
    mut evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    let final_domain = validate_domains(plan.graph(), reservation.external_domain, assignments, control)?;
    if final_domain != reservation.domain {
        return Err(GraphExecuteError::Domain);
    }
    execute_reserved_rois_impl(plan, reservation, rois, Some(assignments), control, |request, budget| evaluate_materialized(request, budget, &mut evaluate))
}
pub(super) fn execute_reserved_rois_domains_available<'a>(
    plan: &GraphPlan<'_>, reservation: ExecutionReservation<'a>, rois: Option<&GraphRoiPlan>,
    assignments: &[DomainAssignment], control: &Interrupt<'_>,
    evaluate: impl FnMut(AvailableNodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    let final_domain = validate_domains(plan.graph(), reservation.external_domain, assignments, control)?;
    if final_domain != reservation.domain { return Err(GraphExecuteError::Domain); }
    execute_reserved_rois_impl(plan, reservation, rois, Some(assignments), control, evaluate)
}
fn execute_reserved_rois_impl<'a>(
    plan: &GraphPlan<'_>,
    reservation: ExecutionReservation<'a>,
    rois: Option<&GraphRoiPlan>,
    assignments: Option<&[DomainAssignment]>,
    control: &Interrupt<'_>,
    mut evaluate: impl FnMut(AvailableNodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput>,
) -> Result<GraphOutput<'a>> {
    poll(control)?;
    let ExecutionReservation { external, roi, domain, external_domain, limits, nodes, edges, index_charge, fixed_resident, sample_units } = reservation;
    let graph = plan.graph();
    if graph.nodes.len() != nodes || graph.edges.len() != edges || plan.external_input().is_some() != external.is_some() {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let n = nodes;
    if rois.is_some_and(|regions| regions.node_outputs().len() != n || regions.edge_inputs().len() != edges) {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let mut identities = HashMap::new();
    identities.try_reserve(n).map_err(|_| GraphExecuteError::Allocation)?;
    for (index, node) in graph.nodes.iter().enumerate() {
        identities.insert(node.id, index);
    }
    let mut uses = Vec::new();
    uses.try_reserve_exact(n).map_err(|_| GraphExecuteError::Allocation)?;
    uses.resize(n, 0usize);
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        poll(control)?;
        if rois.is_some_and(|regions| regions.edge_inputs().get(edge_index).is_none_or(Option::is_none)) {
            continue;
        }
        if Some(edge.from) != plan.external_input() {
            let index = *identities.get(&edge.from).ok_or(GraphExecuteError::InvalidPlan)?;
            let remaining = uses.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
            *remaining = remaining.checked_add(1).ok_or(GraphExecuteError::InvalidPlan)?;
        }
    }
    if let Endpoint::Node { index, .. } = plan.output() {
        let remaining = uses.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
        *remaining = remaining.checked_add(1).ok_or(GraphExecuteError::InvalidPlan)?;
    }
    let mut outputs = Vec::<Option<NodeOutput>>::new();
    outputs.try_reserve_exact(n).map_err(|_| GraphExecuteError::Allocation)?;
    outputs.resize_with(n, || None);
    let mut live = 0u64;
    let mut work_remaining = limits.work_units;
    for (plan_index, planned) in plan.nodes().iter().enumerate() {
        poll(control)?;
        let node_roi = match rois {
            None => roi,
            Some(regions) => match regions.node_outputs().get(planned.index()).ok_or(GraphExecuteError::InvalidPlan)? {
                Some(region) => *region,
                None => continue,
            },
        };
        let node = graph.nodes.get(planned.index()).ok_or(GraphExecuteError::InvalidPlan)?;
        let node_domain = selected_domain(assignments, planned.index(), node.id, domain)?;
        let node_samples = if rois.is_some() || assignments.is_some() {
            count(photocraft_raster::checked_region_byte_len(node_roi, node_domain.format).map_err(GraphExecuteError::Raster)?)?
                .checked_div(node_domain.format.sample.bytes() as u64)
                .ok_or(GraphExecuteError::WorkLimit)?
        } else {
            sample_units
        };
        work_remaining = work_remaining.checked_sub(node_samples).ok_or(GraphExecuteError::WorkLimit)?;
        let edge_indices = plan.incoming_edge_indices(plan_index).ok_or(GraphExecuteError::InvalidPlan)?;
        let mut budget = NodeBudget {
            control,
            remaining: RemainingBudget {
                resident_bytes: limits.resident_bytes.checked_sub(add(fixed_resident, live)?).ok_or(GraphExecuteError::ResidentLimit)?,
                cache_bytes: limits.cache_bytes.checked_sub(live).ok_or(GraphExecuteError::CacheLimit)?,
                index_bytes: limits.index_bytes.checked_sub(index_charge).ok_or(GraphExecuteError::IndexLimit)?,
                work_units: work_remaining,
            },
            node: node.id,
            roi: node_roi,
            domain: node_domain,
            output_reserved: None,
            work_used: 0,
        };
        let output = {
            let mut inputs = Vec::new();
            inputs.try_reserve_exact(edge_indices.len()).map_err(|_| GraphExecuteError::Allocation)?;
            let input_capacity_bytes = inputs.capacity().checked_mul(std::mem::size_of::<AvailableInputBinding<'_>>()).ok_or(GraphExecuteError::IndexLimit)?;
            let input_envelope = edge_indices.len().checked_mul(256).ok_or(GraphExecuteError::IndexLimit)?;
            if input_capacity_bytes > input_envelope { return Err(GraphExecuteError::IndexLimit); }
            for &edge_index in edge_indices {
                poll(control)?;
                let edge = graph.edges.get(edge_index).ok_or(GraphExecuteError::InvalidPlan)?;
                let footprint = match rois {
                    None => None,
                    Some(regions) => Some(regions.edge_inputs().get(edge_index).and_then(|input| *input).ok_or(GraphExecuteError::InvalidPlan)?),
                };
                let input_roi = footprint.map_or(roi, RoiPlan::source_read);
                let (endpoint, pixels, input_domain) = if Some(edge.from) == plan.external_input() {
                    (Endpoint::ExternalInput(edge.from), InputPixels::Materialized(external.ok_or(GraphExecuteError::MissingInput)?), external_domain)
                } else {
                    let index = *identities.get(&edge.from).ok_or(GraphExecuteError::InvalidPlan)?;
                    let upstream = outputs.get(index).and_then(Option::as_ref);
                    if upstream.is_none() {
                        let skipped = rois.is_some_and(|regions| regions.node_outputs().get(index).is_some_and(Option::is_none));
                        if !skipped || footprint.is_none() || !input_roi.is_empty() { return Err(GraphExecuteError::InvalidPlan); }
                        let producer = graph.nodes.get(index).ok_or(GraphExecuteError::InvalidPlan)?;
                        let producer_domain = selected_domain(assignments, index, producer.id, domain)?;
                        inputs.push(AvailableInputBinding { input_port: &edge.input_port, output_port: &edge.output_port,
                            endpoint: Endpoint::Node { id: edge.from, index }, pixels: InputPixels::NoPixelsRequired,
                            domain: producer_domain, roi: input_roi, footprint });
                        continue;
                    }
                    let upstream = upstream.ok_or(GraphExecuteError::InvalidPlan)?;
                    if !input_roi.is_empty()
                        && (input_roi.x0 < upstream.roi.x0
                            || input_roi.y0 < upstream.roi.y0
                            || input_roi.x1 > upstream.roi.x1
                            || input_roi.y1 > upstream.roi.y1)
                    {
                        return Err(GraphExecuteError::Roi);
                    }
                    (Endpoint::Node { id: edge.from, index }, InputPixels::Materialized(&upstream.surface), upstream.domain)
                };
                inputs.push(AvailableInputBinding {
                    input_port: &edge.input_port,
                    output_port: &edge.output_port,
                    endpoint,
                    pixels,
                    domain: input_domain,
                    roi: input_roi,
                    footprint,
                });
            }
            evaluate(AvailableNodeRequest { node, inputs: &inputs, roi: node_roi, domain: node_domain }, &mut budget)?
        };
        poll(control)?;
        if output.node != node.id || output.roi != node_roi || output.domain != node_domain || output.surface.format() != node_domain.format {
            return Err(GraphExecuteError::Domain);
        }
        work_remaining = work_remaining.checked_sub(budget.work_used).ok_or(GraphExecuteError::WorkLimit)?;
        // Charge staged output while upstream owners are still live: this is the COW peak.
        let staged = add(live, output.resident)?;
        if staged > limits.cache_bytes {
            return Err(GraphExecuteError::CacheLimit);
        }
        if add(fixed_resident, staged)? > limits.resident_bytes {
            return Err(GraphExecuteError::ResidentLimit);
        }
        live = staged;
        *outputs.get_mut(planned.index()).ok_or(GraphExecuteError::InvalidPlan)? = Some(output);
        // All input borrows are dropped. Return private cache capacity at last use;
        // performed work stays charged. External input remains borrowed unchanged.
        for &edge_index in edge_indices {
            let edge = graph.edges.get(edge_index).ok_or(GraphExecuteError::InvalidPlan)?;
            if Some(edge.from) != plan.external_input() {
                let index = *identities.get(&edge.from).ok_or(GraphExecuteError::InvalidPlan)?;
                let remaining = uses.get_mut(index).ok_or(GraphExecuteError::InvalidPlan)?;
                *remaining = remaining.checked_sub(1).ok_or(GraphExecuteError::InvalidPlan)?;
                if *remaining == 0 {
                    if let Some(released) = outputs.get_mut(index).and_then(Option::take) {
                        live = live.checked_sub(released.resident).ok_or(GraphExecuteError::InvalidPlan)?;
                    } else if !rois.is_some_and(|regions| regions.node_outputs().get(index).is_some_and(Option::is_none)) {
                        return Err(GraphExecuteError::InvalidPlan);
                    }
                }
            }
        }
        if uses.get(planned.index()).copied() == Some(0) {
            let released = outputs.get_mut(planned.index()).and_then(Option::take).ok_or(GraphExecuteError::InvalidPlan)?;
            live = live.checked_sub(released.resident).ok_or(GraphExecuteError::InvalidPlan)?;
        }
    }
    poll(control)?;
    let storage = match plan.output() {
        Endpoint::ExternalInput(_) => OutputStorage::Borrowed(external.ok_or(GraphExecuteError::MissingInput)?),
        Endpoint::Node { index, .. } => OutputStorage::Owned(outputs.get_mut(index).and_then(Option::take).ok_or(GraphExecuteError::InvalidPlan)?.surface),
    };
    poll(control)?;
    Ok(GraphOutput { storage, roi, domain })
}

#[cfg(test)]
mod tests {
    use super::super::graph_plan::{PortDirection, plan_graph};
    use super::*;
    use photocraft_doc::xcf_filters::{FilterGraph, GraphEdge, GraphNode, GraphVersionOrigin, OperationIdentity};
    fn graph() -> FilterGraph {
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "own:raw-default-copy".into(), stored_version: "1".into(), schema_epoch: "own-test-only".into() },
            version_origin: GraphVersionOrigin::Stored,
            arguments: Vec::new(),
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), output_port: "output".into(), input_port: "input".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2), node(3)],
            edges: vec![edge(9, 1), edge(1, 2), edge(2, 3)],
            output: GraphNodeId(3),
            original_pipeline: None,
        }
    }
    fn ports(_: GraphNodeId, direction: PortDirection, port: &str) -> bool {
        matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output"))
    }
    #[test]
    fn exact_raw_defaults_and_last_use_cache_capacity_are_retained() {
        let graph = graph();
        let plan = plan_graph(&graph, 3, &Interrupt::NONE, ports).unwrap();
        let encoded = [(-0.0_f64).to_bits(), 1_u64, 0x7ff8_1234_5678_9abc, 1.0_f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &encoded).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let charge = 256 + encoded.len() as u64;
        let limits = ExecutionLimits { cache_bytes: 2 * charge, work_units: 12, ..Default::default() };
        let mut calls = 0usize;
        let output = execute_graph(&plan, Some(&source), Rect::new(0, 0, 1, 1), domain, limits, &Interrupt::NONE, |request, budget| {
            calls += 1;
            assert_eq!(request.inputs.len(), 1);
            let input = request.inputs.first().unwrap();
            assert_eq!(input.input_port, "input");
            assert_eq!(input.output_port, "output");
            if calls > 1 {
                assert_eq!(budget.remaining().cache_bytes, charge);
            }
            budget.reserve_output(charge)?;
            let copied = Surface::try_with_default_bytes(request.domain.format, input.surface.default_bytes()).map_err(GraphExecuteError::Raster)?;
            budget.finish_output(copied, request.roi, request.domain)
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(output.surface().default_bytes(), encoded);
        assert_eq!(source, before);
        assert_eq!(output.domain(), domain);
    }
    #[test]
    fn budget_and_late_cancel_drop_private_stages_without_changing_input() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let graph = graph();
        let plan = plan_graph(&graph, 3, &Interrupt::NONE, ports).unwrap();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[7, 8, 9, 100]).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let charge = 260;
        let limits = ExecutionLimits { cache_bytes: 2 * charge - 1, ..Default::default() };
        let copy = |request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>| {
            budget.reserve_output(charge)?;
            let surface =
                Surface::try_with_default_bytes(request.domain.format, request.inputs.first().ok_or(GraphExecuteError::MissingInput)?.surface.default_bytes())
                    .map_err(GraphExecuteError::Raster)?;
            budget.finish_output(surface, request.roi, request.domain)
        };
        assert!(execute_graph(&plan, Some(&source), Rect::new(0, 0, 1, 1), domain, limits, &Interrupt::NONE, copy).is_err());
        assert_eq!(source, before);
        let cancelled = AtomicBool::new(false);
        let cancel = || cancelled.load(Ordering::Relaxed);
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(
            execute_graph(&plan, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &control, |request, budget| {
                budget.reserve_output(charge)?;
                let surface = Surface::try_with_default_bytes(request.domain.format, &[7, 8, 9, 100]).map_err(GraphExecuteError::Raster)?;
                let output = budget.finish_output(surface, request.roi, request.domain)?;
                cancelled.store(true, Ordering::Relaxed);
                Ok(output)
            }),
            Err(GraphExecuteError::Cancelled)
        ));
        assert_eq!(source, before);
    }
    #[test]
    fn explicit_external_passthrough_borrows_without_evaluating_an_operation() {
        let graph = FilterGraph { input: Some(GraphNodeId(9)), nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(9), original_pipeline: None };
        let plan = plan_graph(&graph, 1, &Interrupt::NONE, ports).unwrap();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 4]).unwrap();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let output = execute_graph(&plan, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _| {
            Err(GraphExecuteError::Operation("no operation in passthrough"))
        })
        .unwrap();
        assert!(std::ptr::eq(output.surface(), &source));
        assert!(output.into_owned().is_none());
    }
}
