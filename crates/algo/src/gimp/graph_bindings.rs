//! Prepared exact native-point binding index, not source admission or an executor.
//! Only Gamma/LUT bindings are included: other adapters have independent geometry,
//! footprints and domain contracts and are not coerced into this abstraction.
//! The existing planner still proves DAG/depth/ROI properties before execution.
use super::{
    graph_execute::{ExecutionDomain, ExecutionLimits, GraphExecuteError, GraphOutput, NodeBudget, NodeOutput, NodeRequest},
    graph_pipeline::{GraphPipelineError, execute_pipeline, execute_pipeline_with_roi},
    graph_plan::PortDirection,
    graph_roi::InputFootprint,
    native_point_graph::{self, Binding},
    roi::{Abyss, Halo},
};
use photocraft_doc::{item_identity::GraphNodeId, xcf_filters::FilterGraph};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use std::mem::size_of;

const MAX_NODES: usize = 4096;
const MAX_EDGES: usize = 16384;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        let standard = ExecutionLimits::default();
        Self { resident_bytes: standard.resident_bytes, index_bytes: standard.index_bytes, work_units: standard.work_units }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    /// Live collection plus any explicitly attached known borrowed schema metadata.
    pub retained_resident_bytes: u64,
    /// Live collection index and attached schema-owned indices. Borrowed graph
    /// and prepared kernels remain charged by their enclosing owner.
    pub retained_index_bytes: u64,
    /// Conservative pre-allocation index/sort/validation peak, checked at prepare.
    pub preparation_peak_bytes: u64,
    /// Collection visits plus metered attached-schema validation/lookup work.
    /// Kernel-binder work remains prepaid by its enclosing owner.
    pub preparation_work_units: u64,
}
#[derive(Clone, Copy)]
struct Entry {
    node: GraphNodeId,
    graph: usize,
    binding: usize,
    incoming: u8,
}
pub struct PreparedBindings<'a, 'data> {
    graph: &'a FilterGraph,
    bindings: &'a [Binding<'a, 'data>],
    external_output_port: &'a str,
    entries: Vec<Entry>,
    requirements: Requirements,
}
fn refusal(message: &'static str) -> GraphExecuteError {
    GraphExecuteError::Operation(message)
}
fn poll(control: &Interrupt<'_>) -> Result<(), GraphExecuteError> {
    control.check().map_err(|_| GraphExecuteError::Cancelled)
}
fn port(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}
impl<'a, 'data> PreparedBindings<'a, 'data> {
    /// Opt-in structural schema pass before mandatory kernel argument binding.
    /// Defaults are not resolved or inserted, and this does not enroll operations
    /// in a registry or grant rendering/source admission. Cached known schema
    /// metadata is conservatively retained in the returned budget declaration;
    /// opaque validator captures/backing beyond the borrowed census remain the
    /// enclosing owner's responsibility. Held schema-owned indices are included
    /// in the index debit as well as the known resident census, without adding
    /// them twice to resident storage.
    pub fn prepare_with_schemas(
        graph: &'a FilterGraph,
        bindings: &'a [Binding<'a, 'data>],
        external_output_port: &'a str,
        schemas: &super::typed_schema::PreparedSchemaIndex<'_, '_>,
        limits: Limits,
        control: &Interrupt<'_>,
    ) -> Result<Self, GraphExecuteError> {
        poll(control)?;
        let standard = Limits::default();
        if limits.resident_bytes > standard.resident_bytes || limits.index_bytes > standard.index_bytes || limits.work_units > standard.work_units {
            return Err(GraphExecuteError::RaisedLimit);
        }
        if graph.nodes.len() > MAX_NODES || graph.edges.len() > MAX_EDGES || bindings.len() != graph.nodes.len() {
            return Err(refusal("native schema binding count limit"));
        }
        let schema_index = schemas.requirements();
        let live = schema_index.owned_index_bytes.checked_add(schema_index.known_borrowed_bytes).ok_or(GraphExecuteError::ResidentLimit)?;
        let live_index = schema_index.owned_index_bytes.checked_add(schema_index.held_schema_index_bytes).ok_or(GraphExecuteError::IndexLimit)?;
        if live > limits.resident_bytes {
            return Err(GraphExecuteError::ResidentLimit);
        }
        if live_index > limits.index_bytes {
            return Err(GraphExecuteError::IndexLimit);
        }
        let base_work = (graph.nodes.len() as u64)
            .checked_mul(64)
            .and_then(|work| work.checked_add((graph.edges.len() as u64).checked_mul(32)?))
            .ok_or(GraphExecuteError::WorkLimit)?;
        let mut remaining = limits.work_units.checked_sub(base_work).ok_or(GraphExecuteError::WorkLimit)?;
        let mut performed = 0u64;
        let mut validation_peak = 0u64;
        for node in &graph.nodes {
            poll(control)?;
            remaining = remaining.checked_sub(schema_index.lookup_work_units).ok_or(GraphExecuteError::WorkLimit)?;
            performed = performed.checked_add(schema_index.lookup_work_units).ok_or(GraphExecuteError::WorkLimit)?;
            let schema = schemas.lookup(node, control).map_err(schema_error)?;
            let declared = schema.requirements();
            // Reserve the selected validator's declared ceiling before running
            // it. Actual performed work is recorded below; unused capacity is
            // not treated as performed work or an upstream residency refund.
            if declared.validation_work_limit > remaining {
                return Err(GraphExecuteError::WorkLimit);
            }
            validation_peak = validation_peak.max(declared.validation_retained_limit);
            if live.checked_add(validation_peak).is_none_or(|peak| peak > limits.resident_bytes) {
                return Err(GraphExecuteError::ResidentLimit);
            }
            let (_, usage) = schema.validate_node_with_usage(node, control).map_err(schema_error)?;
            remaining = remaining.checked_sub(usage.work_units).ok_or(GraphExecuteError::WorkLimit)?;
            performed = performed.checked_add(usage.work_units).ok_or(GraphExecuteError::WorkLimit)?;
        }
        let preparation_extra = live.checked_add(validation_peak).ok_or(GraphExecuteError::ResidentLimit)?;
        let reduced = Limits {
            resident_bytes: limits.resident_bytes.checked_sub(preparation_extra).ok_or(GraphExecuteError::ResidentLimit)?,
            index_bytes: limits.index_bytes.checked_sub(live_index).ok_or(GraphExecuteError::IndexLimit)?,
            work_units: limits.work_units.checked_sub(performed).ok_or(GraphExecuteError::WorkLimit)?,
        };
        // Immutable graph borrow: the original prepare can now run each required
        // kernel binder without rehashing or reconstructing schema/default ASTs.
        let mut prepared = Self::prepare(graph, bindings, external_output_port, reduced, control)?;
        prepared.requirements.retained_resident_bytes =
            prepared.requirements.retained_resident_bytes.checked_add(live).ok_or(GraphExecuteError::ResidentLimit)?;
        prepared.requirements.retained_index_bytes = prepared.requirements.retained_index_bytes.checked_add(live_index).ok_or(GraphExecuteError::IndexLimit)?;
        prepared.requirements.preparation_peak_bytes =
            prepared.requirements.preparation_peak_bytes.checked_add(preparation_extra).ok_or(GraphExecuteError::ResidentLimit)?;
        prepared.requirements.preparation_work_units =
            prepared.requirements.preparation_work_units.checked_add(performed).ok_or(GraphExecuteError::WorkLimit)?;
        poll(control)?;
        Ok(prepared)
    }
    /// Input strings/data are borrowed and immutable. All declared nodes must
    /// have exactly one binding and one incoming edge. No unknown-node fallback
    /// or operation-name dispatch is provided. Validators are mandatory and must
    /// be bounded/nonallocating or prepaid by the enclosing owner.
    pub fn prepare(
        graph: &'a FilterGraph,
        bindings: &'a [Binding<'a, 'data>],
        external_output_port: &'a str,
        limits: Limits,
        control: &Interrupt<'_>,
    ) -> Result<Self, GraphExecuteError> {
        poll(control)?;
        let standard = Limits::default();
        if limits.resident_bytes > standard.resident_bytes || limits.index_bytes > standard.index_bytes || limits.work_units > standard.work_units {
            return Err(GraphExecuteError::RaisedLimit);
        }
        if graph.nodes.len() > MAX_NODES || graph.edges.len() > MAX_EDGES || bindings.len() != graph.nodes.len() || !port(external_output_port) {
            return Err(refusal("native graph binding count/port limit"));
        }
        let count = u64::try_from(graph.nodes.len()).map_err(|_| GraphExecuteError::IndexLimit)?;
        let retained =
            count.checked_mul(size_of::<Entry>() as u64).and_then(|bytes| bytes.checked_add(size_of::<Self>() as u64)).ok_or(GraphExecuteError::IndexLimit)?;
        // sort_unstable is in-place. Reserve a fixed conservative implementation
        // stack envelope; this is not arbitrary allocator/process RSS accounting.
        let peak = retained.checked_add(8192).ok_or(GraphExecuteError::IndexLimit)?;
        let work = count.checked_mul(64).and_then(|work| work.checked_add((graph.edges.len() as u64).checked_mul(32)?)).ok_or(GraphExecuteError::WorkLimit)?;
        if peak > limits.resident_bytes {
            return Err(GraphExecuteError::ResidentLimit);
        }
        if peak > limits.index_bytes {
            return Err(GraphExecuteError::IndexLimit);
        }
        if work > limits.work_units {
            return Err(GraphExecuteError::WorkLimit);
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(graph.nodes.len()).map_err(|_| GraphExecuteError::Allocation)?;
        let actual = (entries.capacity() as u64)
            .checked_mul(size_of::<Entry>() as u64)
            .and_then(|bytes| bytes.checked_add(size_of::<Self>() as u64))
            .ok_or(GraphExecuteError::IndexLimit)?;
        if actual.checked_add(8192).is_none_or(|bytes| bytes > limits.resident_bytes) {
            return Err(GraphExecuteError::ResidentLimit);
        }
        if actual.checked_add(8192).is_none_or(|bytes| bytes > limits.index_bytes) {
            return Err(GraphExecuteError::IndexLimit);
        }
        for (index, node) in graph.nodes.iter().enumerate() {
            poll(control)?;
            if node.id.0 == 0 {
                return Err(refusal("zero graph node binding identity"));
            }
            entries.push(Entry { node: node.id, graph: index, binding: usize::MAX, incoming: 0 });
        }
        entries.sort_unstable_by_key(|entry| entry.node.0);
        poll(control)?;
        if entries.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(left, right)| left.node == right.node)) {
            return Err(refusal("duplicate graph node identity"));
        }
        if graph.input.is_some_and(|input| input.0 == 0 || entries.binary_search_by_key(&input.0, |entry| entry.node.0).is_ok()) {
            return Err(refusal("external input aliases a native graph node"));
        }
        for (index, binding) in bindings.iter().enumerate() {
            poll(control)?;
            let slot = entries.binary_search_by_key(&binding.node.0, |entry| entry.node.0).map_err(|_| refusal("binding has no declared node"))?;
            let entry = entries.get_mut(slot).ok_or(GraphExecuteError::InvalidPlan)?;
            if entry.binding != usize::MAX {
                return Err(refusal("duplicate prepared node binding"));
            }
            let node = graph.nodes.get(entry.graph).ok_or(GraphExecuteError::InvalidPlan)?;
            if binding.identity != &node.operation
                || binding.version_origin != node.version_origin
                || binding.identity.schema_epoch.is_empty()
                || binding.identity.name.is_empty()
                || binding.identity.name.len() > 256
                || binding.identity.stored_version.len() > 64
                || binding.identity.schema_epoch.len() > 256
                || node.arguments.len() > 64
                || !port(binding.input_port)
                || !port(binding.output_port)
                || !port(binding.upstream_output_port)
            {
                return Err(refusal("native node binding identity/schema/port mismatch"));
            }
            if bindings.first().is_some_and(|first| first.domain != binding.domain) {
                return Err(GraphExecuteError::Domain);
            }
            let domain_valid = match &binding.kernel {
                native_point_graph::Kernel::Gamma(kernel) => {
                    kernel.format() == binding.domain.format
                        && kernel.context().encoding == binding.domain.encoding
                        && binding.domain.profile_fingerprint == Some(kernel.context().profile_identity)
                }
                native_point_graph::Kernel::Lut(kernel) => {
                    let context = kernel.context();
                    kernel.format() == binding.domain.format
                        && binding.domain.profile_fingerprint == Some(context.profile_marker)
                        && matches!(
                            (context.transfer, binding.domain.encoding),
                            (super::point_native_lut::TransferMarker::Linear, photocraft_doc::SampleEncoding::Linear)
                                | (super::point_native_lut::TransferMarker::Nonlinear, photocraft_doc::SampleEncoding::NonlinearProfile)
                                | (super::point_native_lut::TransferMarker::Perceptual, photocraft_doc::SampleEncoding::Perceptual)
                        )
                }
            };
            if !domain_valid {
                return Err(GraphExecuteError::Domain);
            }
            (binding.validate_arguments)(node)?;
            poll(control)?;
            entry.binding = index;
        }
        let lookup = |id: GraphNodeId| {
            entries.binary_search_by_key(&id.0, |entry| entry.node.0).ok().and_then(|index| entries.get(index)).and_then(|entry| bindings.get(entry.binding))
        };
        let mut port_bytes = 0usize;
        for edge in &graph.edges {
            poll(control)?;
            if !port(&edge.input_port) || !port(&edge.output_port) {
                return Err(refusal("invalid exact graph port declaration"));
            }
            port_bytes = port_bytes
                .checked_add(edge.input_port.len())
                .and_then(|n| n.checked_add(edge.output_port.len()))
                .filter(|n| *n <= 64 << 10)
                .ok_or(GraphExecuteError::IndexLimit)?;
            let to = lookup(edge.to).ok_or_else(|| refusal("native edge target has no binding"))?;
            let expected_output = if Some(edge.from) == graph.input {
                external_output_port
            } else {
                lookup(edge.from).ok_or_else(|| refusal("native edge source has no binding"))?.output_port
            };
            if edge.input_port != to.input_port || edge.output_port != expected_output || edge.output_port != to.upstream_output_port {
                return Err(refusal("native binding port connection mismatch"));
            }
        }
        // Separate mutable pass avoids retaining a lookup borrow while updating
        // incoming counts. The edge count already bounds this deterministic work.
        for edge in &graph.edges {
            poll(control)?;
            let slot = entries.binary_search_by_key(&edge.to.0, |entry| entry.node.0).map_err(|_| GraphExecuteError::InvalidPlan)?;
            let entry = entries.get_mut(slot).ok_or(GraphExecuteError::InvalidPlan)?;
            entry.incoming = entry.incoming.checked_add(1).filter(|incoming| *incoming == 1).ok_or_else(|| refusal("duplicate native input binding"))?;
        }
        if entries.iter().any(|entry| entry.binding == usize::MAX || entry.incoming != 1) {
            return Err(refusal("native node is missing its prepared input binding"));
        }
        if graph.output.0 == 0 || (Some(graph.output) != graph.input && entries.binary_search_by_key(&graph.output.0, |entry| entry.node.0).is_err()) {
            return Err(refusal("missing bound graph output"));
        }
        poll(control)?;
        Ok(Self {
            graph,
            bindings,
            external_output_port,
            entries,
            requirements: Requirements {
                retained_resident_bytes: actual,
                retained_index_bytes: actual,
                preparation_peak_bytes: actual.checked_add(8192).ok_or(GraphExecuteError::IndexLimit)?,
                preparation_work_units: work,
            },
        })
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    /// Debit this live collection and already performed preparation before
    /// supplying limits to graph_pipeline. Call once for each enclosing execution;
    /// no upstream/cache credit, cap raise or serialized budget is introduced.
    pub fn execution_limits(&self, mut limits: ExecutionLimits) -> Result<ExecutionLimits, GraphExecuteError> {
        let standard = ExecutionLimits::default();
        if limits.resident_bytes > standard.resident_bytes
            || limits.cache_bytes > standard.cache_bytes
            || limits.index_bytes > standard.index_bytes
            || limits.work_units > standard.work_units
        {
            return Err(GraphExecuteError::RaisedLimit);
        }
        limits.resident_bytes = limits.resident_bytes.checked_sub(self.requirements.retained_resident_bytes).ok_or(GraphExecuteError::ResidentLimit)?;
        limits.index_bytes = limits.index_bytes.checked_sub(self.requirements.retained_index_bytes).ok_or(GraphExecuteError::IndexLimit)?;
        limits.work_units = limits.work_units.checked_sub(self.requirements.preparation_work_units).ok_or(GraphExecuteError::WorkLimit)?;
        Ok(limits)
    }
    fn execution_domain(&self, external: Option<&Surface>, domain: ExecutionDomain, control: &Interrupt<'_>) -> Result<(), GraphPipelineError> {
        poll(control).map_err(GraphPipelineError::Execute)?;
        if self.graph.input.is_some() != external.is_some() {
            return Err(GraphPipelineError::Execute(GraphExecuteError::MissingInput));
        }
        if external.is_some_and(|surface| surface.format() != domain.format) || self.bindings.first().is_some_and(|binding| binding.domain != domain) {
            return Err(GraphPipelineError::Execute(GraphExecuteError::Domain));
        }
        // The immutable collection already established uniform kernel contexts.
        // External profile/encoding metadata remains an explicit caller assertion;
        // matching storage cannot validate ICC bytes or grant source admission.
        Ok(())
    }
    /// Execute exactly this prepared graph, with automatic live-index and
    /// preparation-work debit. No graph argument or alternate node/domain map can
    /// bypass the immutable witnesses. Empty external passthrough uses the
    /// caller's explicit domain; absence of a profile never implies one.
    pub fn execute<'source>(
        &self,
        external: Option<&'source Surface>,
        roi: Rect,
        domain: ExecutionDomain,
        max_depth: usize,
        limits: ExecutionLimits,
        control: &Interrupt<'_>,
    ) -> Result<GraphOutput<'source>, GraphPipelineError> {
        self.execution_domain(external, domain, control)?;
        let limits = self.execution_limits(limits).map_err(GraphPipelineError::Execute)?;
        execute_pipeline(
            self.graph,
            max_depth,
            external,
            roi,
            domain,
            limits,
            control,
            |id, direction, port| self.port_allowed(id, direction, port),
            |request, budget| self.evaluate(request, budget),
        )
    }
    /// Adaptive scheduling is restricted to identity point footprints. Source
    /// and output share the explicit requested coordinate region; there is no
    /// halo, abyss extension, coordinate remap or implicit format conversion.
    pub fn execute_with_roi<'source>(
        &self,
        external: Option<&'source Surface>,
        roi: Rect,
        domain: ExecutionDomain,
        max_depth: usize,
        limits: ExecutionLimits,
        control: &Interrupt<'_>,
    ) -> Result<GraphOutput<'source>, GraphPipelineError> {
        self.execution_domain(external, domain, control)?;
        let limits = self.execution_limits(limits).map_err(GraphPipelineError::Execute)?;
        execute_pipeline_with_roi(
            self.graph,
            max_depth,
            external,
            roi,
            domain,
            limits,
            control,
            |id, direction, port| self.port_allowed(id, direction, port),
            |request| Ok(InputFootprint { mapped_output: request.output, source_extent: roi, halo: Halo::default(), abyss: Abyss::None, work_per_sample: 1 }),
            |request, budget| self.evaluate(request, budget),
        )
    }
    fn entry(&self, node: GraphNodeId) -> Option<&Entry> {
        self.entries.binary_search_by_key(&node.0, |entry| entry.node.0).ok().and_then(|index| self.entries.get(index))
    }
    pub fn port_allowed(&self, node: GraphNodeId, direction: PortDirection, port: &str) -> bool {
        if Some(node) == self.graph.input {
            return direction == PortDirection::Output && port == self.external_output_port;
        }
        self.entry(node).and_then(|entry| self.bindings.get(entry.binding)).is_some_and(|binding| native_point_graph::accepts_port(binding, direction, port))
    }
    pub fn evaluate(&self, request: NodeRequest<'_>, budget: &mut NodeBudget<'_, '_>) -> Result<NodeOutput, GraphExecuteError> {
        budget.poll()?;
        let entry = self.entry(request.node.id).ok_or_else(|| refusal("unknown native prepared node"))?;
        let original = self.graph.nodes.get(entry.graph).ok_or(GraphExecuteError::InvalidPlan)?;
        // Identical metadata from a different graph is not this prepared borrow.
        if !std::ptr::eq(original, request.node) {
            return Err(refusal("request is not the prepared graph node"));
        }
        let binding = self.bindings.get(entry.binding).ok_or(GraphExecuteError::InvalidPlan)?;
        // The mandatory caller validator ran before this immutable graph/index
        // was published. Reuse that exact borrowed-node witness, never a generic
        // accepting validator or a repeated preparation/hash on each evaluation.
        let validated = |node: &photocraft_doc::xcf_filters::GraphNode| {
            if std::ptr::eq(original, node) { Ok(()) } else { Err(refusal("arguments are not the prepared graph node")) }
        };
        let kernel = match &binding.kernel {
            native_point_graph::Kernel::Gamma(kernel) => native_point_graph::Kernel::Gamma(kernel),
            native_point_graph::Kernel::Lut(kernel) => native_point_graph::Kernel::Lut(kernel),
        };
        let checked = Binding {
            node: binding.node,
            identity: binding.identity,
            version_origin: binding.version_origin,
            domain: binding.domain,
            input_port: binding.input_port,
            upstream_output_port: binding.upstream_output_port,
            output_port: binding.output_port,
            kernel,
            validate_arguments: &validated,
        };
        native_point_graph::evaluate(&checked, request, budget)
    }
}

