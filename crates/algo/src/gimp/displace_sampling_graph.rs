//! Caller-bound adaptive displacement with independent source/vector domains.
//! Requires the graph scheduler's explicit per-node/input domains. No conversion,
//! serialized operation admission, port alias or absent-profile default is used.
use super::{
    geometry_displace_nearest::{self as displace, FieldParams, VectorBounds},
    geometry_nearest::Params,
    geometry_sampling_roi::{self as sampling, DisplacementPlan, FieldMetadata, ProvenVectorBounds, SourceMetadata, SourceRequirement, UncertaintyPolicy},
    graph_execute::{AvailableInputBinding, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, InputBinding, InputPixels, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
    wide_point::WorkingContext,
};
use photocraft_doc::{item_identity::GraphNodeId, xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity}};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError};

#[derive(Clone, Copy, Debug)]
pub struct Domains { pub source: ExecutionDomain, pub field: ExecutionDomain }
#[derive(Clone, Copy, Debug)]
pub struct Configuration {
    pub source_extent: Rect,
    pub field_extent: Rect,
    pub geometry: Params,
    pub field: FieldParams,
    pub proven: ProvenVectorBounds,
    pub uncertainty: UncertaintyPolicy,
}
pub struct Binding<'a> {
    pub node: &'a GraphNode,
    pub declared_index: usize,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub source_node: GraphNodeId,
    pub source_input_port: &'a str,
    pub source_output_port: &'a str,
    pub field_node: GraphNodeId,
    pub field_input_port: &'a str,
    pub field_output_port: &'a str,
    pub output_port: &'a str,
    pub domains: Domains,
    pub configuration: Configuration,
    /// Mandatory independent identity/schema/argument/config/domain binding.
    /// Bounded allocation-free callback, or additional work/buffers prepaid by
    /// enclosing owner. These bridges do not inspect opaque config or prove CMS.
    pub validate_schema: &'a dyn Fn(&GraphNode, &Configuration, Domains) -> Result<(), GraphExecuteError>,
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn kernel(error: displace::Error) -> GraphExecuteError {
    match error { displace::Error::Raster(error) => raster(error), _ => GraphExecuteError::Operation("native displacement configuration/sample refusal") }
}
fn planning_error(error: GraphExecuteError) -> GraphRoiError {
    match error { GraphExecuteError::Cancelled => GraphRoiError::Cancelled, GraphExecuteError::WorkLimit => GraphRoiError::WorkLimit, _ => GraphRoiError::UnsupportedFootprint }
}
fn roi_error(error: sampling::Error) -> GraphRoiError {
    match error { sampling::Error::Raster(RasterInputError::Cancelled) => GraphRoiError::Cancelled, sampling::Error::Raster(RasterInputError::LimitExceeded) => GraphRoiError::WorkLimit, _ => GraphRoiError::Bounds }
}
fn execute_roi_error(error: sampling::Error) -> GraphExecuteError {
    match error { sampling::Error::Raster(error) => raster(error), _ => GraphExecuteError::Roi }
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> { a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit) }
fn field_context(binding: &Binding<'_>) -> Result<WorkingContext, GraphExecuteError> {
    let profile_identity = binding.domains.field.profile_fingerprint.ok_or(GraphExecuteError::Domain)?;
    let context = WorkingContext { encoding: binding.domains.field.encoding, profile_identity };
    if context != binding.configuration.field.context { return Err(GraphExecuteError::Domain); }
    Ok(context)
}
fn exact_binding(binding: &Binding<'_>, node: &GraphNode, output_domain: ExecutionDomain) -> Result<(), GraphExecuteError> {
    if !std::ptr::eq(binding.node, node) || node.id.0 == 0 || binding.source_node.0 == 0 || binding.field_node.0 == 0 || binding.declared_index >= 4096
        || binding.version_origin != node.version_origin || binding.identity.name.is_empty() || binding.identity.name.len() > 256
        || binding.identity.stored_version.is_empty() || binding.identity.stored_version.len() > 64 || binding.identity.schema_epoch.is_empty() || binding.identity.schema_epoch.len() > 256
        || node.operation.name.len() > 256 || node.operation.stored_version.len() > 64 || node.operation.schema_epoch.len() > 256 || binding.identity != &node.operation || node.arguments.len() > 64
        || [binding.source_input_port, binding.source_output_port, binding.field_input_port, binding.field_output_port, binding.output_port].iter().any(|port| port.is_empty() || port.len() > 256)
        || binding.source_input_port == binding.field_input_port { return Err(GraphExecuteError::Operation("displacement requires exact caller schema/version/ports")); }
    if output_domain != binding.domains.source { return Err(GraphExecuteError::Domain); }
    field_context(binding)?;
    (binding.validate_schema)(node, &binding.configuration, binding.domains)
}
pub fn accepts_port(binding: &Binding<'_>, node: GraphNodeId, direction: PortDirection, port: &str) -> bool {
    match (node, direction) {
        (id, PortDirection::Input) if id == binding.node.id => port == binding.source_input_port || port == binding.field_input_port,
        (id, PortDirection::Output) if id == binding.node.id => port == binding.output_port,
        (id, PortDirection::Output) => (id == binding.source_node && port == binding.source_output_port) || (id == binding.field_node && port == binding.field_output_port),
        _ => false,
    }
}
fn regions(binding: &Binding<'_>, output: Rect, control: &Interrupt<'_>) -> Result<DisplacementPlan, sampling::Error> {
    sampling::plan_displacement(SourceMetadata { format: binding.domains.source.format, extent: binding.configuration.source_extent },
        FieldMetadata { format: binding.domains.field.format, extent: binding.configuration.field_extent }, output,
        binding.configuration.geometry, binding.configuration.field, binding.configuration.field.context,
        binding.configuration.proven, binding.configuration.uncertainty, sampling::Limits { work_units: 128 }, control)
}
fn source_rectangle(plan: DisplacementPlan) -> Rect {
    match plan.source { SourceRequirement::None => Rect::EMPTY, SourceRequirement::Rectangle(rect) | SourceRequirement::FullExtentForUncertainDisplacement(rect) => rect }
}

