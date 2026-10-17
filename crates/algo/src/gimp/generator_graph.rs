//! Zero-input callbacks for caller-bound prepared generator primitives.
//! Preparation/identity equality do not qualify an operation schema or source.
use super::{
    generator_counter_noise as noise, generator_linear_gradient as linear, generator_radial_gradient as radial,
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
};
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{ExactNumber, FilterValue, GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};

#[derive(Clone, Copy, Debug)]
pub enum Kernel<'a> {
    Linear(linear::Request<'a>),
    Radial(radial::Request<'a>),
    CounterNoise { configuration: noise::Noise<'a>, roi: Rect },
}
/// Immutable borrowed configuration with allocation-free resource admission.
/// Caller-owned native endpoints/defaults/ranges must remain alive; no bytes,
/// CMS interpretation or operation metadata are inferred from a node name.
pub struct PreparedGenerator<'a> {
    kernel: Kernel<'a>,
    domain: ExecutionDomain,
    roi: Rect,
    requirements: linear::Requirements,
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn noise_error(error: noise::Error) -> GraphExecuteError {
    match error {
        noise::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Operation("counter noise domain/configuration refusal"),
    }
}
impl<'a> PreparedGenerator<'a> {
    pub fn prepare(kernel: Kernel<'a>, domain: ExecutionDomain, control: &Interrupt<'_>) -> Result<Self, GraphExecuteError> {
        control.check().map_err(|_| GraphExecuteError::Cancelled)?;
        let (format, roi, encoding, profile, requirements) = match kernel {
            Kernel::Linear(request) => (
                request.format,
                request.roi,
                request.output_context.encoding,
                request.output_context.profile_identity,
                linear::requirements(request).map_err(raster)?,
            ),
            Kernel::Radial(request) => (
                request.format,
                request.roi,
                request.output_context.encoding,
                request.output_context.profile_identity,
                radial::requirements(request).map_err(raster)?,
            ),
            Kernel::CounterNoise { configuration, roi } => (
                configuration.format,
                roi,
                configuration.context.encoding,
                configuration.context.profile_identity,
                noise::requirements(&configuration, roi, configuration.context).map_err(noise_error)?,
            ),
        };
        if domain.format != format || domain.encoding != encoding || domain.profile_fingerprint != Some(profile) {
            return Err(GraphExecuteError::Domain);
        }
        if requirements.source_retained_bytes != 0 || requirements.peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
            return Err(GraphExecuteError::ResidentLimit);
        }
        control.check().map_err(|_| GraphExecuteError::Cancelled)?;
        Ok(Self { kernel, domain, roi, requirements })
    }
    pub fn kernel(&self) -> Kernel<'a> {
        self.kernel
    }
    pub fn domain(&self) -> ExecutionDomain {
        self.domain
    }
    pub fn roi(&self) -> Rect {
        self.roi
    }
    pub fn requirements(&self) -> linear::Requirements {
        self.requirements
    }
}
pub struct Binding<'a, 'data> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub output_port: &'a str,
    pub generator: &'a PreparedGenerator<'data>,
    /// Mandatory independent schema/configuration validator. It must compare
    /// the exact node values with the prepared kernel, including native colors,
    /// seed/ranges/geometry/default and the established CMS interpretation.
    /// It must be bounded and allocation-free (or prepaid by the enclosing owner).
    /// Arbitrary Rust callback allocation/work cannot be constrained by this ledger.
    pub validate_arguments: &'a dyn Fn(&GraphNode, &PreparedGenerator<'data>) -> Result<(), GraphExecuteError>,
}
/// Planner predicate: these primitives have no input/auxiliary port or source.
pub fn accepts_port(binding: &Binding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    direction == PortDirection::Output && port == binding.output_port
}
fn identity_bound(identity: &OperationIdentity) -> bool {
    !identity.name.is_empty()
        && identity.name.len() <= 256
        && !identity.stored_version.is_empty()
        && identity.stored_version.len() <= 64
        && !identity.schema_epoch.is_empty()
        && identity.schema_epoch.len() <= 256
}
fn number(number: &ExactNumber) -> bool {
    match number {
        ExactNumber::Signed { width, .. } | ExactNumber::Unsigned { width, .. } => (1..=64).contains(width),
        ExactNumber::Float32Bits(bits) => f32::from_bits(*bits).is_finite(),
        ExactNumber::Float64Bits(bits) => f64::from_bits(*bits).is_finite(),
    }
}
fn add(left: u64, right: u64) -> Result<u64, GraphExecuteError> {
    left.checked_add(right).ok_or(GraphExecuteError::ResidentLimit)
}
fn identity_bytes(identity: &OperationIdentity) -> Result<u64, GraphExecuteError> {
    add(add(identity.name.capacity() as u64, identity.stored_version.capacity() as u64)?, identity.schema_epoch.capacity() as u64)
}
/// Bounded exact-value envelope only, never a permissive operation validator.
/// Nested configs/graphs/resources/curves require a separately designed adapter.
fn argument_bytes(node: &GraphNode) -> Result<u64, GraphExecuteError> {
    if node.arguments.len() > 64 {
        return Err(GraphExecuteError::Operation("generator argument count"));
    }
    let mut bytes = (node.arguments.capacity() as u64)
        .checked_mul(std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>() as u64)
        .ok_or(GraphExecuteError::IndexLimit)?;
    for argument in &node.arguments {
        if argument.name.is_empty() || argument.name.len() > 128 {
            return Err(GraphExecuteError::Operation("generator argument name"));
        }
        bytes = add(bytes, argument.name.capacity() as u64)?;
        match &argument.value {
            FilterValue::StoredColor(_) => return Err(GraphExecuteError::Operation("StoredColor generator evaluation is unsupported")),
            FilterValue::Number(value) if number(value) => {}
            FilterValue::Bool(_) => {}
            FilterValue::String(value) if value.len() <= 1024 => bytes = add(bytes, value.capacity() as u64)?,
            FilterValue::Enum { domain, .. } if !domain.is_empty() && domain.len() <= 128 => bytes = add(bytes, domain.capacity() as u64)?,
            FilterValue::Color { components, .. } if !components.is_empty() && components.len() <= 4 && components.iter().all(number) => {
                bytes =
                    add(bytes, (components.capacity() as u64).checked_mul(std::mem::size_of::<ExactNumber>() as u64).ok_or(GraphExecuteError::IndexLimit)?)?;
            }
            _ => return Err(GraphExecuteError::Operation("generator argument envelope is unsupported")),
        }
        if bytes > 1 << 20 {
            return Err(GraphExecuteError::IndexLimit);
        }
    }
    if bytes > 1 << 20 {
        return Err(GraphExecuteError::IndexLimit);
    }
    Ok(bytes)
}
/// Ready for use as the scheduler's evaluate callback. All generator-owned
/// categories and retained caller parameters are debited before generation.
/// Output/index overlap is conservatively charged twice; nothing is refunded.
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if !identity_bound(binding.identity)
        || !identity_bound(&request.node.operation)
        || binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity != &request.node.operation
        || binding.version_origin != request.node.version_origin
        || binding.output_port.is_empty()
        || binding.output_port.len() > 256
    {
        return Err(GraphExecuteError::Operation("generator requires exact caller-bound identity/schema/version/output port"));
    }
    if !request.inputs.is_empty() {
        return Err(GraphExecuteError::Operation("generator requires zero inputs"));
    }
    if request.domain != binding.generator.domain() || request.domain.profile_fingerprint.is_none() {
        return Err(GraphExecuteError::Domain);
    }
    if request.roi != binding.generator.roi() {
        return Err(GraphExecuteError::Roi);
    }
    budget.charge_work(4096)?;
    let arguments = argument_bytes(request.node)?;
    let binding_metadata = add(add(identity_bytes(binding.identity)?, identity_bytes(&request.node.operation)?)?, binding.output_port.len() as u64)?;
    if binding_metadata > 1 << 20 {
        return Err(GraphExecuteError::IndexLimit);
    }
    budget.charge_work(add(arguments, binding_metadata)?)?;
    (binding.validate_arguments)(request.node, binding.generator)?;
    budget.poll()?;
    let requirements = binding.generator.requirements();
    let parameters = add(add(requirements.parameter_retained_bytes, arguments)?, binding_metadata)?;
    let output = add(add(requirements.output_retained_bytes, requirements.output_index_bytes)?, 256)?;
    let scratch = add(requirements.scratch_bytes, parameters)?;
    if add(output, scratch)? < requirements.peak_resident_bytes {
        return Err(GraphExecuteError::Reservation);
    }
    budget.reserve_output(output)?;
    budget.reserve_index(requirements.output_index_bytes)?;
    budget.reserve_scratch(scratch)?;
    // Kernel work is debited separately from the prior bounded schema checks.
    budget.charge_work(requirements.work_units)?;
    let surface = match binding.generator.kernel() {
        Kernel::Linear(request) => linear::try_generate(
            request,
            linear::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
            budget.interrupt(),
        )
        .map_err(raster)?,
        Kernel::Radial(request) => radial::try_generate(
            request,
            radial::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
            budget.interrupt(),
        )
        .map_err(raster)?,
        Kernel::CounterNoise { configuration, roi } => noise::try_generate(
            &configuration,
            roi,
            configuration.context,
            noise::Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
            budget.interrupt(),
        )
        .map_err(noise_error)?,
    };
    budget.poll()?;
    budget.finish_output(surface, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{SampleEncoding, xcf_filters::FilterGraph};
    fn domain() -> ExecutionDomain {
        ExecutionDomain {
            format: PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false),
            encoding: SampleEncoding::Linear,
            profile_fingerprint: Some([9; 32]),
        }
    }
    fn context() -> linear::WorkingContext {
        linear::WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [9; 32] }
    }
    fn graph() -> FilterGraph {
        FilterGraph {
            input: None,
            nodes: vec![GraphNode {
                id: GraphNodeId(1),
                operation: OperationIdentity {
                    name: "own:explicit-generator".into(),
                    stored_version: "own-v1".into(),
                    schema_epoch: "own-test-fixed-parameters".into(),
                },
                version_origin: GraphVersionOrigin::Stored,
                arguments: Vec::new(),
            }],
            edges: Vec::new(),
            output: GraphNodeId(1),
            original_pipeline: None,
        }
    }
    fn validate(node: &GraphNode, prepared: &PreparedGenerator<'_>) -> Result<(), GraphExecuteError> {
        // This fixture's fixed configuration is author-owned, not a serialized
        // operation interpretation. Real callers must bind every actual argument.
        if !node.arguments.is_empty() || prepared.domain() != domain() {
            return Err(GraphExecuteError::Operation("own fixed fixture mismatch"));
        }
        Ok(())
    }
    fn binding<'a, 'data>(graph: &'a FilterGraph, generator: &'a PreparedGenerator<'data>) -> Binding<'a, 'data> {
        Binding {
            node: GraphNodeId(1),
            identity: &graph.nodes[0].operation,
            version_origin: GraphVersionOrigin::Stored,
            output_port: "own-output",
            generator,
            validate_arguments: &validate,
        }
    }

    #[test]
    fn all_three_zero_input_variants_execute_through_real_scheduler() {
        let graph = graph();
        let roi = Rect { x0: 0, y0: 0, x1: 2, y1: 1 };
        let start = (-0.0f64).to_ne_bytes();
        let end = 1.0f64.to_ne_bytes();
        let default = (-2.0f64).to_ne_bytes();
        let ranges = [noise::Range { low: 0.0, high: 1.0 }];
        let kernels = [
            Kernel::Linear(linear::Request {
                format: domain().format,
                roi,
                geometry: linear::Geometry { start: [0.5, 0.5], end: [1.5, 0.5], repeat: linear::Repeat::Clamp },
                start: linear::Endpoint { native: &start, context: context() },
                end: linear::Endpoint { native: &end, context: context() },
                output_context: context(),
                output_default: &default,
            }),
            Kernel::Radial(radial::Request {
                format: domain().format,
                roi,
                geometry: radial::Geometry { center: [0.5, 0.5], radii: [1.0, 1.0], repeat: radial::Repeat::Clamp },
                start: linear::Endpoint { native: &start, context: context() },
                end: linear::Endpoint { native: &end, context: context() },
                output_context: context(),
                output_default: &default,
            }),
            Kernel::CounterNoise {
                configuration: noise::Noise {
                    seed: 17,
                    format: domain().format,
                    context: super::super::wide_point::WorkingContext { encoding: domain().encoding, profile_identity: [9; 32] },
                    domain: noise::Domain::FiniteHdr,
                    ranges: &ranges,
                    alpha: None,
                    output_default: &default,
                },
                roi,
            },
        ];
        for kernel in kernels {
            let prepared = PreparedGenerator::prepare(kernel, domain(), &Interrupt::NONE).unwrap();
            assert_eq!(prepared.requirements().source_retained_bytes, 0);
            let binding = binding(&graph, &prepared);
            assert!(!accepts_port(&binding, PortDirection::Input, "own-output"));
            assert!(!accepts_port(&binding, PortDirection::Output, "output"));
            let output = execute_pipeline(
                &graph,
                32,
                None,
                roi,
                domain(),
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&binding, direction, port),
                |request, budget| evaluate(&binding, request, budget),
            )
            .unwrap();
            assert_eq!(output.surface().default_bytes(), default);
            match kernel {
                Kernel::Linear(_) | Kernel::Radial(_) => {
                    let mut expected = start.to_vec();
                    expected.extend_from_slice(&end);
                    assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), expected);
                }
                Kernel::CounterNoise { .. } => {
                    assert_eq!(output.surface().try_read_region_f64(roi).unwrap(), vec![noise::uniform(17, 0, 0, 0), noise::uniform(17, 1, 0, 0)])
                }
            }
        }
    }

    #[test]
    fn missing_profile_validator_rejection_budget_and_cancel_do_not_generate() {
        let graph = graph();
        let roi = Rect { x0: 0, y0: 0, x1: 1, y1: 1 };
        let zero = 0.0f64.to_ne_bytes();
        let ranges = [noise::Range { low: 0.0, high: 1.0 }];
        let kernel = Kernel::CounterNoise {
            configuration: noise::Noise {
                seed: 0,
                format: domain().format,
                context: super::super::wide_point::WorkingContext { encoding: domain().encoding, profile_identity: [9; 32] },
                domain: noise::Domain::Unit,
                ranges: &ranges,
                alpha: None,
                output_default: &zero,
            },
            roi,
        };
        assert!(matches!(
            PreparedGenerator::prepare(kernel, ExecutionDomain { profile_fingerprint: None, ..domain() }, &Interrupt::NONE),
            Err(GraphExecuteError::Domain)
        ));
        assert!(matches!(PreparedGenerator::prepare(kernel, domain(), &Interrupt::cancel_only(&|| true)), Err(GraphExecuteError::Cancelled)));
        let prepared = PreparedGenerator::prepare(kernel, domain(), &Interrupt::NONE).unwrap();
        let binding = binding(&graph, &prepared);
        let tiny = ExecutionLimits { cache_bytes: 1, ..ExecutionLimits::default() };
        assert!(
            execute_pipeline(
                &graph,
                32,
                None,
                roi,
                domain(),
                tiny,
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&binding, direction, port),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        let called = std::cell::Cell::new(0);
        let deny = |_: &GraphNode, _: &PreparedGenerator<'_>| {
            called.set(called.get() + 1);
            Err(GraphExecuteError::Operation("own required validator refusal"))
        };
        let binding = Binding { validate_arguments: &deny, ..binding };
        assert!(
            execute_pipeline(
                &graph,
                32,
                None,
                roi,
                domain(),
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&binding, direction, port),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        assert_eq!(called.get(), 1);
    }

    #[test]
    fn nested_opaque_argument_envelope_is_not_accepted_by_a_permissive_validator() {
        let mut graph = graph();
        graph.nodes[0].arguments.push(photocraft_doc::xcf_filters::FilterArgument { name: "opaque".into(), value: FilterValue::Config(Vec::new()) });
        let roi = Rect { x0: 0, y0: 0, x1: 1, y1: 1 };
        let zero = 0.0f64.to_ne_bytes();
        let ranges = [noise::Range { low: 0.0, high: 1.0 }];
        let prepared = PreparedGenerator::prepare(
            Kernel::CounterNoise {
                configuration: noise::Noise {
                    seed: 0,
                    format: domain().format,
                    context: super::super::wide_point::WorkingContext { encoding: domain().encoding, profile_identity: [9; 32] },
                    domain: noise::Domain::Unit,
                    ranges: &ranges,
                    alpha: None,
                    output_default: &zero,
                },
                roi,
            },
            domain(),
            &Interrupt::NONE,
        )
        .unwrap();
        let called = std::cell::Cell::new(false);
        let validator = |_: &GraphNode, _: &PreparedGenerator<'_>| {
            called.set(true);
            Ok(())
        };
        let binding = Binding { validate_arguments: &validator, ..binding(&graph, &prepared) };
        assert!(
            execute_pipeline(
                &graph,
                32,
                None,
                roi,
                domain(),
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| accepts_port(&binding, direction, port),
                |request, budget| evaluate(&binding, request, budget)
            )
            .is_err()
        );
        assert!(!called.get());
    }
}
