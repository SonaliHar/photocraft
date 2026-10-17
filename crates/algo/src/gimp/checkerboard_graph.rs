//! Explicit zero-input checkerboard callback. Caller-native colors and exact
//! original configuration stay borrowed; StoredColor alone grants no conversion.
//! The enclosing scheduler holds/admit all original source/profile/graph backing.
use super::{
    generator_checkerboard as kernel,
    graph_execute::{ExecutionDomain, GraphExecuteError as E, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    wide_point::WorkingContext,
};
use photocraft_doc::xcf_filters::{ExactNumber, FilterValue, GraphNode};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R};
use std::sync::atomic::{AtomicU64, Ordering};

type Result<T> = std::result::Result<T, E>;
const PREPARE_WORK: u64 = 128;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(E::ResidentLimit)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| E::Cancelled)
}
fn raster(e: R) -> E {
    if e == R::Cancelled { E::Cancelled } else { E::Raster(e) }
}
fn lower_limits(l: Limits) -> Result<()> {
    let d = Limits::default();
    if l.resident_bytes == 0 || l.resident_bytes > d.resident_bytes || l.work_units > d.work_units { Err(E::RaisedLimit) } else { Ok(()) }
}

/// Mandatory bounded allocation-free validator must bind all six exact stored
/// values to these independently interpreted native colors/configuration. A
/// node/schema name or profile hash alone never supplies that interpretation.
pub trait Validator {
    fn validate(&self, node: &GraphNode, configuration: &kernel::Checkerboard) -> Result<()>;
}
impl<F> Validator for F
where
    F: Fn(&GraphNode, &kernel::Checkerboard) -> Result<()>,
{
    fn validate(&self, node: &GraphNode, configuration: &kernel::Checkerboard) -> Result<()> {
        self(node, configuration)
    }
}
pub struct PreparedCheckerboard<'a> {
    node: &'a GraphNode,
    configuration: &'a kernel::Checkerboard,
    context: WorkingContext,
    domain: ExecutionDomain,
    full_extent: Rect,
    validate: &'a dyn Validator,
    limits: Limits,
    retained_bytes: u64,
    remaining_work: AtomicU64,
}
/// Actual borrowed Vec/String capacities, with bounded exact storage shapes.
/// No payload copy, profile lookup, color conversion or input-derived indexing.
fn metadata(node: &GraphNode, c: &Interrupt<'_>) -> Result<u64> {
    if node.id.0 == 0
        || node.operation.name != "gegl:checkerboard"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch.is_empty()
        || node.operation.schema_epoch.len() > 256
        || node.operation.schema_epoch.contains('\0')
        || node.arguments.len() != 6
    {
        return Err(E::Operation("exact checkerboard identity/configuration required"));
    }
    let mut n = add(std::mem::size_of::<GraphNode>() as u64, node.operation.name.capacity() as u64)?;
    n = add(n, node.operation.stored_version.capacity() as u64)?;
    n = add(n, node.operation.schema_epoch.capacity() as u64)?;
    n = add(
        n,
        (node.arguments.capacity() as u64).checked_mul(std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>() as u64).ok_or(E::IndexLimit)?,
    )?;
    for a in &node.arguments {
        poll(c)?;
        if a.name.is_empty() || a.name.len() > 128 {
            return Err(E::Operation("checkerboard argument name"));
        }
        n = add(n, a.name.capacity() as u64)?;
        match &a.value {
            FilterValue::Number(ExactNumber::Signed { width: 32, value }) if i32::try_from(*value).is_ok() => {}
            FilterValue::StoredColor(v) => {
                if v.layout.is_empty()
                    || v.layout.len() > 256
                    || v.layout.contains('\0')
                    || v.samples.is_empty()
                    || v.samples.len() > 1024
                    || v.opaque_tail.len() > 1 << 20
                {
                    return Err(E::Operation("bounded checkerboard stored color"));
                }
                n = add(n, v.layout.capacity() as u64)?;
                n = add(n, v.samples.capacity() as u64)?;
                n = add(n, v.opaque_tail.capacity() as u64)?;
            }
            _ => return Err(E::Operation("checkerboard needs original Signed32/StoredColor plus explicit native validator")),
        }
    }
    Ok(n)
}
#[derive(Clone, Copy, Debug)]
pub struct PreparationRequirements {
    pub retained_bytes: u64,
    pub work_units: u64,
}
/// Bounded six-field allocation-free prospective census. This admits storage and
/// context only; the independent exact native-color validator remains mandatory.
pub fn preparation_requirements(
    node: &GraphNode,
    configuration: &kernel::Checkerboard,
    full_extent: Rect,
    context: WorkingContext,
    domain: ExecutionDomain,
    limits: Limits,
    c: &Interrupt<'_>,
) -> Result<PreparationRequirements> {
    poll(c)?;
    lower_limits(limits)?;
    if context.profile_identity == [0; 32]
        || domain.profile_fingerprint != Some(context.profile_identity)
        || domain.encoding != context.encoding
        || domain.encoding != configuration.encoding()
        || domain.format != configuration.format()
    {
        return Err(E::Domain);
    }
    // O(1) native/output arithmetic rejects hostile extent before any scan.
    kernel::requirements(configuration, full_extent).map_err(raster)?;
    let retained = add(add(metadata(node, c)?, std::mem::size_of::<PreparedCheckerboard<'_>>() as u64)?, std::mem::size_of::<kernel::Checkerboard>() as u64)?;
    let work = add(PREPARE_WORK, retained)?;
    if retained > limits.resident_bytes {
        return Err(E::ResidentLimit);
    }
    if work > limits.work_units {
        return Err(E::WorkLimit);
    }
    poll(c)?;
    Ok(PreparationRequirements { retained_bytes: retained, work_units: work })
}
impl<'a> PreparedCheckerboard<'a> {
    // Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        node: &'a GraphNode,
        configuration: &'a kernel::Checkerboard,
        full_extent: Rect,
        context: WorkingContext,
        domain: ExecutionDomain,
        validate: &'a dyn Validator,
        limits: Limits,
        c: &Interrupt<'_>,
    ) -> Result<Self> {
        let q = preparation_requirements(node, configuration, full_extent, context, domain, limits, c)?;
        validate.validate(node, configuration)?;
        poll(c)?;
        Ok(Self {
            node,
            configuration,
            context,
            domain,
            full_extent,
            validate,
            limits,
            retained_bytes: q.retained_bytes,
            remaining_work: AtomicU64::new(limits.work_units - q.work_units),
        })
    }
    pub fn configuration(&self) -> &'a kernel::Checkerboard {
        self.configuration
    }
    pub fn original_node(&self) -> &'a GraphNode {
        self.node
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
    pub fn domain(&self) -> ExecutionDomain {
        self.domain
    }
    pub fn full_extent(&self) -> Rect {
        self.full_extent
    }
    pub fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    direction == PortDirection::Output && port == "output"
}
/// Real NodeBudget callback. Caller planner also prices its own graph/ROI work;
/// this immutable owner prepays every attempted kernel/validation invocation.
/// Neither a fresh NodeBudget nor a failed/canceled call resets that quota.
pub fn evaluate(prepared: &PreparedCheckerboard<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput> {
    budget.poll()?;
    if !std::ptr::eq(prepared.node, request.node) || !request.inputs.is_empty() {
        return Err(E::InvalidPlan);
    }
    if request.domain != prepared.domain {
        return Err(E::Domain);
    }
    if !prepared.full_extent.contains_rect(&request.roi) {
        return Err(E::Roi);
    }
    let q = kernel::requirements(prepared.configuration, request.roi).map_err(raster)?;
    // The graph's retained Surface envelope has a conservative 256-byte header;
    // the standalone kernel prices the concrete Surface header instead.
    let output_retained = add(q.output_retained_bytes, 256u64.saturating_sub(std::mem::size_of::<photocraft_raster::Surface>() as u64))?;
    let work = add(q.work_units, add(PREPARE_WORK, prepared.retained_bytes)?)?;
    let scratch = add(q.scratch_bytes, prepared.retained_bytes)?;
    let peak = add(add(output_retained, q.output_index_bytes)?, scratch)?;
    if peak > prepared.limits.resident_bytes {
        return Err(E::ResidentLimit);
    }
    let left = budget.remaining();
    if work > left.work_units {
        return Err(E::WorkLimit);
    }
    if output_retained > left.cache_bytes {
        return Err(E::CacheLimit);
    }
    if q.output_index_bytes > left.index_bytes {
        return Err(E::IndexLimit);
    }
    if peak > left.resident_bytes {
        return Err(E::ResidentLimit);
    }
    // All attempts, including validator refusal/cancellation, keep their debit.
    prepared.remaining_work.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(work)).map_err(|_| E::WorkLimit)?;
    budget.charge_work(work)?;
    prepared.validate.validate(request.node, prepared.configuration)?;
    budget.poll()?;
    budget.reserve_output(output_retained)?;
    budget.reserve_index(q.output_index_bytes)?;
    budget.reserve_scratch(scratch)?;
    let surface = kernel::try_generate(
        prepared.configuration,
        request.roi,
        kernel::Limits { resident_bytes: q.peak_resident_bytes, work_units: q.work_units },
        budget.interrupt(),
    )
    .map_err(raster)?;
    budget.poll()?;
    budget.finish_output(surface, request.roi, request.domain)
}

#[cfg(test)]
#[path = "checkerboard_graph_tests.rs"]
mod tests;
