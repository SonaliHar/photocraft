//! Prepared global-statistics graph bridge. No equalization/pixel mapping is
//! inferred from a histogram; output semantics are an explicit caller callback.
use super::{
    global_histogram::{self as histogram, Specification, Transparency},
    global_histogram_distribution::{self as cdf, Distribution},
    graph_execute::{self, AvailableNodeRequest, ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use std::sync::Arc;
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::WorkLimit)
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn hist_error(error: histogram::Error) -> GraphExecuteError {
    match error {
        histogram::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Operation("histogram specification/sample refusal"),
    }
}
fn cdf_error(error: cdf::Error) -> GraphExecuteError {
    match error {
        cdf::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Operation("histogram distribution schema/count refusal"),
    }
}
/// Cumulative preparation ledger. Work and reservations are not refunded on
/// failure; retained statistics must stay charged while their owner is alive.
pub struct PreparationBudget {
    pub resident_bytes: u64,
    pub work_units: u64,
}
pub struct PreparedGlobalStatistics<'a> {
    source: &'a Surface,
    source_identity: [u8; 32],
    source_domain: ExecutionDomain,
    specifications: &'a [Specification],
    distribution: Distribution,
    retained_bytes: u64,
}
impl<'a> PreparedGlobalStatistics<'a> {
    pub fn distribution(&self) -> &Distribution {
        &self.distribution
    }
    pub fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
    /// One global scan/build, independent of subsequent output ROI callbacks.
    /// Caller supplies an immutable authoritative source and revision identity.
    // Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        source: &'a Surface,
        source_identity: [u8; 32],
        domain: ExecutionDomain,
        extent: Rect,
        specifications: &'a [Specification],
        transparency: Transparency,
        budget: &mut PreparationBudget,
        control: &Interrupt<'_>,
    ) -> Result<Self, GraphExecuteError> {
        control.check().map_err(|_| GraphExecuteError::Cancelled)?;
        if !matches!(domain.format.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale)
            || source.format() != domain.format
            || domain.profile_fingerprint.is_none()
            || extent.is_empty()
            || specifications.is_empty()
            || specifications.len() > 4
            || budget.resident_bytes > MAX_CHECKED_RASTER_BYTES
        {
            return Err(GraphExecuteError::Domain);
        }
        let mut bins = 0u64;
        for spec in specifications {
            control.check().map_err(|_| GraphExecuteError::Cancelled)?;
            if spec.bins == 0 || spec.bins > histogram::MAX_BINS {
                return Err(GraphExecuteError::Operation("histogram bin bound"));
            }
            bins = add(bins, spec.bins as u64)?;
        }
        let retained = add(mul(bins, 8)?, 4096)?;
        // Counts + cumulative output + producer tile scratch simultaneously live.
        let tile = mul(65536, domain.format.bytes_per_pixel() as u64)?;
        let source_upper = add(mul(source.tile_count() as u64, add(tile, 256)?)?, domain.format.bytes_per_pixel() as u64)?;
        let peak = add(source_upper, add(mul(retained, 8)?, add(mul(tile, 2)?, 1024)?)?)?;
        let work = add(histogram::required_work(source, extent).map_err(hist_error)?, add(mul(bins, 32)?, 4096)?)?;
        if peak > MAX_CHECKED_RASTER_BYTES {
            return Err(GraphExecuteError::ResidentLimit);
        }
        budget.resident_bytes = budget.resident_bytes.checked_sub(peak).ok_or(GraphExecuteError::ResidentLimit)?;
        budget.work_units = budget.work_units.checked_sub(work).ok_or(GraphExecuteError::WorkLimit)?;
        let counts =
            histogram::try_histogram(source, extent, specifications, transparency, histogram::Limits { resident_bytes: peak, work_units: work }, control)
                .map_err(hist_error)?;
        let distribution = cdf::try_build(
            &counts,
            cdf::Binding { extent, sample: domain.format.sample, transparency, specifications },
            cdf::Limits { resident_bytes: peak, work_units: work },
            control,
        )
        .map_err(cdf_error)?;
        control.check().map_err(|_| GraphExecuteError::Cancelled)?;
        Ok(Self { source, source_identity, source_domain: domain, specifications, distribution, retained_bytes: retained })
    }
}
#[derive(Clone, Copy)]
pub struct OutputRequirements {
    pub output_bytes: u64,
    pub scratch_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
pub struct RenderRequest<'a> {
    pub statistics: &'a Distribution,
    pub source: &'a Surface,
    pub roi: Rect,
    pub domain: ExecutionDomain,
}
pub struct Binding<'a, 'source> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub source_identity: [u8; 32],
    pub statistics: &'a PreparedGlobalStatistics<'source>,
    pub output_domain: ExecutionDomain,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    /// Explicit typed config/domain validation; never inferred from name alone.
    pub validate_arguments: &'a dyn Fn(&GraphNode, &Distribution) -> Result<(), GraphExecuteError>,
    /// Must be bounded/allocation-free, or prepaid by the enclosing caller.
    pub requirements: &'a dyn Fn(Rect) -> Result<OutputRequirements, GraphExecuteError>,
    /// The caller chooses actual statistics-to-native-pixel math. All output
    /// growth must fit the precharged requirements; no implicit equalization.
    pub render: &'a dyn Fn(RenderRequest<'_>, OutputRequirements, &Interrupt<'_>) -> Result<Surface, GraphExecuteError>,
}
fn valid(binding: &Binding<'_, '_>) -> bool {
    binding.node.0 != 0
        && !binding.identity.name.is_empty()
        && binding.identity.name.len() <= 256
        && !binding.identity.stored_version.is_empty()
        && binding.identity.stored_version.len() <= 64
        && !binding.identity.schema_epoch.is_empty()
        && binding.identity.schema_epoch.len() <= 256
        && [binding.input_port, binding.upstream_output_port, binding.output_port]
            .iter()
            .all(|port| !port.is_empty() && port.len() <= 256 && !port.contains('\0'))
}
pub fn accepts_port(binding: &Binding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    valid(binding)
        && match direction {
            PortDirection::Input => port == binding.input_port,
            PortDirection::Output => port == binding.output_port,
        }
}
pub fn footprint(binding: &Binding<'_, '_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if !valid(binding)
        || request.node != binding.node
        || request.edge.to != binding.node
        || request.edge.input_port != binding.input_port
        || request.edge.output_port != binding.upstream_output_port
        || request.output.is_empty()
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    Ok(InputFootprint {
        mapped_output: binding.statistics.distribution.extent(),
        source_extent: binding.statistics.distribution.extent(),
        halo: Halo::default(),
        abyss: Abyss::None,
        work_per_sample: 64,
    })
}
fn same_storage(prepared: &Surface, current: &Surface, budget: &mut NodeBudget<'_, '_>) -> Result<bool, GraphExecuteError> {
    if prepared.format() != current.format() || prepared.tile_count() != current.tile_count() || prepared.default_bytes() != current.default_bytes() {
        return Ok(false);
    }
    budget.charge_work(mul(prepared.tile_count() as u64, 4)?)?;
    for ((a, ta), (b, tb)) in prepared.tiles().zip(current.tiles()) {
        budget.poll()?;
        if a != b || !Arc::ptr_eq(ta, tb) {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    let prepared = binding.statistics;
    if !valid(binding)
        || binding.node != request.node.id
        || *binding.identity != request.node.operation
        || binding.version_origin != request.node.version_origin
        || request.node.arguments.len() > 64
        || request.domain != binding.output_domain
        || binding.output_domain.profile_fingerprint.is_none()
        || request.inputs.len() != 1
        || binding.source_identity != prepared.source_identity
        || request.roi.is_empty()
    {
        return Err(GraphExecuteError::Domain);
    }
    (binding.validate_arguments)(request.node, &prepared.distribution)?;
    budget.poll()?;
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    let extent = prepared.distribution.extent();
    if input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != prepared.source_domain
        || input.roi != extent
    {
        return Err(GraphExecuteError::Domain);
    }
    let plan = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if plan.output() != extent || plan.support() != extent || plan.source_read() != extent || plan.source_extent() != extent || plan.abyss() != Abyss::None {
        return Err(GraphExecuteError::Roi);
    }
    if !same_storage(prepared.source, input.surface, budget)? {
        return Err(GraphExecuteError::Operation("prepared histogram source storage changed; prepare an explicit new owner"));
    }
    let needs = (binding.requirements)(request.roi)?;
    if needs.output_bytes == 0 {
        return Err(GraphExecuteError::Reservation);
    }
    // Existing preparation and upstream debits are never refunded; retaining
    // statistics through this callback is conservatively charged again.
    let stats = add(prepared.retained_bytes, mul(prepared.specifications.len() as u64, std::mem::size_of::<Specification>() as u64)?)?;
    budget.reserve_output(needs.output_bytes)?;
    budget.reserve_index(needs.index_bytes)?;
    budget.reserve_scratch(add(needs.scratch_bytes, stats)?)?;
    budget.charge_work(needs.work_units)?;
    let output = (binding.render)(
        RenderRequest { statistics: &prepared.distribution, source: input.surface, roi: request.roi, domain: binding.output_domain },
        needs,
        budget.interrupt(),
    )?;
    budget.poll()?;
    budget.finish_output(output, request.roi, binding.output_domain)
}
pub fn evaluate_available(
    binding: &Binding<'_, '_>,
    request: AvailableNodeRequest<'_>,
    budget: &mut NodeBudget<'_, '_>,
) -> Result<NodeOutput, GraphExecuteError> {
    graph_execute::evaluate_materialized(request, budget, |request, budget| evaluate(binding, request, budget))
}
