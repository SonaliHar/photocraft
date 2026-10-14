//! Adaptive caller-bound standard Sobel consumer, not source/GEGL admission.
//! Existing candidate_graph provides only same-ROI Sobel. This companion keeps
//! original finite input extent while requesting exact one-pixel neighborhood
//! support. Direction/sign/transfer and edge policy remain explicit caller input.
use super::{
    edge_sobel::{self, SobelAbyss, SobelLimits, SobelParams, SobelTransfer, SobelWorkingInput, try_sobel_standard},
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
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
    pub params: SobelParams,
    pub transfer: SobelTransfer,
    pub abyss: SobelAbyss,
    /// Mandatory exact schema/argument-to-parameter validator. Arbitrary captured
    /// backing or work beyond these borrowed fields is enclosing-caller prepaid.
    pub validate_arguments: &'a dyn Fn(&GraphNode, SobelParams) -> Result<(), GraphExecuteError>,
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
fn halo() -> Halo {
    Halo { left: 1, right: 1, top: 1, bottom: 1 }
}
fn abyss(binding: &Binding<'_>) -> Abyss {
    match binding.abyss {
        SobelAbyss::None => Abyss::None,
        SobelAbyss::Clamp => Abyss::Clamp,
    }
}
fn validate(binding: &Binding<'_>, output: Rect) -> Result<(), GraphExecuteError> {
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
    let encoding = match binding.transfer {
        SobelTransfer::Linear => SampleEncoding::Linear,
        SobelTransfer::Nonlinear => SampleEncoding::NonlinearProfile,
    };
    if binding.domain.encoding != encoding
        || binding.domain.format.alpha
        || binding.params.horizontal == binding.params.vertical
        || binding.domain.profile_fingerprint.is_none_or(|hash| hash == [0; 32])
        || !matches!(binding.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(binding.domain.format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(GraphExecuteError::Domain);
    }
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(raster)?;
    checked_region_byte_len(output, binding.domain.format).map_err(raster)?;
    if binding.input_extent.is_empty() || output.is_empty() || output.intersect(&binding.input_extent) != output {
        return Err(GraphExecuteError::Roi);
    }
    edge_sobel::sobel_required_input(output).map_err(raster)?;
    Ok(())
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
    validate(binding, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.input_extent, halo: halo(), abyss: abyss(binding), work_per_sample: 256 })
}
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    let span = |first: i32, last: i32| -> Result<u64, GraphExecuteError> {
        let last = last.checked_sub(1).ok_or(GraphExecuteError::Roi)?;
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| GraphExecuteError::Roi)
    };
    mul(span(rect.x0, rect.x1)?, span(rect.y0, rect.y1)?)
}
/// Precharges the leaf's conservative retained-source work/peak and a simultaneous
/// raw trim. Upstream/source bytes remain held by the scheduler: no input refund.
/// Selected colors follow the leaf; alpha is unsupported. Original encoded
/// default and outside bytes remain unchanged by the leaf and raw trimming.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    validate(binding, request.roi)?;
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
    let expected = RoiPlan::new(request.roi, binding.input_extent, halo(), abyss(binding), binding.domain.format, budget.interrupt()).map_err(raster)?;
    let supplied = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != expected.source_read()
        || supplied.output() != expected.output()
        || supplied.support() != expected.support()
        || supplied.source_read() != expected.source_read()
        || supplied.source_extent() != binding.input_extent
        || supplied.abyss() != expected.abyss()
    {
        return Err(GraphExecuteError::Roi);
    }
    let format = binding.domain.format;
    let bpp = format.bytes_per_pixel() as u64;
    let tile = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let source_tiles = input.surface.tile_count() as u64;
    let count = tiles(request.roi)?;
    let all_tiles = add(source_tiles, count)?;
    let retained = add(mul(source_tiles, add(tile, 256)?)?, bpp)?;
    let copied_map = add(mul(source_tiles, 256)?, bpp)?;
    // Mirrors the leaf's metadata-derived peak, including all copied sparse maps
    // and six native tile payloads, before its resident metadata traversal.
    let kernel_peak = add(add(retained, mul(copied_map, 2)?)?, add(mul(count, add(tile, 512)?)?, mul(tile, 6)?)?)?;
    let kernel_work = edge_sobel::required_work(input.surface, request.roi).map_err(raster)?;
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
    let filtered = try_sobel_standard(
        SobelWorkingInput { source: input.surface, extent: binding.input_extent, transfer: binding.transfer },
        request.roi,
        binding.params,
        binding.abyss,
        SobelLimits { resident_bytes: kernel_peak, work_units: kernel_work },
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
    use photocraft_color::PixelFormat;
    use photocraft_doc::xcf_filters::{FilterGraph, GraphEdge};
    use photocraft_raster::{Interrupt, Surface};
    fn graph() -> FilterGraph {
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own:standard-sobel".into(), stored_version: "own".into(), schema_epoch: "own-math-not-source".into() },
                version_origin: Default::default(),
                arguments: Vec::new(),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "output".into(), input_port: "input".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        }
    }
    fn arguments(node: &GraphNode, _: SobelParams) -> Result<(), GraphExecuteError> {
        if node.arguments.is_empty() { Ok(()) } else { Err(GraphExecuteError::InvalidPlan) }
    }
    fn make_binding(graph: &FilterGraph, domain: ExecutionDomain, extent: Rect, abyss: SobelAbyss) -> Binding<'_> {
        Binding {
            node: GraphNodeId(1),
            endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
            identity: &graph.nodes.first().unwrap().operation,
            version_origin: Default::default(),
            domain,
            input_extent: extent,
            input_port: "input",
            upstream_output_port: "output",
            output_port: "output",
            params: SobelParams { horizontal: true, vertical: false, keep_sign: true },
            transfer: SobelTransfer::Linear,
            abyss,
            validate_arguments: &arguments,
        }
    }
    #[test]
    fn partial_adaptive_gradient_uses_neighbor_support_and_keeps_raw_default() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let extent = Rect::new(-1, -1, 2, 2);
        let roi = Rect::new(0, 0, 1, 1);
        let raw = [2f64, 4., 6., 2., 4., 6., 2., 4., 6.].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        let default = 0x7ff8_0000_0000_0042u64.to_ne_bytes();
        let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
        source.try_write_interleaved_with(extent, &raw, &Interrupt::NONE).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let binding = make_binding(&graph, domain, extent, SobelAbyss::None);
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
        assert_eq!(output.roi(), roi);
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), 16f64.to_ne_bytes());
        assert_eq!(output.surface().default_bytes(), default);
        assert_eq!(source, before);
    }
    #[test]
    fn finite_edge_clamp_and_zero_abyss_have_distinct_unscaled_responses() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false };
        let extent = Rect::new(-1, -1, 2, 2);
        let roi = Rect::new(-1, 0, 0, 1);
        let raw = [2f32, 4., 6., 2., 4., 6., 2., 4., 6.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        for (abyss, expected) in [(SobelAbyss::None, 16f32), (SobelAbyss::Clamp, 8f32)] {
            let binding = make_binding(&graph, domain, extent, abyss);
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
            assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), expected.to_ne_bytes());
        }
        assert_eq!(source, before);
    }
    #[test]
    fn invalid_domains_and_budget_or_cancellation_never_change_source() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let extent = Rect::new(0, 0, 3, 3);
        let roi = Rect::new(1, 1, 2, 2);
        let source = Surface::new(format);
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let binding = make_binding(&graph, domain, extent, SobelAbyss::Clamp);
        let cancelled = || true;
        for (limits, ctl) in [
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
                    &ctl,
                    |_, direction, port| accepts_port(&binding, direction, port),
                    |request| footprint(&binding, request),
                    |request, budget| evaluate(&binding, request, budget)
                )
                .is_err()
            );
        }
        for encoding in [SampleEncoding::NonlinearProfile, SampleEncoding::Perceptual] {
            let wrong = Binding { domain: ExecutionDomain { encoding, ..domain }, ..make_binding(&graph, domain, extent, SobelAbyss::Clamp) };
            assert!(matches!(validate(&wrong, roi), Err(GraphExecuteError::Domain)));
        }
        let zero =
            Binding { domain: ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain }, ..make_binding(&graph, domain, extent, SobelAbyss::Clamp) };
        assert!(matches!(validate(&zero, roi), Err(GraphExecuteError::Domain)));
        assert_eq!(source, before);
    }
}
