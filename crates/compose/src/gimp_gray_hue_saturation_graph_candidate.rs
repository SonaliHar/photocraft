//! Explicit retained Gray Hue/Saturation NodeBudget callback.
//! Prepared source/AST/profile/role backing and its constructor lease must remain
//! held and prepaid by the enclosing scheduler, including pruned owners. This
//! callback neither resolves a profile nor admits a scene/default/source filter.
//! Linear versus exact Gamma2 is an independently caller-established input role;
//! no Yfloat/Babl/parent-ICC alias is inferred. The numerical chain is declared,
//! and the six replay ULP differences/historical strict HOLD remain unchanged.
//! Gray source colors are unit-checked BEFORE native F64 -> Float ingress.
//! Optional raw alpha, defaults, edge pixels and outside ROI remain unchanged.
//! Bounded prepaid BTreeMap/Arc finalization is a standard allocator boundary,
//! not a fallible-allocation or process-RSS guarantee.
use crate::{
    gimp_composite::CompositionError,
    gimp_gray_hue_saturation_candidate::{InputProfileWitness, InputRole, NumericPolicy, Prepared, config_bits},
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
    pub policy: NumericPolicy,
    pub input_profile: InputProfileWitness,
    pub domain: ExecutionDomain,
    pub full_extent: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gimp_gray_hue_saturation_candidate as source, gimp_render::PreparationBudget};
    use photocraft_algo::gimp::{
        graph_execute::{DomainAssignment, ExecutionLimits, InputBinding},
        graph_pipeline::execute_pipeline_domains,
        graph_plan::Endpoint,
    };
    use photocraft_doc::{
        item_identity::{FilterId, GraphNodeId},
        xcf_filters::{ExactNumber, FilterArgument, FilterGraph, FilterNode, FilterValue, GraphEdge, OperationIdentity},
    };
    struct PrepBudget;
    impl PreparationBudget for PrepBudget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, _: u64) -> crate::gimp_composite::Result<()> {
            Ok(())
        }
        fn consume_preparation_work(&mut self, _: u64) -> crate::gimp_composite::Result<()> {
            Ok(())
        }
    }
    fn source_node() -> FilterNode {
        let names = [
            "time",
            "all.hue",
            "all.saturation",
            "all.lightness",
            "red.hue",
            "red.saturation",
            "red.lightness",
            "yellow.hue",
            "yellow.saturation",
            "yellow.lightness",
            "green.hue",
            "green.saturation",
            "green.lightness",
            "cyan.hue",
            "cyan.saturation",
            "cyan.lightness",
            "blue.hue",
            "blue.saturation",
            "blue.lightness",
            "magenta.hue",
            "magenta.saturation",
            "magenta.lightness",
            "overlap",
        ];
        let mut words = [0u64; 23];
        words[3] = 0.25f64.to_bits();
        words[4] = (-0f64).to_bits();
        FilterNode {
            id: FilterId(3),
            operation: OperationIdentity { name: "gimp:hue-saturation".into(), stored_version: "0:0".into(), schema_epoch: "explicit-gray-context".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument {
                name: "config".into(),
                value: FilterValue::Config(
                    names
                        .into_iter()
                        .zip(words)
                        .map(|(name, b)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(b)) })
                        .collect(),
                ),
            }],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    #[test]
    fn real_graph_retains_raw_nan_alpha_default_edge_and_both_passes_share_quota() {
        for role in [InputRole::CallerEstablishedLinearGray, InputRole::CallerEstablishedExactGamma2Gray] {
            let original = source_node();
            let before = original.clone();
            let encoding = match role {
                InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
                InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
            };
            let context = WorkingContext { encoding, profile_identity: [8; 32] };
            let input_profile = InputProfileWitness { role, profile_identity: [8; 32] };
            let policy = NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat;
            let format = PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F64, true);
            let mut prep = PrepBudget;
            let prepared = source::prepare(
                &original,
                source::SourceBinding {
                    xcf_version: 22,
                    expected_epoch: "explicit-gray-context",
                    schema_sha256: source::SCHEMA,
                    context,
                    policy,
                    input_profile,
                },
                format,
                source::Limits::default(),
                &mut prep,
                &Interrupt::NONE,
            )
            .unwrap();
            let graph = FilterGraph {
                input: Some(GraphNodeId(99)),
                nodes: vec![GraphNode {
                    id: GraphNodeId(100),
                    operation: original.operation.clone(),
                    version_origin: GraphVersionOrigin::Stored,
                    arguments: original.arguments.clone(),
                }],
                edges: vec![GraphEdge { from: GraphNodeId(99), to: GraphNodeId(100), input_port: "input".into(), output_port: "output".into() }],
                output: GraphNodeId(100),
                original_pipeline: None,
            };
            let domain = ExecutionDomain { format, encoding, profile_fingerprint: Some(context.profile_identity) };
            let assignments = [DomainAssignment { node: GraphNodeId(100), domain }];
            let roi = Rect::new(-1, 0, 0, 1);
            let mut pixels = Surface::new(format);
            let raw: Vec<u8> = [0.25f64.to_bits(), 0x7ff8000000000042].into_iter().flat_map(u64::to_le_bytes).collect();
            pixels.try_write_interleaved(roi, &raw).unwrap();
            let outside: Vec<u8> = [0.75f64.to_bits(), (-0f64).to_bits()].into_iter().flat_map(u64::to_le_bytes).collect();
            pixels.try_write_interleaved(Rect::new(-2, 0, -1, 1), &outside).unwrap();
            let binding = Binding {
                node: &graph.nodes[0],
                version_origin: GraphVersionOrigin::Stored,
                prepared: &prepared,
                context,
                policy,
                input_profile,
                domain,
                full_extent: roi,
            };
            let remaining = prepared.requirements().remaining_work_units;
            let output = execute_pipeline_domains(
                &graph,
                2,
                Some(&pixels),
                roi,
                domain,
                &assignments,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, d, p| accepts_port(d, p),
                |r, b| evaluate(&binding, r, b),
            )
            .unwrap();
            let mut actual = [0u8; 16];
            output.surface().try_read_pixel_bytes(-1, 0, &mut actual).unwrap();
            assert_eq!(&actual[8..], &raw[8..]);
            output.surface().try_read_pixel_bytes(-2, 0, &mut actual).unwrap();
            assert_eq!(actual.as_slice(), outside.as_slice());
            assert_eq!(output.surface().default_bytes(), pixels.default_bytes());
            assert_eq!(prepared.requirements().remaining_work_units, remaining - 2048);
            assert_eq!(prepared.original_node(), &before);
        }
    }
    #[test]
    fn wrong_role_fence_and_insufficient_two_pass_quota_fail_before_output() {
        let original = source_node();
        let context = WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [8; 32] };
        let input_profile = InputProfileWitness { role: InputRole::CallerEstablishedLinearGray, profile_identity: [8; 32] };
        let policy = NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat;
        let format = PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F64, false);
        let mut prep = PrepBudget;
        let base = source::requirements(
            &original,
            source::SourceBinding { xcf_version: 22, expected_epoch: "explicit-gray-context", schema_sha256: source::SCHEMA, context, policy, input_profile },
            format,
            source::Limits::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let prepared = source::prepare(
            &original,
            source::SourceBinding { xcf_version: 22, expected_epoch: "explicit-gray-context", schema_sha256: source::SCHEMA, context, policy, input_profile },
            format,
            source::Limits { work_units: base.preparation_work_units + 2047, ..source::Limits::default() },
            &mut prep,
            &Interrupt::NONE,
        )
        .unwrap();
        let node = GraphNode {
            id: GraphNodeId(100),
            operation: original.operation.clone(),
            version_origin: GraphVersionOrigin::Stored,
            arguments: original.arguments.clone(),
        };
        let domain = ExecutionDomain { format, encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) };
        let roi = Rect::new(0, 0, 1, 1);
        let pixels = Surface::new(format);
        let inputs = [InputBinding {
            input_port: "input",
            output_port: "output",
            endpoint: Endpoint::ExternalInput(GraphNodeId(99)),
            surface: &pixels,
            domain,
            roi,
            footprint: None,
        }];
        let request = NodeRequest { node: &node, inputs: &inputs, roi, domain };
        let binding =
            Binding { node: &node, version_origin: node.version_origin, prepared: &prepared, context, policy, input_profile, domain, full_extent: roi };
        assert_eq!(requirements(&binding, &request, &Interrupt::NONE).unwrap_err(), E::WorkLimit);
        let wrong = Binding { input_profile: InputProfileWitness { role: InputRole::CallerEstablishedExactGamma2Gray, ..input_profile }, ..binding };
        assert!(requirements(&wrong, &request, &Interrupt::NONE).is_err());
        assert_eq!(prepared.requirements().remaining_work_units, 2047);
    }
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
        _ => op("GIMP Gray Hue/Saturation candidate evaluation refused"),
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub fn accepts_port(direction: PortDirection, port: &str) -> bool {
    matches!((direction, port), (PortDirection::Input, "input") | (PortDirection::Output, "output"))
}
fn domain(domain: ExecutionDomain, context: WorkingContext, input_profile: InputProfileWitness) -> Result<(), E> {
    let encoding = match input_profile.role {
        InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
        InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
    };
    if domain.encoding != encoding
        || domain.format.mode != photocraft_color::ColorMode::Grayscale
        || context.encoding != encoding
        || input_profile.profile_identity != context.profile_identity
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
        || b.prepared.policy() != b.policy
        || b.prepared.input_profile() != b.input_profile
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
    domain(b.domain, b.context, b.input_profile)?;
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
        return Err(op("one exact fixed-ROI declared Gray Float input/output"));
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
    let sample_work = binding.prepared.sample_work();
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
fn validate_pixels<L>(
    source: &Surface,
    roi: Rect,
    prepared: &Prepared<'_, L>,
    actual: WorkingContext,
    input_profile: InputProfileWitness,
    c: &Interrupt<'_>,
) -> Result<(), E> {
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
                // Native F64 unit admission precedes Float ingress; the actual
                // declared kernel preflight precedes output allocation. Both
                // passes consume the same immutable prepared/graph work.
                prepared.evaluate_gray_with_interrupt(value, actual, input_profile, c).map_err(comp)?;
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
    validate_pixels(source, request.roi, binding.prepared, binding.context, binding.input_profile, c)?;
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
    let sample_work = binding.prepared.sample_work();
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
                        let output = binding.prepared.evaluate_gray_with_interrupt(input, binding.context, binding.input_profile, c).map_err(comp)?;
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
