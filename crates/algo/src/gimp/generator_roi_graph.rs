//! Coordinate-local partial-ROI consumer of existing prepared generators.
//! Original geometry/native endpoints/default/ranges/seed and typed schema stay
//! immutable. Only the render ROI changes; no source/GEGL admission is inferred.
use super::{
    generator_graph::{self, Binding, Kernel, PreparedGenerator},
    graph_execute::{ExecutionLimits, GraphExecuteError, GraphOutput, NodeBudget, NodeOutput, NodeRequest},
    graph_pipeline::{GraphPipelineError, execute_pipeline_with_roi},
    graph_plan::PortDirection,
    graph_roi::GraphRoiError,
};
use photocraft_doc::xcf_filters::{ExactNumber, FilterGraph, FilterValue, GraphNode};
use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicU64, Ordering};
fn add(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_add(b).ok_or(GraphExecuteError::ResidentLimit)
}
fn mul(a: u64, b: u64) -> Result<u64, GraphExecuteError> {
    a.checked_mul(b).ok_or(GraphExecuteError::WorkLimit)
}
fn number(n: &ExactNumber) -> bool {
    match n {
        ExactNumber::Float32Bits(b) => f32::from_bits(*b).is_finite(),
        ExactNumber::Float64Bits(b) => f64::from_bits(*b).is_finite(),
        _ => true,
    }
}
/// Selected AST backing only; opaque whole-graph/original-source owners and
/// arbitrary validator captures remain enclosing-caller charged and retained.
fn metadata(node: &GraphNode) -> Result<(u64, u64), GraphExecuteError> {
    let id = &node.operation;
    if node.id.0 == 0
        || id.name.is_empty()
        || id.name.len() > 256
        || id.stored_version.is_empty()
        || id.stored_version.len() > 64
        || id.schema_epoch.is_empty()
        || id.schema_epoch.len() > 256
        || node.arguments.len() > 64
    {
        return Err(GraphExecuteError::InvalidPlan);
    }
    let identity = add(add(id.name.capacity() as u64, id.stored_version.capacity() as u64)?, id.schema_epoch.capacity() as u64)?;
    let mut args = mul(node.arguments.capacity() as u64, std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>() as u64)?;
    for arg in &node.arguments {
        if arg.name.is_empty() || arg.name.len() > 128 {
            return Err(GraphExecuteError::InvalidPlan);
        }
        args = add(args, arg.name.capacity() as u64)?;
        match &arg.value {
            FilterValue::StoredColor(_) => return Err(GraphExecuteError::Operation("StoredColor partial generator evaluation is unsupported")),
            FilterValue::Number(n) if number(n) => {}
            FilterValue::Bool(_) => {}
            FilterValue::String(v) if v.len() <= 1024 => args = add(args, v.capacity() as u64)?,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 128 => args = add(args, domain.capacity() as u64)?,
            FilterValue::Color { components, profile: None, .. } if !components.is_empty() && components.len() <= 4 && components.iter().all(number) => {
                args = add(args, mul(components.capacity() as u64, std::mem::size_of::<ExactNumber>() as u64)?)?
            }
            _ => return Err(GraphExecuteError::Operation("partial generator argument envelope")),
        }
        if args > 1 << 20 {
            return Err(GraphExecuteError::IndexLimit);
        }
    }
    if identity > 1 << 20 || args > 1 << 20 {
        return Err(GraphExecuteError::IndexLimit);
    }
    Ok((args, identity))
}
fn partial<'data>(generator: &PreparedGenerator<'data>, roi: Rect, ctl: &Interrupt<'_>) -> Result<PreparedGenerator<'data>, GraphExecuteError> {
    if roi.is_empty() || !generator.roi().contains_rect(&roi) {
        return Err(GraphExecuteError::Roi);
    }
    let kernel = match generator.kernel() {
        Kernel::Linear(request) => Kernel::Linear(super::generator_linear_gradient::Request { roi, ..request }),
        Kernel::Radial(request) => Kernel::Radial(super::generator_radial_gradient::Request { roi, ..request }),
        Kernel::CounterNoise { configuration, .. } => Kernel::CounterNoise { configuration, roi },
    };
    PreparedGenerator::prepare(kernel, generator.domain(), ctl)
}
pub struct PreparedRoiGraph<'a, 'data> {
    graph: &'a FilterGraph,
    binding: &'a Binding<'a, 'data>,
    limits: ExecutionLimits,
    held_bytes: u64,
    held_index: u64,
    preparation_work: u64,
    metadata_work: u64,
    remaining_work: AtomicU64,
}
impl<'a, 'data> PreparedRoiGraph<'a, 'data> {
    /// The required original validator is called after bounded envelope and
    /// prospective header/parameter admission. Its external work is caller paid.
    pub fn prepare(graph: &'a FilterGraph, binding: &'a Binding<'a, 'data>, limits: ExecutionLimits, ctl: &Interrupt<'_>) -> Result<Self, GraphExecuteError> {
        ctl.check().map_err(|_| GraphExecuteError::Cancelled)?;
        let standard = ExecutionLimits::default();
        if limits.resident_bytes > standard.resident_bytes
            || limits.index_bytes > standard.index_bytes
            || limits.cache_bytes > standard.cache_bytes
            || limits.work_units > standard.work_units
        {
            return Err(GraphExecuteError::RaisedLimit);
        }
        if graph.input.is_some()
            || !graph.edges.is_empty()
            || graph.nodes.len() != 1
            || binding.generator.domain().profile_fingerprint.is_none_or(|h| h == [0; 32])
        {
            return Err(GraphExecuteError::Domain);
        }
        let node = graph.nodes.first().ok_or(GraphExecuteError::InvalidPlan)?;
        if graph.output != binding.node
            || node.id != binding.node
            || !std::ptr::eq(binding.identity, &node.operation)
            || node.operation != *binding.identity
            || node.version_origin != binding.version_origin
            || binding.output_port.is_empty()
            || binding.output_port.len() > 256
            || binding.output_port.contains('\0')
        {
            return Err(GraphExecuteError::InvalidPlan);
        }
        let (args, identity) = metadata(node)?;
        // Original renderer charges both retained/request identity views. This
        // exact borrowed node witness shares them; double debit is conservative.
        let metadata_work = add(args, add(mul(identity, 2)?, binding.output_port.len() as u64)?)?;
        let headers = add(
            std::mem::size_of::<Self>() as u64,
            add(std::mem::size_of::<PreparedGenerator<'data>>() as u64, std::mem::size_of::<Binding<'a, 'data>>() as u64)?,
        )?;
        let held_index = add(add(args, identity)?, add(headers, mul(graph.nodes.capacity() as u64, std::mem::size_of::<GraphNode>() as u64)?)?)?;
        let held_bytes = add(held_index, binding.generator.requirements().parameter_retained_bytes)?;
        let preparation_work = add(4096, metadata_work)?;
        if held_bytes > limits.resident_bytes || held_index > limits.index_bytes || preparation_work > limits.work_units {
            return Err(GraphExecuteError::Reservation);
        }
        (binding.validate_arguments)(node, binding.generator)?;
        ctl.check().map_err(|_| GraphExecuteError::Cancelled)?;
        Ok(Self {
            graph,
            binding,
            limits,
            held_bytes,
            held_index,
            preparation_work,
            metadata_work,
            remaining_work: AtomicU64::new(limits.work_units - preparation_work),
        })
    }
    pub fn extent(&self) -> Rect {
        self.binding.generator.roi()
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(Ordering::Relaxed)
    }
    fn evaluate(&self, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
        budget.poll()?;
        if request.node != self.graph.nodes.first().ok_or(GraphExecuteError::InvalidPlan)?
            || !request.inputs.is_empty()
            || request.domain != self.binding.generator.domain()
        {
            return Err(GraphExecuteError::Domain);
        }
        // Only stack metadata is recreated. Original endpoints/defaults/noise
        // backing remain borrowed, with no new seed or coordinate translation.
        budget.charge_work(add(4096, self.binding.generator.requirements().parameter_retained_bytes)?)?;
        let headers = add(std::mem::size_of::<PreparedGenerator<'data>>() as u64, std::mem::size_of::<Binding<'a, 'data>>() as u64)?;
        budget.reserve_scratch(add(headers, 1024)?)?;
        let prepared = partial(self.binding.generator, request.roi, budget.interrupt())?;
        let validator = |node: &GraphNode, _: &PreparedGenerator<'_>| (self.binding.validate_arguments)(node, self.binding.generator);
        let binding = Binding {
            node: self.binding.node,
            identity: self.binding.identity,
            version_origin: self.binding.version_origin,
            output_port: self.binding.output_port,
            generator: &prepared,
            validate_arguments: &validator,
        };
        generator_graph::evaluate(&binding, request, budget)
    }
    /// Fixed graph/kernel plus atomic cumulative work: fresh caller limits cannot
    /// reset spent quota. No-input ROI declarations never request source pixels.
    pub fn execute(&self, roi: Rect, limits: ExecutionLimits, ctl: &Interrupt<'_>) -> Result<GraphOutput<'a>, GraphPipelineError> {
        let error = GraphPipelineError::Execute;
        ctl.check().map_err(|_| error(GraphExecuteError::Cancelled))?;
        if limits.resident_bytes > self.limits.resident_bytes
            || limits.cache_bytes > self.limits.cache_bytes
            || limits.index_bytes > self.limits.index_bytes
            || limits.work_units > self.limits.work_units
        {
            return Err(error(GraphExecuteError::RaisedLimit));
        }
        let prepass_work = add(4096, self.binding.generator.requirements().parameter_retained_bytes).map_err(error)?;
        if prepass_work > limits.work_units.checked_sub(self.preparation_work).ok_or(error(GraphExecuteError::WorkLimit))? {
            return Err(error(GraphExecuteError::WorkLimit));
        }
        self.remaining_work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(prepass_work))
            .map_err(|_| error(GraphExecuteError::WorkLimit))?;
        // Requirements inspect only bounded native endpoint/range scalars; prepay
        // this preliminary metadata pass too, including failures, before reading.
        let prepared = partial(self.binding.generator, roi, ctl).map_err(error)?;
        let req = prepared.requirements();
        let planning = super::graph_execute::planning_requirements(1, 0).map_err(error)?;
        let planner_index = add(planning.index_bytes, 256 + 4096).map_err(error)?;
        let output = add(add(req.output_retained_bytes, req.output_index_bytes).map_err(error)?, 256).map_err(error)?;
        let parameters = add(req.parameter_retained_bytes, self.metadata_work).map_err(error)?;
        let local_headers =
            add(add(std::mem::size_of::<PreparedGenerator<'data>>() as u64, std::mem::size_of::<Binding<'a, 'data>>() as u64).map_err(error)?, 1024)
                .map_err(error)?;
        let callback_resident =
            add(add(output, req.output_index_bytes).map_err(error)?, add(add(req.scratch_bytes, parameters).map_err(error)?, local_headers).map_err(error)?)
                .map_err(error)?;
        let peak = add(add(self.held_bytes, planner_index).map_err(error)?, callback_resident).map_err(error)?;
        let index = add(add(self.held_index, planner_index).map_err(error)?, req.output_index_bytes).map_err(error)?;
        // Genuine partial producer/output/scratch and both planner indices are
        // admitted before the pipeline allocates any graph metadata or pixels.
        if peak > limits.resident_bytes || index > limits.index_bytes || output > limits.cache_bytes {
            return Err(error(GraphExecuteError::Reservation));
        }
        // Two same-ROI declarations (request+merged node), each output+support.
        let declarations = mul(mul(roi.size().area(), self.binding.generator.domain().format.channels() as u64).map_err(error)?, 4).map_err(error)?;
        // Scheduler visits precede NodeBudget and are separate from both ROI
        // declarations and the generator's own sample arithmetic.
        let scheduler_samples = mul(roi.size().area(), self.binding.generator.domain().format.channels() as u64).map_err(error)?;
        let partial_work = add(4096, self.binding.generator.requirements().parameter_retained_bytes).map_err(error)?;
        let callback_work = add(add(4096, self.metadata_work).map_err(error)?, req.work_units).map_err(error)?;
        let work = add(add(add(200_000, declarations).map_err(error)?, scheduler_samples).map_err(error)?, add(partial_work, callback_work).map_err(error)?)
            .map_err(error)?;
        if work
            > limits.work_units.checked_sub(self.preparation_work).and_then(|left| left.checked_sub(prepass_work)).ok_or(error(GraphExecuteError::WorkLimit))?
        {
            return Err(error(GraphExecuteError::WorkLimit));
        }
        self.remaining_work.try_update(Ordering::Relaxed, Ordering::Relaxed, |left| left.checked_sub(work)).map_err(|_| error(GraphExecuteError::WorkLimit))?;
        let limits = ExecutionLimits {
            resident_bytes: limits.resident_bytes.checked_sub(self.held_bytes).ok_or(error(GraphExecuteError::ResidentLimit))?,
            index_bytes: limits.index_bytes.checked_sub(self.held_index).ok_or(error(GraphExecuteError::IndexLimit))?,
            work_units: work,
            ..limits
        };
        execute_pipeline_with_roi(
            self.graph,
            1,
            None,
            roi,
            self.binding.generator.domain(),
            limits,
            ctl,
            |id, direction, port| id == self.binding.node && generator_graph::accepts_port(self.binding, direction, port) && direction == PortDirection::Output,
            |_| Err(GraphRoiError::UnsupportedFootprint),
            |request, budget| self.evaluate(request, budget),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        generator_counter_noise as noise, generator_linear_gradient as linear, generator_radial_gradient as radial, graph_execute::ExecutionDomain,
        graph_pipeline::execute_pipeline, wide_point::WorkingContext,
    };
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{
        SampleEncoding,
        item_identity::GraphNodeId,
        xcf_filters::{GraphVersionOrigin, OperationIdentity},
    };
    fn graph() -> FilterGraph {
        FilterGraph {
            input: None,
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity {
                    name: "own:explicit-generator".into(),
                    stored_version: "own".into(),
                    schema_epoch: "own-no-source-admission".into(),
                },
                version_origin: GraphVersionOrigin::Stored,
                arguments: Vec::new(),
            }],
            edges: Vec::new(),
            output: GraphNodeId(1),
            original_pipeline: None,
        }
    }
    #[test]
    fn all_three_generators_partition_exactly_in_absolute_negative_coordinates() {
        let graph = graph();
        let extent = Rect::new(-2, -1, 2, 1);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([9; 32]) };
        let context = linear::WorkingContext { encoding: domain.encoding, profile_identity: [9; 32] };
        let start = (-0f64).to_ne_bytes();
        let end = 2f64.to_ne_bytes();
        let default = (-3f64).to_ne_bytes();
        let ranges = [noise::Range { low: -2., high: 3. }];
        let kernels = [
            Kernel::Linear(linear::Request {
                format,
                roi: extent,
                geometry: linear::Geometry { start: [-1.5, -0.5], end: [1.5, 0.5], repeat: linear::Repeat::Clamp },
                start: linear::Endpoint { native: &start, context },
                end: linear::Endpoint { native: &end, context },
                output_context: context,
                output_default: &default,
            }),
            Kernel::Radial(radial::Request {
                format,
                roi: extent,
                geometry: radial::Geometry { center: [-0.5, -0.5], radii: [2., 3.], repeat: radial::Repeat::Clamp },
                start: linear::Endpoint { native: &start, context },
                end: linear::Endpoint { native: &end, context },
                output_context: context,
                output_default: &default,
            }),
            Kernel::CounterNoise {
                configuration: noise::Noise {
                    seed: 0xfedcba9876543210,
                    format,
                    context: WorkingContext { encoding: domain.encoding, profile_identity: [9; 32] },
                    domain: noise::Domain::FiniteHdr,
                    ranges: &ranges,
                    alpha: None,
                    output_default: &default,
                },
                roi: extent,
            },
        ];
        for kernel in kernels {
            let prepared = PreparedGenerator::prepare(kernel, domain, &Interrupt::NONE).unwrap();
            let validator = |node: &GraphNode, candidate: &PreparedGenerator<'_>| {
                if node.arguments.is_empty() && candidate.roi() == extent { Ok(()) } else { Err(GraphExecuteError::InvalidPlan) }
            };
            let binding = Binding {
                node: GraphNodeId(1),
                identity: &graph.nodes.first().unwrap().operation,
                version_origin: GraphVersionOrigin::Stored,
                output_port: "output",
                generator: &prepared,
                validate_arguments: &validator,
            };
            let full = execute_pipeline(
                &graph,
                1,
                None,
                extent,
                domain,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| generator_graph::accepts_port(&binding, direction, port),
                |request, budget| generator_graph::evaluate(&binding, request, budget),
            )
            .unwrap();
            let adaptive = PreparedRoiGraph::prepare(&graph, &binding, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
            let initial = adaptive.remaining_work();
            for roi in [Rect::new(-2, -1, 0, 1), Rect::new(0, -1, 2, 1)] {
                let part = adaptive.execute(roi, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(part.surface().try_read_interleaved(roi).unwrap(), full.surface().try_read_interleaved(roi).unwrap());
                assert_eq!(part.surface().default_bytes(), default);
            }
            assert!(adaptive.remaining_work() < initial);
            assert!(adaptive.execute(Rect::new(-3, -1, 0, 1), ExecutionLimits::default(), &Interrupt::NONE).is_err());
        }
    }
    #[test]
    fn fresh_limits_cannot_reset_atomic_quota_or_publish_cancelled_output() {
        let graph = graph();
        let extent = Rect::new(0, 0, 1, 1);
        let default = 0f64.to_ne_bytes();
        let ranges = [noise::Range { low: 0., high: 1. }];
        let domain = ExecutionDomain {
            format: PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
            encoding: SampleEncoding::Linear,
            profile_fingerprint: Some([9; 32]),
        };
        let prepared = PreparedGenerator::prepare(
            Kernel::CounterNoise {
                configuration: noise::Noise {
                    seed: 17,
                    format: domain.format,
                    context: WorkingContext { encoding: domain.encoding, profile_identity: [9; 32] },
                    domain: noise::Domain::Unit,
                    ranges: &ranges,
                    alpha: None,
                    output_default: &default,
                },
                roi: extent,
            },
            domain,
            &Interrupt::NONE,
        )
        .unwrap();
        let validator = |_: &GraphNode, _: &PreparedGenerator<'_>| Ok(());
        let binding = Binding {
            node: GraphNodeId(1),
            identity: &graph.nodes.first().unwrap().operation,
            version_origin: GraphVersionOrigin::Stored,
            output_port: "output",
            generator: &prepared,
            validate_arguments: &validator,
        };
        let limits = ExecutionLimits { work_units: 500_000, ..ExecutionLimits::default() };
        let adaptive = PreparedRoiGraph::prepare(&graph, &binding, limits, &Interrupt::NONE).unwrap();
        assert!(adaptive.execute(extent, limits, &Interrupt::NONE).is_ok());
        assert!(adaptive.execute(extent, limits, &Interrupt::NONE).is_ok());
        assert!(adaptive.execute(extent, limits, &Interrupt::NONE).is_err());
        let cancelled = || true;
        assert!(adaptive.execute(extent, limits, &Interrupt::cancel_only(&cancelled)).is_err());
    }
}
