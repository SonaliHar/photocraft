//! Exact caller-bound adaptive nearest callbacks, not serialized source admission.
//! Identity equality and structural graphs do not qualify operation arguments,
//! coordinate semantics or color interpretation. The mandatory validator does.
use super::{
    geometry_nearest::{self as nearest, Params},
    geometry_sampling_roi::{self as sampling, SourceMetadata, SourceRequirement},
    graph_execute::{
        AvailableNodeRequest, ExecutionDomain, ExecutionLimits, GraphExecuteError, GraphOutput, InputPixels, NodeBudget, NodeOutput, NodeRequest,
        evaluate_materialized,
    },
    graph_pipeline::{GraphPipelineError, execute_pipeline_with_roi},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterGraph, GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface};

pub struct Binding<'a> {
    pub node: &'a GraphNode,
    pub declared_index: usize,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub upstream_node: GraphNodeId,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub source_extent: Rect,
    pub domain: ExecutionDomain,
    pub params: Params,
    /// Must independently bind exact schema/arguments to params and current
    /// domain, without profile/epoch/port aliases. It must be bounded and
    /// allocation-free, or prepaid by its enclosing owner; Rust callbacks are
    /// cooperative. No permissive default or stock operation is registered.
    pub validate_schema: &'a dyn Fn(&GraphNode, &Params, ExecutionDomain) -> Result<(), GraphExecuteError>,
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn kernel(error: nearest::Error) -> GraphExecuteError {
    match error {
        nearest::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Operation("nearest native geometry refused"),
    }
}
fn roi_error(error: sampling::Error) -> GraphRoiError {
    match error {
        sampling::Error::Raster(RasterInputError::Cancelled) => GraphRoiError::Cancelled,
        sampling::Error::Raster(RasterInputError::LimitExceeded) => GraphRoiError::WorkLimit,
        _ => GraphRoiError::Bounds,
    }
}
fn execute_roi_error(error: sampling::Error) -> GraphExecuteError {
    match error {
        sampling::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Roi,
    }
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn planning_error(error: GraphExecuteError) -> GraphRoiError {
    match error {
        GraphExecuteError::Cancelled => GraphRoiError::Cancelled,
        GraphExecuteError::WorkLimit => GraphRoiError::WorkLimit,
        _ => GraphRoiError::UnsupportedFootprint,
    }
}
fn exact_binding(binding: &Binding<'_>, node: &GraphNode, domain: ExecutionDomain) -> Result<(), GraphExecuteError> {
    // Exact borrowed node identity avoids a deep equality walk through opaque
    // graph/config payloads. Their interpretation is solely the bounded validator.
    if binding.node.id.0 == 0
        || binding.upstream_node.0 == 0
        || binding.declared_index >= 4096
        || !std::ptr::eq(binding.node, node)
        || binding.version_origin != node.version_origin
        || binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.is_empty()
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || node.operation.name.len() > 256
        || node.operation.stored_version.len() > 64
        || node.operation.schema_epoch.len() > 256
        || binding.identity != &node.operation
        || node.arguments.len() > 64
        || [binding.input_port, binding.output_port, binding.upstream_output_port].iter().any(|port| port.is_empty() || port.len() > 256)
    {
        return Err(GraphExecuteError::Operation("nearest requires exact caller node/schema/version/ports"));
    }
    if domain != binding.domain {
        return Err(GraphExecuteError::Domain);
    }
    (binding.validate_schema)(node, &binding.params, domain)
}
pub fn accepts_port(binding: &Binding<'_>, node: GraphNodeId, direction: PortDirection, port: &str) -> bool {
    match (node, direction) {
        (id, PortDirection::Input) if id == binding.node.id => port == binding.input_port,
        (id, PortDirection::Output) if id == binding.node.id => port == binding.output_port,
        (id, PortDirection::Output) if id == binding.upstream_node => port == binding.upstream_output_port,
        _ => false,
    }
}
fn source_region(binding: &Binding<'_>, output: Rect, control: &Interrupt<'_>) -> Result<Rect, sampling::Error> {
    match sampling::plan_nearest(
        SourceMetadata { format: binding.domain.format, extent: binding.source_extent },
        output,
        binding.params,
        sampling::Limits { work_units: 128 },
        control,
    )? {
        SourceRequirement::None => Ok(Rect::EMPTY),
        SourceRequirement::Rectangle(rect) => Ok(rect),
        SourceRequirement::FullExtentForUncertainDisplacement(_) => Err(sampling::Error::Coordinate),
    }
}

/// Mathematical footprint only. Clamp edge taps are already mapped into input;
/// constant abyss/no reads is an explicit empty rectangle, never a fake source
/// pixel. All-corners-outside mappings still request conservative interior taps.
/// Mixed-operator enclosing graphs must precharge 128 planning work units per
/// invocation plus their own bounded validator work, before calling this adapter.
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>, control: &Interrupt<'_>) -> Result<InputFootprint, GraphRoiError> {
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    if request.node != binding.node.id
        || request.declared_index != binding.declared_index
        || request.edge.to != binding.node.id
        || request.edge.from != binding.upstream_node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    exact_binding(binding, binding.node, binding.domain).map_err(planning_error)?;
    let mapped_output = source_region(binding, request.output, control).map_err(roi_error)?;
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    Ok(InputFootprint { mapped_output, source_extent: binding.source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 64 })
}