/// Enclosing mixed-domain graph must precharge 256 work units per footprint
/// callback (including empty input requirements), plus its bounded validator.
/// Per-edge format comes from the scheduler's actual upstream DomainAssignment;
/// no field-format substitution or implicit RGBA conversion is permitted.
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>, control: &Interrupt<'_>) -> Result<InputFootprint, GraphRoiError> {
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    if request.node != binding.node.id || request.declared_index != binding.declared_index || request.edge.to != binding.node.id { return Err(GraphRoiError::UnsupportedFootprint); }
    exact_binding(binding, binding.node, binding.domains.source).map_err(planning_error)?;
    let plan = regions(binding, request.output, control).map_err(roi_error)?;
    let (mapped_output, source_extent) = if request.edge.input_port == binding.source_input_port && request.edge.output_port == binding.source_output_port && request.edge.from == binding.source_node {
        (source_rectangle(plan), binding.configuration.source_extent)
    } else if request.edge.input_port == binding.field_input_port && request.edge.output_port == binding.field_output_port && request.edge.from == binding.field_node {
        (plan.field.unwrap_or(Rect::EMPTY), binding.configuration.field_extent)
    } else { return Err(GraphRoiError::UnsupportedFootprint); };
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    Ok(InputFootprint { mapped_output, source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 160 })
}
fn input<'a, 'data>(inputs: &'a [InputBinding<'data>], port: &str) -> Result<&'a InputBinding<'data>, GraphExecuteError> {
    let mut result = None;
    for input in inputs {
        if input.input_port == port {
            if result.is_some() { return Err(GraphExecuteError::InvalidPlan); } result = Some(input);
        }
    }
    result.ok_or(GraphExecuteError::MissingInput)
}
fn check_input(input: &InputBinding<'_>, node: GraphNodeId, output_port: &str, domain: ExecutionDomain, region: Rect, extent: Rect) -> Result<(), GraphExecuteError> {
    let upstream = match input.endpoint { Endpoint::ExternalInput(id) | Endpoint::Node { id, .. } => id };
    if upstream != node || input.output_port != output_port || input.domain != domain || input.surface.format() != domain.format { return Err(GraphExecuteError::Domain); }
    let witness = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != region || witness.output() != region || witness.support() != region || witness.source_read() != region || witness.source_extent() != extent || witness.abyss() != Abyss::None { return Err(GraphExecuteError::Roi); }
    Ok(())
}
/// Actual controlled native kernel after exact mixed-domain/witness checks and
/// pre-reservation. Neither already-held upstream owner is refunded. The kernel
/// verifies every selected field vector against proven bounds before source read.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?; exact_binding(binding, request.node, request.domain)?; budget.poll()?;
    if request.inputs.len() != 2 { return Err(GraphExecuteError::MissingInput); }
    let source = input(request.inputs, binding.source_input_port)?; let field = input(request.inputs, binding.field_input_port)?;
    budget.charge_work(256)?; let plan = regions(binding, request.roi, budget.interrupt()).map_err(execute_roi_error)?;
    check_input(source, binding.source_node, binding.source_output_port, binding.domains.source, source_rectangle(plan), binding.configuration.source_extent)?;
    check_input(field, binding.field_node, binding.field_output_port, binding.domains.field, plan.field.unwrap_or(Rect::EMPTY), binding.configuration.field_extent)?;
    let metadata = (source.surface.tile_count() as u64).checked_add(field.surface.tile_count() as u64).and_then(|n| n.checked_mul(8)).ok_or(GraphExecuteError::WorkLimit)?;
    budget.charge_work(metadata)?;
    let configuration = binding.configuration; let context = field_context(binding)?;
    let requirements = displace::requirements_with_interrupt(source.surface, configuration.source_extent, field.surface, configuration.field_extent, request.roi,
        configuration.field, context, configuration.geometry, budget.interrupt()).map_err(kernel)?;
    let output = add(requirements.output_retained_bytes, requirements.output_index_bytes)?;
    let scratch = add(add(requirements.source_retained_bytes, requirements.field_retained_bytes)?, requirements.scratch_bytes)?;
    if add(output, scratch)? < requirements.peak_resident_bytes { return Err(GraphExecuteError::Reservation); }
    budget.reserve_output(output)?; budget.reserve_index(requirements.output_index_bytes)?; budget.reserve_scratch(scratch)?; budget.charge_work(requirements.work_units)?;
    let output = displace::try_displace_with_vector_bounds(source.surface, configuration.source_extent, field.surface, configuration.field_extent, request.roi,
        configuration.field, context, configuration.geometry, VectorBounds { min: configuration.proven.min, max: configuration.proven.max },
        super::geometry_nearest::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units }, budget.interrupt()).map_err(kernel)?;
    budget.poll()?; budget.finish_output(output, request.roi, request.domain)
}

