//! Explicit retained false/false Stretch Contrast -> existing once-prepared graph.
//! Caller owns exact full linear opaque unit RGBA Float source/profile authority.
//! Canonical source and separately supplied graph IDs/topology remain borrowed;
//! no inferred profile conversion, mask/outer application or default admission.
//! Known metadata/native preparation and each execution envelope hold real caller
//! leases. The execution lease is a conservative selected lower-limit reservation,
//! not measured consumption/RSS; original/pruned owners remain enclosing-held.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
use photocraft_algo::gimp::{
    graph_execute::{ExecutionDomain, ExecutionLimits, GraphExecuteError, GraphOutput},
    graph_pipeline::GraphPipelineError,
    stretch_graph::PreparedGraph,
};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{FilterGraph, FilterNode, FilterValue, GraphVersionOrigin},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError as R, Surface};
pub const SCHEMA: &str = "22652a0a27c020d4792df713e3acb546953954d91663a63fb0057fde53dec838";
pub const EPOCH: &str = "gimp-3.2.6-windows-22652a0a27c020d4";
const HEADER: u64 = 4096;
const BIND_WORK: u64 = 256;
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub domain: ExecutionDomain,
    pub version_origin: GraphVersionOrigin,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub preparation_work_units: u64,
    metadata_bytes: u64,
}
pub struct Prepared<'a, L> {
    original: &'a FilterNode,
    source: &'a Surface,
    graph: &'a FilterGraph,
    binding: SourceBinding<'a>,
    extent: Rect,
    lower: PreparedGraph<'a>,
    requirements: Requirements,
    limits: ExecutionLimits,
    _lease: L,
}
pub struct Output<'p, 'a, P, L> {
    output: GraphOutput<'a>,
    _owner: &'p Prepared<'a, P>,
    _lease: L,
}
impl<P, L> Output<'_, '_, P, L> {
    pub fn surface(&self) -> &Surface {
        self.output.surface()
    }
    pub fn roi(&self) -> Rect {
        self.output.roi()
    }
    pub fn domain(&self) -> ExecutionDomain {
        self.output.domain()
    }
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| R::Cancelled.into())
}
fn limit() -> E {
    R::LimitExceeded.into()
}
fn bad(s: &'static str) -> E {
    E::Unsupported(s)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn lower(e: GraphExecuteError) -> E {
    match e {
        GraphExecuteError::Cancelled => R::Cancelled.into(),
        GraphExecuteError::Raster(r) => r.into(),
        GraphExecuteError::WorkLimit
        | GraphExecuteError::ResidentLimit
        | GraphExecuteError::IndexLimit
        | GraphExecuteError::CacheLimit
        | GraphExecuteError::Reservation
        | GraphExecuteError::RaisedLimit => limit(),
        _ => bad("existing closed Stretch graph refused"),
    }
}
fn pipeline(e: GraphPipelineError) -> E {
    match e {
        GraphPipelineError::Execute(e) => lower(e),
        GraphPipelineError::Plan(photocraft_algo::gimp::graph_plan::GraphPlanError::Cancelled)
        | GraphPipelineError::Roi(photocraft_algo::gimp::graph_roi::GraphRoiError::Cancelled) => R::Cancelled.into(),
        _ => bad("existing Stretch structural/ROI graph refused"),
    }
}
fn bind(n: &FilterNode, g: &FilterGraph, b: SourceBinding<'_>, c: &Interrupt<'_>) -> Result<()> {
    poll(c)?;
    if b.xcf_version != 22
        || b.schema_sha256 != SCHEMA
        || b.expected_epoch != EPOCH
        || n.id.0 == 0
        || n.operation.name != "gegl:stretch-contrast"
        || n.operation.stored_version != "0:0"
        || n.operation.schema_epoch != b.expected_epoch
        || !n.auxiliary.is_empty()
        || n.arguments.len() != 2
        || g.nodes.len() != 1
        || g.edges.len() != 1
        || b.domain.encoding != SampleEncoding::Linear
        || b.domain.profile_fingerprint.is_none_or(|v| v == [0; 32])
    {
        return Err(bad("exact false/false retained Stretch binding"));
    }
    let mut seen = 0u8;
    for arg in &n.arguments {
        let bit = match arg.name.as_str() {
            "keep-colors" => 1,
            "perceptual" => 2,
            _ => return Err(bad("known Stretch Boolean names")),
        };
        if seen & bit != 0 || arg.value != FilterValue::Bool(false) {
            return Err(bad("only explicit keep-colors=false/perceptual=false is supported"));
        }
        seen |= bit;
    }
    let node = g.nodes.first().ok_or_else(limit)?;
    if node.id.0 == 0
        || node.version_origin != b.version_origin
        || node.operation != n.operation
        || node.arguments.len() != 2
        || node.arguments.iter().zip(&n.arguments).any(|(a, o)| a != o)
    {
        return Err(bad("exact canonical original/graph association and declaration order"));
    }
    Ok(())
}
/// Allocation-free single-node metadata capacity census. Original source/outer
/// backing is not reconstructed or silently copied by this wrapper.
pub fn requirements(
    n: &FilterNode,
    g: &FilterGraph,
    source: &Surface,
    extent: Rect,
    b: SourceBinding<'_>,
    limits: ExecutionLimits,
    c: &Interrupt<'_>,
) -> Result<Requirements> {
    bind(n, g, b, c)?;
    let q = PreparedGraph::preparation_requirements(source, g, extent, b.domain, limits, c).map_err(lower)?;
    let mut metadata = HEADER;
    metadata = add(metadata, mul(g.nodes.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_filters::GraphNode>() as u64)?)?;
    metadata = add(metadata, mul(g.edges.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_filters::GraphEdge>() as u64)?)?;
    if let Some(text) = &g.original_pipeline {
        metadata = add(metadata, text.capacity() as u64)?;
    }
    for operation in [&n.operation, &g.nodes.first().ok_or_else(limit)?.operation] {
        metadata =
            add(metadata, add(operation.name.capacity() as u64, add(operation.stored_version.capacity() as u64, operation.schema_epoch.capacity() as u64)?)?)?;
    }
    for arguments in [&n.arguments, &g.nodes.first().ok_or_else(limit)?.arguments] {
        metadata = add(metadata, mul(arguments.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>() as u64)?)?;
        for a in arguments {
            poll(c)?;
            metadata = add(metadata, a.name.capacity() as u64)?;
        }
    }
    let edge = g.edges.first().ok_or_else(limit)?;
    metadata = add(metadata, add(edge.input_port.capacity() as u64, edge.output_port.capacity() as u64)?)?;
    let retained = add(metadata, q.retained_bytes)?;
    let peak = add(metadata, q.peak_resident_bytes)?;
    let index = add(metadata, q.index_bytes)?;
    let work = add(BIND_WORK, q.work_units)?;
    if peak > limits.resident_bytes || index > limits.index_bytes || work > limits.work_units {
        return Err(limit());
    }
    Ok(Requirements { retained_bytes: retained, peak_resident_bytes: peak, index_bytes: index, preparation_work_units: work, metadata_bytes: metadata })
}
pub fn prepare<'a, B: PreparationBudget>(
    n: &'a FilterNode,
    g: &'a FilterGraph,
    source: &'a Surface,
    extent: Rect,
    b: SourceBinding<'a>,
    limits: ExecutionLimits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<Prepared<'a, B::Lease>> {
    poll(c)?;
    if std::mem::size_of::<Prepared<'a, B::Lease>>() as u64 > HEADER {
        return Err(limit());
    }
    let q = requirements(n, g, source, extent, b, limits, c)?;
    budget.consume_preparation_work(q.preparation_work_units)?;
    let lease = budget.reserve_prepared_bytes(q.peak_resident_bytes)?;
    let lowered = ExecutionLimits {
        resident_bytes: limits.resident_bytes.checked_sub(q.metadata_bytes).ok_or_else(limit)?,
        index_bytes: limits.index_bytes.checked_sub(q.metadata_bytes).ok_or_else(limit)?,
        work_units: limits.work_units.checked_sub(BIND_WORK).ok_or_else(limit)?,
        ..limits
    };
    let kernel = PreparedGraph::prepare(source, g, extent, b.domain, lowered, c, &|node| {
        if !std::ptr::eq(node, g.nodes.first().ok_or(GraphExecuteError::InvalidPlan)?)
            || node.version_origin != b.version_origin
            || node.operation != n.operation
            || node.arguments.len() != 2
            || node.arguments.iter().zip(&n.arguments).any(|(a, o)| a != o)
        {
            return Err(GraphExecuteError::InvalidPlan);
        }
        Ok(())
    })
    .map_err(lower)?;
    poll(c)?;
    Ok(Prepared { original: n, source, graph: g, binding: b, extent, lower: kernel, requirements: q, limits, _lease: lease })
}
impl<'a, L> Prepared<'a, L> {
    pub fn original_node(&self) -> &'a FilterNode {
        self.original
    }
    pub fn original_source(&self) -> &'a Surface {
        self.source
    }
    pub fn graph(&self) -> &'a FilterGraph {
        self.graph
    }
    pub fn binding(&self) -> SourceBinding<'a> {
        self.binding
    }
    pub fn extent(&self) -> Rect {
        self.extent
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    pub fn remaining_work(&self) -> u64 {
        self.lower.remaining_work()
    }
    /// Shared caller work is consumed BEFORE planner allocation, even on failure.
    /// A conservative lower invocation resident envelope remains leased with output.
    pub fn execute<'p, B: PreparationBudget>(
        &'p self,
        roi: Rect,
        limits: ExecutionLimits,
        budget: &mut B,
        c: &Interrupt<'_>,
    ) -> Result<Output<'p, 'a, L, B::Lease>> {
        poll(c)?;
        if std::mem::size_of::<Output<'p, 'a, L, B::Lease>>() as u64 > HEADER {
            return Err(limit());
        }
        if limits.resident_bytes > self.limits.resident_bytes
            || limits.index_bytes > self.limits.index_bytes
            || limits.cache_bytes > self.limits.cache_bytes
            || limits.work_units > self.limits.work_units
        {
            return Err(limit());
        }
        let lower_limits = ExecutionLimits {
            resident_bytes: limits.resident_bytes.checked_sub(self.requirements.metadata_bytes).and_then(|v| v.checked_sub(HEADER)).ok_or_else(limit)?,
            index_bytes: limits.index_bytes.checked_sub(self.requirements.metadata_bytes).ok_or_else(limit)?,
            work_units: limits.work_units.checked_sub(BIND_WORK).ok_or_else(limit)?,
            ..limits
        };
        let work = self.lower.execution_work_requirements(roi, lower_limits, c).map_err(pipeline)?;
        budget.consume_preparation_work(work)?;
        // Only immutable retained source/model/header is shared with other live
        // outputs. Preparation scratch cannot be credited again per invocation.
        let additional = limits.resident_bytes.checked_sub(self.requirements.retained_bytes).ok_or_else(limit)?;
        let lease = budget.reserve_prepared_bytes(additional)?;
        let output = self.lower.execute(roi, lower_limits, c).map_err(pipeline)?;
        poll(c)?;
        Ok(Output { output, _owner: self, _lease: lease })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        item_identity::{FilterId, GraphNodeId},
        xcf_filters::{FilterArgument, GraphEdge, GraphNode, OperationIdentity},
    };
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
        deny: bool,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
            self.bytes = self.bytes.checked_add(n).ok_or_else(limit)?;
            if self.deny {
                return Err(limit());
            }
            Ok(())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            self.work = self.work.checked_add(n).ok_or_else(limit)?;
            Ok(())
        }
    }
    fn fixture() -> (FilterNode, FilterGraph, SourceBinding<'static>) {
        let args = vec![
            FilterArgument { name: "keep-colors".into(), value: FilterValue::Bool(false) },
            FilterArgument { name: "perceptual".into(), value: FilterValue::Bool(false) },
        ];
        let operation = OperationIdentity { name: "gegl:stretch-contrast".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() };
        let n = FilterNode {
            id: FilterId(1),
            operation: operation.clone(),
            source_display_name: None,
            source_icon: None,
            arguments: args.clone(),
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
        };
        let g = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode { id: GraphNodeId(10), operation, arguments: args, version_origin: GraphVersionOrigin::Stored }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(10), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(10),
            original_pipeline: None,
        };
        let b = SourceBinding {
            xcf_version: 22,
            expected_epoch: EPOCH,
            schema_sha256: SCHEMA,
            domain: ExecutionDomain { format: PixelFormat::RGBA32F, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) },
            version_origin: GraphVersionOrigin::Stored,
        };
        (n, g, b)
    }
    #[test]
    fn actual_lower_preparation_and_roi_work_are_debited_once_without_rescan() {
        let (n, g, b) = fixture();
        let mut source = Surface::new(PixelFormat::RGBA32F);
        let full = Rect::new(-1, 0, 1, 1);
        let raw: Vec<u8> = [0.25f32, 0.375, 0.5, 1., 0.75, 0.625, 1., 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        source.try_write_interleaved(full, &raw).unwrap();
        let limits = ExecutionLimits::default();
        let mut budget = Budget::default();
        let prepared = prepare(&n, &g, &source, full, b, limits, &mut budget, &Interrupt::NONE).unwrap();
        let before = prepared.remaining_work();
        let debit = budget.work;
        let output = prepared.execute(Rect::new(-1, 0, 0, 1), limits, &mut budget, &Interrupt::NONE).unwrap();
        let mut actual = [0u8; 16];
        output.surface().try_read_pixel_bytes(-1, 0, &mut actual).unwrap();
        assert_eq!(&actual[..12], &[0; 12]);
        assert_eq!(&actual[12..], &1f32.to_le_bytes());
        assert!(budget.work > debit);
        assert_eq!(before - prepared.remaining_work(), budget.work - debit);
        assert!(std::ptr::eq(prepared.original_source(), &source));
        assert_eq!(prepared.original_node(), &n);
        let mut original = [0u8; 32];
        source.try_read_interleaved_into_with(full, &mut original, &Interrupt::NONE).unwrap();
        assert_eq!(original.as_slice(), raw.as_slice());
    }
    #[test]
    fn simultaneous_outputs_each_hold_the_full_nonshared_run_envelope() {
        let (n, g, b) = fixture();
        let mut source = Surface::new(PixelFormat::RGBA32F);
        let full = Rect::new(-1, 0, 1, 1);
        let raw: Vec<u8> = [0.25f32, 0.375, 0.5, 1., 0.75, 0.625, 1., 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        source.try_write_interleaved(full, &raw).unwrap();
        let limits = ExecutionLimits::default();
        let mut budget = Budget::default();
        let prepared = prepare(&n, &g, &source, full, b, limits, &mut budget, &Interrupt::NONE).unwrap();
        let held = budget.bytes;
        let per_run = limits.resident_bytes - prepared.requirements().retained_bytes;
        let first = prepared.execute(Rect::new(-1, 0, 0, 1), limits, &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(budget.bytes, held + per_run);
        let second = prepared.execute(Rect::new(0, 0, 1, 1), limits, &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(budget.bytes, held + 2 * per_run);
        assert_eq!(first.roi(), Rect::new(-1, 0, 0, 1));
        assert_eq!(second.roi(), Rect::new(0, 0, 1, 1));
    }
    #[test]
    fn planner_and_roi_cancellation_remain_typed() {
        for error in [
            GraphPipelineError::Plan(photocraft_algo::gimp::graph_plan::GraphPlanError::Cancelled),
            GraphPipelineError::Roi(photocraft_algo::gimp::graph_roi::GraphRoiError::Cancelled),
        ] {
            assert!(matches!(pipeline(error), E::Raster(R::Cancelled)));
        }
    }
    #[test]
    fn keep_colors_and_wrong_graph_association_refuse_before_reservation() {
        let (mut n, mut g, b) = fixture();
        let source = Surface::new(PixelFormat::RGBA32F);
        n.arguments[0].value = FilterValue::Bool(true);
        g.nodes[0].arguments[0].value = FilterValue::Bool(true);
        let mut budget = Budget::default();
        assert!(prepare(&n, &g, &source, Rect::new(0, 0, 1, 1), b, ExecutionLimits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.work, 0);
        assert_eq!(budget.bytes, 0);
    }
    #[test]
    fn failed_lease_and_cancel_do_not_scan_or_publish_source() {
        let (n, g, b) = fixture();
        let source = Surface::new(PixelFormat::RGBA32F);
        let mut budget = Budget { deny: true, ..Budget::default() };
        assert!(prepare(&n, &g, &source, Rect::new(0, 0, 1, 1), b, ExecutionLimits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert!(budget.work > 0 && budget.bytes > 0);
        let stop = || true;
        let mut budget = Budget::default();
        assert!(matches!(
            prepare(&n, &g, &source, Rect::new(0, 0, 1, 1), b, ExecutionLimits::default(), &mut budget, &Interrupt::cancel_only(&stop)),
            Err(E::Raster(R::Cancelled))
        ));
        assert_eq!(budget.work, 0);
    }
}
