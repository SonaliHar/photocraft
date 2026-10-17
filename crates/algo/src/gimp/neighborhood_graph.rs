//! Caller-bound mathematical convolution adapter; no GEGL schema/source admission.
//! Exact weights, working context, alpha and abyss are explicit inputs. Kernel
//! source retention is privately trimmed before publication into an ROI graph.
use super::{
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    neighborhood_gaussian::{self as gaussian, Abyss, Alpha, Weights},
    roi::{self, Halo, RoiPlan},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, checked_region_byte_len};

pub struct GaussianConfig<'a, 'w> {
    pub node: GraphNodeId,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub input_extent: Rect,
    pub domain: ExecutionDomain,
    pub source_context: WorkingContext,
    pub expected_context: WorkingContext,
    pub weights: &'a Weights<'w>,
    pub abyss: Abyss,
    pub alpha: Alpha,
}
/// Optional explicit operation binding for callers interpreting a source schema.
/// The old GaussianConfig remains a standalone caller-supplied mathematical route.
pub struct GaussianBinding<'a, 'config, 'weights> {
    pub config: &'a GaussianConfig<'config, 'weights>,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub output_port: &'a str,
    /// Mandatory exact arguments/weights/abyss/alpha/context validator. It must
    /// be bounded/allocation-free, or its storage prepaid by the enclosing caller.
    pub validate_arguments: &'a dyn Fn(&GraphNode) -> Result<(), GraphExecuteError>,
}
fn raster(error: photocraft_raster::RasterInputError) -> GraphExecuteError {
    if error == photocraft_raster::RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn abyss(value: Abyss) -> roi::Abyss {
    match value {
        Abyss::None => roi::Abyss::None,
        Abyss::Clamp => roi::Abyss::Clamp,
    }
}
fn kernel_error(error: gaussian::Error) -> GraphExecuteError {
    match error {
        gaussian::Error::Raster(error) => raster(error),
        gaussian::Error::ContextMismatch => GraphExecuteError::Domain,
        gaussian::Error::Weights => GraphExecuteError::Operation("unsupported convolution weights"),
        gaussian::Error::NonFinite => GraphExecuteError::Operation("nonfinite convolution arithmetic"),
        gaussian::Error::AlphaRange => GraphExecuteError::Operation("convolution alpha outside unit range"),
    }
}
fn validate(config: &GaussianConfig<'_, '_>, roi: Rect) -> Result<Halo, GraphExecuteError> {
    if config.node.0 == 0
        || config.input_port.is_empty()
        || config.upstream_output_port.is_empty()
        || config.input_port.len() > 1 << 20
        || config.upstream_output_port.len() > 1 << 20
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if config.source_context != config.expected_context
        || config.domain.encoding != config.expected_context.encoding
        || config.domain.profile_fingerprint != Some(config.expected_context.profile_identity)
    {
        return Err(GraphExecuteError::Domain);
    }
    if !matches!(config.domain.format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(config.domain.format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(GraphExecuteError::Domain);
    }
    checked_region_byte_len(config.input_extent, config.domain.format).map_err(GraphExecuteError::Raster)?;
    checked_region_byte_len(roi, config.domain.format).map_err(GraphExecuteError::Raster)?;
    if config.input_extent.is_empty() || roi.is_empty() || roi.intersect(&config.input_extent) != roi {
        return Err(GraphExecuteError::Roi);
    }
    let x = u32::try_from(config.weights.horizontal().len() / 2).map_err(|_| GraphExecuteError::Roi)?;
    let y = u32::try_from(config.weights.vertical().len() / 2).map_err(|_| GraphExecuteError::Roi)?;
    Ok(Halo { left: x, right: x, top: y, bottom: y })
}
/// Exact halo/read footprint for one bound input port. None/Clamp semantics use
/// existing RoiPlan; no serialized abyss enum or missing-profile default is guessed.
pub fn gaussian_roi_footprint(config: &GaussianConfig<'_, '_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if request.node != config.node
        || request.edge.to != config.node
        || request.edge.input_port != config.input_port
        || request.edge.output_port != config.upstream_output_port
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let halo = validate(config, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    let work = config
        .weights
        .horizontal()
        .len()
        .checked_add(config.weights.vertical().len())
        .and_then(|work| u64::try_from(work).ok())
        .ok_or(GraphRoiError::WorkLimit)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: config.input_extent, halo, abyss: abyss(config.abyss), work_per_sample: work })
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    if rect.is_empty() {
        return Ok(0);
    }
    let span = |first: i32, last: i32| -> Result<u64, GraphExecuteError> {
        let span = i64::from(last.div_euclid(TILE_SIZE))
            .checked_sub(i64::from(first.div_euclid(TILE_SIZE)))
            .and_then(|span| span.checked_add(1))
            .ok_or(GraphExecuteError::Roi)?;
        u64::try_from(span).map_err(|_| GraphExecuteError::Roi)
    };
    mul(span(rect.x0, rect.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?, span(rect.y0, rect.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?)
}
/// Legacy caller-supplied mathematical route, not operation/schema admission.
/// Operation arguments are not interpreted here; source-schema callers must use
/// execute_gaussian_bound. Explicit matching upstream domain/profile is required.
/// Execute only this explicitly bound mathematical node. The source producer
/// covers the scheduler's exact halo read; original alpha can remain raw only
/// under PreserveRaw. No premultiplication, TRC or storage-wide narrowing occurs.
pub fn execute_gaussian(config: &GaussianConfig<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let halo = validate(config, request.roi)?;
    if request.node.id != config.node || request.domain != config.domain || request.inputs.len() != 1 {
        return Err(GraphExecuteError::Domain);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.domain != config.domain
        || input.input_port != config.input_port
        || input.output_port != config.upstream_output_port
        || input.surface.format() != config.domain.format
    {
        return Err(GraphExecuteError::Domain);
    }
    let expected = RoiPlan::new(request.roi, config.input_extent, halo, abyss(config.abyss), config.domain.format, budget.interrupt())
        .map_err(GraphExecuteError::Raster)?;
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
    let requirements = gaussian::requirements_with_interrupt(
        input.surface,
        config.input_extent,
        request.roi,
        config.source_context,
        config.expected_context,
        config.weights,
        config.abyss,
        config.alpha,
        budget.interrupt(),
    )
    .map_err(kernel_error)?;
    let format = config.domain.format;
    let bpp = format.bytes_per_pixel() as u64;
    let payload = mul(65536, bpp)?;
    let output_tiles = tiles(request.roi)?;
    let retained_tiles = add(input.surface.tile_count() as u64, output_tiles)?;
    let tile_charge = add(payload, 256)?;
    let filtered_charge = add(add(256, bpp)?, mul(retained_tiles, tile_charge)?)?;
    let output_charge = add(add(256, bpp)?, mul(output_tiles, tile_charge)?)?;
    let copied = checked_region_byte_len(request.roi, format).map_err(raster)? as u64;
    let trim_peak = add(add(filtered_charge, copied)?, add(mul(output_tiles, tile_charge)?, mul(bpp, 2)?)?)?;
    if requirements.peak_resident_bytes > MAX_CHECKED_RASTER_BYTES || trim_peak > MAX_CHECKED_RASTER_BYTES {
        return Err(GraphExecuteError::ResidentLimit);
    }
    let trim_work = add(add(mul(copied, 2)?, mul(output_tiles, payload)?)?, add(mul(retained_tiles, 4)?, output_tiles)?)?;
    // The source stays charged by the executor. Conservatively retain both the
    // kernel envelope and private filtered owner through final raw crop adoption;
    // shared tile payloads are never refunded or treated as unowned memory.
    let scratch_charge = add(add(requirements.peak_resident_bytes, filtered_charge)?, add(mul(copied, 2)?, mul(bpp, 4)?)?)?;
    let index_charge = add(requirements.output_index_bytes, mul(output_tiles, 256)?)?;
    budget.reserve_output(output_charge)?;
    budget.reserve_index(index_charge)?;
    budget.reserve_scratch(scratch_charge)?;
    budget.charge_work(add(requirements.work_units, trim_work)?)?;
    let filtered = gaussian::try_apply(
        input.surface,
        config.input_extent,
        request.roi,
        config.source_context,
        config.expected_context,
        config.weights,
        config.abyss,
        config.alpha,
        gaussian::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
        budget.interrupt(),
    )
    .map_err(kernel_error)?;
    // try_apply intentionally retains outside source tiles. Graph outputs must
    // discard unrelated tiles, retaining exact default/boundary raw bytes.
    let edges = [f64::from(request.roi.x0), f64::from(request.roi.y0), f64::from(request.roi.x1), f64::from(request.roi.y1)];
    let trimmed = try_crop_raw(
        &filtered,
        config.input_extent,
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
    budget.finish_output(trimmed.surface, request.roi, request.domain)
}

/// Explicit source-schema usage must supply this binding; no identity/schema,
/// version origin, output port or argument validator is inferred from weights.
pub fn execute_gaussian_bound(
    binding: &GaussianBinding<'_, '_, '_>,
    request: NodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || binding.output_port.is_empty()
        || binding.output_port.len() > 256
        || binding.identity != &request.node.operation
        || binding.version_origin != request.node.version_origin
        || request.node.arguments.len() > 64
    {
        return Err(GraphExecuteError::Operation("Gaussian requires exact caller operation/schema/ports"));
    }
    if request.node.id != binding.config.node || request.domain != binding.config.domain {
        return Err(GraphExecuteError::Domain);
    }
    validate(binding.config, request.roi)?;
    (binding.validate_arguments)(request.node)?;
    budget.poll()?;
    execute_gaussian(binding.config, request, budget)
}
pub fn bound_accepts_port(binding: &GaussianBinding<'_, '_, '_>, direction: super::graph_plan::PortDirection, port: &str) -> bool {
    match direction {
        super::graph_plan::PortDirection::Input => port == binding.config.input_port,
        super::graph_plan::PortDirection::Output => port == binding.output_port,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        graph_execute::ExecutionLimits,
        graph_pipeline::{GraphPipelineError, execute_pipeline_with_roi},
    };
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        sample_encoding::SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge, GraphNode, OperationIdentity},
    };
    use photocraft_raster::{Interrupt, Surface};
    fn graph() -> FilterGraph {
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "Own separable convolution".into(), stored_version: "own".into(), schema_epoch: "own-not-gegl".into() },
            version_origin: Default::default(),
            arguments: Vec::new(),
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: "own-input".into(), output_port: "own-output".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2)],
            edges: vec![edge(9, 1), edge(1, 2)],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }
    fn config<'a, 'w>(id: GraphNodeId, domain: ExecutionDomain, extent: Rect, weights: &'a Weights<'w>) -> GaussianConfig<'a, 'w> {
        let context = WorkingContext { encoding: domain.encoding, profile_identity: [1; 32] };
        GaussianConfig {
            node: id,
            input_port: "own-input",
            upstream_output_port: "own-output",
            input_extent: extent,
            domain,
            source_context: context,
            expected_context: context,
            weights,
            abyss: Abyss::None,
            alpha: Alpha::PreserveRaw,
        }
    }
    #[test]
    fn two_adaptive_passes_propagate_halo_and_produce_exact_dyadic_center() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let extent = Rect::new(0, 0, 5, 5);
        let mut source = Surface::new(format);
        source.try_write_region_f64(Rect::new(2, 2, 3, 3), &[1.0]).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([1; 32]) };
        let taps = [0.25, 0.5, 0.25];
        let weights = Weights::new(&taps, &taps).unwrap();
        let mut visited = Vec::new();
        let output = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&source),
            Rect::new(2, 2, 3, 3),
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, _, _| true,
            |request| gaussian_roi_footprint(&config(request.node, domain, extent, &weights), request),
            |request, budget| {
                visited.push((request.node.id, request.roi, request.inputs.first().unwrap().roi));
                execute_gaussian(&config(request.node.id, domain, extent, &weights), request, budget)
            },
        )
        .unwrap();
        assert_eq!(visited, vec![(GraphNodeId(1), Rect::new(1, 1, 4, 4), extent), (GraphNodeId(2), Rect::new(2, 2, 3, 3), Rect::new(1, 1, 4, 4))]);
        assert_eq!(output.surface().try_read_region_f64(output.roi()).unwrap(), vec![0.140625]);
        assert_eq!(source, before);
    }
    #[test]
    fn explicit_clamp_raw_alpha_profile_wide_refusal_and_cancel() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut graph = graph();
        graph.nodes.truncate(1);
        graph.edges.truncate(1);
        graph.output = GraphNodeId(1);
        let extent = Rect::new(-1, 0, 0, 1);
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: true };
        let raw = [2.0_f32.to_ne_bytes(), 0x7fc0_1234u32.to_ne_bytes()].concat();
        let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([1; 32]) };
        let taps = [0.25, 0.5, 0.25];
        let weights = Weights::new(&taps, &taps).unwrap();
        let mut config = config(GraphNodeId(1), domain, extent, &weights);
        config.abyss = Abyss::Clamp;
        let output = execute_pipeline_with_roi(
            &graph,
            1,
            Some(&source),
            extent,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, _, _| true,
            |request| gaussian_roi_footprint(&config, request),
            |request, budget| execute_gaussian(&config, request, budget),
        )
        .unwrap();
        assert_eq!(output.surface().try_read_interleaved(extent).unwrap(), raw);
        config.expected_context.profile_identity = [2; 32];
        assert!(validate(&config, extent).is_err());
        config.expected_context.profile_identity = [1; 32];
        config.domain.format.sample = SampleType::U32;
        assert!(validate(&config, extent).is_err());
        config.domain = domain;
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        let result = execute_pipeline_with_roi(
            &graph,
            1,
            Some(&source),
            extent,
            domain,
            ExecutionLimits::default(),
            &Interrupt::cancel_only(&check),
            |_, _, _| true,
            |request| gaussian_roi_footprint(&config, request),
            |request, budget| {
                let output = execute_gaussian(&config, request, budget)?;
                cancelled.store(true, Ordering::Relaxed);
                Ok(output)
            },
        );
        assert!(matches!(result, Err(GraphPipelineError::Execute(GraphExecuteError::Cancelled))));
        assert_eq!(source, before);
    }
}

