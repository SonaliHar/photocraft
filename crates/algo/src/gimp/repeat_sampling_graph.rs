//! Caller-bound raw repeat/mirror graph evaluation with conservative full-input
//! ROI propagation. No coordinate, CMS, source version or GEGL schema inference.
use super::{
    geometry_nearest::Limits,
    geometry_tile_repeat::{self as kernel, Axis, Params},
    graph_execute::{ExecutionDomain, GraphExecuteError as Error, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, checked_region_byte_len};

/// Trusted Rust validator must bind exact native default, periods, origin and
/// axis policies without allocations/unbounded work. This is not a sandbox.
pub type ArgumentValidator = fn(&[FilterArgument], Rect, Params) -> Result<(), Error>;
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub source_arguments: &'a [FilterArgument],
    pub validate_arguments: ArgumentValidator,
    pub endpoint: Endpoint,
    pub input_extent: Rect,
    pub domain: ExecutionDomain,
    pub params: Params,
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == "input",
        PortDirection::Output => port == "output",
    }
}
fn map_error(error: kernel::Error) -> Error {
    match error {
        kernel::Error::Raster(RasterInputError::Cancelled) => Error::Cancelled,
        kernel::Error::Raster(error) => Error::Raster(error),
        kernel::Error::View(_) => Error::Operation("repeat source view refused"),
    }
}
fn endpoint_id(endpoint: Endpoint) -> Option<GraphNodeId> {
    match endpoint {
        Endpoint::ExternalInput(id) if id.0 != 0 => Some(id),
        Endpoint::Node { id, index } if id.0 != 0 && index < 4096 => Some(id),
        _ => None,
    }
}
fn identity_bounded(identity: &OperationIdentity) -> bool {
    [(identity.name.as_str(), 256), (identity.stored_version.as_str(), 64), (identity.schema_epoch.as_str(), 256)]
        .into_iter()
        .all(|(value, cap)| !value.is_empty() && value.len() <= cap && !value.contains('\0'))
}
fn bounded_arguments(arguments: &[FilterArgument]) -> Result<u64, Error> {
    if arguments.len() > 16 {
        return Err(Error::Operation("repeat argument count limit"));
    }
    let mut bytes = 0u64;
    for (index, argument) in arguments.iter().enumerate() {
        if argument.name.is_empty()
            || argument.name.len() > 128
            || argument.name.contains('\0')
            || arguments.iter().take(index).any(|old| old.name == argument.name)
        {
            return Err(Error::Operation("repeat requires unique bounded argument names"));
        }
        let value_bytes = match &argument.value {
            FilterValue::Number(ExactNumber::Signed { width: 32, value }) if i32::try_from(*value).is_ok() => 32,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 256 && !domain.contains('\0') => domain.len() as u64,
            FilterValue::String(value) if value.len() <= 80 && !value.contains('\0') => value.len() as u64,
            _ => return Err(Error::Operation("repeat requires flat own native geometry arguments")),
        };
        bytes = bytes
            .checked_add(argument.name.len() as u64)
            .and_then(|n| n.checked_add(value_bytes))
            .filter(|n| *n <= 4096)
            .ok_or(Error::Operation("repeat argument byte limit"))?;
    }
    Ok(bytes)
}
/// Closed own schema: exact signed32 origin-x/y and extent-x0/y0/x1/y1;
/// horizontal/vertical enum domain own:repeat-axis (0 repeat, 1 mirror);
/// default-hex is lower-case native byte hex, not a color/profile interpretation.
pub fn validate_arguments(arguments: &[FilterArgument], extent: Rect, params: Params) -> Result<(), Error> {
    bounded_arguments(arguments)?;
    if arguments.len() != 9 {
        return Err(Error::Operation("repeat schema shape differs"));
    }
    for argument in arguments {
        let expected = match argument.name.as_str() {
            "origin-x" => Some(params.origin[0]),
            "origin-y" => Some(params.origin[1]),
            "extent-x0" => Some(extent.x0),
            "extent-y0" => Some(extent.y0),
            "extent-x1" => Some(extent.x1),
            "extent-y1" => Some(extent.y1),
            _ => None,
        };
        let valid = if let Some(expected) = expected {
            matches!(&argument.value, FilterValue::Number(ExactNumber::Signed { width: 32, value }) if *value == i64::from(expected))
        } else if let Some(axis) = match argument.name.as_str() {
            "horizontal" => Some(params.horizontal),
            "vertical" => Some(params.vertical),
            _ => None,
        } {
            let expected = match axis {
                Axis::Repeat => 0,
                Axis::Mirror => 1,
            };
            matches!(&argument.value, FilterValue::Enum { domain, value } if domain == "own:repeat-axis" && *value == expected)
        } else if argument.name == "default-hex" {
            match &argument.value {
                FilterValue::String(text) => {
                    let raw = params.output_default.bytes.get(..usize::from(params.output_default.length)).ok_or(Error::Domain)?;
                    if raw.is_empty() || text.len() != raw.len() * 2 {
                        false
                    } else {
                        text.as_bytes().as_chunks::<2>().0.iter().zip(raw).all(|(pair, byte)| {
                            let digits = b"0123456789abcdef";
                            pair.first() == digits.get(usize::from(byte >> 4)) && pair.get(1) == digits.get(usize::from(byte & 15))
                        })
                    }
                }
                _ => false,
            }
        } else {
            false
        };
        if !valid {
            return Err(Error::Operation("repeat arguments differ from exact caller geometry/default"));
        }
    }
    Ok(())
}
fn validate_binding(binding: &Binding<'_>) -> Result<u64, Error> {
    if !identity_bounded(binding.identity)
        || binding.node.0 == 0
        || binding.identity.name != "own:native-repeat"
        || binding.identity.stored_version != "1"
        || endpoint_id(binding.endpoint).is_none()
    {
        return Err(Error::Operation("repeat requires exact own schema and source endpoint"));
    }
    checked_region_byte_len(binding.input_extent, binding.domain.format).map_err(Error::Raster)?;
    if binding.input_extent.is_empty() || usize::from(binding.params.output_default.length) != binding.domain.format.bytes_per_pixel() {
        return Err(Error::Domain);
    }
    let bytes = bounded_arguments(binding.source_arguments)?;
    (binding.validate_arguments)(binding.source_arguments, binding.input_extent, binding.params)?;
    Ok(bytes)
}
/// Every repeated sample comes from the caller's complete authoritative extent.
/// This deliberately over-reads the full source extent, not the output rectangle
/// or a guessed phase bounding box. Planning never touches a Surface or pixels.
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>, control: &Interrupt<'_>) -> Result<InputFootprint, GraphRoiError> {
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    validate_binding(binding).map_err(|_| GraphRoiError::UnsupportedFootprint)?;
    if request.node != binding.node
        || request.edge.to != binding.node
        || Some(request.edge.from) != endpoint_id(binding.endpoint)
        || request.edge.input_port != "input"
        || request.edge.output_port != "output"
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    checked_region_byte_len(request.output, binding.domain.format)?;
    control.check().map_err(|_| GraphRoiError::Cancelled)?;
    Ok(InputFootprint {
        mapped_output: binding.input_extent,
        source_extent: binding.input_extent,
        halo: Halo::default(),
        abyss: Abyss::None,
        work_per_sample: 64,
    })
}

pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, Error> {
    budget.poll()?;
    let parameters =
        validate_binding(binding)?.checked_add(bounded_arguments(&request.node.arguments)?).and_then(|n| n.checked_add(4096)).ok_or(Error::ResidentLimit)?;
    if !identity_bounded(&request.node.operation)
        || request.node.id != binding.node
        || &request.node.operation != binding.identity
        || request.node.version_origin != binding.version_origin
    {
        return Err(Error::Operation("repeat node identity/version origin changed"));
    }
    (binding.validate_arguments)(&request.node.arguments, binding.input_extent, binding.params)?;
    if request.node.arguments.as_slice() != binding.source_arguments {
        return Err(Error::Operation("repeat source arguments changed"));
    }
    if request.domain != binding.domain || request.inputs.len() != 1 {
        return Err(Error::Domain);
    }
    let input = request.inputs.first().ok_or(Error::MissingInput)?;
    if input.input_port != "input"
        || input.output_port != "output"
        || input.endpoint != binding.endpoint
        || input.domain != binding.domain
        || input.surface.format() != binding.domain.format
        || input.roi != binding.input_extent
    {
        return Err(Error::Domain);
    }
    match input.footprint {
        Some(plan)
            if plan.output() == binding.input_extent
                && plan.support() == binding.input_extent
                && plan.source_extent() == binding.input_extent
                && plan.source_read() == binding.input_extent
                && plan.abyss() == Abyss::None => {}
        None if request.roi == binding.input_extent => {}
        _ => return Err(Error::Operation("repeat requires complete authoritative source footprint")),
    }
    let work = kernel::required_work(input.surface, request.roi).map_err(map_error)?;
    // Charge repeated metadata checks before requirements/kernel, without refunds.
    let metadata_work = (input.surface.tile_count() as u64).checked_mul(512).ok_or(Error::WorkLimit)?;
    budget.charge_work(work.checked_add(metadata_work).ok_or(Error::WorkLimit)?)?;
    let required =
        kernel::requirements_with_interrupt(input.surface, binding.input_extent, request.roi, binding.params, budget.interrupt()).map_err(map_error)?;
    if required.work_units != work {
        return Err(Error::InvalidPlan);
    }
    // Output reservation includes native map owners as finish_output checks
    // actual retained tile/map bytes, independently of the index ceiling.
    let output = required.output_retained_bytes.checked_add(required.output_index_bytes).ok_or(Error::ResidentLimit)?;
    let index = required.output_index_bytes.checked_add(1024).ok_or(Error::IndexLimit)?;
    let scratch = required.source_retained_bytes.checked_add(required.scratch_bytes).and_then(|n| n.checked_add(parameters)).ok_or(Error::ResidentLimit)?;
    let aggregate = scratch.checked_add(index).and_then(|n| n.checked_add(output)).ok_or(Error::ResidentLimit)?;
    let remaining = budget.remaining();
    if aggregate > remaining.resident_bytes {
        return Err(Error::ResidentLimit);
    }
    if output > remaining.cache_bytes {
        return Err(Error::CacheLimit);
    }
    if index > remaining.index_bytes {
        return Err(Error::IndexLimit);
    }
    budget.reserve_index(index)?;
    budget.reserve_output(output)?;
    budget.reserve_scratch(scratch)?;
    let surface = {
        let cancelled = || budget.poll().is_err();
        kernel::try_repeat(
            input.surface,
            binding.input_extent,
            request.roi,
            binding.params,
            Limits { resident_bytes: required.peak_resident_bytes, work_units: work },
            &Interrupt::cancel_only(&cancelled),
        )
        .map_err(map_error)?
    };
    budget.poll()?;
    budget.finish_output(surface, request.roi, request.domain)
}

