//! Required-caller-schema graph bridge for original straight source-over.
//! Metadata alone never admits a node, profile or source operation.
use super::{
    graph_execute::{GraphExecuteError as Error, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    multi_input_source_over::{self as kernel, Input, Outside, Request, Rounding, WorkingContext, ZeroAlpha},
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use std::sync::Arc;

/// Caller-owned validator must be bounded/allocation-free. This trusted Rust
/// seam is not a sandbox or a process-memory guarantee.
pub type ArgumentValidator = fn(&[FilterArgument], ZeroAlpha, Rounding) -> Result<(), Error>;
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub endpoint: Endpoint,
    /// Fixed graph read ROI, including authoritative native default pixels.
    pub extent: Rect,
    pub outside: Outside<'a>,
}
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub source_arguments: &'a [FilterArgument],
    /// Mandatory independently chosen interpretation; called on both the
    /// captured source schema and actual node arguments before equality/work.
    pub validate_arguments: ArgumentValidator,
    pub foreground: SourceBinding<'a>,
    pub background: SourceBinding<'a>,
    pub context: WorkingContext,
    pub zero_alpha: ZeroAlpha,
    pub rounding: Rounding,
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => matches!(port, "foreground" | "background"),
        PortDirection::Output => port == "output",
    }
}
fn bounded_arguments(arguments: &[FilterArgument]) -> Result<(), Error> {
    if arguments.len() > 16 {
        return Err(Error::Operation("source-over argument count limit"));
    }
    let mut bytes = 0usize;
    for (index, argument) in arguments.iter().enumerate() {
        if argument.name.is_empty()
            || argument.name.len() > 128
            || argument.name.contains('\0')
            || arguments.iter().take(index).any(|previous| previous.name == argument.name)
        {
            return Err(Error::Operation("source-over requires unique bounded argument names"));
        }
        let value_bytes = match &argument.value {
            FilterValue::Bool(_) => 8,
            FilterValue::Number(ExactNumber::Float32Bits(bits)) if f32::from_bits(*bits).is_finite() => 32,
            FilterValue::Number(ExactNumber::Float64Bits(bits)) if f64::from_bits(*bits).is_finite() => 32,
            FilterValue::Number(ExactNumber::Signed { width: 8 | 16 | 32 | 64, .. } | ExactNumber::Unsigned { width: 8 | 16 | 32 | 64, .. }) => 32,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 256 && !domain.contains('\0') => domain.len(),
            FilterValue::String(text) if text.len() <= 4096 && !text.contains('\0') => text.len(),
            _ => return Err(Error::Operation("source-over requires flat caller-qualified arguments")),
        };
        bytes = bytes
            .checked_add(argument.name.len())
            .and_then(|n| n.checked_add(value_bytes))
            .filter(|n| *n <= 16 << 10)
            .ok_or(Error::Operation("source-over argument byte limit"))?;
    }
    Ok(())
}
/// One closed own-schema validator, available for explicit caller binding.
/// It does not resolve stored GEGL or GIMP configurations.
pub fn validate_arguments(arguments: &[FilterArgument], zero_alpha: ZeroAlpha, rounding: Rounding) -> Result<(), Error> {
    bounded_arguments(arguments)?;
    let zero = match zero_alpha {
        ZeroAlpha::PreserveBackgroundRaw => 0,
        ZeroAlpha::ZeroColors => 1,
        ZeroAlpha::Reject => 2,
    };
    match rounding {
        Rounding::NearestTiesUp => {}
    }
    if arguments.len() != 2
        || !arguments.iter().any(|argument| {
            argument.name == "zero-alpha" && matches!(&argument.value,FilterValue::Enum {domain,value} if domain=="own:source-over-zero-alpha" && *value==zero)
        })
        || !arguments.iter().any(|argument| {
            argument.name == "rounding" && matches!(&argument.value,FilterValue::Enum {domain,value} if domain=="own:nearest-ties-up" && *value==0)
        })
    {
        return Err(Error::Operation("source-over policies differ from exact caller arguments"));
    }
    Ok(())
}
fn endpoint_valid(endpoint: Endpoint) -> bool {
    match endpoint {
        Endpoint::ExternalInput(id) => id.0 != 0,
        Endpoint::Node { id, index } => id.0 != 0 && index < 4096,
    }
}
fn identity_bounded(identity: &OperationIdentity) -> bool {
    !identity.name.is_empty()
        && identity.name.len() <= 256
        && !identity.name.contains('\0')
        && !identity.stored_version.is_empty()
        && identity.stored_version.len() <= 64
        && !identity.stored_version.contains('\0')
        && !identity.schema_epoch.is_empty()
        && identity.schema_epoch.len() <= 256
        && !identity.schema_epoch.contains('\0')
}

pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, Error> {
    budget.poll()?;
    if !identity_bounded(binding.identity) || !identity_bounded(&request.node.operation) {
        return Err(Error::Operation("source-over identity byte limit"));
    }
    if binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity != &request.node.operation
        || binding.identity.name != "own:straight-source-over"
        || binding.identity.stored_version != "1"
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || binding.version_origin != request.node.version_origin
    {
        return Err(Error::Operation("source-over requires exact caller-bound own schema/version origin"));
    }
    bounded_arguments(binding.source_arguments)?;
    bounded_arguments(&request.node.arguments)?;
    (binding.validate_arguments)(binding.source_arguments, binding.zero_alpha, binding.rounding)?;
    (binding.validate_arguments)(&request.node.arguments, binding.zero_alpha, binding.rounding)?;
    if binding.source_arguments != request.node.arguments.as_slice() {
        return Err(Error::Operation("source-over source arguments changed"));
    }
    if request.domain.encoding != binding.context.encoding || request.domain.profile_fingerprint != Some(binding.context.profile_identity) {
        return Err(Error::Domain);
    }
    if request.inputs.len() != 2
        || binding.foreground.extent != request.roi
        || binding.background.extent != request.roi
        || !endpoint_valid(binding.foreground.endpoint)
        || !endpoint_valid(binding.background.endpoint)
    {
        return Err(Error::Operation("source-over requires two explicit fixed-ROI source bindings"));
    }
    let (mut foreground, mut background) = (None, None);
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
        let (target, source) = match input.input_port {
            "foreground" => (&mut foreground, binding.foreground),
            "background" => (&mut background, binding.background),
            _ => return Err(Error::Operation("source-over input port is not exact")),
        };
        if input.endpoint != source.endpoint || target.replace(input.surface).is_some() {
            return Err(Error::Operation("source-over source endpoint changed or duplicated"));
        }
    }
    let foreground = Input {
        surface: foreground.ok_or(Error::MissingInput)?,
        extent: binding.foreground.extent,
        context: binding.context,
        outside: binding.foreground.outside,
    };
    let background = Input {
        surface: background.ok_or(Error::MissingInput)?,
        extent: binding.background.extent,
        context: binding.context,
        outside: binding.background.outside,
    };
    let kernel_request =
        Request { foreground, background, roi: request.roi, output_context: binding.context, zero_alpha: binding.zero_alpha, rounding: binding.rounding };
    let work = kernel::required_work(kernel_request).map_err(Error::Raster)?;
    let source_tiles = (foreground.surface.tile_count() as u64).checked_add(background.surface.tile_count() as u64).ok_or(Error::WorkLimit)?;
    // Charge allocation-free requirements walks too, with no upstream refunds.
    budget.charge_work(work.checked_add(source_tiles.checked_mul(512).ok_or(Error::WorkLimit)?).ok_or(Error::WorkLimit)?)?;
    let footprint = kernel::requirements(kernel_request, budget.interrupt()).map_err(Error::Raster)?;
    if footprint.work_units != work {
        return Err(Error::InvalidPlan);
    }
    let trim_work = (background.surface.tile_count() as u64).checked_add(footprint.roi_tiles).and_then(|n| n.checked_mul(256)).ok_or(Error::WorkLimit)?;
    budget.charge_work(trim_work)?;
    let index = footprint
        .index_bytes
        .checked_add(footprint.roi_tiles.checked_mul(512).ok_or(Error::IndexLimit)?)
        .and_then(|n| n.checked_add(1024))
        .ok_or(Error::IndexLimit)?;
    // Upstream owners remain charged by the executor. Conservatively charge
    // source retention again for this kernel's lower native peak, never refund.
    let scratch = footprint.source_bytes.checked_add(footprint.scratch_bytes).ok_or(Error::ResidentLimit)?;
    let aggregate = footprint.roi_output_bytes.checked_add(index).and_then(|n| n.checked_add(scratch)).ok_or(Error::ResidentLimit)?;
    let remaining = budget.remaining();
    if aggregate > remaining.resident_bytes || footprint.resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(Error::ResidentLimit);
    }
    if footprint.roi_output_bytes > remaining.cache_bytes {
        return Err(Error::CacheLimit);
    }
    if index > remaining.index_bytes {
        return Err(Error::IndexLimit);
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
    // Retain only ROI-intersecting native tiles, without normalizing border
    // pixels/defaults/alpha. No full-ROI byte buffer or f32 conversion.
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
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{FilterGraph, GraphEdge, GraphNode},
    };
    fn arguments() -> Vec<FilterArgument> {
        vec![
            FilterArgument { name: "zero-alpha".into(), value: FilterValue::Enum { domain: "own:source-over-zero-alpha".into(), value: 0 } },
            FilterArgument { name: "rounding".into(), value: FilterValue::Enum { domain: "own:nearest-ties-up".into(), value: 0 } },
        ]
    }
    #[test]
    fn explicit_two_ports_u32_graph_matches_kernel_without_admitting_metadata() {
        let node = GraphNode {
            id: GraphNodeId(1),
            operation: OperationIdentity {
                name: "own:straight-source-over".into(),
                stored_version: "1".into(),
                schema_epoch: "own-test-source-over-v1".into(),
            },
            version_origin: GraphVersionOrigin::Stored,
            arguments: arguments(),
        };
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node],
            edges: ["foreground", "background"]
                .into_iter()
                .map(|port| GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "output".into(), input_port: port.into() })
                .collect(),
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let plan = plan_graph(&graph, 2, &Interrupt::NONE, |_, direction, port| accepts_port(direction, port)).unwrap();
        let raw: Vec<u8> = [1u32, 17, 29, 0x80000000].into_iter().flat_map(u32::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(photocraft_color::PixelFormat::RGBA32, &raw).unwrap();
        let before = source.clone();
        let roi = Rect::new(-1, 0, 1, 1);
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [3; 32], coordinate_space: [9; 32] };
        let node = graph.nodes.first().unwrap();
        let mut binding = Binding {
            node: node.id,
            identity: &node.operation,
            version_origin: node.version_origin,
            source_arguments: &node.arguments,
            validate_arguments,
            foreground: SourceBinding { endpoint: Endpoint::ExternalInput(GraphNodeId(9)), extent: roi, outside: Outside::Reject },
            background: SourceBinding { endpoint: Endpoint::ExternalInput(GraphNodeId(9)), extent: roi, outside: Outside::Reject },
            context,
            zero_alpha: ZeroAlpha::PreserveBackgroundRaw,
            rounding: Rounding::NearestTiesUp,
        };
        let domain = ExecutionDomain { format: source.format(), encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) };
        let output = execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |request, budget| {
            evaluate(&binding, request, budget)
        })
        .unwrap();
        let native = output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(&native[..12], &raw[..12]);
        assert_eq!(u32::from_ne_bytes(native[12..].try_into().unwrap()), 0xc0000000);
        assert_eq!(output.surface().default_bytes(), raw);
        assert_eq!(source, before);
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
            execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits { work_units: 1, ..Default::default() }, &Interrupt::NONE, |request, budget| {
                evaluate(&binding, request, budget)
            })
            .is_err()
        );
        assert!(
            execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::cancel_only(&|| true), |request, budget| evaluate(
                &binding, request, budget
            ))
            .is_err()
        );
        binding.foreground.endpoint = Endpoint::ExternalInput(GraphNodeId(10));
        assert!(
            execute_graph(&plan, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |request, budget| evaluate(
                &binding, request, budget
            ))
            .is_err()
        );
        assert_eq!(source, before);
    }
    #[test]
    fn closed_policy_arguments_and_ports_reject_unknown_or_stale_values() {
        let mut args = arguments();
        validate_arguments(&args, ZeroAlpha::PreserveBackgroundRaw, Rounding::NearestTiesUp).unwrap();
        args[0].value = FilterValue::Enum { domain: "own:source-over-zero-alpha".into(), value: 2 };
        assert!(validate_arguments(&args, ZeroAlpha::PreserveBackgroundRaw, Rounding::NearestTiesUp).is_err());
        assert!(!accepts_port(PortDirection::Input, "aux"));
        assert!(!accepts_port(PortDirection::Output, "input"));
    }
}