/// Requires the adaptive footprint witness; fixed-ROI/missing witnesses refuse.
/// Exact format, encoding and profile marker stay unchanged. Upstream live
/// owners remain deducted; no upstream reservation is returned to this callback.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    exact_binding(binding, request.node, request.domain)?;
    budget.poll()?;
    if request.inputs.len() != 1 {
        return Err(GraphExecuteError::MissingInput);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    let upstream = match input.endpoint {
        Endpoint::ExternalInput(id) | Endpoint::Node { id, .. } => id,
    };
    if upstream != binding.upstream_node
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != binding.domain
        || input.surface.format() != binding.domain.format
    {
        return Err(GraphExecuteError::Domain);
    }
    budget.charge_work(128)?;
    let needed = source_region(binding, request.roi, budget.interrupt()).map_err(execute_roi_error)?;
    let witness = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != needed
        || witness.output() != needed
        || witness.support() != needed
        || witness.source_read() != needed
        || witness.source_extent() != binding.source_extent
        || witness.abyss() != Abyss::None
    {
        return Err(GraphExecuteError::Roi);
    }
    // Charge both preflight and kernel metadata censuses before either can walk
    // a large sparse map. The second debit is part of kernel work below.
    let metadata = (input.surface.tile_count() as u64).checked_mul(8).ok_or(GraphExecuteError::WorkLimit)?;
    budget.charge_work(metadata)?;
    let requirements =
        nearest::requirements_with_interrupt(input.surface, binding.source_extent, request.roi, binding.params, budget.interrupt()).map_err(kernel)?;
    let output = add(requirements.output_retained_bytes, requirements.output_index_bytes)?;
    let scratch = add(requirements.source_retained_bytes, requirements.scratch_bytes)?;
    if add(output, scratch)? < requirements.peak_resident_bytes {
        return Err(GraphExecuteError::Reservation);
    }
    budget.reserve_output(output)?;
    // Conservative extra resident debit for the separately enforced index cap.
    budget.reserve_index(requirements.output_index_bytes)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(requirements.work_units)?;
    let output = nearest::try_map(
        input.surface,
        binding.source_extent,
        request.roi,
        binding.params,
        nearest::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
        budget.interrupt(),
    )
    .map_err(kernel)?;
    budget.poll()?;
    budget.finish_output(output, request.roi, request.domain)
}

