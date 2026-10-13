//! Explicit fixed-ROI dimensionless native coverage graph adapter.
//! Retained node names do not qualify source semantics or resource authority.
use super::{
    alpha_association_graph as shared,
    graph_execute::{self, AvailableNodeRequest, GraphExecuteError as Error, InputPixels, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    multi_input_coverage as kernel,
    wide_point::WorkingContext,
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterArgument, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
type Result<T> = std::result::Result<T, Error>;
pub type ArgumentValidator = fn(&[FilterArgument], kernel::Params) -> Result<()>;
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub endpoint: Endpoint,
    pub extent: Rect,
    pub outside: kernel::Outside<'a>,
}
pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub arguments: &'a [FilterArgument],
    pub validate_arguments: ArgumentValidator,
    pub first: SourceBinding<'a>,
    pub operand: SourceBinding<'a>,
    pub context: WorkingContext,
    pub params: kernel::Params,
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => matches!(port, "first" | "operand"),
        PortDirection::Output => port == "output",
    }
}
fn error(e: kernel::Error) -> Error {
    match e {
        kernel::Error::Raster(r) => Error::Raster(r),
        _ => Error::Operation("native coverage policy/domain"),
    }
}
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    shared::schema(binding.node, binding.identity, binding.version_origin, binding.arguments, &request, "own:coverage-combination")?;
    (binding.validate_arguments)(binding.arguments, binding.params)?;
    (binding.validate_arguments)(&request.node.arguments, binding.params)?;
    if request.domain.encoding != binding.context.encoding || request.domain.profile_fingerprint != Some(binding.context.profile_identity) {
        return Err(Error::Domain);
    }
    if request.inputs.len() != 2
        || binding.first.extent != request.roi
        || binding.operand.extent != request.roi
        || !shared::endpoint_valid(binding.first.endpoint)
        || !shared::endpoint_valid(binding.operand.endpoint)
    {
        return Err(Error::Roi);
    }
    let mut first = None;
    let mut operand = None;
    for input in request.inputs {
        budget.poll()?;
        if input.output_port != "output"
            || input.roi != request.roi
            || input.domain != request.domain
            || input.surface.format() != request.domain.format
            || input.footprint.is_some()
        {
            return Err(Error::Domain);
        }
        let (slot, source) = match input.input_port {
            "first" => (&mut first, binding.first),
            "operand" => (&mut operand, binding.operand),
            _ => return Err(Error::Operation("native coverage exact source port")),
        };
        if input.endpoint != source.endpoint || slot.replace(input.surface).is_some() {
            return Err(Error::Operation("native coverage changed/duplicate endpoint"));
        }
    }
    let a = first.ok_or(Error::MissingInput)?;
    let b = operand.ok_or(Error::MissingInput)?;
    let (resident, work) = shared::precharge(&[a, b], request.roi, 192, budget)?;
    let staged = {
        let cancel = || budget.poll().is_err();
        kernel::try_apply(
            kernel::Input { surface: a, extent: binding.first.extent, outside: binding.first.outside },
            kernel::Input { surface: b, extent: binding.operand.extent, outside: binding.operand.outside },
            request.roi,
            binding.params,
            kernel::Limits { resident_bytes: resident, work_units: work },
            &Interrupt::cancel_only(&cancel),
        )
        .map_err(error)?
    };
    shared::finish(staged, &request, budget)
}
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    if request.inputs.len() != 2 || request.inputs.iter().any(|i| matches!(i.pixels, InputPixels::NoPixelsRequired)) {
        return Err(Error::MissingInput);
    }
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
