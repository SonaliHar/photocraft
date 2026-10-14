//! Once-prepared continuous extrema, then partial-ROI graph application.
//! Exact existing opaque RGBA32F stretch math; no histogram approximation,
//! source admission, CMS conversion or retained-epoch inference. This narrow
//! prepared graph contains one caller-qualified node and an immutable source.
use super::{
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    global_stretch_contrast::{self as kernel, BoolArgument, GlobalLimits, PreparedStretch, StretchContrast, StretchRequirements},
    graph_execute::{ExecutionDomain, ExecutionLimits, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_pipeline::{GraphPipelineError, execute_pipeline_with_roi},
    graph_plan::PortDirection,
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_color::PixelFormat;
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{FilterGraph, FilterValue, GraphNode},
};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn raster(e: RasterInputError) -> GraphExecuteError {
    if e == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(e) }
}
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    let axis = |first: i32, end: i32| -> Result<u64, GraphExecuteError> {
        let last = end.checked_sub(1).ok_or(GraphExecuteError::Roi)?;
        u64::try_from(i64::from(last.div_euclid(TILE_SIZE)) - i64::from(first.div_euclid(TILE_SIZE)) + 1).map_err(|_| GraphExecuteError::Roi)
    };
    mul(axis(rect.x0, rect.x1)?, axis(rect.y0, rect.y1)?)
}
/// Borrowed AST and source backing remain caller-held. No owned AST copy,
/// user-provided generic lease or unconstrained render callback is hidden here.
pub struct PreparedGraph<'source> {
    graph: &'source FilterGraph,
    node: &'source GraphNode,
    prepared: PreparedStretch<'source>,
    domain: ExecutionDomain,
    limits: ExecutionLimits,
    header_bytes: u64,
    preparation_work: u64,
    remaining_work: std::sync::atomic::AtomicU64,
}
impl<'source> PreparedGraph<'source> {
    /// Required callback binds actual retained epoch/version-origin/source schema
    /// independently. Its arbitrary captures/work are enclosing-caller prepaid.
    /// Construction reduces the whole authoritative extent exactly once.
    pub fn prepare(
        source: &'source Surface,
        graph: &'source FilterGraph,
        extent: Rect,
        domain: ExecutionDomain,
        limits: ExecutionLimits,
        ctl: &Interrupt<'_>,
        validate_arguments: &dyn Fn(&GraphNode) -> Result<(), GraphExecuteError>,
    ) -> Result<Self, GraphExecuteError> {
        ctl.check().map_err(|_| GraphExecuteError::Cancelled)?;
        let standard = ExecutionLimits::default();
        if limits.resident_bytes > standard.resident_bytes
            || limits.index_bytes > standard.index_bytes
            || limits.cache_bytes > standard.cache_bytes
            || limits.work_units > standard.work_units
        {
            return Err(GraphExecuteError::RaisedLimit);
        }
        if graph.nodes.len() != 1
            || graph.edges.len() != 1
            || graph.input.is_none()
            || domain.format != PixelFormat::RGBA32F
            || domain.encoding != SampleEncoding::Linear
            || domain.profile_fingerprint.is_none_or(|h| h == [0; 32])
            || source.format() != domain.format
        {
            return Err(GraphExecuteError::Domain);
        }
        let node = graph.nodes.first().ok_or(GraphExecuteError::InvalidPlan)?;
        let edge = graph.edges.first().ok_or(GraphExecuteError::InvalidPlan)?;
        if node.id.0 == 0
            || graph.output != node.id
            || graph.input != Some(edge.from)
            || edge.from == node.id
            || edge.from.0 == 0
            || edge.to != node.id
            || edge.input_port != "input"
            || edge.output_port != "output"
            || node.operation.schema_epoch.is_empty()
            || node.operation.schema_epoch.len() > 256
            || node.arguments.len() != 2
        {
            return Err(GraphExecuteError::InvalidPlan);
        }
        let mut args = [BoolArgument { name: "", value: false }; 2];
        for (dst, arg) in args.iter_mut().zip(&node.arguments) {
            if arg.name.len() > 32 {
                return Err(GraphExecuteError::InvalidPlan);
            }
            let FilterValue::Bool(value) = arg.value else {
                return Err(GraphExecuteError::Operation("stretch requires exact typed Boolean parameters"));
            };
            *dst = BoolArgument { name: &arg.name, value };
        }
        let config = StretchContrast::from_named_booleans(&node.operation.name, &node.operation.stored_version, &args).map_err(raster)?;
        let header_bytes = std::mem::size_of::<Self>() as u64;
        let needs = kernel::preparation_requirements(source, extent).map_err(raster)?;
        let preparation_work = add(needs.work_units, 256)?;
        if add(needs.peak_resident_bytes, header_bytes)? > limits.resident_bytes
            || add(needs.index_bytes, header_bytes)? > limits.index_bytes
            || preparation_work > limits.work_units
        {
            return Err(GraphExecuteError::Reservation);
        }
        // Callback precedes no source scan/allocation, after its known typed shape
        // and full producer/header prospective envelope have been admitted.
        validate_arguments(node)?;
        ctl.check().map_err(|_| GraphExecuteError::Cancelled)?;
        let prepared = PreparedStretch::prepare(
            source,
            extent,
            config,
            GlobalLimits {
                resident_bytes: limits.resident_bytes.checked_sub(header_bytes).ok_or(GraphExecuteError::ResidentLimit)?,
                work_units: limits.work_units.checked_sub(256).ok_or(GraphExecuteError::WorkLimit)?,
            },
            ctl,
        )
        .map_err(raster)?;
        Ok(Self {
            graph,
            node,
            prepared,
            domain,
            limits,
            header_bytes,
            preparation_work,
            remaining_work: std::sync::atomic::AtomicU64::new(limits.work_units - preparation_work),
        })
    }
    pub fn requirements(&self) -> StretchRequirements {
        self.prepared.requirements()
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn footprint(&self, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
        if request.node != self.node.id
            || request.edge != self.graph.edges.first().ok_or(GraphRoiError::InvalidPlan)?
            || request.output.is_empty()
            || !self.prepared.extent().contains_rect(&request.output)
        {
            return Err(GraphRoiError::UnsupportedFootprint);
        }
        Ok(InputFootprint {
            mapped_output: self.prepared.extent(),
            source_extent: self.prepared.extent(),
            halo: Halo::default(),
            abyss: Abyss::None,
            work_per_sample: 1,
        })
    }
    fn evaluate(&self, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
        budget.poll()?;
        if request.node != self.node || request.domain != self.domain || request.inputs.len() != 1 {
            return Err(GraphExecuteError::Domain);
        }
        let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
        let extent = self.prepared.extent();
        if !std::ptr::eq(input.surface, self.prepared.source())
            || input.domain != self.domain
            || input.input_port != "input"
            || input.output_port != "output"
            || input.roi != extent
        {
            return Err(GraphExecuteError::Domain);
        }
        let witness = input.footprint.ok_or(GraphExecuteError::Roi)?;
        if witness.output() != extent
            || witness.support() != extent
            || witness.source_extent() != extent
            || witness.source_read() != extent
            || witness.abyss() != Abyss::None
        {
            return Err(GraphExecuteError::Roi);
        }
        let needs = self.prepared.application_requirements(request.roi).map_err(raster)?;
        let bpp = 16u64;
        let payload = 65536 * bpp;
        let count = tiles(request.roi)?;
        let all = add(input.surface.tile_count() as u64, count)?;
        let filtered = add(mul(all, add(payload, 256)?)?, bpp)?;
        let copied = checked_region_byte_len(request.roi, self.domain.format).map_err(raster)? as u64;
        let output = add(add(mul(count, add(payload, 256)?)?, bpp)?, 256)?;
        let trim_peak = add(add(filtered, copied)?, add(mul(count, add(payload, 256)?)?, bpp * 2)?)?;
        let trim_work = add(add(mul(copied, 2)?, mul(count, payload)?)?, add(mul(all, 4)?, count)?)?;
        let index = add(needs.index_bytes, mul(add(all, count)?, 1024)?)?;
        let scratch = add(needs.peak_resident_bytes, trim_peak)?;
        if add(add(output, index)?, scratch)? > MAX_CHECKED_RASTER_BYTES {
            return Err(GraphExecuteError::ResidentLimit);
        }
        budget.reserve_output(output)?;
        budget.reserve_index(index)?;
        budget.reserve_scratch(scratch)?;
        budget.charge_work(add(needs.work_units, trim_work)?)?;
        let filtered = self
            .prepared
            .apply_with_limits(request.roi, GlobalLimits { resident_bytes: needs.peak_resident_bytes, work_units: needs.work_units }, budget.interrupt())
            .map_err(raster)?;
        let edges = [f64::from(request.roi.x0), f64::from(request.roi.y0), f64::from(request.roi.x1), f64::from(request.roi.y1)];
        let output = try_crop_raw(
            &filtered,
            extent,
            edges,
            CropOrigin::KeepCoordinates,
            CropLimits { resident_bytes: trim_peak, work_units: trim_work },
            budget.interrupt(),
        )
        .map_err(raster)?;
        drop(filtered);
        budget.poll()?;
        budget.finish_output(output.surface, request.roi, self.domain)
    }
    /// Automatic held preparation debit; callers cannot forget it or substitute
    /// another graph/source. Work is also atomically cumulative in the prepared
    /// kernel across repeated/concurrent ROIs; cancellation never returns credit.
    pub fn execute(&self, roi: Rect, limits: ExecutionLimits, ctl: &Interrupt<'_>) -> Result<super::graph_execute::GraphOutput<'source>, GraphPipelineError> {
        let error = GraphPipelineError::Execute;
        if limits.resident_bytes > self.limits.resident_bytes
            || limits.cache_bytes > self.limits.cache_bytes
            || limits.index_bytes > self.limits.index_bytes
            || limits.work_units > self.limits.work_units
        {
            return Err(error(GraphExecuteError::RaisedLimit));
        }
        ctl.check().map_err(|_| error(GraphExecuteError::Cancelled))?;
        let needs = self.prepared.application_requirements(roi).map_err(|e| error(raster(e)))?;
        let count = tiles(roi).map_err(error)?;
        let all = add(self.prepared.source().tile_count() as u64, count).map_err(error)?;
        let copied = checked_region_byte_len(roi, self.domain.format).map_err(|e| error(raster(e)))? as u64;
        let trim_work = add(
            add(mul(copied, 2).map_err(error)?, mul(count, 65536 * 16).map_err(error)?).map_err(error)?,
            add(mul(all, 4).map_err(error)?, count).map_err(error)?,
        )
        .map_err(error)?;
        // Both the final request and merged node output charge the ROI; both the
        // incoming footprint and merged external input charge the whole extent.
        // Each zero-halo RGBA declaration costs 2*area*4 sample work units.
        // The fixed200k additionally covers both bounded structural planners.
        // Complete prepayment precedes even planner allocations.
        let regions = mul(add(roi.size().area(), self.prepared.extent().size().area()).map_err(error)?, 16).map_err(error)?;
        // Reopened scheduler accounting: graph_execute debits these node samples
        // before the callback, independently of declarations and kernel work.
        let scheduler_samples = mul(roi.size().area(), self.domain.format.channels() as u64).map_err(error)?;
        let run_work = add(
            add(add(add(200_000, regions).map_err(error)?, scheduler_samples).map_err(error)?, add(needs.work_units, trim_work).map_err(error)?)
                .map_err(error)?,
            mul(self.prepared.source().tile_count() as u64, 256).map_err(error)?,
        )
        .map_err(error)?;
        if run_work > limits.work_units.checked_sub(self.preparation_work).ok_or(error(GraphExecuteError::WorkLimit))? {
            return Err(error(GraphExecuteError::WorkLimit));
        }
        self.remaining_work
            .try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |left| left.checked_sub(run_work))
            .map_err(|_| error(GraphExecuteError::WorkLimit))?;
        let prepared = self.prepared.requirements();
        let held = add(prepared.retained_bytes, self.header_bytes).map_err(error)?;
        let index = add(prepared.index_bytes, self.header_bytes).map_err(error)?;
        let limits = ExecutionLimits {
            resident_bytes: limits.resident_bytes.checked_sub(held).ok_or(error(GraphExecuteError::ResidentLimit))?,
            index_bytes: limits.index_bytes.checked_sub(index).ok_or(error(GraphExecuteError::IndexLimit))?,
            work_units: run_work,
            ..limits
        };
        execute_pipeline_with_roi(
            self.graph,
            1,
            Some(self.prepared.source()),
            roi,
            self.domain,
            limits,
            ctl,
            |id, direction, port| match direction {
                PortDirection::Input => id == self.node.id && port == "input",
                PortDirection::Output => (id == self.node.id || self.graph.input == Some(id)) && port == "output",
            },
            |request| self.footprint(request),
            |request, budget| self.evaluate(request, budget),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        item_identity::GraphNodeId,
        xcf_filters::{FilterArgument, GraphEdge, OperationIdentity},
    };
    #[test]
    fn global_extrema_once_multiple_partial_rois_match_exact_existing_equations() {
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity {
                    name: "gegl:stretch-contrast".into(),
                    stored_version: "0:0".into(),
                    schema_epoch: "own-explicit-working-fixture".into(),
                },
                version_origin: Default::default(),
                arguments: ["keep-colors", "perceptual"]
                    .into_iter()
                    .map(|name| FilterArgument { name: name.into(), value: FilterValue::Bool(false) })
                    .collect(),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "output".into(), input_port: "input".into() }],
            output: GraphNodeId(1),
            original_pipeline: None,
        };
        let extent = Rect::new(0, 0, 3, 1);
        let raw = [0f32, 0.125, 0.25, 1., 0.25, 0.5, 0.5, 1., 0.5, 0.875, 0.75, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_from_interleaved(PixelFormat::RGBA32F, extent, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: PixelFormat::RGBA32F, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let limits = ExecutionLimits::default();
        let prepared = PreparedGraph::prepare(&source, &graph, extent, domain, limits, &Interrupt::NONE, &|_| Ok(())).unwrap();
        let initial = prepared.remaining_work();
        for (roi, color) in [(Rect::new(1, 0, 2, 1), 0.5f32), (Rect::new(2, 0, 3, 1), 1f32)] {
            let output = prepared.execute(roi, limits, &Interrupt::NONE).unwrap();
            assert_eq!(
                output.surface().try_read_interleaved(roi).unwrap(),
                [color, color, color, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>()
            );
            assert_eq!(output.surface().default_bytes(), source.default_bytes());
        }
        assert!(prepared.remaining_work() < initial);
        assert!(prepared.execute(Rect::new(1, 0, 2, 1), ExecutionLimits { work_units: 1, ..limits }, &Interrupt::NONE).is_err());
        assert_eq!(source, before);
        // This extent makes the formerly omitted declaration work exceed the
        // extra single-tile metadata allowance, exposing the short prepayment.
        let wide_extent = Rect::new(0, 0, 128, 1);
        let words = (0..128)
            .flat_map(|i| {
                let value = i as f32 / 127.;
                [value, value, value, 1.]
            })
            .flat_map(f32::to_ne_bytes)
            .collect::<Vec<_>>();
        let wide_source = Surface::try_from_interleaved(PixelFormat::RGBA32F, wide_extent, &words).unwrap();
        let wide_before = wide_source.clone();
        let wide = PreparedGraph::prepare(&wide_source, &graph, wide_extent, domain, limits, &Interrupt::NONE, &|_| Ok(())).unwrap();
        let output = wide.execute(Rect::new(64, 0, 65, 1), limits, &Interrupt::NONE).unwrap();
        let value = 64f32 / 127.;
        assert_eq!(
            output.surface().try_read_interleaved(Rect::new(64, 0, 65, 1)).unwrap(),
            [value, value, value, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>()
        );
        assert_eq!(wide_source, wide_before);
        // 512 scheduler samples exceed the 256 extra one-tile scan allowance;
        // admission must explicitly price them instead of accidentally masking it.
        let full = wide.execute(wide_extent, limits, &Interrupt::NONE).unwrap();
        assert_eq!(full.surface().try_read_interleaved(wide_extent).unwrap(), words);
    }
}
