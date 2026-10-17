//! Explicit caller-authorized CMS/configured graph candidate; no qualified stock
//! dispatcher, ICC default, locator lookup, or runtime filesystem authority.
use super::{
    gimp_configured_candidate::{SourceBinding, bind_source_node_with_sample_policy},
    gimp_configured_color_candidate::{
        TransformPair, render_color_candidate_with_requirements, render_prepared_color_candidate_with_requirements, requirements_color_candidate,
        requirements_preparation_work,
    },
    gimp_configured_render_candidate::OutputPolicy,
};
use photocraft_algo::gimp::{
    configured::{CurveSamplePolicy, Limits, Transfer},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{FilterNode, GraphVersionOrigin},
};
use photocraft_geom::Rect;
use photocraft_raster::RasterInputError;

pub struct Binding<'a> {
    pub node: GraphNodeId,
    pub version_origin: GraphVersionOrigin,
    pub source_node: &'a FilterNode,
    pub source_binding: SourceBinding<'a>,
    pub sampling: CurveSamplePolicy,
    /// Native source and final output domain. Intermediate working F64 tiles
    /// are private and do not imply a graph domain conversion or ICC alias.
    pub source_domain: ExecutionDomain,
    pub source_extent: Rect,
    pub transforms: &'a TransformPair<'a>,
    pub working_default: &'a [u8],
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub output_policy: OutputPolicy,
}
fn ports(binding: &Binding<'_>) -> bool {
    [binding.input_port, binding.upstream_output_port, binding.output_port].iter().all(|port| !port.is_empty() && port.len() <= 256 && !port.contains('\0'))
}
fn encoding(transfer: Transfer) -> SampleEncoding {
    match transfer {
        Transfer::Linear => SampleEncoding::Linear,
        Transfer::NonlinearProfile => SampleEncoding::NonlinearProfile,
        Transfer::Perceptual => SampleEncoding::Perceptual,
    }
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    ports(binding)
        && match direction {
            PortDirection::Input => port == binding.input_port,
            PortDirection::Output => port == binding.output_port,
        }
}
pub fn footprint(binding: &Binding<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if !ports(binding)
        || binding.node.0 == 0
        || request.node != binding.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
        || request.output.is_empty()
        || request.output.intersect(&binding.source_extent) != request.output
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let work_per_sample = binding.transforms.work_per_pixel.checked_add(16384).ok_or(GraphRoiError::WorkLimit)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample })
}
/// Exact typed source/config + native domain + paired transform binding. All
/// visible owners/private candidate scratch are precharged before execution;
/// private Transform caches/fingerprints remain explicitly caller-bound in L0.
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    bind_source_node_with_sample_policy(binding.source_node, binding.source_binding, binding.sampling, budget.interrupt()).map_err(raster)?;
    if !ports(binding)
        || binding.node.0 == 0
        || request.node.id != binding.node
        || request.node.version_origin != binding.version_origin
        || request.node.operation != binding.source_node.operation
        || request.node.arguments != binding.source_node.arguments
        || request.inputs.len() != 1
        || request.domain != binding.source_domain
        || request.roi.is_empty()
        || request.roi.intersect(&binding.source_extent) != request.roi
    {
        return Err(GraphExecuteError::Domain);
    }
    let transforms = binding.transforms;
    if binding.source_domain.profile_fingerprint != Some(transforms.source.profile_identity)
        || binding.source_domain.encoding != encoding(transforms.source.transfer)
        || binding.source_binding.working != transforms.working
    {
        return Err(GraphExecuteError::Domain);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != binding.source_domain
        || input.surface.format() != binding.source_domain.format
        || input.roi != request.roi
    {
        return Err(GraphExecuteError::Domain);
    }
    if let Some(plan) = input.footprint
        && (plan.output() != request.roi
            || plan.support() != request.roi
            || plan.source_read() != request.roi
            || plan.source_extent() != binding.source_extent
            || plan.abyss() != Abyss::None)
    {
        return Err(GraphExecuteError::Roi);
    }
    budget.charge_work(requirements_preparation_work(input.surface, transforms).map_err(raster)?)?;
    let required = requirements_color_candidate(input.surface, request.roi, transforms, budget.interrupt()).map_err(raster)?;
    if binding.working_default.len() != required.working_format.bytes_per_pixel() {
        return Err(GraphExecuteError::Domain);
    }
    let output = required.native_output_bytes.checked_add(256).ok_or(GraphExecuteError::ResidentLimit)?;
    let scratch = required.peak_resident_bytes.checked_sub(required.native_output_bytes).ok_or(GraphExecuteError::Reservation)?;
    // Upstream source is already debited. It is conservatively retained again
    // here; reservations never refund upstream or raise the scheduler limits.
    budget.reserve_output(output)?;
    budget.reserve_index(required.index_bytes)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(required.work_units)?;
    let rendered = render_color_candidate_with_requirements(
        binding.source_node,
        binding.source_binding,
        binding.sampling,
        input.surface,
        binding.source_extent,
        request.roi,
        transforms.source,
        transforms,
        binding.working_default,
        binding.output_policy,
        Limits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units },
        budget.interrupt(),
        Some(required),
    )
    .map_err(raster)?;
    budget.poll()?;
    budget.finish_output(rendered.surface, rendered.roi, binding.source_domain)
}
/// Configured CMS evaluation requires real pixels; materialized bridge retains
/// the scheduler's named NoPixelsRequired refusal and conversion-vector charges.
pub fn evaluate_available(binding: &Binding<'_>, request: AvailableNodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}

/// Prepared typed configuration and caller-authorized paired transforms.
/// The validator must bind current graph arguments to the original typed
/// configuration/sample policy. Its arbitrary Rust work is caller-prepaid.
pub struct PreparedBinding<'a, 'config> {
    pub node: GraphNodeId,
    pub identity: &'a photocraft_doc::xcf_filters::OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub prepared: &'a photocraft_algo::gimp::configured::PreparedConfiguration<'config>,
    pub source_domain: ExecutionDomain,
    pub source_extent: Rect,
    pub transforms: &'a TransformPair<'a>,
    pub working_default: &'a [u8],
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub output_policy: OutputPolicy,
    pub validate_arguments: &'a dyn Fn(
        &photocraft_doc::xcf_filters::GraphNode,
        &photocraft_algo::gimp::configured::PreparedConfiguration<'config>,
    ) -> Result<(), GraphExecuteError>,
    pub validation_bytes: u64,
    pub validation_work: u64,
}
fn prepared_ports(binding: &PreparedBinding<'_, '_>) -> bool {
    !binding.identity.name.is_empty()
        && binding.identity.name.len() <= 256
        && !binding.identity.stored_version.is_empty()
        && binding.identity.stored_version.len() <= 64
        && !binding.identity.schema_epoch.is_empty()
        && binding.identity.schema_epoch.len() <= 256
        && [binding.input_port, binding.upstream_output_port, binding.output_port]
            .iter()
            .all(|port| !port.is_empty() && port.len() <= 256 && !port.contains('\0'))
}
pub fn accepts_prepared_port(binding: &PreparedBinding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    prepared_ports(binding)
        && match direction {
            PortDirection::Input => port == binding.input_port,
            PortDirection::Output => port == binding.output_port,
        }
}
pub fn footprint_prepared(binding: &PreparedBinding<'_, '_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if !prepared_ports(binding)
        || binding.node.0 == 0
        || request.node != binding.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
        || request.output.is_empty()
        || request.output.intersect(&binding.source_extent) != request.output
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    let work_per_sample = binding.transforms.work_per_pixel.checked_add(16384).ok_or(GraphRoiError::WorkLimit)?;
    Ok(InputFootprint { mapped_output: request.output, source_extent: binding.source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample })
}
pub fn evaluate_prepared(
    binding: &PreparedBinding<'_, '_>,
    request: NodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if !prepared_ports(binding)
        || binding.node.0 == 0
        || request.node.id != binding.node
        || request.node.version_origin != binding.version_origin
        || request.node.operation != *binding.identity
        || request.inputs.len() != 1
        || request.domain != binding.source_domain
        || request.roi.is_empty()
        || request.roi.intersect(&binding.source_extent) != request.roi
    {
        return Err(GraphExecuteError::Domain);
    }
    let config_charge = binding.prepared.retained_bytes().checked_add(binding.validation_bytes).ok_or(GraphExecuteError::ResidentLimit)?;
    budget.reserve_scratch(config_charge)?;
    budget.charge_work(binding.validation_work)?;
    (binding.validate_arguments)(request.node, binding.prepared)?;
    budget.poll()?;
    let transforms = binding.transforms;
    if binding.source_domain.profile_fingerprint != Some(transforms.source.profile_identity)
        || binding.source_domain.encoding != encoding(transforms.source.transfer)
        || binding.prepared.working_context() != transforms.working
    {
        return Err(GraphExecuteError::Domain);
    }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != binding.source_domain
        || input.surface.format() != binding.source_domain.format
        || input.roi != request.roi
    {
        return Err(GraphExecuteError::Domain);
    }
    if let Some(plan) = input.footprint
        && (plan.output() != request.roi
            || plan.support() != request.roi
            || plan.source_read() != request.roi
            || plan.source_extent() != binding.source_extent
            || plan.abyss() != Abyss::None)
    {
        return Err(GraphExecuteError::Roi);
    }
    budget.charge_work(requirements_preparation_work(input.surface, transforms).map_err(raster)?)?;
    let required = requirements_color_candidate(input.surface, request.roi, transforms, budget.interrupt()).map_err(raster)?;
    if binding.working_default.len() != required.working_format.bytes_per_pixel() {
        return Err(GraphExecuteError::Domain);
    }
    let output = required.native_output_bytes.checked_add(256).ok_or(GraphExecuteError::ResidentLimit)?;
    let scratch = required.peak_resident_bytes.checked_sub(required.native_output_bytes).ok_or(GraphExecuteError::Reservation)?;
    // Upstream source is already debited. It is conservatively retained again
    // here; reservations never refund upstream or raise the scheduler limits.
    budget.reserve_output(output)?;
    budget.reserve_index(required.index_bytes)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(required.work_units)?;
    let total_peak = required.peak_resident_bytes.checked_add(binding.prepared.retained_bytes()).ok_or(GraphExecuteError::ResidentLimit)?;
    if total_peak.checked_add(binding.validation_bytes).is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(GraphExecuteError::ResidentLimit);
    }
    let rendered = render_prepared_color_candidate_with_requirements(
        binding.prepared,
        input.surface,
        binding.source_extent,
        request.roi,
        transforms.source,
        transforms,
        binding.working_default,
        binding.output_policy,
        Limits { resident_bytes: total_peak, work_units: required.work_units },
        budget.interrupt(),
        Some(required),
    )
    .map_err(raster)?;
    budget.poll()?;
    budget.finish_output(rendered.surface, rendered.roi, binding.source_domain)
}
/// Real source pixels are mandatory; the shared bridge rejects NoPixelsRequired.
pub fn evaluate_prepared_available(
    binding: &PreparedBinding<'_, '_>,
    request: AvailableNodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate_prepared(binding, request, budget))
}

#[path = "gimp_configured_color_graph_candidate/profile_owned.rs"]
mod profile_owned;
pub use profile_owned::{
    ProfileOwnedBinding, ProfileOwnedBindingReceipt, ProfileOwnedParameters, accepts_profile_owned_port, evaluate_profile_owned,
    evaluate_profile_owned_available, footprint_profile_owned, prepare_profile_owned_binding,
};
