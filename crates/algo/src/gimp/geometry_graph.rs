//! Explicit raw crop adapter for adaptive DAG execution. This is not serialized
//! GEGL admission: the caller binds geometry, exact ports and the current domain.
use super::{
    geometry_crop::{CropLimits, CropOrigin, try_crop_raw},
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
};
use photocraft_doc::item_identity::GraphNodeId;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct CropConfig<'a> {
    pub node: GraphNodeId,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub source_extent: Rect,
    pub crop: Rect,
    pub origin: CropOrigin,
    pub domain: ExecutionDomain,
}
fn contains(outer: Rect, inner: Rect) -> bool {
    inner.is_empty() || (inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1)
}
fn translate(rect: Rect, dx: i32, dy: i32) -> Result<Rect, RasterInputError> {
    Ok(Rect::new(rect.x0.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?, rect.y0.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?, rect.x1.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?, rect.y1.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?))
}
impl CropConfig<'_> {
    pub fn output_extent(&self) -> Result<Rect, RasterInputError> {
        if self.node.0 == 0 || self.input_port.is_empty() || self.upstream_output_port.is_empty() || self.input_port.len() > 1 << 20 || self.upstream_output_port.len() > 1 << 20 { return Err(RasterInputError::InvalidBounds); }
        checked_region_byte_len(self.source_extent, self.domain.format)?;
        checked_region_byte_len(self.crop, self.domain.format)?;
        if self.crop.is_empty() || !contains(self.source_extent, self.crop) { return Err(RasterInputError::InvalidBounds); }
        let output = match self.origin {
            CropOrigin::KeepCoordinates => self.crop,
            CropOrigin::RebaseToZero => translate(self.crop, self.crop.x0.checked_neg().ok_or(RasterInputError::InvalidBounds)?, self.crop.y0.checked_neg().ok_or(RasterInputError::InvalidBounds)?)?,
        };
        checked_region_byte_len(output, self.domain.format)?;
        Ok(output)
    }
    fn input_region(&self, requested: Rect) -> Result<Rect, RasterInputError> {
        let output = self.output_extent()?;
        checked_region_byte_len(requested, self.domain.format)?;
        if !contains(output, requested) { return Err(RasterInputError::InvalidBounds); }
        match self.origin { CropOrigin::KeepCoordinates => Ok(requested), CropOrigin::RebaseToZero => translate(requested, self.crop.x0, self.crop.y0) }
    }
}