/// Companion for the available-input scheduler. NoPixelsRequired is a checked
/// lack of upstream pixel dependency, never permission to fabricate defaults.
/// Materialized inputs retain the existing evaluator and its exact behavior.
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if request.inputs.len() != 1 {
        return Err(GraphExecuteError::MissingInput);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if matches!(input.pixels, InputPixels::Materialized(_)) {
        return evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget));
    }
    exact_binding(binding, request.node, request.domain)?;
    let upstream = match input.endpoint {
        Endpoint::ExternalInput(id) | Endpoint::Node { id, .. } => id,
    };
    if upstream != binding.upstream_node
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != binding.domain
    {
        return Err(GraphExecuteError::Domain);
    }
    budget.charge_work(128)?;
    // Independently re-plan from exact geometry: an empty witness alone does
    // not prove this sampler needs no reads.
    let needed = sampling::plan_nearest(
        SourceMetadata { format: binding.domain.format, extent: binding.source_extent },
        request.roi,
        binding.params,
        sampling::Limits { work_units: 128 },
        budget.interrupt(),
    )
    .map_err(execute_roi_error)?;
    if needed != SourceRequirement::None {
        return Err(GraphExecuteError::MissingInput);
    }
    let witness = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if !input.roi.is_empty()
        || witness.output() != Rect::EMPTY
        || witness.support() != Rect::EMPTY
        || witness.source_read() != Rect::EMPTY
        || witness.source_extent() != binding.source_extent
        || witness.abyss() != Abyss::None
    {
        return Err(GraphExecuteError::Roi);
    }
    // Requirements and execution each independently run the 128-unit planner.
    budget.charge_work(128)?;
    let requirements =
        nearest::requirements_no_pixels_with_interrupt(binding.domain.format, binding.source_extent, request.roi, binding.params, budget.interrupt())
            .map_err(kernel)?;
    if requirements.source_retained_bytes != 0 {
        return Err(GraphExecuteError::Reservation);
    }
    let output = add(requirements.output_retained_bytes, requirements.output_index_bytes)?;
    let scratch = add(requirements.scratch_bytes, 4096)?;
    budget.reserve_output(output)?;
    budget.reserve_index(requirements.output_index_bytes)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(requirements.work_units)?;
    let output = nearest::try_map_no_pixels(
        binding.domain.format,
        binding.source_extent,
        request.roi,
        binding.params,
        nearest::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
        budget.interrupt(),
    )
    .map_err(kernel)?;
    budget.poll()?;
    budget.finish_output(output, request.roi, request.domain)
}