#[cfg(test)]
mod bound_tests {
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline_with_roi};
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge, GraphNode},
    };
    use photocraft_raster::{Interrupt, Surface};
    fn arguments(node: &GraphNode) -> Result<(), GraphExecuteError> {
        if node.arguments.is_empty() { Ok(()) } else { Err(GraphExecuteError::Operation("own fixture takes no arguments")) }
    }
    #[test]
    fn explicit_bound_identity_profile_and_raw_alpha_are_not_inferred() {
        let node = GraphNode {
            id: GraphNodeId(1),
            operation: OperationIdentity { name: "own:separable".into(), stored_version: "own".into(), schema_epoch: "own-caller-not-source".into() },
            version_origin: Default::default(),
            arguments: vec![],
        };
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let extent = Rect::new(-1, 0, 0, 1);
        let raw = [2.0f64.to_ne_bytes(), 0x7ff8000000000012u64.to_ne_bytes()].concat();
        let source = Surface::try_with_default_bytes(format, &raw).unwrap();
        let before = source.clone();
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [2; 32] };
        let domain = ExecutionDomain { format, encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) };
        let taps = [1.0];
        let weights = Weights::new(&taps, &taps).unwrap();
        let config = GaussianConfig {
            node: GraphNodeId(1),
            input_port: "input",
            upstream_output_port: "output",
            input_extent: extent,
            domain,
            source_context: context,
            expected_context: context,
            weights: &weights,
            abyss: Abyss::Clamp,
            alpha: Alpha::PreserveRaw,
        };
        let bound = GaussianBinding {
            config: &config,
            identity: &graph.nodes[0].operation,
            version_origin: graph.nodes[0].version_origin,
            output_port: "output",
            validate_arguments: &arguments,
        };
        let output = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&source),
            extent,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| bound_accepts_port(&bound, direction, port),
            |request| gaussian_roi_footprint(&config, request),
            |request, budget| execute_gaussian_bound(&bound, request, budget),
        )
        .unwrap();
        assert_eq!(output.surface().try_read_interleaved(extent).unwrap(), raw);
        let mut wrong_identity = graph.nodes[0].operation.clone();
        wrong_identity.schema_epoch = "different-exact-schema".into();
        let wrong = GaussianBinding { identity: &wrong_identity, ..bound };
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&source),
                extent,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, _, _| true,
                |request| gaussian_roi_footprint(&config, request),
                |request, budget| execute_gaussian_bound(&wrong, request, budget)
            )
            .is_err()
        );
        let mut missing = config;
        missing.domain.profile_fingerprint = None;
        assert!(matches!(validate(&missing, extent), Err(GraphExecuteError::Domain)));
        assert_eq!(source, before);
    }
}