/// Backward mapping for this exact caller-bound input only. No partial/external
/// crop, inferred abyss or port alias is accepted. Encoding/profile are opaque.
pub fn crop_roi_footprint(config: &CropConfig<'_>, request: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if request.node != config.node || request.edge.to != config.node || request.edge.input_port != config.input_port || request.edge.output_port != config.upstream_output_port { return Err(GraphRoiError::UnsupportedFootprint); }
    let mapped_output = config.input_region(request.output).map_err(GraphRoiError::from)?;
    Ok(InputFootprint { mapped_output, source_extent: config.source_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 2 })
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> { a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit) }
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> { a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit) }
fn tiles(rect: Rect) -> Result<u64, GraphExecuteError> {
    if rect.is_empty() { return Ok(0); }
    let span = |first: i32, last: i32| -> Result<u64, GraphExecuteError> {
        let count = i64::from(last.div_euclid(TILE_SIZE)).checked_sub(i64::from(first.div_euclid(TILE_SIZE))).and_then(|count| count.checked_add(1)).ok_or(GraphExecuteError::Roi)?;
        u64::try_from(count).map_err(|_| GraphExecuteError::Roi)
    };
    mul(span(rect.x0, rect.x1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?, span(rect.y0, rect.y1.checked_sub(1).ok_or(GraphExecuteError::Roi)?)?)
}

/// Required evaluator callback adapter. Allocations/work are reserved before the
/// raw kernel starts, with the scheduler's own interrupt passed through both raw
/// copies. The sole input producer must cover the exact mapped ROI. Arbitrary
/// source/operation/profile semantics still require independent caller admission.
pub fn execute_crop(config: &CropConfig<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if request.node.id != config.node || request.domain != config.domain || request.inputs.len() != 1 { return Err(GraphExecuteError::Domain); }
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.input_port != config.input_port || input.output_port != config.upstream_output_port || input.domain != config.domain || input.surface.format() != config.domain.format { return Err(GraphExecuteError::Domain); }
    let mapped = config.input_region(request.roi).map_err(GraphExecuteError::Raster)?;
    let footprint = input.footprint.ok_or(GraphExecuteError::Roi)?;
    if input.roi != mapped || footprint.output() != mapped || footprint.support() != mapped || footprint.source_read() != mapped || footprint.source_extent() != config.source_extent || footprint.abyss() != Abyss::None { return Err(GraphExecuteError::Roi); }
    let bpp = config.domain.format.bytes_per_pixel() as u64;
    let copied = checked_region_byte_len(request.roi, config.domain.format).map_err(GraphExecuteError::Raster)? as u64;
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let tile_charge = add(tile_bytes, 256)?;
    let output_tiles = tiles(request.roi)?;
    let kernel_extent = match config.origin {
        CropOrigin::KeepCoordinates => mapped,
        CropOrigin::RebaseToZero if mapped.is_empty() => request.roi,
        CropOrigin::RebaseToZero => translate(mapped, mapped.x0.checked_neg().ok_or(GraphExecuteError::Roi)?, mapped.y0.checked_neg().ok_or(GraphExecuteError::Roi)?).map_err(GraphExecuteError::Raster)?,
    };
    let kernel_tiles = tiles(kernel_extent)?;
    let source_tiles = u64::try_from(input.surface.tile_count()).map_err(|_| GraphExecuteError::ResidentLimit)?;
    let source_charge = add(add(256, bpp)?, mul(source_tiles, tile_charge)?)?;
    let output_charge = add(add(256, bpp)?, mul(output_tiles, tile_charge)?)?;
    let kernel_charge = add(add(256, bpp)?, mul(kernel_tiles, tile_charge)?)?;
    let relocate = kernel_extent != request.roi;
    // copied region buffers, COW/default bookkeeping and temporary cropped tiles.
    let scratch = add(add(mul(copied, 2)?, mul(bpp, 4)?)?, if relocate { kernel_charge } else { 256 })?;
    let kernel_peak = add(add(source_charge, kernel_charge)?, add(copied, mul(bpp, 2)?)?)?;
    if kernel_peak > MAX_CHECKED_RASTER_BYTES { return Err(GraphExecuteError::ResidentLimit); }
    let kernel_work = add(add(mul(copied, 2)?, mul(kernel_tiles, tile_bytes)?)?, add(mul(source_tiles, 4)?, tiles(mapped)?)?)?;
    let work = if relocate { add(kernel_work, add(mul(copied, 2)?, mul(output_tiles, tile_bytes)?)?)? } else { kernel_work };
    budget.reserve_output(output_charge)?;
    budget.reserve_scratch(scratch)?;
    // Byte visits conservatively dominate primitive channel-sample visits.
    budget.charge_work(work)?;
    if request.roi.is_empty() {
        let surface = Surface::try_with_default_bytes(config.domain.format, input.surface.default_bytes()).map_err(GraphExecuteError::Raster)?;
        return budget.finish_output(surface, request.roi, request.domain);
    }
    let edges = [f64::from(mapped.x0), f64::from(mapped.y0), f64::from(mapped.x1), f64::from(mapped.y1)];
    let cropped = try_crop_raw(input.surface, config.source_extent, edges, config.origin, CropLimits { resident_bytes: kernel_peak, work_units: kernel_work }, budget.interrupt()).map_err(GraphExecuteError::Raster)?;
    if cropped.extent != kernel_extent { return Err(GraphExecuteError::Roi); }
    let surface = if relocate {
        let mut shifted = Surface::try_with_default_bytes(config.domain.format, cropped.surface.default_bytes()).map_err(GraphExecuteError::Raster)?;
        if cropped.surface.tile_count() != 0 {
            shifted.try_copy_native_region_from(&cropped.surface, cropped.extent, request.roi, budget.interrupt()).map_err(GraphExecuteError::Raster)?;
        }
        shifted
    } else { cropped.surface };
    budget.poll()?;
    budget.finish_output(surface, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::{GraphPipelineError, execute_pipeline_with_roi}};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{sample_encoding::SampleEncoding, xcf_filters::{FilterGraph, GraphEdge, GraphNode, OperationIdentity}};
    use photocraft_raster::Interrupt;
    fn graph() -> FilterGraph {
        FilterGraph { input: Some(GraphNodeId(9)), nodes: vec![GraphNode { id: GraphNodeId(1), operation: OperationIdentity { name: "Own explicitly bound raw crop".into(), stored_version: "own".into(), schema_epoch: "own-test-not-gegl".into() }, version_origin: Default::default(), arguments: Vec::new() }], edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(1), output_port: "own-output".into(), input_port: "own-input".into() }], output: GraphNodeId(1), original_pipeline: None }
    }
    fn config(domain: ExecutionDomain, extent: Rect, origin: CropOrigin) -> CropConfig<'static> {
        CropConfig { node: GraphNodeId(1), input_port: "own-input", upstream_output_port: "own-output", source_extent: extent, crop: extent, origin, domain }
    }
    #[test]
    fn adaptive_partial_rebase_keeps_u32_low_bits_and_nonzero_requested_origin() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U32, alpha: false };
        let extent = Rect::new(-2, -1, 2, 0);
        let words = [1u32, 0xffff_fffe, 0x8000_0001, 2];
        let raw = words.into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let config = config(domain, extent, CropOrigin::RebaseToZero);
        let roi = Rect::new(1, 0, 3, 1);
        let output = execute_pipeline_with_roi(&graph, 1, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _, _| true, |request| crop_roi_footprint(&config, request), |request, budget| execute_crop(&config, request, budget)).unwrap();
        let expected = [0xffff_fffeu32, 0x8000_0001].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        assert_eq!(output.roi(), roi);
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), expected);
        assert_eq!(output.surface().default_bytes(), source.default_bytes());
        assert_eq!(source, before);
    }
    #[test]
    fn keep_coordinates_retains_f64_nan_signed_zero_and_sparse_default() {
        let graph = graph();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let raw = 0x7ff8_1234_5678_9abcu64.to_ne_bytes();
        let mut source = Surface::try_with_default_bytes(format, &raw).unwrap();
        let extent = Rect::new(-2, -1, 2, 1);
        source.try_write_interleaved_with(Rect::new(-1, 0, 1, 1), &[(-0.0_f64).to_ne_bytes(), 0x7ff8_0000_0000_0001u64.to_ne_bytes()].concat(), &Interrupt::NONE).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let config = config(domain, extent, CropOrigin::KeepCoordinates);
        let roi = Rect::new(-1, 0, 1, 1);
        let output = execute_pipeline_with_roi(&graph, 1, Some(&source), roi, domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _, _| true, |request| crop_roi_footprint(&config, request), |request, budget| execute_crop(&config, request, budget)).unwrap();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), source.try_read_interleaved(roi).unwrap());
        assert_eq!(output.surface().default_bytes(), raw);
        assert_eq!(source, before);
    }
    #[test]
    fn wrong_ports_domain_external_roi_and_late_cancel_leave_input_unchanged() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let graph = graph();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 4]).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::NonlinearProfile, profile_fingerprint: None };
        let config = config(domain, Rect::new(0, 0, 2, 2), CropOrigin::KeepCoordinates);
        let mut wrong = config;
        wrong.input_port = "input-alias";
        assert!(execute_pipeline_with_roi(&graph, 1, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _, _| true, |request| crop_roi_footprint(&wrong, request), |request, budget| execute_crop(&wrong, request, budget)).is_err());
        assert!(execute_pipeline_with_roi(&graph, 1, Some(&source), Rect::new(-1, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _, _| true, |request| crop_roi_footprint(&config, request), |request, budget| execute_crop(&config, request, budget)).is_err());
        let mut wrong_domain = config;
        wrong_domain.domain.encoding = SampleEncoding::Linear;
        assert!(execute_pipeline_with_roi(&graph, 1, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::NONE, |_, _, _| true, |request| crop_roi_footprint(&wrong_domain, request), |request, budget| execute_crop(&wrong_domain, request, budget)).is_err());
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::Relaxed);
        let result = execute_pipeline_with_roi(&graph, 1, Some(&source), Rect::new(0, 0, 1, 1), domain, ExecutionLimits::default(), &Interrupt::cancel_only(&check), |_, _, _| true, |request| crop_roi_footprint(&config, request), |request, budget| {
            let output = execute_crop(&config, request, budget)?;
            cancelled.store(true, Ordering::Relaxed);
            Ok(output)
        });
        assert!(matches!(result, Err(GraphPipelineError::Execute(GraphExecuteError::Cancelled))));
        assert_eq!(source, before);
    }
}
