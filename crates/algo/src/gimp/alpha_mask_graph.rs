//! Caller-bound native alpha-mask graph callback with distinct input domains.
//! Exact identities, ports, versions, contexts and mathematical policies are
//! mandatory. This performs no CMS conversion or GEGL/source admission.
use super::{
    graph_execute::{AvailableInputBinding, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, InputPixels, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    multi_input_alpha_mask as kernel,
    roi::{Abyss, Halo, RoiPlan},
};
use photocraft_color::ColorMode;
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, checked_region_byte_len};

/// Caller mathematical schema; identity/origin are assertions, not GEGL source
/// evidence. Nonempty serialized arguments need a separately qualified resolver
/// and refuse here rather than being ignored or guessed.
pub struct Schema<'a> {
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub apply: kernel::Apply,
    pub outside: kernel::OutsideCoverage,
    pub rounding: kernel::Rounding,
    pub interpretation: kernel::CoverageInterpretation,
}
pub struct Port<'a> {
    pub input: &'a str,
    pub upstream_output: &'a str,
    pub endpoint: Endpoint,
    pub extent: Rect,
    pub domain: ExecutionDomain,
    pub context: kernel::WorkingContext,
}
pub struct Config<'a> {
    pub node: GraphNodeId,
    pub schema: Schema<'a>,
    pub source: Port<'a>,
    pub mask: Port<'a>,
    pub output_port: &'a str,
    pub output_domain: ExecutionDomain,
    pub output_context: kernel::WorkingContext,
    pub coordinate_space: [u8; 32],
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.as_bytes().contains(&0)
}
fn validate(config: &Config<'_>, roi: Rect) -> Result<(), GraphExecuteError> {
    if config.node.0 == 0
        || !text(config.schema.identity.name.as_str())
        || !text(config.schema.identity.stored_version.as_str())
        || !text(config.schema.identity.schema_epoch.as_str())
        || !text(config.source.input)
        || !text(config.mask.input)
        || !text(config.source.upstream_output)
        || !text(config.mask.upstream_output)
        || !text(config.output_port)
        || config.source.input == config.mask.input
        || config.source.endpoint == config.mask.endpoint
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let source = config.source.domain.format;
    let mask = config.mask.domain.format;
    if !matches!(source.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !source.alpha
        || mask.mode != ColorMode::Grayscale
        || mask.alpha
        || config.output_domain != config.source.domain
        || config.source.context != config.output_context
    {
        return Err(GraphExecuteError::Domain);
    }
    for (domain, context) in [(config.source.domain, config.source.context), (config.mask.domain, config.mask.context)] {
        if domain.encoding != context.encoding || domain.profile_fingerprint != Some(context.profile_identity) {
            return Err(GraphExecuteError::Domain);
        }
    }
    checked_region_byte_len(config.source.extent, source).map_err(GraphExecuteError::Raster)?;
    checked_region_byte_len(config.mask.extent, mask).map_err(GraphExecuteError::Raster)?;
    checked_region_byte_len(roi, source).map_err(GraphExecuteError::Raster)?;
    if roi.is_empty() || config.source.extent.is_empty() || !config.source.extent.contains_rect(&roi) {
        return Err(GraphExecuteError::Roi);
    }
    match config.schema.outside {
        kernel::OutsideCoverage::Reject if !config.mask.extent.contains_rect(&roi) => return Err(GraphExecuteError::Roi),
        kernel::OutsideCoverage::Constant { numerator, denominator } if denominator == 0 || numerator > denominator => {
            return Err(GraphExecuteError::Operation("invalid outside coverage rational"));
        }
        _ => {}
    }
    Ok(())
}
/// Exact structural port predicate for the caller's plan_graph callback.
pub fn port_allowed(config: &Config<'_>, node: GraphNodeId, direction: PortDirection, port: &str) -> bool {
    node == config.node
        && match direction {
            PortDirection::Input => port == config.source.input || port == config.mask.input,
            PortDirection::Output => port == config.output_port,
        }
}
fn expected_footprint(port: &Port<'_>, roi: Rect, control: &Interrupt<'_>) -> Result<RoiPlan, GraphExecuteError> {
    RoiPlan::new(roi, port.extent, Halo::default(), Abyss::None, port.domain.format, control).map_err(GraphExecuteError::Raster)
}
/// Exact identity/port footprint for the per-node-domain ROI planner. Each pad
/// is charged by that planner in its own actual upstream native domain.
pub fn alpha_mask_roi_footprint(config: &Config<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    validate(config, request.output).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    if request.node != config.node || request.edge.to != config.node {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let port = if request.edge.input_port == config.source.input && request.edge.output_port == config.source.upstream_output {
        &config.source
    } else if request.edge.input_port == config.mask.input && request.edge.output_port == config.mask.upstream_output {
        &config.mask
    } else {
        return Err(GraphRoiError::UnsupportedFootprint);
    };
    let endpoint_id = match port.endpoint {
        Endpoint::ExternalInput(id) | Endpoint::Node { id, .. } => id,
    };
    if request.edge.from != endpoint_id {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    Ok(InputFootprint { mapped_output: request.output, source_extent: port.extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 64 })
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn checked_add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn checked_mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn tiles(roi: Rect) -> Result<u64, GraphExecuteError> {
    let span = |first: i32, last: i32| -> Result<u64, GraphExecuteError> {
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| GraphExecuteError::Roi)
    };
    checked_mul(span(roi.x0, roi.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?, span(roi.y0, roi.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?)
}
struct TrimBudget {
    ceiling: u64,
    work: u64,
    used: u64,
}
impl StagingBudget for TrimBudget {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> Result<(), RasterInputError> {
        if bytes > self.ceiling { Err(RasterInputError::LimitExceeded) } else { Ok(()) }
    }
    fn work(&mut self, units: u64) -> Result<(), RasterInputError> {
        self.used = self.used.checked_add(units).filter(|value| *value <= self.work).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}

/// Actual NodeRequest callback: strict caller identity/version/origin and exact
/// ports/context/footprints are validated before any callback reservation or
/// allocation. No upstream ownership is added back to remaining budgets.
/// Per-node execution supplies actual source and independent mask domains.
/// The raw kernel colors survive, and only a privately trimmed ROI is published.
pub fn execute_alpha_mask(config: &Config<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    if request.inputs.len() != 2 {
        return Err(GraphExecuteError::Domain);
    }
    fn convert<'a>(input: &super::graph_execute::InputBinding<'a>) -> AvailableInputBinding<'a> {
        AvailableInputBinding {
            input_port: input.input_port,
            output_port: input.output_port,
            endpoint: input.endpoint,
            pixels: InputPixels::Materialized(input.surface),
            domain: input.domain,
            roi: input.roi,
            footprint: input.footprint,
        }
    }
    let inputs =
        [convert(request.inputs.first().ok_or(GraphExecuteError::MissingInput)?), convert(request.inputs.get(1).ok_or(GraphExecuteError::MissingInput)?)];
    execute_alpha_mask_available(config, AvailableNodeRequest { node: request.node, inputs: &inputs, roi: request.roi, domain: request.domain }, budget)
}
/// Explicit no-mask-pixels route: only a proven empty mask read region with an
/// exact caller outside rational may omit the mask owner. Source stays real.
pub fn execute_alpha_mask_available(
    config: &Config<'_>,
    request: AvailableNodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    validate(config, request.roi)?;
    if request.node.id != config.node
        || request.node.operation != *config.schema.identity
        || request.node.version_origin != config.schema.version_origin
        || !request.node.arguments.is_empty()
    {
        return Err(GraphExecuteError::Operation("alpha-mask schema or serialized arguments not bound"));
    }
    if request.domain != config.output_domain || request.inputs.len() != 2 {
        return Err(GraphExecuteError::Domain);
    }
    for port in [&config.source, &config.mask] {
        let input = request.inputs.iter().find(|input| input.input_port == port.input).ok_or(GraphExecuteError::MissingInput)?;
        if input.output_port != port.upstream_output
            || input.endpoint != port.endpoint
            || input.domain != port.domain
            || matches!(input.pixels, InputPixels::Materialized(surface) if surface.format() != port.domain.format)
        {
            return Err(GraphExecuteError::Domain);
        }
        let expected = expected_footprint(port, request.roi, budget.interrupt())?;
        if input.roi != expected.source_read() {
            return Err(GraphExecuteError::Roi);
        }
        if matches!(input.pixels, InputPixels::NoPixelsRequired)
            && (port.input != config.mask.input
                || !expected.source_read().is_empty()
                || input.footprint.is_none()
                || !matches!(config.schema.outside, kernel::OutsideCoverage::Constant { .. }))
        {
            return Err(GraphExecuteError::Roi);
        }
        if let Some(actual) = input.footprint {
            if actual.output() != expected.output()
                || actual.support() != expected.support()
                || actual.source_read() != expected.source_read()
                || actual.source_extent() != expected.source_extent()
                || actual.abyss() != expected.abyss()
            {
                return Err(GraphExecuteError::Roi);
            }
        } else if expected.source_read() != request.roi {
            return Err(GraphExecuteError::Roi);
        }
    }
    let source = request.inputs.iter().find(|input| input.input_port == config.source.input).ok_or(GraphExecuteError::MissingInput)?;
    let mask = request.inputs.iter().find(|input| input.input_port == config.mask.input).ok_or(GraphExecuteError::MissingInput)?;
    let InputPixels::Materialized(source_surface) = source.pixels else {
        return Err(GraphExecuteError::MissingInput);
    };
    let mask_surface = match mask.pixels {
        InputPixels::Materialized(surface) => Some(surface),
        InputPixels::NoPixelsRequired => None,
    };
    let native = kernel::AvailableRequest {
        source: kernel::Source {
            surface: source_surface,
            extent: config.source.extent,
            context: config.source.context,
            coordinate_space: config.coordinate_space,
        },
        mask: kernel::AvailableMask {
            surface: mask_surface,
            format: config.mask.domain.format,
            extent: config.mask.extent,
            coordinate_space: config.coordinate_space,
            interpretation: config.schema.interpretation,
        },
        roi: request.roi,
        output_context: config.output_context,
        apply: config.schema.apply,
        outside: config.schema.outside,
        rounding: config.schema.rounding,
    };
    let remaining = budget.remaining();
    let requirements = kernel::requirements_available(
        native,
        kernel::Limits { resident_bytes: remaining.resident_bytes.min(MAX_CHECKED_RASTER_BYTES), work_units: remaining.work_units },
        budget.interrupt(),
    )
    .map_err(raster)?;
    let format = request.domain.format;
    let bpp = format.bytes_per_pixel() as u64;
    let tile_payload = checked_mul(checked_mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let roi_tiles = tiles(request.roi)?;
    let output_bytes = checked_add(checked_add(256, bpp)?, checked_mul(roi_tiles, checked_add(tile_payload, 512)?)?)?;
    let index_bytes = checked_add(checked_mul(roi_tiles, 128)?, 4096)?;
    let cell_bytes = checked_mul(64 * 64, bpp)?;
    let scratch = checked_add(requirements.peak_bytes, checked_add(cell_bytes, 4096)?)?;
    let trim_work = checked_add(
        checked_mul(checked_region_byte_len(request.roi, format).map_err(raster)? as u64, 16)?,
        checked_add(checked_mul(roi_tiles, checked_mul(tile_payload, 8)?)?, checked_mul(roi_tiles, 1024)?)?,
    )?;
    // Original/live upstream reservations stay deducted. Private retained COW,
    // output/index/cell owners are additional conservative reservations.
    budget.reserve_output(output_bytes)?;
    budget.reserve_index(index_bytes)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(checked_add(checked_mul(requirements.work_units, 2)?, trim_work)?)?;
    let filtered = kernel::try_apply_available(
        native,
        kernel::Limits { resident_bytes: requirements.peak_bytes, work_units: requirements.work_units },
        budget.interrupt(),
    )
    .map_err(raster)?;
    let mut staging = TrimBudget { ceiling: checked_add(output_bytes, index_bytes)?, work: trim_work, used: 0 };
    let mut builder = StagedSurfaceBuilder::new(format, filtered.default_bytes(), request.roi, &mut staging, budget.interrupt()).map_err(raster)?;
    let mut cell = Vec::new();
    let capacity = usize::try_from(cell_bytes).map_err(|_| GraphExecuteError::ResidentLimit)?;
    cell.try_reserve_exact(capacity).map_err(|_| GraphExecuteError::Allocation)?;
    if cell.capacity() != capacity {
        return Err(GraphExecuteError::Allocation);
    }
    while cell.len() < capacity {
        budget.poll()?;
        cell.resize(cell.len().saturating_add(8192).min(capacity), 0);
    }
    let mut y = request.roi.y0;
    while y < request.roi.y1 {
        budget.poll()?;
        let next_y = y.checked_add(64).ok_or(GraphExecuteError::Roi)?.min(request.roi.y1);
        let mut x = request.roi.x0;
        while x < request.roi.x1 {
            budget.poll()?;
            let next_x = x.checked_add(64).ok_or(GraphExecuteError::Roi)?.min(request.roi.x1);
            let area = Rect::new(x, y, next_x, next_y);
            let length = checked_region_byte_len(area, format).map_err(raster)?;
            let raw = cell.get_mut(..length).ok_or(GraphExecuteError::ResidentLimit)?;
            filtered.try_read_interleaved_into_with(area, raw, budget.interrupt()).map_err(raster)?;
            builder.push_region(area, raw, &mut staging, budget.interrupt()).map_err(raster)?;
            x = next_x;
        }
        y = next_y;
    }
    let (trimmed, ()) = builder.finalize(&mut staging, budget.interrupt()).map_err(raster)?.into_parts();
    drop(filtered);
    drop(cell);
    budget.poll()?;
    budget.finish_output(trimmed, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{PixelFormat, SampleType};
    use photocraft_doc::{SampleEncoding, xcf_filters::GraphEdge};
    #[test]
    fn availability_pipeline_skips_mask_producer_and_keeps_exact_rational_alpha() {
        use super::super::{
            graph_execute::{DomainAssignment, ExecutionLimits},
            graph_pipeline::execute_pipeline_with_roi_domains_available,
        };
        use photocraft_doc::xcf_filters::{FilterGraph, GraphNode};
        use photocraft_raster::Surface;
        for sample in [SampleType::U32, SampleType::F64] {
            let context = kernel::WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [6; 32] };
            let domain = ExecutionDomain {
                format: PixelFormat::new(ColorMode::Rgb, sample, true),
                encoding: context.encoding,
                profile_fingerprint: Some(context.profile_identity),
            };
            let mask_domain = ExecutionDomain { format: PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false), ..domain };
            let raw = if sample == SampleType::U32 {
                [1u32, 17, u32::MAX, 3].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
            } else {
                [0x7ff8_0000_0000_1234u64, (-0.0f64).to_bits(), (-3.5f64).to_bits(), 0.75f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect()
            };
            let source = Surface::try_with_default_bytes(domain.format, &raw).unwrap();
            let before = source.clone();
            let identity = OperationIdentity { name: "Own absent mask".into(), stored_version: "1".into(), schema_epoch: "own".into() };
            let node = |id| GraphNode { id: GraphNodeId(id), operation: identity.clone(), version_origin: GraphVersionOrigin::Stored, arguments: vec![] };
            let graph = FilterGraph {
                input: Some(GraphNodeId(9)),
                nodes: vec![node(1), node(2)],
                edges: vec![
                    GraphEdge { from: GraphNodeId(9), to: GraphNodeId(2), input_port: "source".into(), output_port: "color".into() },
                    GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), input_port: "mask".into(), output_port: "coverage".into() },
                ],
                output: GraphNodeId(2),
                original_pipeline: None,
            };
            let roi = Rect::new(-1, 0, 0, 1);
            let config = Config {
                node: GraphNodeId(2),
                schema: Schema {
                    identity: &identity,
                    version_origin: GraphVersionOrigin::Stored,
                    apply: kernel::Apply::Multiply,
                    outside: kernel::OutsideCoverage::Constant { numerator: 1, denominator: 2 },
                    rounding: kernel::Rounding::NearestTiesUp,
                    interpretation: kernel::CoverageInterpretation::UnitSamples,
                },
                source: Port { input: "source", upstream_output: "color", endpoint: Endpoint::ExternalInput(GraphNodeId(9)), extent: roi, domain, context },
                mask: Port {
                    input: "mask",
                    upstream_output: "coverage",
                    endpoint: Endpoint::Node { id: GraphNodeId(1), index: 0 },
                    extent: Rect::new(1, 0, 2, 1),
                    domain: mask_domain,
                    context,
                },
                output_port: "result",
                output_domain: domain,
                output_context: context,
                coordinate_space: [7; 32],
            };
            let domains = [DomainAssignment { node: GraphNodeId(1), domain: mask_domain }, DomainAssignment { node: GraphNodeId(2), domain }];
            let output = execute_pipeline_with_roi_domains_available(
                &graph,
                2,
                Some(&source),
                roi,
                domain,
                &domains,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, _, _| true,
                |request| alpha_mask_roi_footprint(&config, request),
                |request, budget| {
                    assert_eq!(request.node.id, GraphNodeId(2), "no-read mask producer must never execute");
                    execute_alpha_mask_available(&config, request, budget)
                },
            )
            .unwrap();
            let pixel = output.surface().try_read_interleaved(roi).unwrap();
            let color_bytes = 3 * sample.bytes();
            assert_eq!(&pixel[..color_bytes], &raw[..color_bytes]);
            if sample == SampleType::U32 {
                assert_eq!(u32::from_ne_bytes(pixel[color_bytes..].try_into().unwrap()), 2);
            } else {
                assert_eq!(photocraft_color::try_read_sample_f64(&pixel, sample, 3).unwrap(), 0.375);
            }
            assert_eq!(output.surface().default_bytes(), raw);
            assert_eq!(source, before);
        }
    }
    #[test]
    fn typed_schema_keeps_independent_mask_domain_footprint_explicit() {
        let identity = OperationIdentity { name: "Own alpha-mask arithmetic".into(), stored_version: "own-v1".into(), schema_epoch: "own-not-gegl".into() };
        let context = kernel::WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let source = ExecutionDomain {
            format: PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
            encoding: context.encoding,
            profile_fingerprint: Some(context.profile_identity),
        };
        let mask = ExecutionDomain {
            format: PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false),
            encoding: context.encoding,
            profile_fingerprint: Some(context.profile_identity),
        };
        let extent = Rect::new(0, 0, 1, 1);
        let config = Config {
            node: GraphNodeId(3),
            schema: Schema {
                identity: &identity,
                version_origin: GraphVersionOrigin::Stored,
                apply: kernel::Apply::Multiply,
                outside: kernel::OutsideCoverage::Reject,
                rounding: kernel::Rounding::NearestTiesUp,
                interpretation: kernel::CoverageInterpretation::UnitSamples,
            },
            source: Port {
                input: "owned-source",
                upstream_output: "owned-color",
                endpoint: Endpoint::ExternalInput(GraphNodeId(1)),
                extent,
                domain: source,
                context,
            },
            mask: Port {
                input: "owned-mask",
                upstream_output: "owned-coverage",
                endpoint: Endpoint::Node { id: GraphNodeId(2), index: 0 },
                extent,
                domain: mask,
                context,
            },
            output_port: "owned-result",
            output_domain: source,
            output_context: context,
            coordinate_space: [2; 32],
        };
        assert!(validate(&config, extent).is_ok());
        assert!(port_allowed(&config, GraphNodeId(3), PortDirection::Input, "owned-source"));
        assert!(!port_allowed(&config, GraphNodeId(3), PortDirection::Input, "input"));
        let edge = GraphEdge { from: GraphNodeId(2), to: GraphNodeId(3), input_port: "owned-mask".into(), output_port: "owned-coverage".into() };
        assert!(
            alpha_mask_roi_footprint(&config, FootprintRequest { node: GraphNodeId(3), declared_index: 0, edge_index: 0, edge: &edge, output: extent }).is_ok()
        );
    }
    #[test]
    fn per_node_pipeline_executes_native_alpha_with_independent_mask_depth() {
        use super::super::{
            graph_execute::{DomainAssignment, ExecutionLimits},
            graph_pipeline::execute_pipeline_with_roi_domains,
        };
        use photocraft_doc::xcf_filters::{FilterGraph, GraphNode};
        use photocraft_raster::Surface;
        let source_context = kernel::WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [1; 32] };
        let mask_context = kernel::WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [2; 32] };
        let source_domain =
            ExecutionDomain { format: PixelFormat::RGBA64F, encoding: source_context.encoding, profile_fingerprint: Some(source_context.profile_identity) };
        let mask_domain = ExecutionDomain {
            format: PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false),
            encoding: mask_context.encoding,
            profile_fingerprint: Some(mask_context.profile_identity),
        };
        let color_words = [0x7ff8_0000_0000_1234u64, (-0.0f64).to_bits(), (-3.5f64).to_bits(), 0.75f64.to_bits()];
        let raw: Vec<u8> = color_words.into_iter().flat_map(u64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(source_domain.format, &raw).unwrap();
        let before = source.clone();
        let alpha_identity = OperationIdentity { name: "Own alpha-mask".into(), stored_version: "own-v1".into(), schema_epoch: "own-not-gegl".into() };
        let generator_identity =
            OperationIdentity { name: "Own unit mask default".into(), stored_version: "own-v1".into(), schema_epoch: "own-not-gegl".into() };
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![
                GraphNode { id: GraphNodeId(1), operation: generator_identity, version_origin: GraphVersionOrigin::Stored, arguments: vec![] },
                GraphNode { id: GraphNodeId(2), operation: alpha_identity.clone(), version_origin: GraphVersionOrigin::Stored, arguments: vec![] },
            ],
            edges: vec![
                GraphEdge { from: GraphNodeId(9), to: GraphNodeId(2), input_port: "source".into(), output_port: "color".into() },
                GraphEdge { from: GraphNodeId(1), to: GraphNodeId(2), input_port: "mask".into(), output_port: "coverage".into() },
            ],
            output: GraphNodeId(2),
            original_pipeline: None,
        };
        let roi = Rect::new(-1, 0, 1, 1);
        let config = Config {
            node: GraphNodeId(2),
            schema: Schema {
                identity: &alpha_identity,
                version_origin: GraphVersionOrigin::Stored,
                apply: kernel::Apply::Multiply,
                outside: kernel::OutsideCoverage::Constant { numerator: 1, denominator: 2 },
                rounding: kernel::Rounding::NearestTiesUp,
                interpretation: kernel::CoverageInterpretation::UnitSamples,
            },
            source: Port {
                input: "source",
                upstream_output: "color",
                endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
                extent: roi,
                domain: source_domain,
                context: source_context,
            },
            mask: Port {
                input: "mask",
                upstream_output: "coverage",
                endpoint: Endpoint::Node { id: GraphNodeId(1), index: 0 },
                extent: Rect::new(0, 0, 1, 1),
                domain: mask_domain,
                context: mask_context,
            },
            output_port: "result",
            output_domain: source_domain,
            output_context: source_context,
            coordinate_space: [4; 32],
        };
        let domains = [DomainAssignment { node: GraphNodeId(1), domain: mask_domain }, DomainAssignment { node: GraphNodeId(2), domain: source_domain }];
        let output = execute_pipeline_with_roi_domains(
            &graph,
            2,
            Some(&source),
            roi,
            source_domain,
            &domains,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |id, direction, port| {
                if id == GraphNodeId(2) {
                    port_allowed(&config, id, direction, port)
                } else {
                    matches!((id, direction, port), (GraphNodeId(9), PortDirection::Output, "color") | (GraphNodeId(1), PortDirection::Output, "coverage"))
                }
            },
            |request| alpha_mask_roi_footprint(&config, request),
            |request, budget| {
                if request.node.id == GraphNodeId(1) {
                    assert_eq!(request.domain, mask_domain);
                    assert!(request.inputs.is_empty());
                    assert_eq!(request.roi, Rect::new(0, 0, 1, 1));
                    budget.reserve_output(258)?;
                    let mask = Surface::try_with_default_bytes(mask_domain.format, &32768u16.to_ne_bytes()).map_err(GraphExecuteError::Raster)?;
                    budget.finish_output(mask, request.roi, mask_domain)
                } else {
                    assert_eq!(request.inputs[0].domain, source_domain);
                    assert_eq!(request.inputs[1].domain, mask_domain);
                    execute_alpha_mask(&config, request, budget)
                }
            },
        )
        .unwrap();
        assert_eq!(output.domain(), source_domain);
        assert_eq!(output.roi(), roi);
        let outside = output.surface().try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap();
        let inside = output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(outside.get(..24).unwrap(), raw.get(..24).unwrap());
        assert_eq!(inside.get(..24).unwrap(), raw.get(..24).unwrap());
        assert_eq!(photocraft_color::try_read_sample_f64(&outside, SampleType::F64, 3).unwrap(), 0.375);
        assert_eq!(photocraft_color::try_read_sample_f64(&inside, SampleType::F64, 3).unwrap(), 0.75 * (32768.0 / 65535.0));
        assert_eq!(output.surface().default_bytes(), raw);
        assert_eq!(source, before);
    }
}
