//! Caller-bound native gamma/LUT callbacks for the existing graph scheduler.
//! Prepared math, identity equality and structural planning do not qualify a
//! serialized operation/schema, its arguments, CMS interpretation or source epoch.
//! The required validator owns that independent binding. This bridge never reads
//! JSON/resources, performs CMS, guesses ports or narrows the native format.
use super::{
    graph_execute::{ExecutionDomain, GraphExecuteError, NodeBudget, NodeOutput, NodeRequest},
    graph_plan::PortDirection,
    point_gamma::{self as gamma, PreparedGamma},
    point_native_lut::{self as lut, PreparedNativeLut, TransferMarker},
};
use photocraft_doc::{
    SampleEncoding,
    item_identity::GraphNodeId,
    xcf_filters::{GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_geom::TILE_SIZE;
use photocraft_raster::{MAX_CHECKED_RASTER_BYTES, RasterInputError};

pub enum Kernel<'a, 'data> {
    Gamma(&'a PreparedGamma<'data>),
    Lut(&'a PreparedNativeLut<'data>),
}
pub struct Binding<'a, 'data> {
    pub node: GraphNodeId,
    pub identity: &'a OperationIdentity,
    pub version_origin: GraphVersionOrigin,
    pub domain: ExecutionDomain,
    pub input_port: &'a str,
    pub upstream_output_port: &'a str,
    pub output_port: &'a str,
    pub kernel: Kernel<'a, 'data>,
    /// Mandatory exact configuration/schema validator, supplied by the owner of
    /// the independently interpreted node. No permissive default is provided.
    /// It must be bounded and allocation-free, or have its retained/scratch work
    /// prepaid by the enclosing caller. The ledger cannot police arbitrary Rust.
    pub validate_arguments: &'a dyn Fn(&GraphNode) -> Result<(), GraphExecuteError>,
}
pub fn accepts_port(binding: &Binding<'_, '_>, direction: PortDirection, port: &str) -> bool {
    match direction {
        PortDirection::Input => port == binding.input_port,
        PortDirection::Output => port == binding.output_port,
    }
}
fn add(left: u64, right: u64) -> Result<u64, GraphExecuteError> {
    left.checked_add(right).ok_or(GraphExecuteError::ResidentLimit)
}
fn raster(error: RasterInputError) -> GraphExecuteError {
    if error == RasterInputError::Cancelled { GraphExecuteError::Cancelled } else { GraphExecuteError::Raster(error) }
}
fn gamma_error(error: gamma::Error) -> GraphExecuteError {
    match error {
        gamma::Error::Raster(error) => raster(error),
        _ => GraphExecuteError::Operation("native gamma domain/configuration refusal"),
    }
}
fn lut_domain(context: lut::WorkingContext, domain: ExecutionDomain) -> bool {
    context.format == domain.format
        && domain.profile_fingerprint == Some(context.profile_marker)
        && matches!(
            (context.transfer, domain.encoding),
            (TransferMarker::Linear, SampleEncoding::Linear)
                | (TransferMarker::Nonlinear, SampleEncoding::NonlinearProfile)
                | (TransferMarker::Perceptual, SampleEncoding::Perceptual)
        )
}
/// Exactly one caller-selected input, same native domain and ROI. Adaptive
/// callers must supply an identity footprint: expanded/mapped inputs need their
/// own qualified adapter. Source defaults/boundary bytes stay native.
pub fn evaluate(binding: &Binding<'_, '_>, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
    budget.poll()?;
    if binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity != &request.node.operation
        || binding.version_origin != request.node.version_origin
        || binding.identity.name.is_empty()
        || binding.identity.name.len() > 256
        || binding.identity.stored_version.len() > 64
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || request.node.arguments.len() > 64
        || binding.input_port.is_empty()
        || binding.input_port.len() > 256
        || binding.upstream_output_port.is_empty()
        || binding.upstream_output_port.len() > 256
        || binding.output_port.is_empty()
        || binding.output_port.len() > 256
    {
        return Err(GraphExecuteError::Operation("native point callback requires exact caller identity/schema/ports"));
    }
    if binding.domain != request.domain || request.inputs.len() != 1 {
        return Err(GraphExecuteError::Domain);
    }
    (binding.validate_arguments)(request.node)?;
    budget.poll()?;
    let input = request.inputs.first().ok_or(GraphExecuteError::MissingInput)?;
    if input.input_port != binding.input_port
        || input.output_port != binding.upstream_output_port
        || input.domain != binding.domain
        || input.surface.format() != binding.domain.format
    {
        return Err(GraphExecuteError::Domain);
    }
    if input.roi != request.roi {
        return Err(GraphExecuteError::Roi);
    }
    if let Some(footprint) = input.footprint
        && (footprint.output() != request.roi || footprint.source_read() != request.roi || footprint.support() != request.roi)
    {
        return Err(GraphExecuteError::Roi);
    }
    let (source, parameters, output, index, scratch, peak, work) = match &binding.kernel {
        Kernel::Gamma(kernel) => {
            let context = kernel.context();
            if kernel.format() != binding.domain.format
                || context.encoding != binding.domain.encoding
                || binding.domain.profile_fingerprint != Some(context.profile_identity)
            {
                return Err(GraphExecuteError::Domain);
            }
            let requirements = kernel.requirements_with_interrupt(input.surface, request.roi, context, budget.interrupt()).map_err(gamma_error)?;
            (
                requirements.source_retained_bytes,
                requirements.parameter_retained_bytes,
                requirements.output_retained_bytes,
                requirements.output_index_bytes,
                requirements.scratch_bytes,
                requirements.peak_resident_bytes,
                requirements.work_units,
            )
        }
        Kernel::Lut(kernel) => {
            let context = kernel.context();
            if kernel.format() != binding.domain.format || !lut_domain(context, binding.domain) {
                return Err(GraphExecuteError::Domain);
            }
            let requirements = kernel.requirements_with_interrupt(input.surface, request.roi, context, budget.interrupt()).map_err(raster)?;
            (
                requirements.source_retained_bytes,
                requirements.parameter_retained_bytes,
                requirements.output_retained_bytes,
                requirements.output_index_bytes,
                requirements.scratch_bytes,
                requirements.peak_resident_bytes,
                requirements.work_units,
            )
        }
    };
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(GraphExecuteError::ResidentLimit);
    }
    // Every live upstream owner is already deducted by the scheduler. Source
    // COW is conservatively charged again, never added back to remaining budget.
    // Native map capacity participates in both the output/cache and index limits.
    let output_charge = add(add(output, index)?, 256)?;
    let scratch_charge = add(add(source, parameters)?, scratch)?;
    if add(output_charge, scratch_charge)? < peak {
        return Err(GraphExecuteError::Reservation);
    }
    budget.reserve_output(output_charge)?;
    budget.reserve_index(index)?;
    budget.reserve_scratch(scratch_charge)?;
    let coverage_work =
        u64::try_from(input.surface.tile_count()).map_err(|_| GraphExecuteError::WorkLimit)?.checked_mul(4).ok_or(GraphExecuteError::WorkLimit)?;
    budget.charge_work(work.checked_add(coverage_work).ok_or(GraphExecuteError::WorkLimit)?)?;
    // Kernel COW outputs retain source tiles. Reject unrelated source tiles
    // before execution rather than hiding them behind an uncharged trim pass.
    for (coordinate, _) in input.surface.tiles() {
        budget.poll()?;
        let x = i64::from(coordinate.tx) * i64::from(TILE_SIZE);
        let y = i64::from(coordinate.ty) * i64::from(TILE_SIZE);
        if x >= i64::from(request.roi.x1)
            || x + i64::from(TILE_SIZE) <= i64::from(request.roi.x0)
            || y >= i64::from(request.roi.y1)
            || y + i64::from(TILE_SIZE) <= i64::from(request.roi.y0)
        {
            return Err(GraphExecuteError::Roi);
        }
    }

    let output = match &binding.kernel {
        Kernel::Gamma(kernel) => kernel
            .apply(input.surface, request.roi, kernel.context(), gamma::Limits { resident_bytes: peak, work_units: work }, budget.interrupt())
            .map_err(gamma_error)?,
        Kernel::Lut(kernel) => kernel
            .apply(input.surface, request.roi, kernel.context(), lut::LutLimits { resident_bytes: peak, work_units: work }, budget.interrupt())
            .map_err(raster)?,
    };
    budget.poll()?;
    // Checks actual retained output capacity/tiles/domain against the reservation;
    // performed work and retained upstream bytes are never refunded here.
    budget.finish_output(output, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::super::{graph_execute::ExecutionLimits, graph_pipeline::execute_pipeline, wide_point::WorkingContext};
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::xcf_filters::{FilterGraph, GraphEdge};
    use photocraft_geom::Rect;
    use photocraft_raster::{Interrupt, Surface};

    fn graph() -> FilterGraph {
        let node = |id, name: &str| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: name.into(), stored_version: "own".into(), schema_epoch: "own-independent-binding".into() },
            version_origin: Default::default(),
            arguments: vec![],
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), output_port: "own-output".into(), input_port: "own-input".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1, "own:gamma"), node(2, "own:lut")],
            edges: vec![edge(9, 1), edge(1, 2)],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }
    fn validate(node: &GraphNode) -> Result<(), GraphExecuteError> {
        if !node.arguments.is_empty() {
            return Err(GraphExecuteError::Operation("own fixture has no arguments"));
        }
        Ok(())
    }
    #[test]
    fn two_prepared_nodes_execute_f64_without_narrowing_raw_alpha_or_outside_defaults() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let current = WorkingContext { encoding: domain.encoding, profile_identity: [7; 32] };
        let alpha = 0x7ff8000000000042u64;
        let original = [4f64.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(format, &original).unwrap();
        let before = source.clone();
        let roi = Rect::new(0, 0, 1, 1);
        let exponents = vec![gamma::ChannelExponent { channel: 0, exponent: 2. }];
        let prepared_gamma = PreparedGamma::prepare(
            format,
            current,
            &exponents,
            gamma::Policy { negative: gamma::NegativePolicy::Reject, domain: gamma::Domain::FiniteHdr, overflow: gamma::OverflowPolicy::Reject },
            gamma::Limits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let table = vec![(16f64.to_bits(), 25f64.to_bits())];
        let channels = vec![lut::ChannelTable { channel: 0, table: lut::NativeTable::ExactF64Bits(&table) }];
        let prepared_lut = PreparedNativeLut::prepare(
            lut::WorkingContext { format, transfer: TransferMarker::Linear, profile_marker: [7; 32] },
            &channels,
            lut::LookupPolicy::ExactCodesOnly,
            lut::LutLimits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let output = execute_pipeline(
            &graph,
            32,
            Some(&source),
            roi,
            domain,
            ExecutionLimits::default(),
            &Interrupt::NONE,
            |_, direction, port| matches!((direction, port), (PortDirection::Input, "own-input") | (PortDirection::Output, "own-output")),
            |request, budget| {
                let kernel = if request.node.id == GraphNodeId(1) { Kernel::Gamma(&prepared_gamma) } else { Kernel::Lut(&prepared_lut) };
                let binding = Binding {
                    node: request.node.id,
                    identity: &request.node.operation,
                    version_origin: request.node.version_origin,
                    domain,
                    input_port: "own-input",
                    upstream_output_port: "own-output",
                    output_port: "own-output",
                    kernel,
                    validate_arguments: &validate,
                };
                evaluate(&binding, request, budget)
            },
        )
        .unwrap();
        let expected = [25f64.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), expected);
        assert_eq!(output.surface().default_bytes(), original.as_slice());
        assert_eq!(source, before);
    }
    #[test]
    fn missing_profile_marker_is_not_an_implicit_working_profile() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let context = lut::WorkingContext { format, transfer: TransferMarker::Linear, profile_marker: [0; 32] };
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: None };
        assert!(!lut_domain(context, domain));
        assert!(!lut_domain(
            lut::WorkingContext { transfer: TransferMarker::Opaque(1), ..context },
            ExecutionDomain { profile_fingerprint: Some([0; 32]), ..domain }
        ));
    }
    #[test]
    fn source_is_exact_on_graph_budget_refusal_and_early_cancellation() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let bytes = (-0f64).to_bits().to_ne_bytes();
        let source = Surface::try_with_default_bytes(format, &bytes).unwrap();
        let before = source.clone();
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let roi = Rect::new(0, 0, 1, 1);
        let lower = ExecutionLimits { work_units: 0, ..Default::default() };
        assert!(
            execute_pipeline(&graph, 32, Some(&source), roi, domain, lower, &Interrupt::NONE, |_, _, _| true, |_, _| Err(GraphExecuteError::WorkLimit))
                .is_err()
        );
        assert!(
            execute_pipeline(
                &graph,
                32,
                Some(&source),
                roi,
                domain,
                ExecutionLimits::default(),
                &Interrupt::cancel_only(&|| true),
                |_, _, _| true,
                |_, _| Err(GraphExecuteError::Cancelled)
            )
            .is_err()
        );
        assert_eq!(source, before);
        assert_eq!(source.default_bytes(), bytes.as_slice());
    }
    #[test]
    fn mapped_gamma_and_lut_refuse_same_format_upstream_encoding_or_profile_mismatch() {
        use super::super::{
            graph_execute::DomainAssignment,
            graph_pipeline::{GraphPipelineError, execute_pipeline_domains},
        };
        let mut graph = graph();
        graph.nodes.truncate(1);
        graph.edges.retain(|edge| edge.to == GraphNodeId(1));
        graph.output = GraphNodeId(1);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([7; 32]) };
        let raw = [(-0f64).to_bits(), 0x7ff8000000000055].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let source = Surface::try_with_default_bytes(format, &raw).unwrap();
        let before = source.clone();
        let roi = Rect::new(0, 0, 1, 1);
        let channels = vec![gamma::ChannelExponent { channel: 0, exponent: 1. }];
        let prepared_gamma = PreparedGamma::prepare(
            format,
            WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [7; 32] },
            &channels,
            gamma::Policy { negative: gamma::NegativePolicy::SignedMagnitude, domain: gamma::Domain::FiniteHdr, overflow: gamma::OverflowPolicy::Reject },
            gamma::Limits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let table = vec![((-0f64).to_bits(), (-0f64).to_bits())];
        let tables = vec![lut::ChannelTable { channel: 0, table: lut::NativeTable::ExactF64Bits(&table) }];
        let prepared_lut = PreparedNativeLut::prepare(
            lut::WorkingContext { format, transfer: TransferMarker::Linear, profile_marker: [7; 32] },
            &tables,
            lut::LookupPolicy::ExactCodesOnly,
            lut::LutLimits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let assignments = [DomainAssignment { node: GraphNodeId(1), domain }];
        for is_lut in [false, true] {
            for upstream in
                [ExecutionDomain { encoding: SampleEncoding::NonlinearProfile, ..domain }, ExecutionDomain { profile_fingerprint: Some([9; 32]), ..domain }]
            {
                let mut callbacks = 0usize;
                let result = execute_pipeline_domains(
                    &graph,
                    32,
                    Some(&source),
                    roi,
                    upstream,
                    &assignments,
                    ExecutionLimits::default(),
                    &Interrupt::NONE,
                    |_, direction, port| matches!((direction, port), (PortDirection::Input, "own-input") | (PortDirection::Output, "own-output")),
                    |request, budget| {
                        callbacks += 1;
                        assert_eq!(request.domain, domain);
                        assert_eq!(request.inputs.first().unwrap().domain, upstream);
                        let kernel = if is_lut { Kernel::Lut(&prepared_lut) } else { Kernel::Gamma(&prepared_gamma) };
                        let binding = Binding {
                            node: request.node.id,
                            identity: &request.node.operation,
                            version_origin: request.node.version_origin,
                            domain,
                            input_port: "own-input",
                            upstream_output_port: "own-output",
                            output_port: "own-output",
                            kernel,
                            validate_arguments: &validate,
                        };
                        evaluate(&binding, request, budget)
                    },
                );
                assert!(matches!(result, Err(GraphPipelineError::Execute(GraphExecuteError::Domain))));
                assert_eq!(callbacks, 1);
                assert_eq!(source, before);
                assert_eq!(source.default_bytes(), raw.as_slice());
            }
            let output = execute_pipeline_domains(
                &graph,
                32,
                Some(&source),
                roi,
                domain,
                &assignments,
                ExecutionLimits::default(),
                &Interrupt::NONE,
                |_, direction, port| matches!((direction, port), (PortDirection::Input, "own-input") | (PortDirection::Output, "own-output")),
                |request, budget| {
                    let kernel = if is_lut { Kernel::Lut(&prepared_lut) } else { Kernel::Gamma(&prepared_gamma) };
                    let binding = Binding {
                        node: request.node.id,
                        identity: &request.node.operation,
                        version_origin: request.node.version_origin,
                        domain,
                        input_port: "own-input",
                        upstream_output_port: "own-output",
                        output_port: "own-output",
                        kernel,
                        validate_arguments: &validate,
                    };
                    evaluate(&binding, request, budget)
                },
            )
            .unwrap();
            assert_eq!(output.surface().try_read_interleaved(roi).unwrap(), raw);
        }
    }
}