fn available_input<'a, 'data>(inputs: &'a [AvailableInputBinding<'data>], port: &str) -> Result<&'a AvailableInputBinding<'data>, GraphExecuteError> {
    let mut result = None;
    for input in inputs { if input.input_port == port { if result.is_some() { return Err(GraphExecuteError::InvalidPlan); } result = Some(input); } }
    result.ok_or(GraphExecuteError::MissingInput)
}
fn available_surface<'a>(input: &AvailableInputBinding<'a>, node: GraphNodeId, output_port: &str, domain: ExecutionDomain, region: Rect, extent: Rect) -> Result<Option<&'a photocraft_raster::Surface>, GraphExecuteError> {
    let upstream = match input.endpoint { Endpoint::ExternalInput(id) | Endpoint::Node { id, .. } => id };
    if upstream != node || input.output_port != output_port || input.domain != domain { return Err(GraphExecuteError::Domain); }
    let witness = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != region || witness.output() != region || witness.support() != region || witness.source_read() != region || witness.source_extent() != extent || witness.abyss() != Abyss::None { return Err(GraphExecuteError::Roi); }
    match input.pixels {
        InputPixels::Materialized(surface) if surface.format() == domain.format => Ok(Some(surface)),
        InputPixels::Materialized(_) => Err(GraphExecuteError::Domain),
        InputPixels::NoPixelsRequired if region.is_empty() => Ok(None),
        InputPixels::NoPixelsRequired => Err(GraphExecuteError::MissingInput),
    }
}
/// Availability-aware scheduler companion. A skipped producer stays absent;
/// each no-read source/field witness is independently recomputed here and in
/// the kernel. No sentinel surface, implicit default or upstream refund occurs.
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?; exact_binding(binding, request.node, request.domain)?; budget.poll()?;
    if request.inputs.len() != 2 { return Err(GraphExecuteError::MissingInput); }
    budget.charge_work(256)?; let plan = regions(binding, request.roi, budget.interrupt()).map_err(execute_roi_error)?;
    let source_input = available_input(request.inputs, binding.source_input_port)?; let field_input = available_input(request.inputs, binding.field_input_port)?;
    let source = available_surface(source_input, binding.source_node, binding.source_output_port, binding.domains.source, source_rectangle(plan), binding.configuration.source_extent)?;
    let field = available_surface(field_input, binding.field_node, binding.field_output_port, binding.domains.field, plan.field.unwrap_or(Rect::EMPTY), binding.configuration.field_extent)?;
    let metadata = (source.map_or(0, photocraft_raster::Surface::tile_count) as u64).checked_add(field.map_or(0, photocraft_raster::Surface::tile_count) as u64).and_then(|n| n.checked_mul(8)).ok_or(GraphExecuteError::WorkLimit)?;
    // Includes the independent pure no-read planner before metadata preparation.
    budget.charge_work(metadata.checked_add(128).ok_or(GraphExecuteError::WorkLimit)?)?;
    let configuration = binding.configuration; let context = field_context(binding)?;
    let source_metadata = SourceMetadata { format: binding.domains.source.format, extent: configuration.source_extent };
    let field_metadata = FieldMetadata { format: binding.domains.field.format, extent: configuration.field_extent };
    let range = VectorBounds { min: configuration.proven.min, max: configuration.proven.max };
    let requirements = displace::requirements_available_with_interrupt(source, source_metadata, field, field_metadata, request.roi, configuration.field, context,
        configuration.geometry, range, budget.interrupt()).map_err(kernel)?;
    let output = add(requirements.output_retained_bytes, requirements.output_index_bytes)?;
    let scratch = add(add(requirements.source_retained_bytes, requirements.field_retained_bytes)?, requirements.scratch_bytes)?;
    if add(output, scratch)? < requirements.peak_resident_bytes { return Err(GraphExecuteError::Reservation); }
    budget.reserve_output(output)?; budget.reserve_index(requirements.output_index_bytes)?; budget.reserve_scratch(scratch)?; budget.charge_work(requirements.work_units)?;
    let output = displace::try_displace_available(source, source_metadata, field, field_metadata, request.roi, configuration.field, context, configuration.geometry, range,
        super::geometry_nearest::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units }, budget.interrupt()).map_err(kernel)?;
    budget.poll()?; budget.finish_output(output, request.roi, request.domain)
}

