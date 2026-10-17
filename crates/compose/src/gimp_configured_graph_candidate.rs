//! Opt-in graph adapter for configured source candidates; no stock dispatch join.
use super::{
    gimp_configured_candidate::{SourceBinding, bind_source_node_with_sample_policy},
    gimp_configured_render_candidate::{OutputPolicy, render_source_candidate_with_sample_policy},
};
use photocraft_algo::gimp::{
    configured::{CurveSamplePolicy, Limits, Transfer},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{FilterNode, GraphVersionOrigin},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub version_origin: GraphVersionOrigin,
    pub source_node: &'a FilterNode,
    pub source_binding: SourceBinding<'a>,
    pub domain: ExecutionDomain,
    pub source_extent: Rect,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub output_policy: OutputPolicy,
}
fn ports(binding: &Binding<'_>) -> bool {
    [binding.input_port, binding.upstream_output_port, binding.output_port].iter().all(|port| !port.is_empty() && port.len() <= 256 && !port.contains('\0'))
}
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    ports(binding)
        && match direction {
            PortDirection::Input => port == binding.input_port,
            PortDirection::Output => port == binding.output_port,
        }
}
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if !ports(binding)
        || binding.node != request.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
        || request.output.is_empty()
        || request.output.intersect(&binding.source_extent) != request.output
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 4096 })
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
/// One explicit real source, identity coordinates and exact native working
/// domain. NoPixelsRequired is deliberately unsupported: no fabricated input.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    evaluate_with_sample_policy(binding, CurveSamplePolicy::RequireExact256, request, budget)
}
pub fn evaluate_with_sample_policy(
    binding: &Binding<'_>,
    sampling: CurveSamplePolicy,
    request: NodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    bind_source_node_with_sample_policy(binding.source_node, binding.source_binding, sampling, budget.interrupt()).map_err(raster)?;
    if !matches!(binding.domain.format.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
        || binding.domain.format.bytes_per_pixel() == 0
    {
        return Err(GraphExecuteError::Domain);
    }
    if !ports(binding)
        || binding.node.0 == 0
        || request.node.id != binding.node
        || request.node.version_origin != binding.version_origin
        || request.node.operation != binding.source_node.operation
        || request.domain != binding.domain
        || request.inputs.len() != 1
    {
        return Err(GraphExecuteError::Domain);
    }
    // Borrowed shape/type validation bounds subsequent argument equality. A
    // matching identity never supplies source authority or renderer admission.

    if request.node.arguments.len() != binding.source_node.arguments.len() || request.node.arguments != binding.source_node.arguments {
        return Err(GraphExecuteError::Operation("configured graph arguments differ from explicit source binding"));
    }
    let working = binding.source_binding.working;
    let encoding = match working.transfer {
        Transfer::Linear => SampleEncoding::Linear,
        Transfer::NonlinearProfile => SampleEncoding::NonlinearProfile,
        Transfer::Perceptual => SampleEncoding::Perceptual,
    };
    if binding.domain.encoding != encoding || binding.domain.profile_fingerprint != Some(working.profile_identity) {
        return Err(GraphExecuteError::Domain);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.domain != binding.domain
        || input.surface.format() != binding.domain.format
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.roi != request.roi
        || request.roi.is_empty()
        || request.roi.intersect(&binding.source_extent) != request.roi
    {
        return Err(GraphExecuteError::Roi);
    }
    if let Some(plan) = input.footprint
        && (plan.output() != request.roi || plan.source_read() != request.roi || plan.support() != request.roi)
    {
        return Err(GraphExecuteError::Roi);
    }
    let format = binding.domain.format;
    let first = TileCoord::containing(request.roi.x0, request.roi.y0);
    let last =
        TileCoord::containing(request.roi.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?, request.roi.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?);
    let columns = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| GraphExecuteError::Roi)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| GraphExecuteError::Roi)?;
    let tiles = columns.checked_mul(rows).ok_or(GraphExecuteError::ResidentLimit)?;
    let tile_bytes = (TILE_SIZE as u64)
        .checked_mul(TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64))
        .ok_or(GraphExecuteError::ResidentLimit)?;
    let remaining = budget.remaining();
    let source_lower = (input.surface.tile_count() as u64).checked_mul(add(tile_bytes, 256)?).ok_or(GraphExecuteError::ResidentLimit)?;
    if source_lower > remaining.resident_bytes {
        return Err(GraphExecuteError::ResidentLimit);
    }
    let source = input.surface.resident_accounted_bytes().map_err(raster)?;
    let output = tiles.checked_mul(add(tile_bytes, 1024)?).ok_or(GraphExecuteError::ResidentLimit)?;
    let index = tiles.checked_mul(256).ok_or(GraphExecuteError::IndexLimit)?;
    let scratch = add(source, add(4096u64.checked_mul(format.bytes_per_pixel() as u64).ok_or(GraphExecuteError::ResidentLimit)?, 65536)?)?;
    let output_charge = add(output, 256)?;
    let peak = add(add(output_charge, index)?, scratch)?;
    let bytes = checked_region_byte_len(request.roi, format).map_err(raster)?;
    let pixels = u64::try_from(bytes / format.bytes_per_pixel()).map_err(|_| GraphExecuteError::WorkLimit)?;
    let work = pixels
        .checked_mul(4096)
        .and_then(|n| n.checked_add(output.checked_mul(16)?))
        .and_then(|n| n.checked_add(16384))
        .and_then(|n| n.checked_add((input.surface.tile_count() as u64).checked_mul(8)?))
        .ok_or(GraphExecuteError::WorkLimit)?;
    if peak > MAX_CHECKED_RASTER_BYTES || peak > remaining.resident_bytes || work > remaining.work_units {
        return Err(GraphExecuteError::ResidentLimit);
    }
    // Upstream retained owners are already debited by the scheduler; this
    // conservative source charge never refunds them or raises remaining limits.
    budget.reserve_output(output_charge)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(work)?;
    let rendered = render_source_candidate_with_sample_policy(
        binding.source_node,
        binding.source_binding,
        sampling,
        input.surface,
        binding.source_extent,
        request.roi,
        working,
        binding.output_policy,
        Limits { resident_bytes: peak, work_units: work },
        budget.interrupt(),
    )
    .map_err(raster)?;
    budget.poll()?;
    budget.finish_output(rendered.surface, rendered.roi, binding.domain)
}
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}

pub fn evaluate_available_with_sample_policy(
    binding: &Binding<'_>,
    sampling: CurveSamplePolicy,
    request: AvailableNodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate_with_sample_policy(binding, sampling, request, budget))
}