// Unrun adaptive graph/raw precision/refusal drafts; no source admission.
#[cfg(test)]
mod tests {
    use super::super::{geometry_nearest::NativePixel, graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline_with_roi};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge, GraphNode},
    };
    use photocraft_raster::Surface;
    fn arguments(extent: Rect, params: Params) -> Vec<FilterArgument> {
        let mut arguments = [
            ("origin-x", params.origin[0]),
            ("origin-y", params.origin[1]),
            ("extent-x0", extent.x0),
            ("extent-y0", extent.y0),
            ("extent-x1", extent.x1),
            ("extent-y1", extent.y1),
        ]
        .into_iter()
        .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value: i64::from(value) }) })
        .collect::<Vec<_>>();
        for (name, axis) in [("horizontal", params.horizontal), ("vertical", params.vertical)] {
            arguments.push(FilterArgument {
                name: name.into(),
                value: FilterValue::Enum {
                    domain: "own:repeat-axis".into(),
                    value: match axis {
                        Axis::Repeat => 0,
                        Axis::Mirror => 1,
                    },
                },
            });
        }
        let raw = params.output_default.bytes.get(..usize::from(params.output_default.length)).unwrap();
        arguments.push(FilterArgument { name: "default-hex".into(), value: FilterValue::String(raw.iter().map(|byte| format!("{byte:02x}")).collect()) });
        arguments
    }
    #[test]
    fn negative_origin_mirror_full_source_witness_matches_native_and_refuses_context_limits_cancel() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let mut source = Surface::new(format);
        let extent = Rect::new(-4, -2, -2, -1);
        let raw = [0x8000000000000000u64, 0x7ff8000000000019, 1e100f64.to_bits(), 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &raw).unwrap();
        let original = source.clone();
        let params = Params {
            origin: [-1, 2],
            horizontal: Axis::Mirror,
            vertical: Axis::Repeat,
            output_default: NativePixel::from_slice(&[0x7ff0000000000000u64, 0x8000000000000000].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>())
                .unwrap(),
        };
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own:native-repeat".into(), stored_version: "1".into(), schema_epoch: "own-test-v1".into() },
                version_origin: GraphVersionOrigin::Stored,
                arguments: arguments(extent, params),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let node = graph.nodes.first().unwrap();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Perceptual, profile_fingerprint: None };
        // None is an explicitly opaque matching marker, not an assumed profile.
        let mut binding = Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            source_arguments: &node.arguments,
            validate_arguments,
            endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
            input_extent: extent,
            domain,
            params,
        };
        let roi = Rect::new(-257, -3, 257, 1);
        let output = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&source),
            roi,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| accepts_port(direction, port),
            |request| footprint(&binding, request, &Interrupt::NONE),
            |request, budget| evaluate(&binding, request, budget),
        )
        .unwrap();
        let requirements = kernel::requirements_with_interrupt(&source, extent, roi, params, &Interrupt::NONE).unwrap();
        let direct = kernel::try_repeat(
            &source,
            extent,
            roi,
            params,
            Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), direct.try_read_interleaved(roi).unwrap());
        assert_eq!(output.surface().default_bytes(), direct.default_bytes());
        // Explicit footprint must cover the period's complete logical extent.
        let edge = graph.edges.first().unwrap();
        let declared = footprint(&binding, FootprintRequest { node: node.id, declared_index: 0, edge_index: 0, edge, output: roi }, &Interrupt::NONE).unwrap();
        assert_eq!(declared.mapped_output, extent);
        assert_eq!(declared.source_extent, extent);
        assert_eq!(declared.halo, Halo::default());
        for limits in [
            ExecutionLimits { work_units: 1, ..Default::default() },
            ExecutionLimits { cache_bytes: 1, ..Default::default() },
            ExecutionLimits { index_bytes: 1, ..Default::default() },
        ] {
            assert!(
                execute_pipeline_with_roi(
                    &graph,
                    2,
                    Some(&source),
                    roi,
                    domain,
                    limits,
                    &Interrupt::NONE,
                    |_, direction, port| accepts_port(direction, port),
                    |request| footprint(&binding, request, &Interrupt::NONE),
                    |request, budget| evaluate(&binding, request, budget)
                )
                .is_err()
            );
        }
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&source),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::cancel_only(&|| true),
                |_, direction, port| accepts_port(direction, port),
                |request| footprint(&binding, request, &Interrupt::NONE),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        binding.domain.encoding = SampleEncoding::Linear;
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&source),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(direction, port),
                |request| footprint(&binding, request, &Interrupt::NONE),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        assert_eq!(source, original);
    }
    #[test]
    fn incomplete_footprint_and_changed_default_bits_are_not_guessed() {
        let extent = Rect::new(-2, 0, 2, 1);
        let params = Params { origin: [0, 0], horizontal: Axis::Repeat, vertical: Axis::Mirror, output_default: NativePixel::from_slice(&[0, 1]).unwrap() };
        let mut args = arguments(extent, params);
        validate_arguments(&args, extent, params).unwrap();
        args.last_mut().unwrap().value = FilterValue::String("0000".into());
        assert!(validate_arguments(&args, extent, params).is_err());
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own:native-repeat".into(), stored_version: "1".into(), schema_epoch: "own-test".into() },
                version_origin: Default::default(),
                arguments: arguments(extent, params),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let node = graph.nodes.first().unwrap();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false);
        let source = Surface::new(format);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([4; 32]) };
        let binding = Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            source_arguments: &node.arguments,
            validate_arguments,
            endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
            input_extent: extent,
            domain,
            params,
        };
        let roi = Rect::new(-1, 0, 1, 1);
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&source),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(direction, port),
                |request| Ok(InputFootprint {
                    mapped_output: request.output,
                    source_extent: extent,
                    halo: Halo::default(),
                    abyss: Abyss::None,
                    work_per_sample: 64
                }),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
    }
}
