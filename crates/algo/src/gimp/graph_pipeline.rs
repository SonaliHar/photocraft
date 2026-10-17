//! One bounded planning/execution transaction over an immutable borrowed graph.
//! This facade does not select evaluators or qualify operation/profile semantics.
use super::{
    graph_execute::{
        ExecutionDomain, ExecutionLimits, GraphExecuteError, GraphOutput, NodeBudget, NodeOutput, NodeRequest, execute_reserved, execute_reserved_rois,
        reserve_execution,
    },
    graph_plan::{GraphPlanError, PortDirection, plan_graph},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint, propagate_roi},
    roi::RoiLimits,
};
use photocraft_doc::{item_identity::GraphNodeId, xcf_filters::FilterGraph};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphPipelineError {
    Plan(GraphPlanError),
    Execute(GraphExecuteError),
    Roi(GraphRoiError),
}
impl std::fmt::Display for GraphPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => write!(f, "{error}"),
            Self::Execute(error) => write!(f, "{error}"),
            Self::Roi(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for GraphPipelineError {}

/// Pre-reserve one joint planner/scheduler allowance, build exactly one structural
/// plan, then execute it with the same reservation and cancellation/deadline token.
/// All stages stay private until success; errors preserve the borrowed source.
/// Nested operation/configuration schemas and exact port names still require caller
/// qualification. Callback allocation/work accounting is cooperative as documented
/// by graph_execute; absent profile metadata is never an implied color space.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn execute_pipeline<'a>(
    graph: &FilterGraph,
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if !(1..=4096).contains(&max_depth) {
        return Err(GraphPipelineError::Plan(GraphPlanError::DepthLimit));
    }
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Plan(GraphPlanError::CountLimit));
    }
    if graph.input.is_some() != external.is_some() {
        return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
    }
    // This happens before plan_graph can allocate identity/port/scheduler scratch.
    let reservation = reserve_execution(external, roi, domain, limits, graph.nodes.len(), graph.edges.len(), control).map_err(GraphPipelineError::Execute)?;
    let plan = plan_graph(graph, max_depth, control, port_allowed).map_err(GraphPipelineError::Plan)?;
    execute_reserved(&plan, reservation, control, evaluate).map_err(GraphPipelineError::Execute)
}

/// Adaptive rectangular execution, with explicit exact-port coordinate/halo/abyss
/// footprints. Only output-reachable nodes execute; this is dependency pruning,
/// not an implicit enable/disable policy. Nonrectangular requirements use checked
/// bounding unions. Each callback receives its propagated output and edge-specific
/// input read/mapping witness; an upstream surface covers its consumers' union.
///
/// Held source, plan/scheduler and ROI witness are reserved before planning. The
/// propagation ledger conservatively charges declared buffers (including union
/// gaps); execution separately checks actual source/live-output COW residency and
/// returns cache capacity at last use. Their work charges share the same ceiling.
/// The 200k structural reservation covers both bounded planners' 100k visits.
/// Callback allocations/work remain cooperative, with no operation/profile oracle.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn execute_pipeline_with_roi<'a>(
    graph: &FilterGraph,
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
    footprints: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError>,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if !(1..=4096).contains(&max_depth) {
        return Err(GraphPipelineError::Plan(GraphPlanError::DepthLimit));
    }
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Plan(GraphPlanError::CountLimit));
    }
    if graph.input.is_some() != external.is_some() {
        return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
    }
    let mut reservation =
        reserve_execution(external, roi, domain, limits, graph.nodes.len(), graph.edges.len(), control).map_err(GraphPipelineError::Execute)?;
    let (remaining_resident, remaining_work, witness_bytes) = reservation.reserve_roi_witness().map_err(GraphPipelineError::Execute)?;
    let plan = plan_graph(graph, max_depth, control, port_allowed).map_err(GraphPipelineError::Plan)?;
    // propagate_roi deducts its witness from this declaration subbudget; it is
    // already reserved as actual held metadata above, so add back that exact amount.
    // Declaration sums are conservative cumulative requirements, not the live
    // cache peak: enforce actual cache limits with last-use release in execution.
    let declared_bytes = remaining_resident.checked_add(witness_bytes).ok_or(GraphPipelineError::Execute(GraphExecuteError::ResidentLimit))?;
    let regions = propagate_roi(
        &plan,
        roi,
        domain.format,
        RoiLimits { area: remaining_work, samples: remaining_work, work: remaining_work, retained_bytes: declared_bytes },
        control,
        footprints,
    )
    .map_err(GraphPipelineError::Roi)?;
    reservation.charge_roi_work(regions.usage().work).map_err(GraphPipelineError::Execute)?;
    execute_reserved_rois(&plan, reservation, Some(&regions), control, evaluate).map_err(GraphPipelineError::Execute)
}

