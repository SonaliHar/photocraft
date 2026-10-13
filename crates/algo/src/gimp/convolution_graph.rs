//! Caller-bound arbitrary convolution graph callback; no serialized-source admission.
//! Mandatory owner validation binds the independently interpreted operation/schema,
//! coefficients/factor/bias/context/alpha/abyss. This bridge supplies no defaults.
use super::{
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    neighborhood_convolution::{self as convolution, Abyss, Params},
    roi::{self, Halo, RoiPlan},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
pub struct Binding<'a, 'data> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub domain: ExecutionDomain,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub input_extent: Rect,
    pub source_context: WorkingContext,
    pub expected_context: WorkingContext,
    pub params: Params<'data>,
    /// Required bounded/allocation-free validator of the complete parameter/schema
    /// binding; any external retained validation storage is prepaid by its owner.
    pub validate_arguments: &'a dyn Fn(&GraphNode) -> Result<(), GraphExecuteError>,
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn raster(e: RasterInputError) -> GraphExecuteError {
    if e == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(e) }
}
fn kernel(e: convolution::Error) -> GraphExecuteError {
    match e {
        convolution::Error::Raster(e) => raster(e),
        convolution::Error::ContextMismatch => GraphExecuteError::Domain,
        convolution::Error::Kernel => GraphExecuteError::Operation("invalid explicit convolution kernel"),
        convolution::Error::NonFinite => GraphExecuteError::Operation("nonfinite convolution arithmetic"),
        convolution::Error::AlphaRange => GraphExecuteError::Operation("convolution alpha outside unit range"),
    }
}
fn spatial_abyss(value: Abyss<'_>) -> roi::Abyss {
    match value {
        Abyss::Clamp => roi::Abyss::Clamp,
        Abyss::ConstantNative(_) => roi::Abyss::None,
    }
}
fn validate(binding: &Binding<'_, '_>, output: Rect) -> Result<Halo, GraphExecuteError> {
    if binding.node.0 == 0
        || binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || [binding.input_port, binding.upstream_output_port, binding.output_port].iter().any(|port| port.is_empty() || port.len() > 256)
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if binding.source_context != binding.expected_context
        || binding.domain.encoding != binding.expected_context.encoding
        || binding.domain.profile_fingerprint != Some(binding.expected_context.profile_identity)
    {
        return Err(GraphExecuteError::Domain);
    }
    if !matches!(binding.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(binding.domain.format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(GraphExecuteError::Domain);
    }
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(raster)?;
    checked_region_byte_len(output, binding.domain.format).map_err(raster)?;
    if binding.input_extent.is_empty() || output.is_empty() || !binding.input_extent.contains_rect(&output) {
        return Err(GraphExecuteError::Roi);
    }
    let k = binding.params.kernel;
    let w = usize::from(k.width);
    let h = usize::from(k.height);
    if w == 0 || h == 0 || w > 65 || h > 65 || w % 2 == 0 || h % 2 == 0 || k.coefficients.len() != w * h || !k.factor.is_finite() || !k.bias.is_finite() {
        return Err(GraphExecuteError::Operation("invalid explicit convolution shape"));
    }
    Ok(Halo { left: (w / 2) as u32, right: (w / 2) as u32, top: (h / 2) as u32, bottom: (h / 2) as u32 })
}
pub fn accepts_port(binding: &Binding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == binding.input_port,
        PortDirection::Output => port == binding.output_port,
    }
}
/// ConstantNative uses None only for spatial source-read intersection. The
/// actual constant bytes remain explicit in the evaluator, never treated as zero.
pub fn roi_footprint(binding: &Binding<'_, '_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if request.node != binding.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let halo = validate(binding, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    let taps = u64::from(binding.params.kernel.width).checked_mul(u64::from(binding.params.kernel.height)).ok_or(GraphRoiError::WorkLimit)?;
    let work = taps.checked_mul(3).and_then(|v| v.checked_add(48)).ok_or(GraphRoiError::WorkLimit)?;
    Ok(InputFootprint {
        mapped_output: request.output,
        source_extent: binding.input_extent,
        halo,
        abyss: spatial_abyss(binding.params.abyss),
        work_per_sample: work,
    })
}
fn tiles(r: Rect) -> Result<u64, GraphExecuteError> {
    let span = |a: i32, b: i32| u64::try_from(i64::from(b.div_euclid(TILE_SIZE)) - i64::from(a.div_euclid(TILE_SIZE)) + 1).map_err(|_| GraphExecuteError::Roi);
    mul(span(r.x0, r.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?, span(r.y0, r.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?)
}
/// Precharges the exact kernel envelope plus final ROI map adoption. The private
/// result retains source COW tiles; final output shares only tiles intersecting
/// its ROI, with original default/boundary bytes intact. No whole-ROI copy/scan,
/// pixel reinterpretation or upstream reservation refund is performed.
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let halo = validate(binding, request.roi)?;
    if request.node.id != binding.node
        || &request.node.operation != binding.identity
        || request.node.version_origin != binding.version_origin
        || request.node.arguments.len() > 64
    {
        return Err(GraphExecuteError::Operation("convolution requires exact caller operation/schema binding"));
    }
    if request.domain != binding.domain || request.inputs.len() != 1 {
        return Err(GraphExecuteError::Domain);
    }
    (binding.validate_arguments)(request.node)?;
    budget.poll()?;
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.input_port != binding.input_port || input.output_port != binding.upstream_output_port || input.domain != binding.domain || input.surface.format() != binding.domain.format {
        return Err(GraphExecuteError::Domain);
    }
    let expected = RoiPlan::new(request.roi, binding.input_extent, halo, spatial_abyss(binding.params.abyss), binding.domain.format, budget.interrupt())
        .map_err(raster)?;
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
    let req = convolution::requirements_with_interrupt(
        input.surface,
        binding.input_extent,
        request.roi,
        binding.source_context,
        binding.expected_context,
        binding.params,
        budget.interrupt(),
    )
    .map_err(kernel)?;
    if req.peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(GraphExecuteError::ResidentLimit);
    }
    if req.work_units > 1 << 30 {
        return Err(GraphExecuteError::WorkLimit);
    }
    let output_tiles = tiles(request.roi)?;
    let bpp = binding.domain.format.bytes_per_pixel() as u64;
    let payload = mul(65536, bpp)?;
    let output_charge = add(add(256, bpp)?, mul(output_tiles, add(payload, 256)?)?)?;
    let index_charge = add(req.output_index_bytes, mul(output_tiles, 256)?)?;
    let visits = add(input.surface.tile_count() as u64, output_tiles)?;
    let trim_work = mul(visits, 4)?;
    // Conservative simultaneous private result + final ROI owner charge. Shared
    // tiles are charged in both owners; no address-based memory refund is assumed.
    budget.reserve_output(output_charge)?;
    budget.reserve_index(index_charge)?;
    budget.reserve_scratch(req.peak_resident_bytes)?;
    budget.charge_work(req.work_units.checked_add(trim_work).ok_or(GraphExecuteError::WorkLimit)?)?;
    let filtered = convolution::try_apply(
        input.surface,
        binding.input_extent,
        request.roi,
        binding.source_context,
        binding.expected_context,
        binding.params,
        convolution::Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
        budget.interrupt(),
    )
    .map_err(kernel)?;
    budget.poll()?;
    let mut output = Surface::try_with_default_bytes(binding.domain.format, filtered.default_bytes()).map_err(raster)?;
    for (coordinate, tile) in filtered.tiles() {
        budget.poll()?;
        let x = i64::from(coordinate.tx) * i64::from(TILE_SIZE);
        let y = i64::from(coordinate.ty) * i64::from(TILE_SIZE);
        if x < i64::from(request.roi.x1)
            && x + i64::from(TILE_SIZE) > i64::from(request.roi.x0)
            && y < i64::from(request.roi.y1)
            && y + i64::from(TILE_SIZE) > i64::from(request.roi.y0)
        {
            output.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
    }
    drop(filtered);
    budget.poll()?;
    budget.finish_output(output, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline_with_roi};
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge},
    };
    use photocraft_raster::Interrupt;
    fn graph() -> FilterGraph {
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity {
                name: "own:explicit-convolution".into(),
                stored_version: "own".into(),
                schema_epoch: "own-clean-room-not-source".into(),
            },
            version_origin: Default::default(),
            arguments: vec![],
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: "input".into(), output_port: "output".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2)],
            edges: vec![edge(9, 1), edge(1, 2)],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }
    fn validator(node: &GraphNode) -> Result<(), GraphExecuteError> {
        if node.arguments.is_empty() { Ok(()) } else { Err(GraphExecuteError::Operation("own fixture expects no arguments")) }
    }
    fn binding<'a>(node: &'a GraphNode, domain: ExecutionDomain, extent: Rect, weights: &'a Vec<f64>) -> Binding<'a, 'a> {
        let context = WorkingContext { encoding: domain.encoding, profile_identity: [8; 32] };
        Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            domain,
            input_port: "input",
            upstream_output_port: "output",
            output_port: "output",
            input_extent: extent,
            source_context: context,
            expected_context: context,
            params: Params {
                kernel: convolution::Kernel { width: 3, height: 3, coefficients: weights, factor: 1.0, bias: 0.0 },
                alpha: convolution::Alpha::PreserveRaw,
                abyss: Abyss::Clamp,
            },
            validate_arguments: &validator,
        }
    }
    #[test]
    fn two_adaptive_passes_exact_dyadic_center_and_raw_alpha() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let extent = Rect::new(-2, -2, 3, 3);
        let default = [0.0f64.to_ne_bytes(), (-0.0f64).to_ne_bytes()].concat();
        let mut source = Surface::try_with_default_bytes(format, &default).unwrap();
        source.try_write_region_f64(Rect::new(0, 0, 1, 1), &[1.0, -0.0]).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([8; 32]) };
        let weights = vec![0.0, 0.0, 0.0, 0.25, 0.5, 0.25, 0.0, 0.0, 0.0];
        let output = execute_pipeline_with_roi(
            &graph,
            4,
            Some(&source),
            Rect::new(0, 0, 1, 1),
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output")),
            |request| {
                let node = graph.nodes.iter().find(|node| node.id == request.node).ok_or(GraphRoiError::UnsupportedFootprint)?;
                roi_footprint(&binding(node, domain, extent, &weights), request)
            },
            |request, budget| evaluate(&binding(request.node, domain, extent, &weights), request, budget),
        )
        .unwrap();
        let expected = [0.375f64.to_ne_bytes(), (-0.0f64).to_ne_bytes()].concat();
        assert_eq!(output.surface().try_read_interleaved(output.roi()).unwrap(), expected);
        assert_eq!(source, before);
    }
    #[test]
    fn missing_profile_wrong_schema_budget_and_cancel_keep_source() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let extent = Rect::new(0, 0, 1, 1);
        let source = Surface::try_with_default_bytes(format, &2.0f64.to_ne_bytes()).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([8; 32]) };
        let weights = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let mut wrong = binding(&graph.nodes[0], domain, extent, &weights);
        wrong.domain.profile_fingerprint = None;
        assert!(matches!(validate(&wrong, extent), Err(GraphExecuteError::Domain)));
        let result = execute_pipeline_with_roi(
            &graph,
            4,
            Some(&source),
            extent,
            domain,
            ExecutionLimits { work_units: 0, ..Default::default() },
            &Interrupt::NONE,
            |_, _, _| true,
            |request| {
                let node = graph.nodes.iter().find(|node| node.id == request.node).ok_or(GraphRoiError::UnsupportedFootprint)?;
                roi_footprint(&binding(node, domain, extent, &weights), request)
            },
            |request, budget| evaluate(&binding(request.node, domain, extent, &weights), request, budget),
        );
        assert!(result.is_err());
        let cancelled = || true;
        assert!(
            execute_pipeline_with_roi(
                &graph,
                4,
                Some(&source),
                extent,
                domain,
                ExecutionLimits::default(),
                &Interrupt::cancel_only(&cancelled),
                |_, _, _| true,
                |_| Err(GraphRoiError::Cancelled),
                |_, _| Err(GraphExecuteError::Cancelled)
            )
            .is_err()
        );
        let mut wrong_identity = graph.nodes[0].operation.clone();
        wrong_identity.schema_epoch = "different-caller-schema".into();
        let wrong_result = execute_pipeline_with_roi(
            &graph,
            4,
            Some(&source),
            extent,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, _, _| true,
            |request| {
                let node = graph.nodes.iter().find(|node| node.id == request.node).ok_or(GraphRoiError::UnsupportedFootprint)?;
                roi_footprint(&binding(node, domain, extent, &weights), request)
            },
            |request, budget| {
                let mut bound = binding(request.node, domain, extent, &weights);
                bound.identity = &wrong_identity;
                evaluate(&bound, request, budget)
            },
        );
        assert!(wrong_result.is_err());
        assert_eq!(source, before);
    }
}