fn schema_error(error: super::typed_schema::Error) -> GraphExecuteError {
    match error {
        super::typed_schema::Error::Cancelled => GraphExecuteError::Cancelled,
        super::typed_schema::Error::RaisedLimits => GraphExecuteError::RaisedLimit,
        _ => refusal("exact typed graph schema validation refused"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        graph_execute::ExecutionDomain,
        native_point_graph::Kernel,
        point_gamma::{self as gamma, PreparedGamma},
        wide_point::WorkingContext,
    };
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{
        SampleEncoding,
        xcf_filters::{GraphEdge, GraphNode, OperationIdentity},
    };
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn graph() -> FilterGraph {
        let node = |id| GraphNode {
            id: GraphNodeId(id),
            operation: OperationIdentity { name: "own:prepared-gamma".into(), stored_version: "own".into(), schema_epoch: "own-configuration".into() },
            version_origin: Default::default(),
            arguments: vec![],
        };
        let edge = |from, to| GraphEdge { from: GraphNodeId(from), to: GraphNodeId(to), input_port: "in".into(), output_port: "out".into() };
        FilterGraph {
            input: Some(GraphNodeId(9)),
            nodes: vec![node(1), node(2)],
            edges: vec![edge(9, 1), edge(1, 2)],
            output: GraphNodeId(2),
            original_pipeline: None,
        }
    }
    #[test]
    fn prepared_index_dispatches_pipeline_and_runs_caller_validators_only_once() {
        let graph = graph();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([3; 32]) };
        let context = WorkingContext { encoding: domain.encoding, profile_identity: [3; 32] };
        let channels = vec![gamma::ChannelExponent { channel: 0, exponent: 1. }];
        let kernel = PreparedGamma::prepare(
            format,
            context,
            &channels,
            gamma::Policy { negative: gamma::NegativePolicy::SignedMagnitude, domain: gamma::Domain::FiniteHdr, overflow: gamma::OverflowPolicy::Reject },
            gamma::Limits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let calls = AtomicUsize::new(0);
        let validate = |node: &GraphNode| {
            calls.fetch_add(1, Ordering::Relaxed);
            if node.arguments.is_empty() { Ok(()) } else { Err(refusal("unexpected own arguments")) }
        };
        let bindings: Vec<_> = graph
            .nodes
            .iter()
            .map(|node| Binding {
                node: node.id,
                identity: &node.operation,
                version_origin: node.version_origin,
                domain,
                input_port: "in",
                upstream_output_port: "out",
                output_port: "out",
                kernel: Kernel::Gamma(&kernel),
                validate_arguments: &validate,
            })
            .collect();
        let prepared = PreparedBindings::prepare(&graph, &bindings, "out", Limits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(!prepared.port_allowed(GraphNodeId(100), PortDirection::Input, "in"));
        let raw = (-0f64).to_bits().to_ne_bytes();
        let source = Surface::try_with_default_bytes(format, &raw).unwrap();
        let before = source.clone();
        let output = prepared.execute(Some(&source), Rect::new(0, 0, 1, 1), domain, 32, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(source, before);
        let adaptive = prepared.execute_with_roi(Some(&source), Rect::new(0, 0, 1, 1), domain, 32, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(adaptive.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(
            prepared
                .execute(
                    Some(&source),
                    Rect::new(0, 0, 1, 1),
                    ExecutionDomain { profile_fingerprint: None, ..domain },
                    32,
                    ExecutionLimits::default(),
                    &Interrupt::NONE
                )
                .is_err()
        );
        assert!(prepared.execute(None, Rect::new(0, 0, 1, 1), domain, 32, ExecutionLimits::default(), &Interrupt::NONE).is_err());
        assert!(
            prepared
                .execute(Some(&source), Rect::new(0, 0, 1, 1), domain, 32, ExecutionLimits { index_bytes: 0, ..Default::default() }, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(source, before);
        assert!(prepared.execution_limits(ExecutionLimits { index_bytes: 0, ..Default::default() }).is_err());
    }
    #[test]
    fn missing_bindings_budget_raise_and_early_cancel_never_publish_collection() {
        let graph = graph();
        let before = graph.clone();
        assert!(PreparedBindings::prepare(&graph, &[], "out", Limits::default(), &Interrupt::NONE).is_err());
        assert!(matches!(
            PreparedBindings::prepare(&graph, &[], "out", Limits { resident_bytes: u64::MAX, ..Default::default() }, &Interrupt::NONE),
            Err(GraphExecuteError::RaisedLimit)
        ));
        assert!(matches!(
            PreparedBindings::prepare(&graph, &[], "out", Limits::default(), &Interrupt::cancel_only(&|| true)),
            Err(GraphExecuteError::Cancelled)
        ));
        assert_eq!(graph, before);
    }
    #[test]
    fn empty_passthrough_still_requires_bounded_index_and_exact_external_port() {
        let graph = FilterGraph { input: Some(GraphNodeId(7)), nodes: vec![], edges: vec![], output: GraphNodeId(7), original_pipeline: None };
        let prepared = PreparedBindings::prepare(&graph, &[], "exact-out", Limits::default(), &Interrupt::NONE).unwrap();
        assert!(prepared.port_allowed(GraphNodeId(7), PortDirection::Output, "exact-out"));
        assert!(!prepared.port_allowed(GraphNodeId(7), PortDirection::Output, "alias"));
        assert!(PreparedBindings::prepare(&graph, &[], "exact-out", Limits { index_bytes: 0, ..Default::default() }, &Interrupt::NONE).is_err());
    }

    #[test]
    fn schema_pass_precedes_kernel_binding_and_preserves_absent_defaults() {
        use super::super::typed_schema::{self as typed, NumberRule, Presence, Property, ValueRule};
        use photocraft_doc::xcf_filters::{ExactNumber, FilterArgument, FilterValue};
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let domain = ExecutionDomain { format, encoding: SampleEncoding::Linear, profile_fingerprint: Some([3; 32]) };
        let context = WorkingContext { encoding: domain.encoding, profile_identity: [3; 32] };
        let channels = vec![gamma::ChannelExponent { channel: 0, exponent: 1. }];
        let kernel = PreparedGamma::prepare(
            format,
            context,
            &channels,
            gamma::Policy { negative: gamma::NegativePolicy::SignedMagnitude, domain: gamma::Domain::FiniteHdr, overflow: gamma::OverflowPolicy::Reject },
            gamma::Limits { resident_bytes: 128 << 20, work_units: 1 << 30 },
            &Interrupt::NONE,
        )
        .unwrap();
        let calls = AtomicUsize::new(0);
        let validate = |node: &GraphNode| {
            calls.fetch_add(1, Ordering::Relaxed);
            if node.arguments.len() == 2 { Ok(()) } else { Err(refusal("own binder requires exactly two explicit arguments")) }
        };
        for variant in 0..4 {
            let mut graph = graph();
            let argument = |name: &str, value| FilterArgument { name: name.into(), value };
            for node in &mut graph.nodes {
                node.arguments = vec![
                    argument("code", FilterValue::Number(ExactNumber::Unsigned { width: if variant == 1 { 16 } else { 32 }, value: 7 })),
                    argument("flag", FilterValue::Bool(true)),
                ];
                if variant == 2 {
                    node.arguments.swap(0, 1);
                }
                if variant == 3 {
                    node.version_origin = photocraft_doc::xcf_filters::GraphVersionOrigin::SchemaResolved;
                }
            }
            let before = graph.clone();
            let identity = graph.nodes[0].operation.clone();
            let default = FilterValue::Number(ExactNumber::Float64Bits((-0f64).to_bits()));
            let properties = [
                Property {
                    name: "code",
                    rule: ValueRule::Number(NumberRule::Unsigned { width: 32, minimum: 0, maximum: u64::from(u32::MAX) }),
                    presence: Presence::Required,
                },
                Property { name: "flag", rule: ValueRule::Bool, presence: Presence::Required },
                Property {
                    name: "unused-default",
                    rule: ValueRule::Number(NumberRule::Float64(typed::Float64Policy::RawBits)),
                    presence: Presence::OptionalDefault(&default),
                },
            ];
            let schema = typed::PreparedSchema::prepare(
                typed::Schema { identity: &identity, version_origin: before.nodes[0].version_origin, properties: &properties },
                typed::Limits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            // The origin-negative case deliberately binds the original stored
            // origin rather than granting a schema-resolved alias.
            let stored_schema = typed::PreparedSchema::prepare(
                typed::Schema { identity: &identity, version_origin: Default::default(), properties: &properties },
                typed::Limits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            let selected = if variant == 3 { &stored_schema } else { &schema };
            let schemas = [selected];
            let index = typed::PreparedSchemaIndex::prepare(&schemas, typed::Limits::default(), &Interrupt::NONE).unwrap();
            let bindings: Vec<_> = graph
                .nodes
                .iter()
                .map(|node| Binding {
                    node: node.id,
                    identity: &node.operation,
                    version_origin: node.version_origin,
                    domain,
                    input_port: "in",
                    upstream_output_port: "out",
                    output_port: "out",
                    kernel: Kernel::Gamma(&kernel),
                    validate_arguments: &validate,
                })
                .collect();
            calls.store(0, Ordering::Relaxed);
            let result = PreparedBindings::prepare_with_schemas(&graph, &bindings, "out", &index, Limits::default(), &Interrupt::NONE);
            if variant != 0 {
                assert!(result.is_err());
                assert_eq!(calls.load(Ordering::Relaxed), 0);
            } else {
                let prepared = result.unwrap();
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                let ordinary = PreparedBindings::prepare(&graph, &bindings, "out", Limits::default(), &Interrupt::NONE).unwrap();
                let held = index.requirements();
                assert_eq!(
                    prepared.requirements().retained_resident_bytes,
                    ordinary.requirements().retained_resident_bytes + held.owned_index_bytes + held.known_borrowed_bytes
                );
                assert_eq!(
                    prepared.requirements().retained_index_bytes,
                    ordinary.requirements().retained_index_bytes + held.owned_index_bytes + held.held_schema_index_bytes
                );
                assert!(prepared.requirements().preparation_work_units > ordinary.requirements().preparation_work_units);
                let reduced = prepared.execution_limits(ExecutionLimits::default()).unwrap();
                assert_eq!(reduced.resident_bytes, ExecutionLimits::default().resident_bytes - prepared.requirements().retained_resident_bytes);
                let raw = (-0f64).to_bits().to_ne_bytes();
                let source = Surface::try_with_default_bytes(format, &raw).unwrap();
                let output = prepared.execute(Some(&source), Rect::new(0, 0, 1, 1), domain, 32, ExecutionLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(output.surface().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), raw);
                // Optional defaults do not enter the AST or select Gamma math.
                assert_eq!(graph, before);
                assert_eq!(default, FilterValue::Number(ExactNumber::Float64Bits((-0f64).to_bits())));
                calls.store(0, Ordering::Relaxed);
                assert!(
                    PreparedBindings::prepare_with_schemas(
                        &graph,
                        &bindings,
                        "out",
                        &index,
                        Limits { work_units: 128, ..Default::default() },
                        &Interrupt::NONE
                    )
                    .is_err()
                );
                assert!(
                    PreparedBindings::prepare_with_schemas(
                        &graph,
                        &bindings,
                        "out",
                        &index,
                        Limits { index_bytes: held.owned_index_bytes, ..Default::default() },
                        &Interrupt::NONE
                    )
                    .is_err()
                );
                assert_eq!(calls.load(Ordering::Relaxed), 0);
            }
            assert_eq!(graph, before);
        }
    }
}
