//! Typed fixed-ROI graph bridge for the original weighted-sum primitive.
//! No stored GEGL/GIMP schema, color conversion or source admission is inferred.
use super::{
    graph_execute::{GraphExecuteError as Error, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    multi_input_weighted_sum::{self as kernel, Alpha, Channel, Identity, Input, Outside, Request, WorkingContext},
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub input_port: &'a str,
    pub endpoint: Endpoint,
    pub extent: Rect,
    pub outside: Outside<'a>,
    pub coefficient: f64,
    pub channels: [Channel; 4],
}
/// Trusted caller validator must be bounded and allocation-free, and bind every
/// coefficient, selector, bias and alpha/identity policy to exact stored values.
pub type ArgumentValidator = fn(&[FilterArgument], &[SourceBinding<'_>], [f64; 4], Alpha, Identity) -> Result<(), Error>;
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub source_arguments: &'a [FilterArgument],
    pub validate_arguments: ArgumentValidator,
    /// Order is arithmetic source order, independently of graph edge order.
    pub sources: &'a [SourceBinding<'a>],
    pub context: WorkingContext,
    pub bias: [f64; 4],
    pub alpha: Alpha,
    pub identity_policy: Identity,
}
fn port_valid(port: &str) -> bool {
    !port.is_empty() && port.len() <= 64 && port.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
/// Planner callback still requires the caller's exact binding at evaluation.
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Output => port == "output",
        PortDirection::Input => {
            binding.sources.len() <= kernel::MAX_INPUTS && binding.sources.iter().any(|source| source.input_port == port && port_valid(port))
        }
    }
}
fn endpoint_valid(endpoint: Endpoint) -> bool {
    match endpoint {
        Endpoint::ExternalInput(id) => id.0 != 0,
        Endpoint::Node { id, index } => id.0 != 0 && index < 4096,
    }
}
fn identity_bounded(identity: &OperationIdentity) -> bool {
    [(identity.name.as_str(), 256), (identity.stored_version.as_str(), 64), (identity.schema_epoch.as_str(), 256)]
        .into_iter()
        .all(|(value, cap)| !value.is_empty() && value.len() <= cap && !value.contains('\0'))
}
fn bounded_arguments(arguments: &[FilterArgument]) -> Result<u64, Error> {
    if arguments.len() > 96 {
        return Err(Error::Operation("weighted-sum argument count limit"));
    }
    let mut bytes = 0u64;
    for (index, argument) in arguments.iter().enumerate() {
        if argument.name.is_empty()
            || argument.name.len() > 128
            || argument.name.contains('\0')
            || arguments.iter().take(index).any(|old| old.name == argument.name)
        {
            return Err(Error::Operation("weighted-sum requires unique bounded argument names"));
        }
        let value = match &argument.value {
            FilterValue::Number(ExactNumber::Float64Bits(bits)) if f64::from_bits(*bits).is_finite() => 32,
            FilterValue::Number(ExactNumber::Unsigned { width: 8, value }) if *value <= 255 => 32,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 256 && !domain.contains('\0') => domain.len() as u64,
            _ => return Err(Error::Operation("weighted-sum requires closed scalar arguments")),
        };
        bytes = bytes
            .checked_add(argument.name.len() as u64)
            .and_then(|n| n.checked_add(value))
            .filter(|n| *n <= 16 << 10)
            .ok_or(Error::Operation("weighted-sum argument byte limit"))?;
    }
    Ok(bytes)
}
fn float_matches(value: &FilterValue, expected: f64) -> bool {
    matches!(value, FilterValue::Number(ExactNumber::Float64Bits(bits)) if *bits == expected.to_bits())
}
/// Closed own schema: coefficient/<port>, selector/<port>/<0..3>, bias/<0..3>,
/// alpha and identity. Every floating payload is exact Float64Bits; native
/// selectors are Unsigned(width8). No implicit parameters or GEGL aliases.
pub fn validate_arguments(arguments: &[FilterArgument], sources: &[SourceBinding<'_>], bias: [f64; 4], alpha: Alpha, identity: Identity) -> Result<(), Error> {
    bounded_arguments(arguments)?;
    if sources.is_empty() || sources.len() > kernel::MAX_INPUTS || arguments.len() != sources.len() * 5 + 6 {
        return Err(Error::Operation("weighted-sum schema shape differs"));
    }
    for argument in arguments {
        let valid = if let Some(port) = argument.name.strip_prefix("coefficient/") {
            sources.iter().find(|source| source.input_port == port).is_some_and(|source| float_matches(&argument.value, source.coefficient))
        } else if let Some(selector) = argument.name.strip_prefix("selector/") {
            selector
                .rsplit_once('/')
                .and_then(|(port, channel)| {
                    let index = match channel {
                        "0" => 0,
                        "1" => 1,
                        "2" => 2,
                        "3" => 3,
                        _ => return None,
                    };
                    sources.iter().find(|source| source.input_port == port).and_then(|source| source.channels.get(index)).map(|selector| match selector {
                        Channel::Native(index) => {
                            matches!(&argument.value, FilterValue::Number(ExactNumber::Unsigned { width: 8, value }) if *value == u64::from(*index))
                        }
                        Channel::Constant(value) => float_matches(&argument.value, *value),
                    })
                })
                .unwrap_or(false)
        } else if let Some(channel) = argument.name.strip_prefix("bias/") {
            let index = match channel {
                "0" => Some(0),
                "1" => Some(1),
                "2" => Some(2),
                "3" => Some(3),
                _ => None,
            };
            index.and_then(|index| bias.get(index)).is_some_and(|value| float_matches(&argument.value, *value))
        } else if argument.name == "alpha" {
            let expected = match alpha {
                Alpha::PreserveFirstRaw => 0,
                Alpha::Compute => 1,
            };
            matches!(&argument.value, FilterValue::Enum { domain, value } if domain == "own:weighted-sum-alpha" && *value == expected)
        } else if argument.name == "identity" {
            let expected = match identity {
                Identity::Evaluate => 0,
                Identity::PreserveExact => 1,
            };
            matches!(&argument.value, FilterValue::Enum { domain, value } if domain == "own:weighted-sum-identity" && *value == expected)
        } else {
            false
        };
        if !valid {
            return Err(Error::Operation("weighted-sum arguments differ from typed caller parameters"));
        }
    }
    Ok(())
}

pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, Error> {
    budget.poll()?;
    if !identity_bounded(binding.identity)
        || !identity_bounded(&request.node.operation)
        || binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity != &request.node.operation
        || binding.identity.name != "own:weighted-sum"
        || binding.identity.stored_version != "1"
        || binding.version_origin != request.node.version_origin
    {
        return Err(Error::Operation("weighted-sum requires exact caller-bound own schema/version origin"));
    }
    let parameters = bounded_arguments(binding.source_arguments)?
        .checked_add(bounded_arguments(&request.node.arguments)?)
        .and_then(|n| n.checked_add((std::mem::size_of::<Input<'_>>() * kernel::MAX_INPUTS) as u64 + 4096))
        .ok_or(Error::ResidentLimit)?;
    if binding.sources.is_empty() || binding.sources.len() > kernel::MAX_INPUTS || request.inputs.len() != binding.sources.len() {
        return Err(Error::Operation("weighted-sum requires one to sixteen exact source ports"));
    }
    for (index, source) in binding.sources.iter().enumerate() {
        if !port_valid(source.input_port)
            || !endpoint_valid(source.endpoint)
            || source.extent != request.roi
            || binding.sources.iter().take(index).any(|old| old.input_port == source.input_port)
        {
            return Err(Error::Operation("weighted-sum requires unique fixed-ROI source bindings"));
        }
    }
    (binding.validate_arguments)(binding.source_arguments, binding.sources, binding.bias, binding.alpha, binding.identity_policy)?;
    (binding.validate_arguments)(&request.node.arguments, binding.sources, binding.bias, binding.alpha, binding.identity_policy)?;
    if binding.source_arguments != request.node.arguments.as_slice() {
        return Err(Error::Operation("weighted-sum source arguments changed"));
    }
    if request.domain.encoding != binding.context.encoding || request.domain.profile_fingerprint != Some(binding.context.profile_identity) {
        return Err(Error::Domain);
    }
    let first = request.inputs.first().ok_or(Error::MissingInput)?;
    let declared = binding.sources.first().ok_or(Error::MissingInput)?;
    let filler = Input {
        source: first.surface,
        extent: declared.extent,
        context: binding.context,
        outside: declared.outside,
        coefficient: declared.coefficient,
        channels: declared.channels,
    };
    let mut inputs = [filler; kernel::MAX_INPUTS];
    let mut seen = [false; kernel::MAX_INPUTS];
    for input in request.inputs {
        budget.poll()?;
        if input.output_port != "output"
            || input.roi != request.roi
            || input.footprint.is_some()
            || input.domain != request.domain
            || input.surface.format() != request.domain.format
        {
            return Err(Error::Domain);
        }
        let (index, source) = binding
            .sources
            .iter()
            .enumerate()
            .find(|(_, source)| source.input_port == input.input_port)
            .ok_or(Error::Operation("weighted-sum input port changed"))?;
        let visited = seen.get_mut(index).ok_or(Error::InvalidPlan)?;
        if *visited || input.endpoint != source.endpoint {
            return Err(Error::Operation("weighted-sum source endpoint changed or duplicated"));
        }
        *visited = true;
        *inputs.get_mut(index).ok_or(Error::InvalidPlan)? = Input {
            source: input.surface,
            extent: source.extent,
            context: binding.context,
            outside: source.outside,
            coefficient: source.coefficient,
            channels: source.channels,
        };
    }
    let count = binding.sources.len();
    if seen.get(..count).ok_or(Error::InvalidPlan)?.iter().any(|visited| !visited) {
        return Err(Error::MissingInput);
    }
    let kernel_request = Request {
        inputs: inputs.get(..count).ok_or(Error::InvalidPlan)?,
        roi: request.roi,
        output_context: binding.context,
        bias: binding.bias,
        alpha: binding.alpha,
        identity: binding.identity_policy,
    };
    let work = kernel::required_work(kernel_request).map_err(Error::Raster)?;
    budget.charge_work(work)?;
    let footprint = kernel::requirements(kernel_request, budget.interrupt()).map_err(Error::Raster)?;
    if footprint.work_units != work {
        return Err(Error::InvalidPlan);
    }
    let source_tiles = kernel_request.inputs.iter().try_fold(0u64, |n, input| n.checked_add(input.source.tile_count() as u64).ok_or(Error::WorkLimit))?;
    let trim_work = source_tiles
        .checked_add(filler.source.tile_count() as u64)
        .and_then(|n| n.checked_add(footprint.roi_tiles))
        .and_then(|n| n.checked_mul(256))
        .ok_or(Error::WorkLimit)?;
    budget.charge_work(trim_work)?;
    let index = footprint
        .index_bytes
        .checked_add(footprint.roi_tiles.checked_mul(512).ok_or(Error::IndexLimit)?)
        .and_then(|n| n.checked_add(1024))
        .ok_or(Error::IndexLimit)?;
    // Executor-owned inputs remain charged. This adapter overcharges shared
    // source payloads rather than refunding or guessing Arc ownership.
    let scratch = footprint.source_bytes.checked_add(footprint.scratch_bytes).and_then(|n| n.checked_add(parameters)).ok_or(Error::ResidentLimit)?;
    let aggregate = scratch.checked_add(index).and_then(|n| n.checked_add(footprint.roi_output_bytes)).ok_or(Error::ResidentLimit)?;
    let remaining = budget.remaining();
    if aggregate > remaining.resident_bytes || footprint.resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(Error::ResidentLimit);
    }
    if index > remaining.index_bytes {
        return Err(Error::IndexLimit);
    }
    if footprint.roi_output_bytes > remaining.cache_bytes {
        return Err(Error::CacheLimit);
    }
    budget.reserve_index(index)?;
    budget.reserve_output(footprint.roi_output_bytes)?;
    budget.reserve_scratch(scratch)?;
    let staged = {
        let cancelled = || budget.poll().is_err();
        kernel::try_apply(kernel_request, kernel::Limits { resident_bytes: footprint.resident_bytes, work_units: work }, &Interrupt::cancel_only(&cancelled))
            .map_err(Error::Raster)?
    };
    budget.poll()?;
    let mut trimmed = Surface::try_with_default_bytes(staged.format(), staged.default_bytes()).map_err(Error::Raster)?;
    for (coordinate, tile) in staged.tiles() {
        budget.poll()?;
        if !coordinate.rect().intersect(&request.roi).is_empty() {
            trimmed.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
    }
    drop(staged);
    budget.poll()?;
    budget.finish_output(trimmed, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::super::{
        graph_execute::{ExecutionDomain, ExecutionLimits, execute_graph},
        graph_plan::plan_graph,
    };
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge, GraphNode},
    };
    fn arguments(sources: &[SourceBinding<'_>]) -> Vec<FilterArgument> {
        let mut args = Vec::new();
        for source in sources {
            args.push(FilterArgument {
                name: format!("coefficient/{}", source.input_port),
                value: FilterValue::Number(ExactNumber::Float64Bits(source.coefficient.to_bits())),
            });
            for (index, selector) in source.channels.iter().enumerate() {
                let value = match selector {
                    Channel::Native(index) => ExactNumber::Unsigned { width: 8, value: u64::from(*index) },
                    Channel::Constant(value) => ExactNumber::Float64Bits(value.to_bits()),
                };
                args.push(FilterArgument { name: format!("selector/{}/{index}", source.input_port), value: FilterValue::Number(value) });
            }
        }
        for index in 0..4 {
            args.push(FilterArgument { name: format!("bias/{index}"), value: FilterValue::Number(ExactNumber::Float64Bits(0)) });
        }
        args.push(FilterArgument { name: "alpha".into(), value: FilterValue::Enum { domain: "own:weighted-sum-alpha".into(), value: 0 } });
        args.push(FilterArgument { name: "identity".into(), value: FilterValue::Enum { domain: "own:weighted-sum-identity".into(), value: 0 } });
        args
    }
    #[test]
    fn shared_two_port_graph_f64_matches_direct_kernel_and_refuses_stale_domain_budget_cancel() {
        let roi = Rect::new(-1, 0, 1, 1);
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [3; 32], coordinate_space: [7; 32] };
        let sources = [0.5, 2.0]
            .into_iter()
            .zip(["left", "right"])
            .map(|(coefficient, input_port)| SourceBinding {
                input_port,
                endpoint: Endpoint::ExternalInput(GraphNodeId(9)),
                extent: roi,
                outside: Outside::Reject,
                coefficient,
                channels: [Channel::Native(0), Channel::Native(1), Channel::Constant(0.), Channel::Constant(0.)],
            })
            .collect::<Vec<_>>();
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own:weighted-sum".into(), stored_version: "1".into(), schema_epoch: "own-test-v1".into() },
                version_origin: GraphVersionOrigin::Stored,
                arguments: arguments(&sources),
            }],
            edges: ["right", "left"]
                .into_iter()
                .map(|port| GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "output".into(), input_port: port.into() })
                .collect(),
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let node = graph.nodes.first().unwrap();
        let binding = Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            source_arguments: &node.arguments,
            validate_arguments,
            sources: &sources,
            context,
            bias: [0.; 4],
            alpha: Alpha::PreserveFirstRaw,
            identity_policy: Identity::Evaluate,
        };
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, direction, port| accepts_port(&binding, direction, port)).unwrap();
        let raw = [2f64, 0.25].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: true }, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) };
        let output = execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
            evaluate(&binding, request, budget)
        })
        .unwrap();
        let inputs = sources
            .iter()
            .map(|binding| Input {
                source: &source,
                extent: binding.extent,
                context,
                outside: binding.outside,
                coefficient: binding.coefficient,
                channels: binding.channels,
            })
            .collect::<Vec<_>>();
        let request = Request { inputs: &inputs, roi, output_context: context, bias: [0.; 4], alpha: Alpha::PreserveFirstRaw, identity: Identity::Evaluate };
        let footprint = kernel::requirements(request, &Interrupt::NONE).unwrap();
        let direct =
            kernel::try_apply(request, kernel::Limits { resident_bytes: footprint.resident_bytes, work_units: footprint.work_units }, &Interrupt::NONE)
                .unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), direct.try_read_interleaved(roi).unwrap());
        let pixel = output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 0).unwrap(), 5.);
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 1).unwrap(), 0.25);
        assert_eq!(output.surface().default_bytes(), raw);
        for limits in [ExecutionLimits { work_units: 1, ..Default::default() }, ExecutionLimits { index_bytes: 1, ..Default::default() }] {
            assert!(execute_graph(&plan, Some(&source), roi, domain, limits, &Interrupt::NONE, |request, budget| evaluate(&binding, request, budget)).is_err());
        }
        assert!(
            execute_graph(
                &plan,
                Some(&source),
                roi,
                ExecutionDomain { profile_fingerprint: None, ..domain },
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        assert!(
            execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::cancel_only(&|| true), |request, budget| evaluate(
                &binding, request, budget
            ))
            .is_err()
        );
        assert_eq!(source, before);
    }
    #[test]
    fn exact_scalar_schema_refuses_duplicate_missing_and_changed_bit_values() {
        let sources = [SourceBinding {
            input_port: "source",
            endpoint: Endpoint::ExternalInput(GraphNodeId(2)),
            extent: Rect::new(0, 0, 1, 1),
            outside: Outside::Reject,
            coefficient: 1.,
            channels: [Channel::Native(0), Channel::Constant(0.), Channel::Constant(0.), Channel::Constant(0.)],
        }];
        let mut args = arguments(&sources);
        validate_arguments(&args, &sources, [0.; 4], Alpha::PreserveFirstRaw, Identity::Evaluate).unwrap();
        args[0].value = FilterValue::Number(ExactNumber::Float64Bits((-0f64).to_bits()));
        assert!(validate_arguments(&args, &sources, [0.; 4], Alpha::PreserveFirstRaw, Identity::Evaluate).is_err());
        let mut args = arguments(&sources);
        args[1].name = args[0].name.clone();
        assert!(validate_arguments(&args, &sources, [0.; 4], Alpha::PreserveFirstRaw, Identity::Evaluate).is_err());
        assert!(validate_arguments(&args[..1], &sources, [0.; 4], Alpha::PreserveFirstRaw, Identity::Evaluate).is_err());
    }
}