/// Explicit per-node native-domain opt-in. Assignments are borrowed, immutable,
/// and strictly aligned to original graph.nodes; external input keeps its own
/// caller domain. Existing uniform pipeline behavior remains unchanged.
#[allow(clippy::too_many_arguments)]
pub fn execute_pipeline_domains<'a>(
    graph: &FilterGraph,
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    assignments: &[super::graph_execute::DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if !(1..=4096).contains(&max_depth) {
        return Err(GraphPipelineError::Plan(GraphPlanError::DepthLimit));
    }
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Plan(GraphPlanError::CountLimit));
    }
    if graph.input.is_some() != external.is_some() {
        return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
    }
    let domain = super::graph_execute::validate_domains(graph, external_domain, assignments, control).map_err(GraphPipelineError::Execute)?;
    let reservation =
        super::graph_execute::reserve_execution_domains(external, roi, external_domain, domain, limits, graph.nodes.len(), graph.edges.len(), control)
            .map_err(GraphPipelineError::Execute)?;
    let plan = plan_graph(graph, max_depth, control, port_allowed).map_err(GraphPipelineError::Plan)?;
    super::graph_execute::execute_reserved_rois_domains(&plan, reservation, None, assignments, control, evaluate).map_err(GraphPipelineError::Execute)
}
/// Adaptive ROI route with native per-node domains. Producer outputs and input
/// footprints are charged in their own formats; callback input.domain records
/// the actual upstream context. There are no default conversions or aliases.
#[allow(clippy::too_many_arguments)]
pub fn execute_pipeline_with_roi_domains<'a>(
    graph: &FilterGraph,
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    assignments: &[super::graph_execute::DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
    footprints: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError>,
    evaluate: impl FnMut(NodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if !(1..=4096).contains(&max_depth) {
        return Err(GraphPipelineError::Plan(GraphPlanError::DepthLimit));
    }
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Plan(GraphPlanError::CountLimit));
    }
    if graph.input.is_some() != external.is_some() {
        return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
    }
    let domain = super::graph_execute::validate_domains(graph, external_domain, assignments, control).map_err(GraphPipelineError::Execute)?;
    let mut reservation =
        super::graph_execute::reserve_execution_domains(external, roi, external_domain, domain, limits, graph.nodes.len(), graph.edges.len(), control)
            .map_err(GraphPipelineError::Execute)?;
    let (remaining_resident, remaining_work, witness_bytes) = reservation.reserve_roi_witness().map_err(GraphPipelineError::Execute)?;
    let plan = plan_graph(graph, max_depth, control, port_allowed).map_err(GraphPipelineError::Plan)?;
    // Only already-reserved ROI metadata is added back to its declaration cap;
    // external/upstream sample residency is never refunded or added back.
    let declared_bytes = remaining_resident.checked_add(witness_bytes).ok_or(GraphPipelineError::Execute(GraphExecuteError::ResidentLimit))?;
    let regions = super::graph_roi::propagate_roi_domains(
        &plan,
        roi,
        external_domain,
        assignments,
        RoiLimits { area: remaining_work, samples: remaining_work, work: remaining_work, retained_bytes: declared_bytes },
        control,
        footprints,
    )
    .map_err(GraphPipelineError::Roi)?;
    reservation.charge_roi_work(regions.usage().work).map_err(GraphPipelineError::Execute)?;
    super::graph_execute::execute_reserved_rois_domains(&plan, reservation, Some(&regions), assignments, control, evaluate).map_err(GraphPipelineError::Execute)
}