fn find<'a, 'data>(bindings: &'a [Binding<'data>], id: GraphNodeId, control: &Interrupt<'_>) -> Result<&'a Binding<'data>, GraphExecuteError> {
    for (index, binding) in bindings.iter().enumerate() {
        if index.is_multiple_of(32) {
            control.check().map_err(|_| GraphExecuteError::Cancelled)?;
        }
        if binding.node.id == id {
            return Ok(binding);
        }
    }
    Err(GraphExecuteError::Operation("missing exact nearest binding"))
}
/// Convenience all-nearest graph lane. Binding/lookup/footprint work is debited
/// before structural planning; validators' own allocations/work remain caller
/// responsibility. Mixed graphs can instead reuse the two callback adapters.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn execute<'a>(
    graph: &FilterGraph,
    bindings: &[Binding<'_>],
    max_depth: usize,
    external: Option<&'a Surface>,
    roi: Rect,
    domain: ExecutionDomain,
    mut limits: ExecutionLimits,
    control: &Interrupt<'_>,
) -> Result<GraphOutput<'a>, GraphPipelineError> {
    control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
    if bindings.len() != graph.nodes.len() || bindings.len() > 4096 || graph.edges.len() > 16384 {
        return Err(GraphPipelineError::Execute(GraphExecuteError::InvalidPlan));
    }
    let standard = ExecutionLimits::default();
    if limits.resident_bytes > standard.resident_bytes
        || limits.cache_bytes > standard.cache_bytes
        || limits.index_bytes > standard.index_bytes
        || limits.work_units > standard.work_units
    {
        return Err(GraphPipelineError::Execute(GraphExecuteError::RaisedLimit));
    }
    // Covers repeated bounded linear binding/port lookup and fixed ROI math.
    let visits = (graph.nodes.len() as u64)
        .checked_add(graph.edges.len() as u64)
        .and_then(|n| n.checked_mul(4))
        .ok_or(GraphPipelineError::Execute(GraphExecuteError::WorkLimit))?;
    let planning = visits
        .checked_mul((bindings.len() as u64).checked_add(128).ok_or(GraphPipelineError::Execute(GraphExecuteError::WorkLimit))?)
        .ok_or(GraphPipelineError::Execute(GraphExecuteError::WorkLimit))?;
    limits.work_units = limits.work_units.checked_sub(planning).ok_or(GraphPipelineError::Execute(GraphExecuteError::WorkLimit))?;
    for (index, binding) in bindings.iter().enumerate() {
        control.check().map_err(|_| GraphPipelineError::Execute(GraphExecuteError::Cancelled))?;
        let node = graph.nodes.get(index).ok_or(GraphPipelineError::Execute(GraphExecuteError::InvalidPlan))?;
        if binding.declared_index != index {
            return Err(GraphPipelineError::Execute(GraphExecuteError::InvalidPlan));
        }
        exact_binding(binding, node, domain).map_err(GraphPipelineError::Execute)?;
        if Some(binding.upstream_node) != graph.input {
            let producer = find(bindings, binding.upstream_node, control).map_err(GraphPipelineError::Execute)?;
            if producer.output_port != binding.upstream_output_port || producer.domain != binding.domain {
                return Err(GraphPipelineError::Execute(GraphExecuteError::Domain));
            }
        }
    }
    execute_pipeline_with_roi(
        graph,
        max_depth,
        external,
        roi,
        domain,
        limits,
        control,
        |id, direction, port| bindings.iter().any(|binding| accepts_port(binding, id, direction, port)),
        |request| {
            let binding = find(bindings, request.node, control).map_err(planning_error)?;
            footprint(binding, request, control)
        },
        |request, budget| {
            let binding = find(bindings, request.node.id, control)?;
            evaluate(binding, request, budget)
        },
    )
}

