//! Explicit retained Vibrance Linear RGBA Float graph callback; no source/input conversion.
//! The enclosing scheduler must retain and debit PreparedSource/profile/full original
//! FilterNode and graph backing before planning (including pruned nodes), and charge
//! constructor work once. This callback never recreates or refunds that provider.
//! Only RGB words change. Alpha, default and edge-tile outside words stay raw.
//! StagedSurfaceBuilder's bounded prepaid BTreeMap/Arc publication is a standard
//! allocator boundary, not a fallible-allocation or process-RSS guarantee.
use crate::gimp_vibrance_prepared_candidate::PreparedSource;
use photocraft_algo::gimp::point_vibrance::Context;
use photocraft_algo::gimp::{
    graph_execute::{ExecutionDomain, GraphExecuteError as E, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
};
use photocraft_color::{PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterValue, GraphNode, GraphVersionOrigin},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R, Surface, checked_region_byte_len,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use std::mem::size_of;
pub struct Binding<'a, 'p> {
    pub node: &'a GraphNode,
    pub version_origin: GraphVersionOrigin,
    pub prepared: &'a PreparedSource<'p>,
    pub context: Context,
    pub domain: ExecutionDomain,
    pub full_extent: Rect,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
    staging_bytes: u64,
    staging_work: u64,
    kernel_work: u64,
    enclosure: Rect,
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::WorkLimit)
}
fn op(s: &'static str) -> E {
    E::Operation(s)
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output"))
}
fn domain(domain: ExecutionDomain, context: Context) -> Result<(), E> {
    if domain.format != PixelFormat::RGBA32F
        || domain.encoding != SampleEncoding::Linear
        || domain.profile_fingerprint != Some(context.rgb.profile_identity)
        || context.rgb.profile_identity == [0; 32]
    {
        return Err(E::Domain);
    }
    Ok(())
}
fn input<'a>(b: &Binding<'_, '_>, r: &NodeRequest<'a>) -> Result<&'a Surface, E> {
    let original = b.prepared.original_node();
    if b.node.id.0 == 0
        || !std::ptr::eq(b.node, r.node)
        || b.version_origin != r.node.version_origin
        || b.prepared.context() != b.context
        || r.node.operation.name != original.operation.name
        || r.node.operation.stored_version != original.operation.stored_version
        || r.node.operation.schema_epoch != original.operation.schema_epoch
        || r.node.arguments.len() != 2
        || original.arguments.len() != 2
    {
        return Err(op("exact prepared graph node/arguments/context witness"));
    }
    // Separate FilterId and GraphNodeId are never treated as the same identity.
    // Compare only the closed native-number schema, not arbitrary nested values.
    for (graph_arg, source_arg) in r.node.arguments.iter().zip(&original.arguments) {
        if graph_arg.name.len() > 64 || graph_arg.name != source_arg.name {
            return Err(op("retained Vibrance graph argument order/name"));
        }
        let (FilterValue::Number(ExactNumber::Float32Bits(a)), FilterValue::Number(ExactNumber::Float32Bits(b))) = (&graph_arg.value, &source_arg.value) else {
            return Err(op("retained Vibrance native Float32 arguments"));
        };
        if a != b {
            return Err(op("retained Vibrance scalar words changed"));
        }
    }
    domain(b.domain, b.context)?;
    if r.domain != b.domain || r.roi != b.full_extent || r.inputs.len() != 1 {
        return Err(E::Domain);
    }
    let i = r.inputs.first().ok_or(E::MissingInput)?;
    if !accepts_port(PortDirection::Input, i.input_port)
        || !accepts_port(PortDirection::Output, i.output_port)
        || i.footprint.is_some()
        || i.roi != r.roi
        || i.domain != r.domain
        || i.surface.format() != r.domain.format
    {
        return Err(op("one exact fixed-ROI Linear input/output"));
    }
    Ok(i.surface)
}
fn enclosure(roi: Rect) -> Result<(Rect, u64), E> {
    checked_region_byte_len(roi, PixelFormat::RGBA32F).map_err(E::Raster)?;
    if roi.is_empty() {
        return Err(E::Roi);
    }
    let a = TileCoord::containing(roi.x0, roi.y0);
    let b = TileCoord::containing(roi.x1.checked_sub(1).ok_or(E::Roi)?, roi.y1.checked_sub(1).ok_or(E::Roi)?);
    let count = mul(
        u64::try_from(i64::from(b.tx) - i64::from(a.tx) + 1).map_err(|_| E::Roi)?,
        u64::try_from(i64::from(b.ty) - i64::from(a.ty) + 1).map_err(|_| E::Roi)?,
    )?;
    let bounds = Rect::new(
        a.tx.checked_mul(TILE_SIZE).ok_or(E::Roi)?,
        a.ty.checked_mul(TILE_SIZE).ok_or(E::Roi)?,
        b.tx.checked_add(1).and_then(|v| v.checked_mul(TILE_SIZE)).ok_or(E::Roi)?,
        b.ty.checked_add(1).and_then(|v| v.checked_mul(TILE_SIZE)).ok_or(E::Roi)?,
    );
    checked_region_byte_len(bounds, PixelFormat::RGBA32F).map_err(E::Raster)?;
    Ok((bounds, count))
}
/// Allocation-free request census. Live source storage is already deducted by
/// the scheduler; its traversal cost and all NEW builder owners are priced here.
pub fn requirements(binding: &Binding<'_, '_>, request: &NodeRequest<'_>, c: &Interrupt<'_>) -> Result<Requirements, E> {
    poll(c)?;
    let source = input(binding, request)?;
    let (enclosure, tiles) = enclosure(request.roi)?;
    if source.tile_count() as u64 > tiles || source.default_bytes().len() != 16 {
        return Err(E::Domain);
    }
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, 16)?;
    let cells = mul(tiles, 16)?;
    let bitmap = add(cells, 7)? / 8;
    let output_bytes = add(mul(tiles, add(tile_bytes, 256)?)?, 272)?;
    let index_bytes = add(mul(tiles, size_of::<Option<Vec<u8>>>() as u64)?, add(bitmap, 1024)?)?;
    let staging_bytes = add(mul(tiles, add(tile_bytes, 256 + size_of::<Option<Vec<u8>>>() as u64)?)?, add(bitmap, 16)?)?;
    // One64px encoded cell plus read/write/provider stack slack. Vec is fallible
    // with exact capacity; builder reconciles its own actual capacities.
    let point = binding.prepared.graph_point_requirements(c).map_err(E::Raster)?;
    let scratch_bytes = add(64 * 64 * 16 + 4096, point.scratch_bytes)?;
    let whole_bytes = checked_region_byte_len(enclosure, source.format()).map_err(E::Raster)? as u64;
    let staging_work = add(mul(whole_bytes, 4)?, add(mul(cells, 256)?, add(bitmap, add(tiles, 16)?)?)?)?;
    let kernel_work = mul(request.roi.size().area(), point.pixel_work_units)?;
    let validation = add(mul(request.roi.size().area(), 64)?, mul(source.tile_count() as u64, 256)?)?;
    let work_units = add(kernel_work, add(staging_work, validation)?)?;
    if kernel_work > point.remaining_work_units {
        return Err(E::WorkLimit);
    }
    if add(add(output_bytes, index_bytes)?, scratch_bytes)? > MAX_CHECKED_RASTER_BYTES {
        return Err(E::ResidentLimit);
    }
    poll(c)?;
    Ok(Requirements { output_bytes, index_bytes, scratch_bytes, work_units, staging_bytes, staging_work, kernel_work, enclosure })
}
struct BuilderBudget {
    bytes: u64,
    work: u64,
    reserved: bool,
}
impl StagingBudget for BuilderBudget {
    type Lease = ();
    fn reserve(&mut self, n: u64) -> Result<(), R> {
        if self.reserved || n > self.bytes {
            return Err(R::LimitExceeded);
        }
        self.reserved = true;
        Ok(())
    }
    fn work(&mut self, n: u64) -> Result<(), R> {
        self.work = self.work.checked_sub(n).ok_or(R::LimitExceeded)?;
        Ok(())
    }
}
fn validate_pixels(source: &Surface, roi: Rect, c: &Interrupt<'_>) -> Result<(), E> {
    source.try_validate_tile_origins_with(c).map_err(E::Raster)?;
    for (coord, _) in source.tiles() {
        poll(c)?;
        if coord.rect().intersect(&roi).is_empty() {
            return Err(op("unrelated outside-ROI source tile"));
        }
    }
    for y in roi.y0..roi.y1 {
        poll(c)?;
        for x in roi.x0..roi.x1 {
            if x.abs_diff(roi.x0).is_multiple_of(256) {
                poll(c)?
            }
            let mut rgba = [0.; 4];
            source.try_read_pixel_f64(x, y, &mut rgba).map_err(E::Raster)?;
            if rgba.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
                return Err(op("finite unit Linear RGBA input"));
            }
        }
    }
    poll(c)
}
/// Concrete NodeBudget prepayment precedes all native output/scratch allocation.
/// Local adapters only consume those prepaid partitions; no refund or fresh max.
/// Prepared independently keeps its immutable cumulative evaluation work ceiling.
/// Its prospective check is not an exclusive reservation: concurrent exhaustion
/// still returns a private failure through per-pixel atomic admission.
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, E> {
    budget.poll()?;
    let c = budget.interrupt();
    let q = requirements(binding, &request, c)?;
    let source = input(binding, &request)?;
    let remaining = budget.remaining();
    if q.work_units > remaining.work_units {
        return Err(E::WorkLimit);
    }
    if q.index_bytes > remaining.index_bytes {
        return Err(E::IndexLimit);
    }
    if q.output_bytes > remaining.cache_bytes {
        return Err(E::CacheLimit);
    }
    if add(add(q.output_bytes, q.index_bytes)?, q.scratch_bytes)? > remaining.resident_bytes {
        return Err(E::ResidentLimit);
    }
    budget.charge_work(q.work_units)?;
    validate_pixels(source, request.roi, c)?;
    budget.reserve_index(q.index_bytes)?;
    budget.reserve_output(q.output_bytes)?;
    budget.reserve_scratch(q.scratch_bytes)?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(64 * 64 * 16).map_err(|_| E::Allocation)?;
    if raw.capacity() != 64 * 64 * 16 {
        return Err(E::Allocation);
    }
    raw.resize(64 * 64 * 16, 0);
    let mut stage_budget = BuilderBudget { bytes: q.staging_bytes, work: q.staging_work, reserved: false };
    let mut builder = StagedSurfaceBuilder::new(source.format(), source.default_bytes(), q.enclosure, &mut stage_budget, c).map_err(E::Raster)?;
    let mut kernel_left = q.kernel_work;
    let point = binding.prepared.graph_point_requirements(c).map_err(E::Raster)?;
    for y in (q.enclosure.y0..q.enclosure.y1).step_by(64) {
        for x in (q.enclosure.x0..q.enclosure.x1).step_by(64) {
            poll(c)?;
            let cell = Rect::new(x, y, x.checked_add(64).ok_or(E::Roi)?, y.checked_add(64).ok_or(E::Roi)?);
            source.try_read_interleaved_into_with(cell, &mut raw, c).map_err(E::Raster)?;
            for (row, pixels) in raw.chunks_exact_mut(64 * 16).enumerate() {
                poll(c)?;
                let py = y.checked_add(i32::try_from(row).map_err(|_| E::Roi)?).ok_or(E::Roi)?;
                for (column, pixel) in pixels.chunks_exact_mut(16).enumerate() {
                    let px = x.checked_add(i32::try_from(column).map_err(|_| E::Roi)?).ok_or(E::Roi)?;
                    if px < request.roi.x0 || px >= request.roi.x1 || py < request.roi.y0 || py >= request.roi.y1 {
                        continue;
                    }
                    let mut input = [0.; 4];
                    for (channel, value) in input.iter_mut().enumerate() {
                        *value = try_read_sample_f64(pixel, SampleType::F32, channel).map_err(|_| E::Raster(R::InvalidSample))?
                    }
                    kernel_left = kernel_left.checked_sub(point.pixel_work_units).ok_or(E::WorkLimit)?;
                    let native = input.map(|v| v as f32);
                    let output = binding.prepared.evaluate_graph_pixel_with_interrupt(native, binding.context, c).map_err(E::Raster)?;
                    if output[3].to_bits() != native[3].to_bits() {
                        return Err(op("Vibrance raw alpha changed"));
                    }
                    for (channel, value) in output.iter().take(3).enumerate() {
                        if !value.is_finite() {
                            return Err(E::Raster(R::InvalidSample));
                        }
                        try_write_sample_f64(pixel, SampleType::F32, channel, f64::from(*value)).map_err(|_| E::Raster(R::InvalidSample))?
                    }
                }
            }
            builder.push_region(cell, &raw, &mut stage_budget, c).map_err(E::Raster)?;
        }
    }
    let (surface, ()) = builder.finalize(&mut stage_budget, c).map_err(E::Raster)?.into_parts();
    if kernel_left != 0 {
        return Err(E::WorkLimit);
    }
    budget.finish_output(surface, request.roi, request.domain)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_edge_tile_enclosure_is_not_only_roi_bytes() {
        let (r, n) = enclosure(Rect::new(-1, 2, 1, 3)).unwrap();
        assert_eq!(n, 2);
        assert_eq!(r, Rect::new(-256, 0, 256, 256));
    }
    #[test]
    fn roi_nan_refuses_without_interpreting_unselected_default() {
        let format = PixelFormat::RGBA32F;
        let raw = [f32::NAN, -0., 0., 1.].into_iter().flat_map(f32::to_le_bytes).collect::<Vec<_>>();
        let mut source = Surface::try_with_default_bytes(format, &raw).unwrap();
        source.try_write_pixel_f64(0, 0, &[0.25, 0.5, 0.75, 0.5]).unwrap();
        assert!(validate_pixels(&source, Rect::new(0, 0, 1, 1), &Interrupt::NONE).is_ok());
        assert!(validate_pixels(&source, Rect::new(0, 0, 2, 1), &Interrupt::NONE).is_err());
        assert!(matches!(validate_pixels(&source, Rect::new(0, 0, 1, 1), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled)));
        assert_eq!(source.default_bytes(), raw.as_slice());
    }
}
