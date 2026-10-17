//! Caller-bound prepared Curve/Levels graph consumer. Preparation is reused
//! across ROI cells; no serialized source identity or default admission is inferred.
use super::gimp_configured_render_candidate::{OutputPolicy, render_prepared_window};
use photocraft_algo::gimp::{
    configured::{PreparedConfiguration, Transfer},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError as Error, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

pub struct Binding<'a, 'config> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub prepared: &'a PreparedConfiguration<'config>,
    pub domain: ExecutionDomain,
    pub source_extent: Rect,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub output_policy: OutputPolicy,
    /// Must bind the exact caller schema and current arguments to the original
    /// typed configuration and sampling policy, not merely the operation name.
    pub validate_arguments: &'a dyn Fn(&GraphNode, &PreparedConfiguration<'config>) -> Result<(), Error>,
    /// Caller-bound requirements for the validator's arbitrary Rust implementation.
    pub validation_bytes: u64,
    pub validation_work: u64,
}
fn add(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_add(b).ok_or(Error::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, Error> {
    a.checked_mul(b).ok_or(Error::ResidentLimit)
}
fn raster(error: RasterInputError) -> Error {
    if error == RasterInputError::Cancelled { Error::Cancelled } else { Error::Raster(error) }
}
fn valid(binding: &Binding<'_, '_>) -> bool {
    binding.node.0 != 0
        && !binding.identity.name.is_empty()
        && binding.identity.name.len() <= 256
        && !binding.identity.stored_version.is_empty()
        && binding.identity.stored_version.len() <= 64
        && !binding.identity.schema_epoch.is_empty()
        && binding.identity.schema_epoch.len() <= 256
        && [binding.input_port, binding.upstream_output_port, binding.output_port].iter().all(|p| !p.is_empty() && p.len() <= 256 && !p.contains('\0'))
        && !binding.source_extent.is_empty()
}
pub fn accepts_port(binding: &Binding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    valid(binding)
        && match direction {
            PortDirection::Input => port == binding.input_port,
            PortDirection::Output => port == binding.output_port,
        }
}
pub fn footprint(binding: &Binding<'_, '_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if !valid(binding)
        || request.node != binding.node
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
/// Native precision and coverage use the same numerical/staging helper as the
/// retained-source renderer. Source samples must already be in the exact working
/// profile/transfer; this entry performs no implicit CMS conversion.
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, Error> {
    budget.poll()?;
    let format = binding.domain.format;
    let context = binding.prepared.working_context();
    let colors = match format.mode {
        photocraft_color::ColorMode::Rgb => 3,
        photocraft_color::ColorMode::Grayscale => 1,
        _ => return Err(Error::Domain),
    };
    let encoding = match context.transfer {
        Transfer::Linear => SampleEncoding::Linear,
        Transfer::NonlinearProfile => SampleEncoding::NonlinearProfile,
        Transfer::Perceptual => SampleEncoding::Perceptual,
    };
    if !valid(binding)
        || request.node.id != binding.node
        || request.node.operation != *binding.identity
        || request.node.version_origin != binding.version_origin
        || request.domain != binding.domain
        || request.inputs.len() != 1
        || request.roi.is_empty()
        || request.roi.intersect(&binding.source_extent) != request.roi
        || context.color_channels != colors
        || context.alpha != format.alpha
        || binding.domain.encoding != encoding
        || binding.domain.profile_fingerprint != Some(context.profile_identity)
        || format.bytes_per_pixel() == 0
    {
        return Err(Error::Domain);
    }
    // Validator and immutable prepared configuration are conservatively charged
    // for every cell; no upstream reservations are refunded.
    budget.reserve_scratch(add(binding.prepared.retained_bytes(), binding.validation_bytes)?)?;
    budget.charge_work(binding.validation_work)?;
    (binding.validate_arguments)(request.node, binding.prepared)?;
    budget.poll()?;
    let input = request.inputs.first().ok_or(Error::MissingInput)?;
    if input.domain != binding.domain
        || input.surface.format() != format
        || input.roi != request.roi
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
    {
        return Err(Error::Domain);
    }
    if let Some(plan) = input.footprint
        && (plan.output() != request.roi
            || plan.source_read() != request.roi
            || plan.support() != request.roi
            || plan.source_extent() != binding.source_extent
            || plan.abyss() != Abyss::None)
    {
        return Err(Error::Roi);
    }
    let first = TileCoord::containing(request.roi.x0, request.roi.y0);
    let last = TileCoord::containing(request.roi.x1.checked_sub(1).ok_or(Error::Roi)?, request.roi.y1.checked_sub(1).ok_or(Error::Roi)?);
    let columns = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| Error::Roi)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| Error::Roi)?;
    let tiles = mul(columns, rows)?;
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, format.bytes_per_pixel() as u64)?;
    let source_lower = mul(input.surface.tile_count() as u64, add(tile_bytes, 256)?)?;
    if source_lower > budget.remaining().resident_bytes {
        return Err(Error::ResidentLimit);
    }
    if mul(input.surface.tile_count() as u64, 8)? > budget.remaining().work_units {
        return Err(Error::WorkLimit);
    }
    let retained = input.surface.resident_accounted_bytes().map_err(raster)?;
    let output = mul(tiles, add(tile_bytes, 1024)?)?;
    let index = mul(tiles, 256)?;
    let scratch = mul(4096, format.bytes_per_pixel() as u64)?;
    let peak = add(add(add(retained, output)?, index)?, add(scratch, 256)?)?;
    if add(peak, add(binding.prepared.retained_bytes(), binding.validation_bytes)?)? > MAX_CHECKED_RASTER_BYTES {
        return Err(Error::ResidentLimit);
    }
    let pixels = u64::try_from(checked_region_byte_len(request.roi, format).map_err(raster)? / format.bytes_per_pixel()).map_err(|_| Error::WorkLimit)?;
    let work = add(add(mul(pixels, 4096)?, mul(output, 16)?)?, mul(input.surface.tile_count() as u64, 8)?)?;
    budget.reserve_output(add(output, 256)?)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(add(retained, scratch)?)?;
    budget.charge_work(work)?;
    input.surface.try_validate_tile_origins_with(budget.interrupt()).map_err(raster)?;
    let rendered =
        render_prepared_window(binding.prepared, input.surface, request.roi, context, binding.output_policy, output, scratch, work, budget.interrupt())
            .map_err(raster)?;
    budget.poll()?;
    budget.finish_output(rendered.surface, rendered.roi, binding.domain)
}
/// NoPixelsRequired is unsupported for a configured point consumer; the shared
/// bridge checks this and precharges its materialized binding vector.
pub fn evaluate_available(binding: &Binding<'_, '_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, Error> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