/// Opt-in adaptive callback with explicit no-pixels-required input availability.
#[allow(clippy::too_many_arguments)]
pub fn execute_pipeline_with_roi_domains_available<'a>(
    graph: &FilterGraph,
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    external_domain: ExecutionDomain,
    assignments: &[super::graph_execute::DomainAssignment],
    limits: ExecutionLimits,
    control: &Interrupt<'_>,
    port_allowed: impl FnMut(GraphNodeId, PortDirection, &str) -> bool,
    footprints: impl FnMut(FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError>,
    evaluate: impl FnMut(super::graph_execute::AvailableNodeRequest<'_>, &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if !(1..=4096).contains(&max_depth) {
        return Err(GraphPipelineError::Plan(GraphPlanError::DepthLimit));
    }
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Plan(GraphPlanError::CountLimit));
    }
    if graph.input.is_some() != external.is_some() {
        return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
    }
    let domain = super::graph_execute::validate_domains(graph, external_domain, assignments, control).map_err(GraphPipelineError::Execute)?;
    let mut reservation =
        super::graph_execute::reserve_execution_domains(external, roi, external_domain, domain, limits, graph.nodes.len(), graph.edges.len(), control)
            .map_err(GraphPipelineError::Execute)?;
    let (remaining_resident, remaining_work, witness_bytes) = reservation.reserve_roi_witness().map_err(GraphPipelineError::Execute)?;
    let plan = plan_graph(graph, max_depth, control, port_allowed).map_err(GraphPipelineError::Plan)?;
    // Only already-reserved ROI metadata is added back to its declaration cap;
    // external/upstream sample residency is never refunded or added back.
    let declared_bytes = remaining_resident.checked_add(witness_bytes).ok_or(GraphPipelineError::Execute(GraphExecuteError::ResidentLimit))?;
    let regions = super::graph_roi::propagate_roi_domains(
        &plan,
        roi,
        external_domain,
        assignments,
        RoiLimits { area: remaining_work, samples: remaining_work, work: remaining_work, retained_bytes: declared_bytes },
        control,
        footprints,
    )
    .map_err(GraphPipelineError::Roi)?;
    reservation.charge_roi_work(regions.usage().work).map_err(GraphPipelineError::Execute)?;
    super::graph_execute::execute_reserved_rois_domains_available(&plan, reservation, Some(&regions), assignments, control, evaluate)
        .map_err(GraphPipelineError::Execute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::sample_encoding::SampleEncoding;
    // Unrun regression draft: an all-outside constant consumer must not execute
    // its upstream producer or receive fabricated source/default pixels.
    #[test]
    fn adaptive_no_pixels_skips_producer_and_preserves_explicit_availability() {
        use super::super::{
            graph_execute::{DomainAssignment, InputPixels},
            roi::{Abyss, Halo},
        };
        use photocraft_doc::xcf_filters::{GraphEdge, GraphNode, OperationIdentity};
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "Own no-read constant".into(), stored_version: "1".into(), schema_epoch: "own".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        };
        let graph = FilterGraph {
            input: None,
            nodes: vec![node(1), node(2)],
            edges: vec![GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), input_port: "source".into(), output_port: "output".into() }],
            output: GraphNodeId(2),
            original_pipeline: None,
        };
        let domain = ExecutionDomain { format: PixelFormat::RGBA8, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let assignments = [DomainAssignment { node: GraphNodeId(1), domain }, DomainAssignment { node: GraphNodeId(2), domain }];
        let roi = Rect::new(10, 10, 11, 11);
        let run = |legacy: bool, output_roi| {
            execute_pipeline_with_roi_domains_available(
                &graph,
                2,
                None,
                output_roi,
                domain,
                &assignments,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, _, _| true,
                |_| {
                    Ok(InputFootprint {
                        mapped_output: Rect::EMPTY,
                        source_extent: Rect::new(0, 0, 1, 1),
                        halo: Halo::default(),
                        abyss: Abyss::None,
                        work_per_sample: 1,
                    })
                },
                |request, budget| {
                    assert_eq!(request.node.id, GraphNodeId(2), "skipped producer must never execute");
                    if legacy {
                        return super::super::graph_execute::evaluate_materialized(request, budget, |_, _| {
                            panic!("legacy callback must not receive invented pixels")
                        });
                    }
                    assert_eq!(request.inputs.len(), 1);
                    let input = request.inputs.first().unwrap();
                    assert!(matches!(input.pixels, InputPixels::NoPixelsRequired));
                    assert_eq!(input.domain, domain);
                    assert!(input.roi.is_empty());
                    assert!(input.footprint.unwrap().source_read().is_empty());
                    budget.reserve_output(4096)?;
                    budget.finish_output(Surface::try_with_default_bytes(domain.format, &[7, 11, 13, 255]).unwrap(), request.roi, request.domain)
                },
            )
        };
        let output = run(false, roi).unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), [7, 11, 13, 255]);
        assert!(matches!(run(true, roi), Err(GraphPipelineError::Execute(GraphExecuteError::Operation("callback requires materialized input pixels")))));
        // Empty dependency demands are pruned without widening the executor's
        // existing refusal of an explicitly requested empty final output.
        assert!(matches!(run(false, Rect::EMPTY), Err(GraphPipelineError::Execute(GraphExecuteError::Roi))));
    }
    #[test]
    fn zero_index_refuses_before_port_or_operation_callback_and_passthrough_borrows() {
        let graph = FilterGraph { input: Some(GraphNodeId(7)), nodes: Vec::new(), edges: Vec::new(), output: GraphNodeId(7), original_pipeline: None };
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 4]).unwrap();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let before = source.clone();
        let denied = execute_pipeline(
            &graph,
            1,
            Some(&source),
            Rect::new(0, 0, 1, 1),
            domain,
            ExecutionLimits { index_bytes: 0, ..Default::default() },
            &Interrupt::NONE,
            |_, _, _| panic!("ports must not be visited before reservation"),
            |_, _| panic!("operation must not execute before reservation"),
        );
        assert!(matches!(denied, Err(GraphPipelineError::Execute(GraphExecuteError::IndexLimit))));
        assert_eq!(source, before);
        let output = execute_pipeline(
            &graph,
            1,
            Some(&source),
            Rect::new(0, 0, 1, 1),
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, _, _| false,
            |_, _| Err(GraphExecuteError::Operation("passthrough has no operator")),
        )
        .unwrap();
        assert!(std::ptr::eq(output.surface(), &source));
        assert!(output.into_owned().is_none());
    }
    fn adaptive_graph() -> FilterGraph {
        use photocraft_doc::xcf_filters::{GraphEdge, GraphNode, OperationIdentity};
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "Own adaptive raw default".into(), stored_version: "0:0".into(), schema_epoch: "own-synthetic".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        };
        let edge = |from, to, name: &str| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: name.into(), output_port: "output".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2), node(3)],
            edges: vec![edge(9, 1, "input"), edge(1, 2, "input"), edge(1, 2, "aux")],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }
    #[test]
    fn adaptive_nodes_receive_union_and_distinct_input_witnesses_with_last_use_peak() {
        use super::super::roi::{Abyss, Halo};
        let graph = adaptive_graph();
        let raw = [(-0.0_f64).to_bits(), 1, 0x7ff8_1234_5678_9abc, 1.0_f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let mut calls = Vec::new();
        let charge = 256 + raw.len() as u64;
        let output = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&source),
            Rect::new(4, 4, 6, 6),
            domain,
            ExecutionLimits { cache_bytes: 2 * charge, ..Default::default() },
            &Interrupt::NONE,
            |_, _, _| true,
            |request| {
                Ok(InputFootprint {
                    mapped_output: request.output,
                    source_extent: Rect::new(0, 0, 20, 20),
                    halo: if request.edge.input_port == "aux" { Halo { left: 3, ..Default::default() } } else { Halo { right: 2, ..Default::default() } },
                    abyss: Abyss::None,
                    work_per_sample: 1,
                })
            },
            |request, budget| {
                calls.push((request.node.id, request.roi));
                if request.node.id == GraphNodeId(2) {
                    assert_eq!(request.inputs[0].roi, Rect::new(4, 4, 8, 6));
                    assert_eq!(request.inputs[1].roi, Rect::new(1, 4, 6, 6));
                    assert_eq!(request.inputs[1].footprint.unwrap().support(), Rect::new(1, 4, 6, 6));
                    assert_eq!(budget.remaining().cache_bytes, charge);
                }
                budget.reserve_output(charge)?;
                let surface = Surface::try_with_default_bytes(
                    request.domain.format,
                    request.inputs.first().ok_or(GraphExecuteError::MissingInput)?.surface.default_bytes(),
                )
                .map_err(GraphExecuteError::Raster)?;
                budget.finish_output(surface, request.roi, request.domain)
            },
        )
        .unwrap();
        assert_eq!(calls, vec![(GraphNodeId(1), Rect::new(1, 4, 8, 6)), (GraphNodeId(2), Rect::new(4, 4, 6, 6))]);
        assert_eq!(output.roi(), Rect::new(4, 4, 6, 6));
        assert_eq!(output.surface().default_bytes(), raw);
        assert_eq!(source, before);
    }
    #[test]
    fn adaptive_footprint_refusal_and_late_cancel_do_not_publish_or_change_source() {
        use super::super::roi::{Abyss, Halo};
        use std::sync::atomic::{AtomicBool, Ordering};
        let graph = adaptive_graph();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 4]).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        assert!(matches!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&source),
                Rect::new(0, 0, 1, 1),
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, _, _| true,
                |_| Err(GraphRoiError::UnsupportedFootprint),
                |_, _| panic!("footprints must qualify before execution")
            ),
            Err(GraphPipelineError::Roi(GraphRoiError::UnsupportedFootprint))
        ));
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        let result = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&source),
            Rect::new(0, 0, 1, 1),
            domain,
            ExecutionLimits::default(),
            &Interrupt::cancel_only(&check),
            |_, _, _| true,
            |request| {
                Ok(InputFootprint {
                    mapped_output: request.output,
                    source_extent: request.output,
                    halo: Halo::default(),
                    abyss: Abyss::None,
                    work_per_sample: 1,
                })
            },
            |request, budget| {
                budget.reserve_output(260)?;
                let surface = Surface::try_with_default_bytes(request.domain.format, &[1, 2, 3, 4]).map_err(GraphExecuteError::Raster)?;
                let output = budget.finish_output(surface, request.roi, request.domain)?;
                if request.node.id == GraphNodeId(2) {
                    cancelled.store(true, Ordering::Relaxed);
                }
                Ok(output)
            },
        );
        assert!(matches!(result, Err(GraphPipelineError::Execute(GraphExecuteError::Cancelled))));
        assert_eq!(source, before);
    }
}
