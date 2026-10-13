//! Caller-bound rectangular mean graph candidate. Reuses the native kernel;
//! does not admit gegl:box-blur identities, schedules or source equivalence.
use super::{
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    neighborhood_box_mean::{self as mean, Params},
    roi::{self, Halo, RoiPlan},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub domain: ExecutionDomain,
    pub source_context: WorkingContext,
    pub expected_context: WorkingContext,
    pub input_extent: Rect,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub params: Params,
    /// Required exact typed schema/config validator. Its own arbitrary Rust
    /// allocation/work must be bounded and prepaid by the enclosing caller.
    pub validate_arguments: &'a dyn Fn(&GraphNode, Params) -> Result<(), GraphExecuteError>,
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn kernel_error(error: mean::Error) -> GraphExecuteError {
    match error {
        mean::Error::Raster(RasterInputError::Cancelled) => GraphExecuteError::Cancelled,
        mean::Error::Raster(error) => GraphExecuteError::Raster(error),
        _ => GraphExecuteError::Operation("rectangular mean configuration/domain refusal"),
    }
}
fn validate(binding: &Binding<'_>, output: Rect) -> Result<Halo, GraphExecuteError> {
    if binding.node.0 == 0
        || binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.is_empty()
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || [binding.input_port, binding.upstream_output_port, binding.output_port].iter().any(|p| p.is_empty() || p.len() > 256 || p.contains('\0'))
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if binding.source_context != binding.expected_context
        || binding.domain.encoding != binding.expected_context.encoding
        || binding.domain.profile_fingerprint != Some(binding.expected_context.profile_identity)
        || !matches!(binding.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(binding.domain.format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(GraphExecuteError::Domain);
    }
    if binding.input_extent.is_empty() || output.is_empty() || output.intersect(&binding.input_extent) != output {
        return Err(GraphExecuteError::Roi);
    }
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(GraphExecuteError::Raster)?;
    checked_region_byte_len(output, binding.domain.format).map_err(GraphExecuteError::Raster)?;
    mean::required_input(output, binding.params).map_err(kernel_error)?;
    Ok(Halo {
        left: u32::from(binding.params.radius_x),
        right: u32::from(binding.params.radius_x),
        top: u32::from(binding.params.radius_y),
        bottom: u32::from(binding.params.radius_y),
    })
}
fn abyss(binding: &Binding<'_>) -> roi::Abyss {
    match binding.params.abyss {
        mean::Abyss::Clamp => roi::Abyss::Clamp,
        mean::Abyss::Zero => roi::Abyss::None,
    }
}
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == binding.input_port,
        PortDirection::Output => port == binding.output_port,
    }
}
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if binding.node != request.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let halo = validate(binding, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.input_extent, halo, abyss: abyss(binding), work_per_sample: 65536 })
}
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    let first = TileCoord::containing(rect.x0, rect.y0);
    let last = TileCoord::containing(rect.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?, rect.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?);
    let cols = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| GraphExecuteError::Roi)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| GraphExecuteError::Roi)?;
    mul(cols, rows)
}
/// Identity/halo checked callback. Kernel preserves outside input; a separately
/// prepaid raw trim removes unrelated tiles before scheduler publication.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let halo = validate(binding, request.roi)?;
    if request.node.id != binding.node
        || request.node.operation != *binding.identity
        || request.node.version_origin != binding.version_origin
        || request.node.arguments.len() > 64
        || request.inputs.len() != 1
        || request.domain != binding.domain
    {
        return Err(GraphExecuteError::Domain);
    }
    (binding.validate_arguments)(request.node, binding.params)?;
    budget.poll()?;
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.domain != binding.domain
        || input.surface.format() != binding.domain.format
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
    {
        return Err(GraphExecuteError::Domain);
    }
    let expected =
        RoiPlan::new(request.roi, binding.input_extent, halo, abyss(binding), binding.domain.format, budget.interrupt()).map_err(GraphExecuteError::Raster)?;
    let supplied = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != expected.source_read()
        || supplied.output() != expected.output()
        || supplied.support() != expected.support()
        || supplied.source_read() != expected.source_read()
        || supplied.source_extent() != expected.source_extent()
        || supplied.abyss() != expected.abyss()
    {
        return Err(GraphExecuteError::Roi);
    }
    let format = binding.domain.format;
    let bpp = format.bytes_per_pixel() as u64;
    let count = tiles(request.roi)?;
    let retained_tiles = input.surface.tile_count() as u64;
    let side = TILE_SIZE as u64;
    let payload = mul(mul(side, side)?, bpp)?;
    let channels = format.channels() as u64;
    let colors = if binding.params.alpha == mean::Alpha::Filter { channels } else { format.mode.color_channels() as u64 };
    let rx = u64::from(binding.params.radius_x);
    let ry = u64::from(binding.params.radius_y);
    let replacements = mul(count, add(payload, 512)?)?;
    // Mirror the kernel's published conservative capacity/work envelope, not its
    // rolling numerical algorithm. Actual allocations remain checked in kernel.
    let samples = add(mul(mul(add(side, mul(2, ry)?)?, side)?, colors)?, add(mul(add(side, mul(2, rx)?)?, channels)?, mul(side, colors)?)?)?;
    let scratch = add(mul(samples, 8)?, add(payload, mul(4096, bpp)?)?)?;
    let maps = mul(add(retained_tiles, count)?, 1024)?;
    let staging = add(mul(payload, 4)?, 8192)?;
    let extra = add(add(replacements, scratch)?, add(maps, staging)?)?;
    let source_upper = add(mul(retained_tiles, payload)?, bpp)?;
    let kernel_peak = add(source_upper, extra)?;
    let per_tile = add(mul(mul(add(side, 256)?, add(side, 256)?)?, mul(channels, 256)?)?, mul(payload, 16)?)?;
    let kernel_work = add(mul(count, per_tile)?, add(mul(retained_tiles, 32)?, mul(count, mul(add(retained_tiles, count)?, 512)?)?)?)?;
    let all_tiles = add(retained_tiles, count)?;
    let charge = add(payload, 256)?;
    let filtered = add(mul(all_tiles, charge)?, bpp)?;
    let output = add(add(mul(count, charge)?, bpp)?, 256)?;
    let copied = checked_region_byte_len(request.roi, format).map_err(GraphExecuteError::Raster)? as u64;
    let trim_peak = add(add(filtered, copied)?, add(mul(count, charge)?, mul(bpp, 2)?)?)?;
    let trim_work = add(add(mul(copied, 2)?, mul(count, payload)?)?, add(mul(all_tiles, 4)?, count)?)?;
    let scratch_charge = add(kernel_peak, add(filtered, add(mul(copied, 2)?, mul(bpp, 4)?)?)?)?;
    let index = mul(add(all_tiles, count)?, 1024)?;
    let peak = add(add(output, index)?, scratch_charge)?;
    let work = add(kernel_work, trim_work)?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(GraphExecuteError::ResidentLimit);
    }
    budget.reserve_output(output)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(scratch_charge)?;
    budget.charge_work(work)?;
    let filtered = mean::try_apply(
        input.surface,
        binding.input_extent,
        request.roi,
        binding.source_context,
        binding.expected_context,
        binding.params,
        mean::Limits { resident_bytes: kernel_peak, work_units: kernel_work },
        budget.interrupt(),
    )
    .map_err(kernel_error)?;
    let edges = [f64::from(request.roi.x0), f64::from(request.roi.y0), f64::from(request.roi.x1), f64::from(request.roi.y1)];
    let trimmed = try_crop_raw(
        &filtered,
        binding.input_extent,
        edges,
        CropOrigin::KeepCoordinates,
        CropLimits { resident_bytes: trim_peak, work_units: trim_work },
        budget.interrupt(),
    )
    .map_err(GraphExecuteError::Raster)?;
    if trimmed.extent != request.roi {
        return Err(GraphExecuteError::Roi);
    }
    drop(filtered);
    budget.poll()?;
    budget.finish_output(trimmed.surface, request.roi, binding.domain)
}
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