// Unrun adaptive raw/coverage/schema/budget drafts; no serialized admission.
#[cfg(test)]
mod tests {
    use super::*;
    use nearest::{Affine, NativePixel, Ties};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{SampleEncoding, xcf_filters::GraphEdge};
    #[test]
    fn available_constant_abyss_dag_skips_owned_producer_without_inventing_source_pixels() {
        use super::super::{graph_execute::DomainAssignment, graph_pipeline::execute_pipeline_with_roi_domains_available};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let constant =
            NativePixel::from_slice(&[0x8000000000000000u64, 0x7ff8000000000019].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        let params = Params {
            map: Affine { xx: 0., xy: 0., yx: 0., yy: 0., tx: 1000., ty: 1000. },
            ties: Ties::Lower,
            abyss: nearest::Abyss::ConstantNative(constant),
            output_default: NativePixel::from_slice(&[0; 16]).unwrap(),
        };
        let node = |id, name: &str| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: name.into(), stored_version: "1".into(), schema_epoch: "own-constant-nearest-v1".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        };
        let graph = FilterGraph {
            input: None,
            nodes: vec![node(1, "own unneeded source"), node(2, "own constant nearest")],
            edges: vec![GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), input_port: "source".into(), output_port: "output".into() }],
            output: GraphNodeId(2),
            original_pipeline: None,
        };
        let consumer = graph.nodes.get(1).unwrap();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Perceptual, profile_fingerprint: None };
        let extent = Rect::new(-2, -1, 2, 1);
        let validate = |node: &GraphNode, params: &Params, actual: ExecutionDomain| {
            let values = [params.map.xx, params.map.xy, params.map.yx, params.map.yy, params.map.tx, params.map.ty];
            let expected = [0f64, 0., 0., 0., 1000., 1000.];
            if node.operation.name != "own constant nearest"
                || !node.arguments.is_empty()
                || actual != domain
                || values.iter().zip(expected).any(|(a, b)| a.to_bits() != b.to_bits())
                || params.ties != Ties::Lower
                || params.abyss != nearest::Abyss::ConstantNative(constant)
                || params.output_default != NativePixel::from_slice(&[0; 16]).unwrap()
            {
                return Err(GraphExecuteError::Operation("own constant-nearest exact schema refused"));
            }
            Ok(())
        };
        let binding = Binding {
            node: consumer,
            declared_index: 1,
            identity: &consumer.operation,
            version_origin: consumer.version_origin,
            upstream_node: GraphNodeId(1),
            input_port: "source",
            upstream_output_port: "output",
            output_port: "output",
            source_extent: extent,
            domain,
            params,
            validate_schema: &validate,
        };
        let assignments = [DomainAssignment { node: GraphNodeId(1), domain }, DomainAssignment { node: GraphNodeId(2), domain }];
        let roi = Rect::new(-257, -2, 257, 2);
        let output = execute_pipeline_with_roi_domains_available(
            &graph,
            2,
            None,
            roi,
            domain,
            &assignments,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |id, direction, port| accepts_port(&binding, id, direction, port),
            |request| footprint(&binding, request, &Interrupt::NONE),
            |request, budget| {
                assert_eq!(request.node.id, consumer.id, "owned producer callback must never execute");
                assert!(matches!(request.inputs.first().unwrap().pixels, InputPixels::NoPixelsRequired));
                evaluate_available(&binding, request, budget)
            },
        )
        .unwrap();
        assert_eq!(output.surface().try_read_interleaved(Rect::new(-1, 0, 1, 1)).unwrap(), constant.bytes.get(..16).unwrap().repeat(2));
        assert_eq!(output.surface().default_bytes(), &[0; 16]);
        // A forged empty footprint cannot make an inside-reading map source-free.
        let inside_params = Params { map: Affine { xx: 0., xy: 0., yx: 0., yy: 0., tx: 0., ty: 0. }, ..params };
        let inside_validate = |_: &GraphNode, _: &Params, _: ExecutionDomain| Ok(());
        let inside = Binding { params: inside_params, validate_schema: &inside_validate, ..binding };
        assert!(
            execute_pipeline_with_roi_domains_available(
                &graph,
                2,
                None,
                roi,
                domain,
                &assignments,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |id, direction, port| accepts_port(&inside, id, direction, port),
                |_| Ok(InputFootprint { mapped_output: Rect::EMPTY, source_extent: extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 1 }),
                |request, budget| evaluate_available(&inside, request, budget)
            )
            .is_err()
        );
    }
    fn graph() -> FilterGraph {
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own nearest".into(), stored_version: "own".into(), schema_epoch: "caller-bound-test".into() },
                version_origin: Default::default(),
                arguments: vec![],
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "own-in".into(), output_port: "own-out".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        }
    }
    fn validate(node: &GraphNode, _: &Params, _: ExecutionDomain) -> Result<(), GraphExecuteError> {
        if !node.arguments.is_empty() {
            return Err(GraphExecuteError::Operation("own schema requires zero arguments"));
        }
        Ok(())
    }
    fn binding<'a>(graph: &'a FilterGraph, domain: ExecutionDomain, extent: Rect, params: Params) -> Binding<'a> {
        Binding {
            node: &graph.nodes[0],
            declared_index: 0,
            identity: &graph.nodes[0].operation,
            version_origin: graph.nodes[0].version_origin,
            upstream_node: GraphNodeId(9),
            input_port: "own-in",
            upstream_output_port: "own-out",
            output_port: "own-out",
            source_extent: extent,
            domain,
            params,
            validate_schema: &validate,
        }
    }
    fn params(format: PixelFormat) -> Params {
        Params {
            map: Affine { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0, tx: 0.0, ty: 0.0 },
            ties: Ties::Lower,
            abyss: nearest::Abyss::Clamp,
            output_default: NativePixel::from_slice(&vec![0; format.bytes_per_pixel()]).unwrap(),
        }
    }
    #[test]
    fn adaptive_negative_affine_keeps_low_u32_bits_and_exact_domain() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let extent = Rect::new(-2, 0, 1, 1);
        let raw = [16_777_217u32, 16_777_218, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([5; 32]) };
        let mut p = params(format);
        p.map.tx = -12.0;
        let b = binding(&graph, domain, extent, p);
        let roi = Rect::new(10, 0, 13, 1);
        let output = execute(&graph, &[b], 1, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), raw);
        assert_eq!(output.domain(), domain);
        assert_eq!(source, before);
    }
    #[test]
    fn all_corners_outside_requests_interior_and_constant_none_stays_empty() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = Rect::new(-1, -1, 2, 2);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Perceptual, profile_fingerprint: Some([4; 32]) };
        let mut p = params(format);
        p.abyss = nearest::Abyss::ConstantNative(NativePixel::from_slice(&0x7ff8000000000007u64.to_ne_bytes()).unwrap());
        let mut b = binding(&graph, domain, extent, p);
        let request = |output| FootprintRequest { node: GraphNodeId(1), declared_index: 0, edge_index: 0, edge: &graph.edges[0], output };
        assert_eq!(footprint(&b, request(Rect::new(-5, -5, 6, 6)), &Interrupt::NONE).unwrap().mapped_output, extent);
        b.params.map.tx = 100.0;
        assert!(footprint(&b, request(Rect::new(0, 0, 1, 1)), &Interrupt::NONE).unwrap().mapped_output.is_empty());
        let source = Surface::new(format);
        let output = execute(&graph, &[b], 1, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), 0x7ff8000000000007u64.to_ne_bytes());
    }
    #[test]
    fn schema_domain_budget_and_cancellation_refuse_without_source_mutation() {
        let graph = graph();
        let format = PixelFormat::RGBA8;
        let source = Surface::new(format);
        let before = source.clone();
        let extent = Rect::new(0, 0, 2, 2);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let b = binding(&graph, domain, extent, params(format));
        let wrong = ExecutionDomain { encoding: SampleEncoding::Linear, ..domain };
        assert!(execute(&graph, &[b], 1, Some(&source), extent, wrong, ExecutionLimits::default(), &Interrupt::NONE).is_err());
        let b = binding(&graph, domain, extent, params(format));
        assert!(execute(&graph, &[b], 1, Some(&source), extent, domain, ExecutionLimits { cache_bytes: 1, ..Default::default() }, &Interrupt::NONE).is_err());
        let reject = |_: &GraphNode, _: &Params, _: ExecutionDomain| Err(GraphExecuteError::Operation("independent schema refusal"));
        let mut b = binding(&graph, domain, extent, params(format));
        b.validate_schema = &reject;
        assert!(execute(&graph, &[b], 1, Some(&source), extent, domain, ExecutionLimits::default(), &Interrupt::NONE).is_err());
        let cancel = || true;
        let b = binding(&graph, domain, extent, params(format));
        assert!(execute(&graph, &[b], 1, Some(&source), extent, domain, ExecutionLimits::default(), &Interrupt::cancel_only(&cancel)).is_err());
        assert_eq!(source, before);
    }
    #[test]
    fn clamp_edge_footprint_and_wrong_port_refusal_are_exact() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false);
        let extent = Rect::new(-1, -1, 2, 2);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let mut p = params(format);
        p.map.tx = 100.0;
        let mut b = binding(&graph, domain, extent, p);
        let roi = Rect::new(0, 0, 1, 1);
        let request = FootprintRequest { node: GraphNodeId(1), declared_index: 0, edge_index: 0, edge: &graph.edges[0], output: roi };
        assert_eq!(footprint(&b, request, &Interrupt::NONE).unwrap().mapped_output, Rect::new(1, 0, 2, 1));
        b.input_port = "alias";
        let request = FootprintRequest { node: GraphNodeId(1), declared_index: 0, edge_index: 0, edge: &graph.edges[0], output: roi };
        assert!(matches!(footprint(&b, request, &Interrupt::NONE), Err(GraphRoiError::UnsupportedFootprint)));
        let source = Surface::new(format);
        assert!(execute(&graph, &[b], 1, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE).is_err());
    }
}
