//! Explicit already-owned auxiliary buffers for the graph scheduler.
//! A borrowed surface is supplied by the enclosing authorized transaction;
//! operation metadata and stored locators never create a read capability here.
//! The enclosing owner debits every held auxiliary buffer before supplying
//! scheduler limits, including buffers belonging to nodes that will not execute.
//! Callback reservations conservatively charge the active borrowed source again.
//! No CMS interpretation, source schema or stock identity is inferred.
use super::{
    graph_execute::{ExecutionDomain, ExecutionLimits, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
};
use photocraft_doc::xcf_filters::GraphNode;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;

pub struct Binding<'a> {
    /// Exact immutable graph node, not a reconstructed metadata match.
    pub node: &'a GraphNode,
    pub surface: &'a Surface,
    pub extent: Rect,
    pub domain: ExecutionDomain,
    pub output_port: &'a str,
    /// Bind the complete node configuration to this owned buffer/domain/extent.
    /// Must be bounded and allocation-free, or prepaid by the enclosing owner.
    pub validate_arguments: &'a dyn Fn(&GraphNode) -> Result<(), GraphExecuteError>,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub work_units: u64,
}
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn poll(control: &Interrupt<'_>) -> Result<(), GraphExecuteError> {
    control.check().map_err(|_| GraphExecuteError::Cancelled)
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn intersects(coordinate: photocraft_geom::TileCoord, roi: Rect) -> bool {
    let x = i64::from(coordinate.tx) * i64::from(TILE_SIZE);
    let y = i64::from(coordinate.ty) * i64::from(TILE_SIZE);
    x < i64::from(roi.x1) && x + i64::from(TILE_SIZE) > i64::from(roi.x0) && y < i64::from(roi.y1) && y + i64::from(TILE_SIZE) > i64::from(roi.y0)
}
pub fn accepts_port(binding: &Binding<'_>, direction: PortDirection, port: &str) -> bool {
    direction == PortDirection::Output && port == binding.output_port
}
/// Allocation-free metadata preflight. Output charges include only held native
/// tiles intersecting the ROI; actual output adopts those same shared tiles.
/// Shared source payload remains charged, without refunding scheduler inputs.
pub fn requirements(binding: &Binding<'_>, roi: Rect, control: &Interrupt<'_>) -> Result<Requirements, GraphExecuteError> {
    poll(control)?;
    if binding.node.id.0 == 0
        || binding.node.operation.name.is_empty()
        || binding.node.operation.name.len() > 256
        || binding.node.operation.stored_version.is_empty()
        || binding.node.operation.stored_version.len() > 64
        || binding.node.operation.schema_epoch.is_empty()
        || binding.node.operation.schema_epoch.len() > 256
        || binding.node.arguments.len() > 64
        || binding.output_port.is_empty()
        || binding.output_port.len() > 256
        || binding.output_port.contains('\0')
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if binding.domain.profile_fingerprint.is_none() || binding.surface.format() != binding.domain.format {
        return Err(GraphExecuteError::Domain);
    }
    checked_region_byte_len(binding.extent, binding.domain.format).map_err(raster)?;
    checked_region_byte_len(roi, binding.domain.format).map_err(raster)?;
    if binding.extent.is_empty() || roi.is_empty() || !binding.extent.contains_rect(&roi) {
        return Err(GraphExecuteError::Roi);
    }
    let bpp = binding.domain.format.bytes_per_pixel() as u64;
    if binding.surface.default_bytes().len() as u64 != bpp {
        return Err(GraphExecuteError::Domain);
    }
    let tiles = u64::try_from(binding.surface.tile_count()).map_err(|_| GraphExecuteError::IndexLimit)?;
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let source = add(add(256, bpp)?, mul(tiles, add(tile_bytes, 256)?)?)?;
    let standard = ExecutionLimits::default();
    if source > standard.resident_bytes {
        return Err(GraphExecuteError::ResidentLimit);
    }
    // Includes source metadata checks, repeated persistent-map insertion and
    // actual output adoption validation. No pixel decoding or copying occurs.
    let work = add(256, add(mul(tiles, 32)?, mul(mul(tiles, tiles)?, 4)?)?)?;
    if work > standard.work_units {
        return Err(GraphExecuteError::WorkLimit);
    }
    let mut selected = 0_u64;
    for (coordinate, _) in binding.surface.tiles() {
        poll(control)?;
        if intersects(*coordinate, roi) {
            selected = add(selected, 1)?;
        }
    }
    let index = add(1024, mul(selected, 512)?)?;
    let output = add(add(add(256, bpp)?, mul(selected, add(tile_bytes, 256)?)?)?, index)?;
    if add(source, output)? > standard.resident_bytes || output > standard.cache_bytes {
        return Err(GraphExecuteError::ResidentLimit);
    }
    if index > standard.index_bytes {
        return Err(GraphExecuteError::IndexLimit);
    }
    poll(control)?;
    Ok(Requirements { source_retained_bytes: source, output_retained_bytes: output, output_index_bytes: index, work_units: work })
}
pub fn evaluate(binding: &Binding<'_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if !std::ptr::eq(binding.node, request.node) || !request.inputs.is_empty() {
        return Err(GraphExecuteError::InvalidPlan);
    }
    if request.domain != binding.domain {
        return Err(GraphExecuteError::Domain);
    }
    let metadata_work =
        u64::try_from(binding.surface.tile_count()).map_err(|_| GraphExecuteError::WorkLimit)?.checked_mul(4).ok_or(GraphExecuteError::WorkLimit)?;
    budget.charge_work(metadata_work)?;
    let req = requirements(binding, request.roi, budget.interrupt())?;
    (binding.validate_arguments)(request.node)?;
    budget.poll()?;
    budget.reserve_output(req.output_retained_bytes)?;
    budget.reserve_index(req.output_index_bytes)?;
    budget.reserve_scratch(req.source_retained_bytes)?;
    budget.charge_work(req.work_units)?;
    binding.surface.try_validate_tile_origins_with(budget.interrupt()).map_err(raster)?;
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, binding.domain.format.bytes_per_pixel() as u64)?;
    let mut output = Surface::try_with_default_bytes(binding.domain.format, binding.surface.default_bytes()).map_err(raster)?;
    for (coordinate, tile) in binding.surface.tiles() {
        budget.poll()?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(GraphExecuteError::Domain);
        }
        if intersects(*coordinate, request.roi) {
            output.put_tiles(std::iter::once((*coordinate, Arc::clone(tile))));
        }
    }
    budget.poll()?;
    budget.finish_output(output, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::super::graph_pipeline::{GraphPipelineError, execute_pipeline};
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        SampleEncoding,
        item_identity::GraphNodeId,
        xcf_filters::{FilterGraph, OperationIdentity},
    };
    fn graph() -> FilterGraph {
        FilterGraph {
            input: None,
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity { name: "own:held-buffer".into(), stored_version: "own".into(), schema_epoch: "explicit-owned-context".into() },
                version_origin: Default::default(),
                arguments: vec![],
            }],
            edges: vec![],
            output: GraphNodeId(1),
            original_pipeline: None,
        }
    }
    #[test]
    fn owned_auxiliary_preserves_raw_nan_defaults_and_never_reads_a_locator() {
        let graph = graph();
        let raw = [1_u64, (-0_f64).to_bits(), 0x7ff8_1234_5678_9abc, 1_f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::Linear, profile_fingerprint: Some([4; 32]) };
        let validator = |node: &GraphNode| if node.arguments.is_empty() { Ok(()) } else { Err(GraphExecuteError::InvalidPlan) };
        let binding = Binding {
            node: &graph.nodes[0],
            surface: &source,
            extent: Rect::new(-10, -10, 10, 10),
            domain,
            output_port: "owned",
            validate_arguments: &validator,
        };
        let result = execute_pipeline(
            &graph,
            8,
            None,
            Rect::new(-1, -1, 1, 1),
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| accepts_port(&binding, direction, port),
            |request, budget| evaluate(&binding, request, budget),
        )
        .unwrap();
        assert_eq!(result.surface().default_bytes(), raw);
        assert_eq!(source, before);
    }
    #[test]
    fn missing_profile_and_budget_or_cancel_refuse_without_mutating_owned_input() {
        let graph = graph();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 4]).unwrap();
        let before = source.clone();
        let roi = Rect::new(0, 0, 1, 1);
        let domain = ExecutionDomain { format: source.format(), encoding: SampleEncoding::Linear, profile_fingerprint: None };
        let validator = |_: &GraphNode| Ok(());
        let mut binding = Binding { node: &graph.nodes[0], surface: &source, extent: roi, domain, output_port: "owned", validate_arguments: &validator };
        assert!(matches!(requirements(&binding, roi, &Interrupt::NONE), Err(GraphExecuteError::Domain)));
        binding.domain.profile_fingerprint = Some([4; 32]);
        assert!(matches!(requirements(&binding, roi, &Interrupt::cancel_only(&|| true)), Err(GraphExecuteError::Cancelled)));
        let denied = execute_pipeline(
            &graph,
            8,
            None,
            roi,
            binding.domain,
            ExecutionLimits { cache_bytes: 1, ..Default::default() },
            &Interrupt::NONE,
            |_, direction, port| accepts_port(&binding, direction, port),
            |request, budget| evaluate(&binding, request, budget),
        );
        assert!(matches!(denied, Err(GraphPipelineError::Execute(GraphExecuteError::CacheLimit))));
        assert_eq!(source, before);
    }
}
