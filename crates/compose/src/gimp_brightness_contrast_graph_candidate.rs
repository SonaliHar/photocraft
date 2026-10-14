//! Retained GIMP wrapper graph callback to a DISTINCT analytic Float hypothesis.
//! No GEGL affine alias, source-exact/default enrollment, ICC/TRC or format alias.
//! The enclosing scheduler must retain and debit Prepared's complete lease/profile/
//! original-argument backing before planning (including pruned nodes), and charge
//! constructor work once. This callback never recreates or refunds that provider.
//! RGB/Gray colors change inside ROI; raw alpha (even NaN), defaults and edges
//! remain exact. F64 colors deliberately enter/finalize Float32, then widen.
//! StagedSurfaceBuilder's bounded prepaid BTreeMap/Arc publication is a standard
//! allocator boundary, not a fallible-allocation or process-RSS guarantee.
use crate::{
    gimp_brightness_contrast_candidate::{Prepared, config_bits},
    gimp_composite::CompositionError,
};
use photocraft_algo::gimp::wide_point::WorkingContext;
use photocraft_algo::gimp::{
    graph_execute::{ExecutionDomain, GraphExecuteError as E, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
};
use photocraft_color::{PixelFormat, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{GraphNode, GraphVersionOrigin},
};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R, Surface, checked_region_byte_len,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use std::mem::size_of;
pub struct Binding<'a, 'p, L> {
    pub node: &'a GraphNode,
    pub version_origin: GraphVersionOrigin,
    pub prepared: &'a Prepared<'p, L>,
    pub context: WorkingContext,
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
fn comp(e: CompositionError) -> E {
    match e {
        CompositionError::Raster(R::Cancelled) => E::Cancelled,
        CompositionError::Raster(R::LimitExceeded) => E::WorkLimit,
        CompositionError::Raster(r) => E::Raster(r),
        _ => op("GIMP brightness/contrast candidate evaluation refused"),
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output"))
}
fn domain(domain: ExecutionDomain, context: WorkingContext) -> Result<(), E> {
    if domain.encoding != SampleEncoding::NonlinearProfile
        || domain.profile_fingerprint != Some(context.profile_identity)
        || context.profile_identity == [0; 32]
    {
        return Err(E::Domain);
    }
    Ok(())
}
fn input<'a, L>(b: &Binding<'_, '_, L>, r: &NodeRequest<'a>) -> Result<&'a Surface, E> {
    let original = b.prepared.original_node();
    if b.node.id.0 == 0
        || !std::ptr::eq(b.node, r.node)
        || b.version_origin != r.node.version_origin
        || b.prepared.context() != b.context
        || r.node.operation.name != original.operation.name
        || r.node.operation.stored_version != original.operation.stored_version
        || r.node.operation.schema_epoch != original.operation.schema_epoch
        || original.id.0 == 0
        || r.node.arguments.len() != 1
        || b.domain.format != b.prepared.format()
    {
        return Err(op("exact prepared graph node/arguments/context witness"));
    }
    if config_bits(&r.node.arguments).map_err(comp)? != b.prepared.scalar_bits() {
        return Err(op("exact GIMP native configuration words changed"));
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
        return Err(op("one exact fixed-ROI declared nonlinear Float input/output"));
    }
    Ok(i.surface)
}
fn enclosure(roi: Rect, format: PixelFormat) -> Result<(Rect, u64), E> {
    checked_region_byte_len(roi, format).map_err(E::Raster)?;
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
    checked_region_byte_len(bounds, format).map_err(E::Raster)?;
    Ok((bounds, count))
}
/// Allocation-free request census. Live source storage is already deducted by
/// the scheduler; its traversal cost and all NEW builder owners are priced here.
pub fn requirements<L>(binding: &Binding<'_, '_, L>, request: &NodeRequest<'_>, c: &Interrupt<'_>) -> Result<Requirements, E> {
    poll(c)?;
    let source = input(binding, request)?;
    let (enclosure, tiles) = enclosure(request.roi, source.format())?;
    let bpp = source.format().bytes_per_pixel() as u64;
    if source.tile_count() as u64 > tiles || source.default_bytes().len() as u64 != bpp {
        return Err(E::Domain);
    }
    let tile_bytes = mul(mul(TILE_SIZE as u64, TILE_SIZE as u64)?, bpp)?;
    let cells = mul(tiles, 16)?;
    let bitmap = add(cells, 7)? / 8;
    let output_bytes = add(mul(tiles, add(tile_bytes, 256)?)?, add(256, bpp)?)?;
    let index_bytes = add(mul(tiles, size_of::<Option<Vec<u8>>>() as u64)?, add(bitmap, 1024)?)?;
    let staging_bytes = add(mul(tiles, add(tile_bytes, 256 + size_of::<Option<Vec<u8>>>() as u64)?)?, add(bitmap, bpp)?)?;
    // One64px encoded cell plus read/write/provider stack slack. Vec is fallible
    // with exact capacity; builder reconciles its own actual capacities.
    let scratch_bytes = add(mul(64 * 64, bpp)?, 4096)?;
    let whole_bytes = checked_region_byte_len(enclosure, source.format()).map_err(E::Raster)? as u64;
    let staging_work = add(mul(whole_bytes, 4)?, add(mul(cells, 256)?, add(bitmap, add(tiles, 16)?)?)?)?;
    let sample_work = binding.prepared.requirements().sample_work_units;
    let kernel_work = mul(mul(mul(request.roi.size().area(), source.format().mode.color_channels() as u64)?, sample_work)?, 2)?;
    let validation = add(mul(request.roi.size().area(), 64)?, mul(source.tile_count() as u64, 256)?)?;
    let work_units = add(kernel_work, add(staging_work, validation)?)?;
    if kernel_work > binding.prepared.requirements().remaining_work_units {
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
fn validate_pixels<L>(source: &Surface, roi: Rect, prepared: &Prepared<'_, L>, actual: WorkingContext, c: &Interrupt<'_>) -> Result<(), E> {
    source.try_validate_tile_origins_with(c).map_err(E::Raster)?;
    for (coord, _) in source.tiles() {
        poll(c)?;
        if coord.rect().intersect(&roi).is_empty() {
            return Err(op("unrelated outside-ROI source tile"));
        }
    }
    let format = source.format();
    let mut raw = [0u8; 40];
    let pixel = raw.get_mut(..format.bytes_per_pixel()).ok_or(E::Domain)?;
    for y in roi.y0..roi.y1 {
        poll(c)?;
        for x in roi.x0..roi.x1 {
            if x.abs_diff(roi.x0).is_multiple_of(256) {
                poll(c)?
            }
            source.try_read_pixel_bytes(x, y, pixel).map_err(E::Raster)?;
            for channel in 0..format.mode.color_channels() {
                let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| E::Raster(R::InvalidSample))?;
                // Full scalar preflight catches finite HDR -> Float overflow and output
                // overflow before allocation. Both passes consume genuine prepaid work.
                prepared.evaluate_color_with_interrupt(value, actual, c).map_err(comp)?;
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
pub fn evaluate<L>(binding: &Binding<'_, '_, L>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, E> {
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
    validate_pixels(source, request.roi, binding.prepared, binding.context, c)?;
    if q.kernel_work / 2 > binding.prepared.requirements().remaining_work_units {
        return Err(E::WorkLimit);
    }
    budget.reserve_index(q.index_bytes)?;
    budget.reserve_output(q.output_bytes)?;
    budget.reserve_scratch(q.scratch_bytes)?;
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let cell_bytes = 64usize.checked_mul(64).and_then(|v| v.checked_mul(bpp)).ok_or(E::ResidentLimit)?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(cell_bytes).map_err(|_| E::Allocation)?;
    if raw.capacity() != cell_bytes {
        return Err(E::Allocation);
    }
    raw.resize(cell_bytes, 0);
    let mut stage_budget = BuilderBudget { bytes: q.staging_bytes, work: q.staging_work, reserved: false };
    let mut builder = StagedSurfaceBuilder::new(source.format(), source.default_bytes(), q.enclosure, &mut stage_budget, c).map_err(E::Raster)?;
    let mut kernel_left = q.kernel_work / 2;
    let sample_work = binding.prepared.requirements().sample_work_units;
    for y in (q.enclosure.y0..q.enclosure.y1).step_by(64) {
        for x in (q.enclosure.x0..q.enclosure.x1).step_by(64) {
            poll(c)?;
            let cell = Rect::new(x, y, x.checked_add(64).ok_or(E::Roi)?, y.checked_add(64).ok_or(E::Roi)?);
            source.try_read_interleaved_into_with(cell, &mut raw, c).map_err(E::Raster)?;
            for (row, pixels) in raw.chunks_exact_mut(64 * bpp).enumerate() {
                poll(c)?;
                let py = y.checked_add(i32::try_from(row).map_err(|_| E::Roi)?).ok_or(E::Roi)?;
                for (column, pixel) in pixels.chunks_exact_mut(bpp).enumerate() {
                    let px = x.checked_add(i32::try_from(column).map_err(|_| E::Roi)?).ok_or(E::Roi)?;
                    if px < request.roi.x0 || px >= request.roi.x1 || py < request.roi.y0 || py >= request.roi.y1 {
                        continue;
                    }
                    for channel in 0..format.mode.color_channels() {
                        let input = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| E::Raster(R::InvalidSample))?;
                        kernel_left = kernel_left.checked_sub(sample_work).ok_or(E::WorkLimit)?;
                        let output = binding.prepared.evaluate_color_with_interrupt(input, binding.context, c).map_err(comp)?;
                        try_write_sample_f64(pixel, format.sample, channel, output).map_err(|_| E::Raster(R::InvalidSample))?;
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
    fn full_native_edge_tile_not_only_roi_is_priced() {
        let (r, n) = enclosure(
            Rect::new(-1, 2, 1, 3),
            photocraft_color::PixelFormat { sample: photocraft_color::SampleType::F64, ..photocraft_color::PixelFormat::RGBA32F },
        )
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(r, Rect::new(-256, 0, 256, 256));
    }
}
