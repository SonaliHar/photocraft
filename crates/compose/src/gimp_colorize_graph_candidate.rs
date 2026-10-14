//! Once-prepared Colorize real NodeBudget callback, explicit nonlinear RGB
//! Float domain only. Full original source/constructor lease and all pruned original
//! backing must stay held/prepaid before planning; preparation is never repeated. Y weights and transfer role are explicit witnesses;
//! neither is inferred from the current ICC or stored HSL framing.
//! Exact caller-selected scalar versus stored-HSL policy is fenced independently
//! of both retained triplets and cannot be changed by property order.
//! Output represents its authoritative ROI and intersecting edge tiles only.
//! Defaults, alpha and outside-ROI words WITHIN those tiles remain raw; unrelated
//! original tiles stay in the held source, not this graph output. No source,
//! profile, mask, outer/default admission or unique GIMP arithmetic is claimed.
//! Builder map/Arc finalization is a bounded prepaid standard allocator boundary.
use crate::{gimp_colorize_candidate::Prepared, gimp_composite::CompositionError};
use photocraft_algo::gimp::wide_point::WorkingContext;
use photocraft_algo::gimp::{
    graph_execute::{ExecutionDomain, GraphExecuteError as E, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::{Endpoint, PortDirection},
    graph_roi::{FootprintRequest, GraphRoiError, InputFootprint},
    roi::{Abyss, Halo},
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
    pub source: &'a Surface,
    pub external_input: photocraft_doc::item_identity::GraphNodeId,
    pub version_origin: GraphVersionOrigin,
    pub prepared: &'a Prepared<'p, L>,
    pub context: WorkingContext,
    pub policy: crate::gimp_colorize_candidate::NumericPolicy,
    pub transfer: crate::gimp_colorize_candidate::TransferWitness,
    pub weights: crate::gimp_colorize_candidate::YWeights,
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
        _ => op("declared Colorize candidate evaluation refused"),
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
        || r.node.arguments.len() != 4
        || b.domain.format != b.prepared.format()
        || b.context != b.prepared.context()
        || b.policy != b.prepared.policy()
        || b.transfer != b.prepared.transfer()
        || b.weights != b.prepared.weights()
    {
        return Err(op("exact prepared graph node/arguments/context witness"));
    }
    if crate::gimp_colorize_candidate::stored_parameters_for_policy(&r.node.arguments, b.policy).map_err(comp)? != b.prepared.parameters()
        || r.node.arguments.iter().zip(&original.arguments).any(|(a, o)| a.name != o.name)
    {
        return Err(op("independent Colorize scalar/HSL words or declaration order changed"));
    }
    domain(b.domain, b.context)?;
    if r.domain != b.domain || !b.full_extent.contains_rect(&r.roi) || r.inputs.len() != 1 {
        return Err(E::Domain);
    }
    let i = r.inputs.first().ok_or(E::MissingInput)?;
    if !accepts_port(PortDirection::Input, i.input_port)
        || !accepts_port(PortDirection::Output, i.output_port)
        || i.endpoint != Endpoint::ExternalInput(b.external_input)
        || b.external_input.0 == 0
        || b.external_input == b.node.id
        || i.roi != b.full_extent
        || i.domain != r.domain
        || i.surface.format() != r.domain.format
        || !std::ptr::eq(i.surface, b.source)
    {
        return Err(op("one exact held-source nonlinear RGB Float dependency"));
    }
    let witness = i.footprint.ok_or(E::Roi)?;
    if witness.output() != b.full_extent
        || witness.support() != b.full_extent
        || witness.source_extent() != b.full_extent
        || witness.source_read() != b.full_extent
        || witness.abyss() != Abyss::None
    {
        return Err(E::Roi);
    }
    Ok(i.surface)
}
/// Explicit held-source dependency: the ROI planner retains/charges the
/// complete original input while this callback only stages requested output.
pub fn footprint<L>(b: &Binding<'_, '_, L>, r: FootprintRequest<'_>) -> Result<InputFootprint, GraphRoiError> {
    if r.node != b.node.id
        || r.edge.to != b.node.id
        || r.edge.from != b.external_input
        || b.external_input.0 == 0
        || b.external_input == b.node.id
        || r.edge.input_port != "input"
        || r.edge.output_port != "output"
        || r.output.is_empty()
        || !b.full_extent.contains_rect(&r.output)
    {
        return Err(GraphRoiError::UnsupportedFootprint);
    }
    Ok(InputFootprint { mapped_output: b.full_extent, source_extent: b.full_extent, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 1 })
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
    if source.default_bytes().len() as u64 != bpp {
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
    let kernel_work = mul(mul(request.roi.size().area(), sample_work)?, 2)?;
    let validation = mul(request.roi.size().area(), 64)?;
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
            let mut values = [0f32; 3];
            for (channel, value) in values.iter_mut().enumerate() {
                *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| E::Raster(R::InvalidSample))? as f32;
            }
            prepared.evaluate_rgb_with_interrupt(values, actual, prepared.transfer(), c).map_err(comp)?;
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
                    let mut values = [0f32; 3];
                    for (channel, value) in values.iter_mut().enumerate() {
                        *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| E::Raster(R::InvalidSample))? as f32;
                    }
                    kernel_left = kernel_left.checked_sub(sample_work).ok_or(E::WorkLimit)?;
                    let output = binding.prepared.evaluate_rgb_with_interrupt(values, binding.context, binding.transfer, c).map_err(comp)?;
                    for (channel, value) in output.into_iter().enumerate() {
                        try_write_sample_f64(pixel, format.sample, channel, f64::from(value)).map_err(|_| E::Raster(R::InvalidSample))?;
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
    fn real_partial_roi_preserves_raw_alpha_edge_default_and_fences_supplied_weights() {
        use crate::{gimp_colorize_candidate as source, gimp_render::PreparationBudget};
        use photocraft_algo::gimp::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline_with_roi};
        use photocraft_doc::{
            item_identity::{FilterId, GraphNodeId},
            xcf_filters::{FilterGraph, FilterNode, GraphEdge, GraphNode, OperationIdentity},
        };
        struct Prep;
        impl PreparationBudget for Prep {
            type Lease = ();
            fn reserve_prepared_bytes(&mut self, _: u64) -> crate::gimp_composite::Result<()> {
                Ok(())
            }
            fn consume_preparation_work(&mut self, _: u64) -> crate::gimp_composite::Result<()> {
                Ok(())
            }
        }
        let original = FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:colorize".into(), stored_version: "0:0".into(), schema_epoch: source::EPOCH.into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                photocraft_doc::xcf_filters::FilterArgument {
                    name: "hue".into(),
                    value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(0.25f32.to_bits())),
                },
                photocraft_doc::xcf_filters::FilterArgument {
                    name: "saturation".into(),
                    value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(0.5f32.to_bits())),
                },
                photocraft_doc::xcf_filters::FilterArgument {
                    name: "lightness".into(),
                    value: photocraft_doc::xcf_filters::FilterValue::Number(photocraft_doc::xcf_filters::ExactNumber::Float32Bits(0.0f32.to_bits())),
                },
                photocraft_doc::xcf_filters::FilterArgument {
                    name: "color".into(),
                    value: photocraft_doc::xcf_filters::FilterValue::StoredColor(photocraft_doc::xcf_filters::ExactColorStorage {
                        layout: "HSL float".into(),
                        samples: [0.125f32.to_bits(), 0.25f32.to_bits(), 0.75f32.to_bits()].into_iter().flat_map(u32::to_le_bytes).collect(),
                        opaque_tail: vec![],
                    }),
                },
            ],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        };
        let context = WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] };
        let policy = source::NumericPolicy::DeclaredScalarTripletBinary64FinalFloat;
        let transfer = source::TransferWitness { role: source::TransferRole::CallerEstablishedIecSrgb, profile_identity: context.profile_identity };
        let weights = source::YWeights::from_bits([0.25f64.to_bits(), 0.5f64.to_bits(), 0.25f64.to_bits()]).unwrap();
        let full = Rect::new(-1, 0, 1, 1);
        let roi = Rect::new(-1, 0, 0, 1);
        let mut pixels = Surface::new(PixelFormat::RGBA32F);
        let raw: Vec<u8> =
            [0.25f32.to_bits(), 0.25f32.to_bits(), 0.25f32.to_bits(), 0x7fc00042, 0.75f32.to_bits(), 0.375f32.to_bits(), 0.375f32.to_bits(), 0x80000000]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
        pixels.try_write_interleaved(full, &raw).unwrap();
        let edge: Vec<u8> = [0.125f32.to_bits(), 0.5f32.to_bits(), 0.25f32.to_bits(), 0x80000000].into_iter().flat_map(u32::to_le_bytes).collect();
        pixels.try_write_interleaved(Rect::new(-2, 0, -1, 1), &edge).unwrap();
        let prepared = source::prepare(
            &original,
            source::SourceBinding { xcf_version: 22, expected_epoch: source::EPOCH, schema_sha256: source::SCHEMA, context, policy, transfer, weights },
            PixelFormat::RGBA32F,
            source::Limits::default(),
            &mut Prep,
            &Interrupt::NONE,
        )
        .unwrap();
        let graph = FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![GraphNode {
                id: GraphNodeId(10),
                operation: original.operation.clone(),
                version_origin: GraphVersionOrigin::Stored,
                arguments: original.arguments.clone(),
            }],
            edges: vec![GraphEdge { from: GraphNodeId(9), to: GraphNodeId(10), input_port: "input".into(), output_port: "output".into() }],
            output: GraphNodeId(10),
            original_pipeline: None,
        };
        let domain = ExecutionDomain { format: PixelFormat::RGBA32F, encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) };
        let binding = Binding {
            node: &graph.nodes[0],
            source: &pixels,
            transfer,
            weights,
            external_input: GraphNodeId(9),
            version_origin: GraphVersionOrigin::Stored,
            prepared: &prepared,
            context,
            policy,
            domain,
            full_extent: full,
        };
        let before = prepared.requirements().remaining_work_units;
        let output = execute_pipeline_with_roi(
            &graph,
            2,
            Some(&pixels),
            roi,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, d, p| accepts_port(d, p),
            |r| footprint(&binding, r),
            |r, b| evaluate(&binding, r, b),
        )
        .unwrap();
        let mut actual = [0u8; 16];
        output.surface().try_read_pixel_bytes(-1, 0, &mut actual).unwrap();
        assert_eq!(&actual[12..], &raw[12..16]);
        assert_ne!(&actual[..12], &raw[..12]);
        assert_eq!(output.surface().default_bytes(), pixels.default_bytes());
        output.surface().try_read_pixel_bytes(-2, 0, &mut actual).unwrap();
        assert_eq!(actual.as_slice(), edge.as_slice());
        assert_eq!(prepared.requirements().remaining_work_units, before - 2048);
        let mut preserved = [0u8; 32];
        pixels.try_read_interleaved_into_with(full, &mut preserved, &Interrupt::NONE).unwrap();
        assert_eq!(preserved.as_slice(), raw.as_slice());
        let remaining = prepared.requirements().remaining_work_units;
        let wrong_policy = Binding { policy: source::NumericPolicy::DeclaredHslTripletBinary64FinalFloat, ..binding };
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&pixels),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, d, p| accepts_port(d, p),
                |r| footprint(&wrong_policy, r),
                |r, b| evaluate(&wrong_policy, r, b)
            )
            .is_err()
        );
        assert_eq!(prepared.requirements().remaining_work_units, remaining);
        let wrong = Binding { weights: source::YWeights::from_bits([0.5f64.to_bits(), 0.25f64.to_bits(), 0.25f64.to_bits()]).unwrap(), ..binding };
        assert!(
            execute_pipeline_with_roi(
                &graph,
                2,
                Some(&pixels),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, d, p| accepts_port(d, p),
                |r| footprint(&wrong, r),
                |r, b| evaluate(&wrong, r, b)
            )
            .is_err()
        );
        assert_eq!(prepared.requirements().remaining_work_units, remaining);
        let replay_policy = source::NumericPolicy::DeclaredStoredOrderBinary64FinalFloat;
        let replay = source::prepare(
            &original,
            source::SourceBinding {
                xcf_version: 22,
                expected_epoch: source::EPOCH,
                schema_sha256: source::SCHEMA,
                context,
                policy: replay_policy,
                transfer,
                weights,
            },
            PixelFormat::RGBA32F,
            source::Limits::default(),
            &mut Prep,
            &Interrupt::NONE,
        )
        .unwrap();
        let mut reordered = graph.clone();
        reordered.nodes[0].arguments.swap(0, 3);
        let cross_order = Binding { node: &reordered.nodes[0], prepared: &replay, policy: replay_policy, ..binding };
        let left = replay.requirements().remaining_work_units;
        assert!(
            execute_pipeline_with_roi(
                &reordered,
                2,
                Some(&pixels),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, d, p| accepts_port(d, p),
                |r| footprint(&cross_order, r),
                |r, b| evaluate(&cross_order, r, b)
            )
            .is_err()
        );
        assert_eq!(replay.requirements().remaining_work_units, left);
    }
    #[test]
    fn negative_partial_roi_prices_complete_edge_tiles() {
        let (bounds, count) = enclosure(Rect::new(-1, 2, 1, 3), photocraft_color::PixelFormat::RGBA32F).unwrap();
        assert_eq!(count, 2);
        assert_eq!(bounds, Rect::new(-256, 0, 256, 256));
    }
    #[test]
    fn empty_and_overflowing_staging_rectangles_fail() {
        assert!(enclosure(Rect::new(0, 0, 0, 1), photocraft_color::PixelFormat::RGBA32F).is_err());
        assert!(enclosure(Rect::new(i32::MAX - 1, 0, i32::MAX, 1), photocraft_color::PixelFormat::RGBA32F).is_err());
    }
}
