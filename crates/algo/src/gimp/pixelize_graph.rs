//! Explicit grid-aligned Pixelize graph consumer, not source/GEGL admission.
//! The caller establishes linear working samples, identity and typed arguments.
//! Whole intersected blocks are requested even for a single output pixel; finite
//! edge denominators and associated-alpha arithmetic remain the existing leaf's.
use super::{
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    neighborhood::{GaussianLimits, PixelizeParams, try_pixelize_square},
    roi::{Abyss, Halo, RoiPlan},
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub endpoint: Endpoint,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub domain: ExecutionDomain,
    pub input_extent: Rect,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub params: PixelizeParams,
    /// Mandatory exact schema/argument-to-parameter validator. Arbitrary captured
    /// backing or work beyond these borrowed fields is enclosing-caller prepaid.
    pub validate_arguments: &'a dyn Fn(&GraphNode, PixelizeParams) -> Result<(), GraphExecuteError>,
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
/// Smallest rectangular union of complete grid blocks intersecting output,
/// clipped to the finite input extent. Alignment uses Euclidean division.
/// Clipping precedes i32 conversion, so an outside block edge never wraps.
pub fn required_input(output: Rect, input: Rect, params: PixelizeParams) -> Result<Rect, GraphExecuteError> {
    if output.is_empty()
        || input.is_empty()
        || output.intersect(&input) != output
        || params.size_x == 0
        || params.size_y == 0
        || params.size_x > i32::MAX as u32
        || params.size_y > i32::MAX as u32
    {
        return Err(GraphExecuteError::Roi);
    }
    let axis = |first: i32, last: i32, low: i32, high: i32, size: u32, offset: i32| -> Result<(i32, i32), GraphExecuteError> {
        let size = i64::from(size);
        let offset = i64::from(offset);
        let start = (i64::from(first) - offset).div_euclid(size) * size + offset;
        let end = (i64::from(last) - 1 - offset).div_euclid(size) * size + offset + size;
        Ok((
            i32::try_from(start.max(i64::from(low))).map_err(|_| GraphExecuteError::Roi)?,
            i32::try_from(end.min(i64::from(high))).map_err(|_| GraphExecuteError::Roi)?,
        ))
    };
    let (x0, x1) = axis(output.x0, output.x1, input.x0, input.x1, params.size_x, params.offset_x)?;
    let (y0, y1) = axis(output.y0, output.y1, input.y0, input.y1, params.size_y, params.offset_y)?;
    Ok(Rect::new(x0, y0, x1, y1))
}
fn validate(binding: &Binding<'_>, output: Rect) -> Result<Rect, GraphExecuteError> {
    if binding.node.0 == 0
        || binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.is_empty()
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || [binding.input_port, binding.upstream_output_port, binding.output_port].iter().any(|port| port.is_empty() || port.len() > 256 || port.contains('\0'))
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if binding.domain.encoding != SampleEncoding::Linear
        || binding.domain.profile_fingerprint.is_none_or(|hash| hash == [0; 32])
        || !matches!(binding.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(binding.domain.format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(GraphExecuteError::Domain);
    }
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(raster)?;
    checked_region_byte_len(output, binding.domain.format).map_err(raster)?;
    required_input(output, binding.input_extent, binding.params)
}
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == binding.input_port,
        PortDirection::Output => port == binding.output_port,
    }
}
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if request.node != binding.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let from = match binding.endpoint {
        Endpoint::ExternalInput(id) => id,
        Endpoint::Node { id, .. } => id,
    };
    if request.edge.from != from {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let mapped = validate(binding, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    Ok(InputFootprint { mapped_output: mapped, source_extent: binding.input_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 256 })
}
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    let span = |first: i32, last: i32| -> Result<u64, GraphExecuteError> {
        let last = last.checked_sub(1).ok_or(GraphExecuteError::Roi)?;
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| GraphExecuteError::Roi)
    };
    mul(span(rect.x0, rect.x1)?, span(rect.y0, rect.y1)?)
}
/// Precharges the leaf's conservative full-input work/peak and a simultaneous
/// raw trim. Upstream/source bytes remain held by the scheduler: no input refund.
/// Selected colors and alpha follow the leaf; default/outside bytes are never
/// numerically interpreted by trimming. This is not alpha-preserving identity.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let read = validate(binding, request.roi)?;
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
    if input.endpoint != binding.endpoint
        || input.domain != binding.domain
        || input.surface.format() != binding.domain.format
        || input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
    {
        return Err(GraphExecuteError::Domain);
    }
    let expected = RoiPlan::new(read, binding.input_extent, Halo::default(), Abyss::None, binding.domain.format, budget.interrupt()).map_err(raster)?;
    let supplied = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != read
        || supplied.output() != expected.output()
        || supplied.support() != expected.support()
        || supplied.source_read() != read
        || supplied.source_extent() != binding.input_extent
        || supplied.abyss() != Abyss::None
    {
        return Err(GraphExecuteError::Roi);
    }
    let format = binding.domain.format;
    let bpp = format.bytes_per_pixel() as u64;
    let channels = format.channels() as u64;
    let tile = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let source_tiles = input.surface.tile_count() as u64;
    let count = tiles(request.roi)?;
    let all_tiles = add(source_tiles, count)?;
    let source_payload = add(mul(source_tiles, tile)?, bpp)?;
    // Same conservative peak as try_pixelize_square, with every output tile
    // priced as growth even when present. Additional graph/trim maps are separate.
    let kernel_peak = add(add(mul(source_payload, 2)?, mul(count, add(tile, 128)?)?)?, add(mul(tile, 3)?, add(mul(source_tiles, 128)?, 4096)?)?)?;
    let input_pixels = mul(binding.input_extent.width() as u64, binding.input_extent.height() as u64)?;
    let output_pixels = mul(request.roi.width() as u64, request.roi.height() as u64)?;
    let kernel_work = add(mul(input_pixels, mul(channels, 32)?)?, mul(output_pixels, mul(channels, 16)?)?)?;
    let charge = add(tile, 256)?;
    let filtered = add(mul(all_tiles, charge)?, bpp)?;
    let copied = checked_region_byte_len(request.roi, format).map_err(raster)? as u64;
    let output = add(add(mul(count, charge)?, bpp)?, 256)?;
    let trim_peak = add(add(filtered, copied)?, add(mul(count, charge)?, mul(bpp, 2)?)?)?;
    let trim_work = add(add(mul(copied, 2)?, mul(count, tile)?)?, add(mul(all_tiles, 4)?, count)?)?;
    let index = mul(add(all_tiles, count)?, 1024)?;
    // Deliberately conservative simultaneous stages; not allocator/RSS census.
    let scratch = add(kernel_peak, trim_peak)?;
    if add(add(output, index)?, scratch)? > MAX_CHECKED_RASTER_BYTES {
        return Err(GraphExecuteError::ResidentLimit);
    }
    budget.reserve_output(output)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(add(add(kernel_work, trim_work)?, mul(source_tiles, 32)?)?)?;
    let filtered = try_pixelize_square(
        input.surface,
        request.roi,
        binding.input_extent,
        binding.params,
        GaussianLimits { resident_bytes: kernel_peak, work_units: kernel_work },
        budget.interrupt(),
    )
    .map_err(raster)?;
    let edges = [f64::from(request.roi.x0), f64::from(request.roi.y0), f64::from(request.roi.x1), f64::from(request.roi.y1)];
    let trimmed = try_crop_raw(
        &filtered,
        binding.input_extent,
        edges,
        CropOrigin::KeepCoordinates,
        CropLimits { resident_bytes: trim_peak, work_units: trim_work },
        budget.interrupt(),
    )
    .map_err(raster)?;
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

