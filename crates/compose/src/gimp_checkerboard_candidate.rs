//! Typed opt-in retained checkerboard -> actual prepared generator graph.
//! Raw stored colors are identities, not profiles or interpreted working colors.
//! Caller supplies native colors independently, and holds all decoder/profile/
//! graph/pruned backing. No unknown format parameter/default, CMS, mask, outer
//! composition, original-file authentication or source enrollment is inferred.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
use photocraft_algo::gimp::{
    checkerboard_graph as graph,
    generator_checkerboard::Checkerboard,
    graph_execute::{ExecutionDomain, GraphExecuteError as G, NodeBudget, NodeOutput, NodeRequest},
    wide_point::WorkingContext,
};
use photocraft_doc::xcf_filters::{ExactColorStorage, ExactNumber, FilterNode, FilterValue, GraphNode, GraphVersionOrigin};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError as R};
use std::sync::atomic::{AtomicBool, Ordering};
pub const SCHEMA: &str = "ebd4bd09a17546fdcb3142db0748e8a65ffe4a7d304d4c2850b59e4ec6792bdd";
pub use graph::accepts_port;
pub const EPOCH: &str = "gimp-3.2.6-windows-ebd4bd09a17546fd";
const HEADER: u64 = 4096;
#[derive(Clone, Copy)]
pub struct ColorInterpretation<'a> {
    pub original_storage: &'a ExactColorStorage,
    pub working_native: &'a [u8],
}
#[derive(Clone, Copy)]
pub struct Identity<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub version_origin: GraphVersionOrigin,
    pub context: WorkingContext,
    pub domain: ExecutionDomain,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub preparation_work_units: u64,
}
/// Private validated fields prevent an asserted total or forged successful callback.
/// The two interpreted colors are caller declarations, never derived from storage.
pub struct CheckerboardSourceBinding<'a, L> {
    original: &'a FilterNode,
    node: &'a GraphNode,
    configuration: &'a Checkerboard,
    colors: [ColorInterpretation<'a>; 2],
    identity: Identity<'a>,
    extent: Rect,
    limits: graph::Limits,
    requirements: Requirements,
    prepared: AtomicBool,
    _lease: L,
}
pub struct Prepared<'b, 'a, L> {
    binding: &'b CheckerboardSourceBinding<'a, L>,
    lower: graph::PreparedCheckerboard<'b>,
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
fn lower(e: G) -> E {
    match e {
        G::Cancelled => R::Cancelled.into(),
        G::Raster(r) => r.into(),
        G::ResidentLimit | G::IndexLimit | G::CacheLimit | G::WorkLimit | G::Reservation | G::RaisedLimit => limit(),
        _ => bad("typed checkerboard graph refused"),
    }
}
fn shape(n: &FilterNode, node: &GraphNode, id: Identity<'_>) -> Result<()> {
    if id.xcf_version != 22
        || id.schema_sha256 != SCHEMA
        || id.expected_epoch != EPOCH
        || n.id.0 == 0
        || n.operation.name != "gegl:checkerboard"
        || n.operation.stored_version != "0:0"
        || n.operation.schema_epoch != id.expected_epoch
        || !n.auxiliary.is_empty()
        || n.arguments.len() != 6
        || node.arguments.len() != 6
        || node.operation != n.operation
        || node.version_origin != id.version_origin
        || id.context.profile_identity == [0; 32]
        || id.domain.profile_fingerprint != Some(id.context.profile_identity)
        || id.domain.encoding != id.context.encoding
    {
        return Err(bad("exact retained checkerboard/schema/context/graph identity"));
    }
    let mut seen = 0u8;
    for a in &n.arguments {
        let index = match a.name.as_str() {
            "x" => 0,
            "y" => 1,
            "x-offset" => 2,
            "y-offset" => 3,
            "color1" => 4,
            "color2" => 5,
            _ => return Err(bad("six original checkerboard argument names")),
        };
        let bit = 1u8 << index;
        if seen & bit != 0 {
            return Err(bad("duplicate checkerboard argument"));
        }
        seen |= bit;
        match &a.value {
            FilterValue::Number(ExactNumber::Signed { width: 32, value }) if index < 4 && i32::try_from(*value).is_ok() => {}
            FilterValue::StoredColor(v)
                if index >= 4
                    && !v.layout.is_empty()
                    && v.layout.len() <= 256
                    && !v.layout.contains('\0')
                    && !v.samples.is_empty()
                    && v.samples.len() <= 1024
                    && v.opaque_tail.len() <= 1 << 20 => {}
            _ => return Err(bad("original Signed32/raw StoredColor shape")),
        }
    }
    Ok(())
}
/// Allocation-free capacity census; no color byte interpretation or large
/// equality scan occurs before the real shared preparation lease/work debit.
pub fn requirements(
    n: &FilterNode,
    node: &GraphNode,
    configuration: &Checkerboard,
    extent: Rect,
    id: Identity<'_>,
    limits: graph::Limits,
    c: &Interrupt<'_>,
) -> Result<Requirements> {
    poll(c)?;
    shape(n, node, id)?;
    let q = graph::preparation_requirements(node, configuration, extent, id.context, id.domain, limits, c).map_err(lower)?;
    let mut held = add(HEADER, std::mem::size_of::<FilterNode>() as u64)?;
    held = add(held, mul(n.arguments.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>() as u64)?)?;
    held = add(held, add(n.operation.name.capacity() as u64, add(n.operation.stored_version.capacity() as u64, n.operation.schema_epoch.capacity() as u64)?)?)?;
    let mut comparisons = 512;
    for a in &n.arguments {
        poll(c)?;
        held = add(held, a.name.capacity() as u64)?;
        if let FilterValue::StoredColor(v) = &a.value {
            held = add(held, add(v.layout.capacity() as u64, add(v.samples.capacity() as u64, v.opaque_tail.capacity() as u64)?)?)?;
            comparisons = add(comparisons, mul(add(v.layout.len() as u64, add(v.samples.len() as u64, v.opaque_tail.len() as u64)?)?, 2)?)?;
        }
    }
    let retained = add(held, q.retained_bytes)?;
    let work = add(comparisons, q.work_units)?;
    if retained > limits.resident_bytes || work > limits.work_units {
        return Err(limit());
    }
    Ok(Requirements { retained_bytes: retained, preparation_work_units: work })
}
fn same_bytes(a: &[u8], b: &[u8], c: &Interrupt<'_>) -> Result<()> {
    if a.len() != b.len() {
        return Err(bad("original checkerboard raw storage identity"));
    }
    for (x, y) in a.chunks(4096).zip(b.chunks(4096)) {
        poll(c)?;
        if x != y {
            return Err(bad("checkerboard storage changed"));
        }
    }
    Ok(())
}
fn exact_binding(n: &FilterNode, node: &GraphNode, k: &Checkerboard, colors: [ColorInterpretation<'_>; 2], c: &Interrupt<'_>) -> Result<()> {
    for (a, b) in n.arguments.iter().zip(&node.arguments) {
        poll(c)?;
        if a.name != b.name {
            return Err(bad("original checkerboard declaration order"));
        }
        match (&a.value, &b.value) {
            (FilterValue::Number(x), FilterValue::Number(y)) if x == y => {
                let ExactNumber::Signed { width: 32, value } = x else {
                    return Err(bad("native integer width"));
                };
                let expected = match a.name.as_str() {
                    "x" => k.cell_width(),
                    "y" => k.cell_height(),
                    "x-offset" => k.offsets()[0],
                    "y-offset" => k.offsets()[1],
                    _ => return Err(bad("known integer field")),
                };
                if *value != i64::from(expected) {
                    return Err(bad("exact original checkerboard integer/configuration"));
                }
            }
            (FilterValue::StoredColor(original), FilterValue::StoredColor(mapped)) => {
                let slot = match a.name.as_str() {
                    "color1" => 0,
                    "color2" => 1,
                    _ => return Err(bad("known raw color field")),
                };
                let color = colors.get(slot).ok_or_else(limit)?;
                if !std::ptr::eq(original, color.original_storage) {
                    return Err(bad("caller color binds actual original StoredColor owner"));
                }
                same_bytes(original.layout.as_bytes(), mapped.layout.as_bytes(), c)?;
                same_bytes(&original.samples, &mapped.samples, c)?;
                same_bytes(&original.opaque_tail, &mapped.opaque_tail, c)?;
                let native = if slot == 0 { k.color1_bytes() } else { k.color2_bytes() };
                if native != color.working_native {
                    return Err(bad("explicit native working-color identity"));
                }
            }
            _ => return Err(bad("exact independent checkerboard field types")),
        }
    }
    Ok(())
}
impl<'a, L> CheckerboardSourceBinding<'a, L> {
    pub fn original_node(&self) -> &'a FilterNode {
        self.original
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    /// Exactly one lower preparation per admitted binding; failures do not reset it.
    pub fn prepare<'b>(&'b self, c: &Interrupt<'_>) -> Result<Prepared<'b, 'a, L>> {
        poll(c)?;
        self.prepared.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).map_err(|_| bad("checkerboard binding already prepared"))?;
        let lower =
            graph::PreparedCheckerboard::prepare(self.node, self.configuration, self.extent, self.identity.context, self.identity.domain, self, self.limits, c)
                .map_err(lower)?;
        Ok(Prepared { binding: self, lower })
    }
}
impl<L> graph::Validator for CheckerboardSourceBinding<'_, L> {
    fn validate(&self, node: &GraphNode, k: &Checkerboard) -> std::result::Result<(), G> {
        // All retained source/graph fields are immutable borrowed owners. Heavy exact
        // raw byte comparison was tokened and admitted once during binding, never replayed.
        if !std::ptr::eq(node, self.node)
            || !std::ptr::eq(k, self.configuration)
            || k.encoding() != self.identity.context.encoding
            || k.format() != self.identity.domain.format
            || k.color1_bytes() != self.colors[0].working_native
            || k.color2_bytes() != self.colors[1].working_native
        {
            return Err(G::Operation("immutable typed original/native checkerboard witness changed"));
        }
        Ok(())
    }
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn bind<'a, B: PreparationBudget>(
    n: &'a FilterNode,
    node: &'a GraphNode,
    k: &'a Checkerboard,
    colors: [ColorInterpretation<'a>; 2],
    extent: Rect,
    id: Identity<'a>,
    limits: graph::Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<CheckerboardSourceBinding<'a, B::Lease>> {
    poll(c)?;
    if add(std::mem::size_of::<CheckerboardSourceBinding<'a, B::Lease>>() as u64, std::mem::size_of::<Prepared<'_, 'a, B::Lease>>() as u64)? > HEADER {
        return Err(limit());
    }
    let q = requirements(n, node, k, extent, id, limits, c)?;
    budget.consume_preparation_work(q.preparation_work_units)?;
    let lease = budget.reserve_prepared_bytes(q.retained_bytes)?;
    exact_binding(n, node, k, colors, c)?;
    // Preserve one immutable cumulative lower ceiling: source binding work is not
    // available again to ROI generation, nor reset by a new NodeBudget.
    let lower_q = graph::preparation_requirements(node, k, extent, id.context, id.domain, limits, c).map_err(lower)?;
    let lower_limits = graph::Limits {
        resident_bytes: limits.resident_bytes.checked_sub(q.retained_bytes.checked_sub(lower_q.retained_bytes).ok_or_else(limit)?).ok_or_else(limit)?,
        work_units: limits.work_units.checked_sub(q.preparation_work_units.checked_sub(lower_q.work_units).ok_or_else(limit)?).ok_or_else(limit)?,
    };
    poll(c)?;
    Ok(CheckerboardSourceBinding {
        original: n,
        node,
        configuration: k,
        colors,
        identity: id,
        extent,
        limits: lower_limits,
        requirements: q,
        prepared: AtomicBool::new(false),
        _lease: lease,
    })
}
impl<L> Prepared<'_, '_, L> {
    pub fn original_node(&self) -> &FilterNode {
        self.binding.original
    }
    pub fn remaining_work(&self) -> u64 {
        self.lower.remaining_work()
    }
    pub fn domain(&self) -> ExecutionDomain {
        self.lower.domain()
    }
    pub fn context(&self) -> WorkingContext {
        self.lower.context()
    }
    pub fn full_extent(&self) -> Rect {
        self.lower.full_extent()
    }
    pub fn evaluate(&self, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> std::result::Result<NodeOutput, G> {
        graph::evaluate(&self.lower, request, budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::{generator_checkerboard::Argument, graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline};
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        item_identity::{FilterId, GraphNodeId},
        xcf_filters::{FilterArgument, FilterGraph, OperationIdentity},
    };
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
            self.bytes = n;
            Ok(())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            self.work = n;
            Ok(())
        }
    }
    fn original() -> FilterNode {
        let mut args: Vec<_> = [("x", 16), ("y", 16), ("x-offset", 0), ("y-offset", 0)]
            .into_iter()
            .map(|(name, value)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Signed { width: 32, value }) })
            .collect();
        for (name, value) in [("color1", 1f32), ("color2", 0f32)] {
            args.push(FilterArgument {
                name: name.into(),
                value: FilterValue::StoredColor(ExactColorStorage {
                    layout: "R'G'B'A float".into(),
                    samples: [value, value, value, 1.].into_iter().flat_map(f32::to_le_bytes).collect(),
                    opaque_tail: vec![0; 4],
                }),
            });
        }
        FilterNode {
            id: FilterId(1),
            operation: photocraft_doc::xcf_filters::OperationIdentity {
                name: "gegl:checkerboard".into(),
                stored_version: "0:0".into(),
                schema_epoch: EPOCH.into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: args,
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: None,
            source_region: None,
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    fn configuration(a: &[u8], b: &[u8]) -> Checkerboard {
        Checkerboard::from_named(
            "gegl:checkerboard",
            "0:0",
            PixelFormat::RGBA32F,
            photocraft_doc::SampleEncoding::Linear,
            &[
                Argument::Integer { name: "x", value: 16 },
                Argument::Integer { name: "y", value: 16 },
                Argument::Integer { name: "x-offset", value: 0 },
                Argument::Integer { name: "y-offset", value: 0 },
                Argument::Color { name: "color1", encoding: photocraft_doc::SampleEncoding::Linear, pixel: a },
                Argument::Color { name: "color2", encoding: photocraft_doc::SampleEncoding::Linear, pixel: b },
            ],
        )
        .unwrap()
    }
    fn identity() -> Identity<'static> {
        let context = WorkingContext { encoding: photocraft_doc::SampleEncoding::Linear, profile_identity: [7; 32] };
        Identity {
            xcf_version: 22,
            expected_epoch: EPOCH,
            schema_sha256: SCHEMA,
            version_origin: GraphVersionOrigin::Stored,
            context,
            domain: ExecutionDomain { format: PixelFormat::RGBA32F, encoding: context.encoding, profile_fingerprint: Some(context.profile_identity) },
        }
    }
    fn storage(n: &FilterNode, i: usize) -> &ExactColorStorage {
        let FilterValue::StoredColor(v) = &n.arguments[i].value else { panic!("fixture raw color") };
        v
    }
    #[test]
    fn actual_zero_input_graph_preserves_native_colors_and_does_not_infer_raw_encoding() {
        let n = original();
        let before = n.clone();
        let graph = FilterGraph {
            input: None,
            nodes: vec![GraphNode {
                id: GraphNodeId(9),
                operation: n.operation.clone(),
                version_origin: GraphVersionOrigin::Stored,
                arguments: n.arguments.clone(),
            }],
            edges: vec![],
            output: GraphNodeId(9),
            original_pipeline: None,
        };
        let first: Vec<u8> = [0.25f32, 0.5, 0.75, 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        let second: Vec<u8> = [0.75f32, 0.25, 0.5, 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        let k = configuration(&first, &second);
        let roi = Rect::new(0, 0, 32, 16);
        let id = identity();
        let mut ledger = Budget::default();
        let binding = bind(
            &n,
            &graph.nodes[0],
            &k,
            [
                ColorInterpretation { original_storage: storage(&n, 4), working_native: &first },
                ColorInterpretation { original_storage: storage(&n, 5), working_native: &second },
            ],
            roi,
            id,
            graph::Limits::default(),
            &mut ledger,
            &Interrupt::NONE,
        )
        .unwrap();
        let prepared = binding.prepare(&Interrupt::NONE).unwrap();
        let work = prepared.remaining_work();
        let out = execute_pipeline(
            &graph,
            1,
            None,
            roi,
            id.domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, d, p| accepts_port(d, p),
            |r, b| prepared.evaluate(r, b),
        )
        .unwrap();
        let mut raw = [0; 16];
        out.surface().try_read_pixel_bytes(0, 0, &mut raw).unwrap();
        assert_eq!(raw.as_slice(), first.as_slice());
        out.surface().try_read_pixel_bytes(16, 0, &mut raw).unwrap();
        assert_eq!(raw.as_slice(), second.as_slice());
        assert!(prepared.remaining_work() < work);
        assert!(binding.prepare(&Interrupt::NONE).is_err());
        assert_eq!(n, before);
    }
    #[test]
    fn wrong_native_color_or_source_color_owner_never_produces_binding() {
        let n = original();
        let node = GraphNode { id: GraphNodeId(9), operation: n.operation.clone(), version_origin: GraphVersionOrigin::Stored, arguments: n.arguments.clone() };
        let first: Vec<u8> = [1f32; 4].into_iter().flat_map(f32::to_le_bytes).collect();
        let second: Vec<u8> = [0f32, 0., 0., 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        let k = configuration(&first, &second);
        let mut ledger = Budget::default();
        assert!(
            bind(
                &n,
                &node,
                &k,
                [
                    ColorInterpretation { original_storage: storage(&n, 5), working_native: &first },
                    ColorInterpretation { original_storage: storage(&n, 4), working_native: &second }
                ],
                Rect::new(0, 0, 16, 16),
                identity(),
                graph::Limits::default(),
                &mut ledger,
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(ledger.work > 0);
    }
    #[test]
    fn source_or_epoch_and_early_cancellation_stay_closed() {
        let n = original();
        let node = GraphNode {
            id: GraphNodeId(9),
            operation: OperationIdentity { schema_epoch: "other".into(), ..n.operation.clone() },
            version_origin: GraphVersionOrigin::Stored,
            arguments: n.arguments.clone(),
        };
        let first: Vec<u8> = [1f32; 4].into_iter().flat_map(f32::to_le_bytes).collect();
        let second: Vec<u8> = [0f32, 0., 0., 1.].into_iter().flat_map(f32::to_le_bytes).collect();
        let k = configuration(&first, &second);
        let colors = [
            ColorInterpretation { original_storage: storage(&n, 4), working_native: &first },
            ColorInterpretation { original_storage: storage(&n, 5), working_native: &second },
        ];
        let mut ledger = Budget::default();
        assert!(bind(&n, &node, &k, colors, Rect::new(0, 0, 16, 16), identity(), graph::Limits::default(), &mut ledger, &Interrupt::NONE).is_err());
        assert_eq!(ledger.work, 0);
        let stop = || true;
        assert!(matches!(
            bind(&n, &node, &k, colors, Rect::new(0, 0, 16, 16), identity(), graph::Limits::default(), &mut ledger, &Interrupt::cancel_only(&stop)),
            Err(E::Raster(R::Cancelled))
        ));
        assert_eq!(ledger.bytes, 0);
    }
}