// Unrun independent-domain footprint and actual adaptive pipeline drafts.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{SampleEncoding, xcf_filters::GraphEdge};
    use super::super::{geometry_displace_nearest::{FieldCoordinates, VectorUnits}, geometry_nearest::{Affine, NativePixel, Ties}};
    fn validate(_: &GraphNode, _: &Configuration, _: Domains) -> Result<(), GraphExecuteError> { Ok(()) }
    fn node() -> GraphNode { GraphNode { id: GraphNodeId(1), operation: OperationIdentity { name: "own displacement".into(), stored_version: "own".into(), schema_epoch: "caller-bound-test".into() }, version_origin: Default::default(), arguments: vec![] } }
    fn binding(node: &GraphNode) -> Binding<'_> {
        let source = ExecutionDomain { format: PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: Some([8; 32]) };
        let field = ExecutionDomain { format: PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), encoding: SampleEncoding::Linear, profile_fingerprint: Some([19; 32]) };
        Binding { node, declared_index: 0, identity: &node.operation, version_origin: node.version_origin, source_node: GraphNodeId(9), source_input_port: "own-source", source_output_port: "own-source-out",
            field_node: GraphNodeId(2), field_input_port: "own-field", field_output_port: "own-field-out", output_port: "own-out", domains: Domains { source, field },
            configuration: Configuration { source_extent: Rect::new(-3, -2, 4, 3), field_extent: Rect::new(-1, -1, 2, 2),
                geometry: Params { map: Affine { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0, tx: 0.0, ty: 0.0 }, ties: Ties::Lower, abyss: super::super::geometry_nearest::Abyss::Clamp, output_default: NativePixel::from_slice(&[0; 40]).unwrap() },
                field: FieldParams { channels: [0, 0], scales: [1.0, -1.0], outside: [0.0; 2], coordinates: FieldCoordinates::OutputPixelGrid, units: VectorUnits::SourcePixels, context: WorkingContext { encoding: field.encoding, profile_identity: [19; 32] } },
                proven: ProvenVectorBounds { min: [-1.0; 2], max: [1.0; 2] }, uncertainty: UncertaintyPolicy::Reject }, validate_schema: &validate }
    }
    #[test]
    fn independent_field_footprint_keeps_its_own_extent_and_source_bound() {
        let node = node(); let b = binding(&node); let output = Rect::new(-4, -3, 5, 4);
        for (from, input_port, output_port, expected) in [(9, "own-source", "own-source-out", b.configuration.source_extent), (2, "own-field", "own-field-out", b.configuration.field_extent)] {
            let edge = GraphEdge { from: GraphNodeId(from), to: node.id, input_port: input_port.into(), output_port: output_port.into() };
            let required = footprint(&b, FootprintRequest { node: node.id, declared_index: 0, edge_index: 0, edge: &edge, output }, &Interrupt::NONE).unwrap();
            assert_eq!(required.mapped_output, expected); assert_eq!(required.source_extent, expected); assert_eq!(required.abyss, Abyss::None);
        }
    }
    #[test]
    fn absent_field_profile_wrong_schema_pointer_and_port_alias_refuse() {
        let node = node(); let mut b = binding(&node); b.domains.field.profile_fingerprint = None;
        assert!(matches!(exact_binding(&b, &node, b.domains.source), Err(GraphExecuteError::Domain)));
        let mut b = binding(&node); b.field_input_port = b.source_input_port; assert!(exact_binding(&b, &node, b.domains.source).is_err());
        let b = binding(&node); let copied = node.clone(); assert!(exact_binding(&b, &copied, b.domains.source).is_err());
    }
    fn graph() -> photocraft_doc::xcf_filters::FilterGraph {
        let mut sampler = node(); sampler.id = GraphNodeId(1);
        let generator = GraphNode { id: GraphNodeId(2), operation: OperationIdentity { name: "own literal vector field".into(), stored_version: "own".into(), schema_epoch: "own-test-generator".into() }, version_origin: Default::default(), arguments: vec![] };
        photocraft_doc::xcf_filters::FilterGraph { input: Some(GraphNodeId(9)), nodes: vec![generator, sampler], edges: vec![
            GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "own-source".into(), output_port: "own-source-out".into() },
            GraphEdge { from: GraphNodeId(2), to: GraphNodeId(1), input_port: "own-field".into(), output_port: "own-field-out".into() },
        ], output: GraphNodeId(1), original_pipeline: None }
    }
    #[test]
    fn adaptive_cmyk_f64_source_with_gray_f32_field_keeps_raw_pixels_and_domains() {
        use super::super::{graph_execute::{DomainAssignment, ExecutionLimits}, graph_pipeline::execute_pipeline_with_roi_domains};
        use photocraft_raster::Surface;
        let graph = graph(); let mut b = binding(&graph.nodes[1]); b.declared_index = 1;
        b.configuration.field.scales = [1.0, 0.0]; b.configuration.proven = ProvenVectorBounds { min: [1.0; 2], max: [1.0; 2] };
        let mut source = Surface::new(b.domains.source.format);
        let raw = [0x8000000000000000u64, 0x7ff8000000000017, 1e100f64.to_bits(), 7u64, 0x7ff0000000000000].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(Rect::new(1, 0, 2, 1), &raw).unwrap(); let before = source.clone();
        let assignments = [DomainAssignment { node: GraphNodeId(2), domain: b.domains.field }, DomainAssignment { node: GraphNodeId(1), domain: b.domains.source }];
        let roi = Rect::new(0, 0, 1, 1); let mut field_seen = false;
        // Precharge both footprint callbacks; literal field producer's small
        // descriptor/default allocations are separately reserved in its callback.
        let limits = ExecutionLimits { work_units: ExecutionLimits::default().work_units - 512, ..Default::default() };
        let output = execute_pipeline_with_roi_domains(&graph, 2, Some(&source), roi, b.domains.source, &assignments, limits, &Interrupt::NONE,
            |id, direction, port| accepts_port(&b, id, direction, port),
            |request| footprint(&b, request, &Interrupt::NONE),
            |request, budget| {
                if request.node.id == GraphNodeId(2) {
                    assert_eq!(request.domain, b.domains.field); field_seen = true;
                    budget.reserve_output(260)?; budget.charge_work(4)?;
                    let vector = Surface::try_with_default_bytes(request.domain.format, &1.0f32.to_ne_bytes()).map_err(raster)?;
                    budget.finish_output(vector, request.roi, request.domain)
                } else { evaluate(&b, request, budget) }
            }).unwrap();
        assert!(field_seen); assert_eq!(output.domain(), b.domains.source); assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), raw); assert_eq!(source, before);
    }
    #[test]
    fn false_vector_proof_and_wrong_actual_upstream_domain_refuse_atomically() {
        use super::super::{graph_execute::{DomainAssignment, ExecutionLimits}, graph_pipeline::execute_pipeline_with_roi_domains};
        use photocraft_raster::Surface;
        let graph = graph(); let mut b = binding(&graph.nodes[1]); b.declared_index = 1; b.configuration.field.scales = [1.0, 0.0];
        b.configuration.proven = ProvenVectorBounds { min: [0.0; 2], max: [0.0; 2] };
        let source = Surface::new(b.domains.source.format); let before = source.clone(); let roi = Rect::new(0, 0, 1, 1);
        for actual_field_domain in [b.domains.field, ExecutionDomain { profile_fingerprint: Some([77; 32]), ..b.domains.field }] {
            let assignments = [DomainAssignment { node: GraphNodeId(2), domain: actual_field_domain }, DomainAssignment { node: GraphNodeId(1), domain: b.domains.source }];
            let result = execute_pipeline_with_roi_domains(&graph, 2, Some(&source), roi, b.domains.source, &assignments,
                ExecutionLimits { work_units: ExecutionLimits::default().work_units - 512, ..Default::default() }, &Interrupt::NONE,
                |id, direction, port| accepts_port(&b, id, direction, port), |request| footprint(&b, request, &Interrupt::NONE),
                |request, budget| {
                    if request.node.id == GraphNodeId(2) {
                        budget.reserve_output(260)?; budget.charge_work(4)?;
                        let vector = Surface::try_with_default_bytes(request.domain.format, &1.0f32.to_ne_bytes()).map_err(raster)?;
                        budget.finish_output(vector, request.roi, request.domain)
                    } else { evaluate(&b, request, budget) }
                });
            assert!(result.is_err()); assert_eq!(source, before);
        }
    }
    #[test]
    fn no_read_field_producer_is_skipped_without_a_pixel_sentinel() {
        use super::super::{graph_execute::{DomainAssignment, ExecutionLimits}, graph_pipeline::execute_pipeline_with_roi_domains_available};
        use photocraft_raster::Surface;
        let graph = graph(); let mut b = binding(&graph.nodes[1]); b.declared_index = 1; b.configuration.field_extent = Rect::new(10, 10, 11, 11); b.configuration.geometry.map.tx = 100.0;
        let constant = [0x8000000000000000u64, 0x7ff8000000000037, 1e100f64.to_bits(), 7, 0x7ff0000000000000].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        b.configuration.geometry.abyss = super::super::geometry_nearest::Abyss::ConstantNative(NativePixel::from_slice(&constant).unwrap());
        let source = Surface::new(b.domains.source.format); let before = source.clone(); let roi = Rect::new(0, 0, 1, 1);
        let assignments = [DomainAssignment { node: GraphNodeId(2), domain: b.domains.field }, DomainAssignment { node: GraphNodeId(1), domain: b.domains.source }];
        let mut calls = 0;
        let output = execute_pipeline_with_roi_domains_available(&graph, 2, Some(&source), roi, b.domains.source, &assignments,
            ExecutionLimits { work_units: ExecutionLimits::default().work_units - 512, ..Default::default() }, &Interrupt::NONE,
            |id, direction, port| accepts_port(&b, id, direction, port), |request| footprint(&b, request, &Interrupt::NONE),
            |request, budget| {
                calls += 1; assert_eq!(request.node.id, GraphNodeId(1));
                let field = available_input(request.inputs, b.field_input_port).unwrap(); assert!(matches!(field.pixels, InputPixels::NoPixelsRequired));
                evaluate_available(&b, request, budget)
            }).unwrap();
        assert_eq!(calls, 1); assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), constant); assert_eq!(source, before);
    }
}