#[cfg(test)]
mod tests {
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline_with_roi};
    use super::*;
    use photocraft_doc::xcf_filters::{FilterGraph, GraphEdge};
    #[test]
    fn adaptive_graph_reads_neighbors_outside_requested_pixel_and_refuses_wrong_domain() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        let identity = OperationIdentity { name: "own:pixelize".into(), stored_version: "own".into(), schema_epoch: "own-math-not-source".into() };
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode { id: GraphNodeId(1), operation: identity.clone(), version_origin: Default::default(), arguments: Vec::new() }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "output".into(), input_port: "input".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let extent = Rect::new(-1, 0, 2, 1);
        let roi = Rect::new(0, 0, 1, 1);
        let source = Surface::try_from_interleaved(format, extent, &[2f64.to_ne_bytes(), 4f64.to_ne_bytes(), 12f64.to_ne_bytes()].concat()).unwrap();
        let original = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let validator = |node: &GraphNode, _: PixelizeParams| if node.arguments.is_empty() { Ok(()) } else { Err(GraphExecuteError::InvalidPlan) };
        let binding = Binding {
            node: GraphNodeId(1),
            endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
            identity: &identity,
            version_origin: Default::default(),
            domain,
            input_extent: extent,
            input_port: "input",
            upstream_output_port: "output",
            output_port: "output",
            params: PixelizeParams { size_x: 3, size_y: 1, offset_x: -2, offset_y: 0 },
            validate_arguments: &validator,
        };
        let output = execute_pipeline_with_roi(
            &graph,
            1,
            Some(&source),
            roi,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| accepts_port(&binding, direction, port),
            |request| footprint(&binding, request),
            |request, budget| evaluate(&binding, request, budget),
        )
        .unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), 3f64.to_ne_bytes());
        assert_eq!(output.surface().default_bytes(), source.default_bytes());
        let cancelled = || true;
        for (limits, control) in [
            (ExecutionLimits { work_units: 1, ..ExecutionLimits::default() }, Interrupt::NONE),
            (ExecutionLimits::default(), Interrupt::cancel_only(&cancelled)),
        ] {
            assert!(
                execute_pipeline_with_roi(
                    &graph,
                    1,
                    Some(&source),
                    roi,
                    domain,
                    limits,
                    &control,
                    |_, direction, port| accepts_port(&binding, direction, port),
                    |request| footprint(&binding, request),
                    |request, budget| evaluate(&binding, request, budget)
                )
                .is_err()
            );
        }
        let wrong = ExecutionDomain { encoding: SampleEncoding::NonlinearProfile, ..domain };
        assert!(
            execute_pipeline_with_roi(
                &graph,
                1,
                Some(&source),
                roi,
                wrong,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&binding, direction, port),
                |request| footprint(&binding, request),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        let zero_profile = Binding { domain: ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain }, ..binding };
        assert!(matches!(validate(&zero_profile, roi), Err(GraphExecuteError::Domain)));
        assert!(
            execute_pipeline_with_roi(
                &graph,
                1,
                Some(&source),
                roi,
                zero_profile.domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&zero_profile, direction, port),
                |request| footprint(&zero_profile, request),
                |request, budget| evaluate(&zero_profile, request, budget),
            )
            .is_err()
        );
        assert_eq!(source, original);
    }
    #[test]
    fn negative_grid_and_partial_edges_request_entire_contributing_blocks() {
        let input = Rect::new(-7, -5, 8, 7);
        let params = PixelizeParams { size_x: 4, size_y: 3, offset_x: 1, offset_y: -1 };
        assert_eq!(required_input(Rect::new(-4, -3, -3, -2), input, params).unwrap(), Rect::new(-7, -4, -3, -1));
        assert_eq!(required_input(Rect::new(7, 6, 8, 7), input, params).unwrap(), Rect::new(5, 5, 8, 7));
    }
    #[test]
    fn huge_outside_block_edges_clip_before_coordinate_conversion() {
        let input = Rect::new(i32::MIN + 1, 0, i32::MIN + 3, 1);
        let params = PixelizeParams { size_x: i32::MAX as u32, size_y: 1, offset_x: i32::MAX, offset_y: 0 };
        assert_eq!(required_input(input, input, params).unwrap(), input);
        assert!(required_input(input, input, PixelizeParams { size_x: 0, ..params }).is_err());
    }
    #[test]
    fn finite_edge_block_denominator_and_source_are_preserved() {
        use photocraft_color::PixelFormat;
        use photocraft_raster::{Interrupt, Surface};
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let input = Rect::new(-1, 0, 2, 1);
        let roi = Rect::new(0, 0, 1, 1);
        let source = Surface::try_from_interleaved(format, input, &[2f64.to_ne_bytes(), 4f64.to_ne_bytes(), 12f64.to_ne_bytes()].concat()).unwrap();
        let original = source.clone();
        let params = PixelizeParams { size_x: 3, size_y: 1, offset_x: -2, offset_y: 0 };
        assert_eq!(required_input(roi, input, params).unwrap(), Rect::new(-1, 0, 1, 1));
        let output = try_pixelize_square(&source, roi, input, params, GaussianLimits::default(), &Interrupt::NONE).unwrap();
        let bytes = output.try_read_interleaved_with(roi, &Interrupt::NONE).unwrap();
        assert_eq!(bytes, 3f64.to_ne_bytes());
        assert_eq!(source, original);
    }
}
