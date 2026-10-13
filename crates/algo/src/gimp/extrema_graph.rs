//! Exact caller-schema graph candidate for existing native rectangular extrema.
//! No GEGL defaults, stock morphology shape, source admission or CMS inferred.
use super::{
    alpha_association_graph as shared,
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError as Error, InputPixels, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    neighborhood_morphology as kernel,
    roi::{self, Halo, RoiPlan},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterArgument, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};
type Result<T> = std::result::Result<T, Error>;
/// Mandatory trusted Rust validator; caller must bound its own work/allocations.
pub type ArgumentValidator = fn(&[FilterArgument], kernel::Params) -> Result<()>;
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub arguments: &'a [FilterArgument],
    pub validate_arguments: ArgumentValidator,
    pub endpoint: Endpoint,
    pub input_extent: Rect,
    pub domain: ExecutionDomain,
    pub expected_context: WorkingContext,
    pub params: kernel::Params,
    /// Explicit lower-only per-tile halo-pixel limit. No radius/abyss defaults.
    pub halo_pixels: u64,
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(Error::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(Error::ResidentLimit)
}
fn error(e: kernel::Error) -> Error {
    match e {
        kernel::Error::Raster(RasterInputError::Cancelled) => Error::Cancelled,
        kernel::Error::Raster(e) => Error::Raster(e),
        _ => Error::Operation("native extrema context/view policy"),
    }
}
fn validate(binding: &Binding<'_>, output: Rect) -> Result<Halo> {
    if binding.identity.name != "own:rectangular-extrema"
        || binding.identity.stored_version != "1"
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || binding.identity.schema_epoch.contains('\0')
        || binding.node.0 == 0
        || !shared::endpoint_valid(binding.endpoint)
        || binding.params.radius_x > 4096
        || binding.params.radius_y > 4096
        || binding.halo_pixels == 0
    {
        return Err(Error::InvalidPlan);
    }
    if binding.params.source_context != binding.expected_context
        || binding.domain.encoding != binding.expected_context.encoding
        || binding.domain.profile_fingerprint != Some(binding.expected_context.profile_identity)
        || !matches!(binding.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(binding.domain.format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
    {
        return Err(Error::Domain);
    }
    if output.is_empty() || binding.input_extent.is_empty() || output.intersect(&binding.input_extent) != output {
        return Err(Error::Roi);
    }
    checked_region_byte_len(output, binding.domain.format).map_err(Error::Raster)?;
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(Error::Raster)?;
    shared::bounded(binding.arguments)?;
    (binding.validate_arguments)(binding.arguments, binding.params)?;
    if let kernel::Abyss::Constant { bytes, length } = binding.params.abyss {
        let bpp = binding.domain.format.bytes_per_pixel();
        if usize::from(length) != bpp {
            return Err(Error::Domain);
        }
        let pixel = bytes.get(..bpp).ok_or(Error::Domain)?;
        for channel in 0..binding.domain.format.channels() {
            let value = photocraft_color::try_read_sample_f64(pixel, binding.domain.format.sample, channel).map_err(|_| Error::Domain)?;
            if !value.is_finite() {
                return Err(Error::Domain);
            }
        }
    }
    Ok(Halo { left: binding.params.radius_x, right: binding.params.radius_x, top: binding.params.radius_y, bottom: binding.params.radius_y })
}
fn abyss(binding: &Binding<'_>) -> roi::Abyss {
    match binding.params.abyss {
        kernel::Abyss::Clamp => roi::Abyss::Clamp,
        kernel::Abyss::Constant { .. } => roi::Abyss::None,
    }
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == "source",
        PortDirection::Output => port == "output",
    }
}
/// Constant abyss maps absent support to the explicit native kernel pixel, not
/// zero. This admitted lane still needs a materialized source/default/center.
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>) -> std::result::Result<InputFootprint, GraphRoiError> {
    if request.node != binding.node || request.edge.to != binding.node || request.edge.input_port != "source" || request.edge.output_port != "output" {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let source_id = match binding.endpoint {
        Endpoint::ExternalInput(id) => id,
        Endpoint::Node { id, .. } => id,
    };
    if request.edge.from != source_id {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let halo = validate(binding, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    let pixels = (u64::from(binding.params.radius_x) * 2 + 1).checked_mul(u64::from(binding.params.radius_y) * 2 + 1).ok_or(GraphRoiError::WorkLimit)?;
    let work = pixels
        .checked_mul(64 + (binding.domain.format.bytes_per_pixel() as u64) * 4 + (binding.domain.format.channels() as u64) * 32)
        .ok_or(GraphRoiError::WorkLimit)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.input_extent, halo, abyss: abyss(binding), work_per_sample: work })
}
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    let halo = validate(binding, request.roi)?;
    shared::schema(binding.node, binding.identity, binding.version_origin, binding.arguments, &request, "own:rectangular-extrema")?;
    (binding.validate_arguments)(&request.node.arguments, binding.params)?;
    if request.domain != binding.domain || request.inputs.len() != 1 {
        return Err(Error::Domain);
    }
    let input = request.inputs.first().ok_or(Error::MissingInput)?;
    if input.endpoint != binding.endpoint
        || input.input_port != "source"
        || input.output_port != "output"
        || input.domain != binding.domain
        || input.surface.format() != binding.domain.format
    {
        return Err(Error::Domain);
    }
    let expected = RoiPlan::new(request.roi, binding.input_extent, halo, abyss(binding), binding.domain.format, budget.interrupt()).map_err(Error::Raster)?;
    let supplied = input.footprint.ok_or(Error::Roi)?;
    if input.roi != expected.source_read()
        || supplied.output() != expected.output()
        || supplied.support() != expected.support()
        || supplied.source_read() != expected.source_read()
        || supplied.source_extent() != expected.source_extent()
        || supplied.abyss() != expected.abyss()
    {
        return Err(Error::Roi);
    }
    let bpp = binding.domain.format.bytes_per_pixel() as u64;
    let channels = binding.domain.format.channels() as u64;
    let width = u64::from(request.roi.width()).min(TILE_SIZE as u64);
    let height = u64::from(request.roi.height()).min(TILE_SIZE as u64);
    let nx = u64::try_from(i64::from(request.roi.x1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(request.roi.x0).div_euclid(i64::from(TILE_SIZE)) + 1)
        .map_err(|_| Error::Roi)?;
    let ny = u64::try_from(i64::from(request.roi.y1 - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(request.roi.y0).div_euclid(i64::from(TILE_SIZE)) + 1)
        .map_err(|_| Error::Roi)?;
    let output_tiles = mul(nx, ny)?;
    let source_tiles = input.surface.tile_count() as u64;
    let payload = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let hw = add(width, mul(u64::from(binding.params.radius_x), 2)?)?;
    let hh = add(height, mul(u64::from(binding.params.radius_y), 2)?)?;
    let halo_area = mul(hw, hh)?;
    if halo_area > binding.halo_pixels {
        return Err(Error::ResidentLimit);
    }
    let filtered = if binding.domain.format.alpha && binding.params.alpha == kernel::Alpha::PreserveCenter {
        channels.checked_sub(1).ok_or(Error::Domain)?
    } else {
        channels
    };
    let window = add(mul(u64::from(binding.params.radius_x.max(binding.params.radius_y)), 2)?, 1)?;
    // Kernel Entry is private: conservatively allow64 bytes per deque entry,
    // retaining its two-times-capacity check without duplicating sample math.
    let deque = mul(mul(mul(window, 2)?, filtered)?, 64)?;
    let scratch = add(add(mul(mul(width, hh)?, bpp)?, mul(mul(width, height)?, bpp)?)?, add(deque, 8192)?)?;
    let source = add(add(mul(source_tiles, payload)?, bpp)?, mul(source_tiles, 256)?)?;
    let maps = mul(source_tiles, 512)?;
    let replacements = mul(output_tiles, add(payload, 512)?)?;
    let lower_peak = add(add(source, maps)?, add(replacements, add(scratch, mul(payload, 4)?)?)?)?;
    let per_tile = add(add(mul(halo_area, 64 + bpp * 4 + channels * 32)?, mul(mul(width, hh)?, channels * 32)?)?, mul(mul(width, height)?, 64 + bpp * 4)?)?;
    let work = add(add(mul(output_tiles, per_tile)?, mul(source_tiles, 16)?)?, mul(mul(output_tiles, add(source_tiles, output_tiles)?)?, 256)?)?;
    let output = add(add(256, bpp)?, mul(output_tiles, add(payload, 256)?)?)?;
    let index = add(mul(add(source_tiles, mul(output_tiles, 2)?)?, 1024)?, 1024)?;
    let scratch_charge = add(source, add(scratch, mul(payload, 4)?)?)?;
    let peak = add(output, add(index, scratch_charge)?)?;
    if lower_peak > MAX_CHECKED_RASTER_BYTES || peak > MAX_CHECKED_RASTER_BYTES {
        return Err(Error::ResidentLimit);
    }
    budget.charge_work(add(work, mul(add(source_tiles, output_tiles)?, 256)?)?)?;
    budget.reserve_output(output)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(scratch_charge)?;
    let staged = kernel::try_rectangular_extrema(
        input.surface,
        request.roi,
        binding.input_extent,
        binding.params,
        binding.expected_context,
        kernel::Limits { resident_bytes: lower_peak, work_units: work, halo_pixels: binding.halo_pixels },
        budget.interrupt(),
    )
    .map_err(error)?;
    shared::finish(staged, &request, budget)
}
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    if request.inputs.len() != 1 || request.inputs.iter().any(|i| matches!(i.pixels, InputPixels::NoPixelsRequired)) {
        return Err(Error::MissingInput);
    }
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
