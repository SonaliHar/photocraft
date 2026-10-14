//! Inert mapping of the documented GEGL chain grammar. This never resolves a URI,
//! invokes GEGL, chooses an evaluator, or supplies a missing operation schema.
use crate::IoError;
use photocraft_doc::{
    item_identity::GraphNodeId,
    xcf_filters::{FilterArgument, FilterGraph, FilterValue, GraphEdge, GraphNode, GraphVersionOrigin, OperationIdentity},
};
use photocraft_raster::Interrupt;
use photocraft_xcf::{GraphEntry, GraphValue, StoredGraph};
#[derive(Clone, Copy, Debug)]
pub struct GraphMappingLimits {
    pub nodes: usize,
    pub edges: usize,
    pub depth: usize,
    pub entries: usize,
    pub resident_bytes: usize,
}
impl Default for GraphMappingLimits {
    fn default() -> Self {
        Self { nodes: 4096, edges: 16384, depth: 24, entries: 100000, resident_bytes: 32 << 20 }
    }
}
/// Exact literal spelling; quoted strings and relative units require the operation schema.
#[derive(Clone, Copy, Debug)]
pub struct GraphLiteral<'a> {
    pub name: &'a str,
    pub source: &'a str,
    pub numeric_bits: Option<u64>,
}
pub type GraphSchemaResolver<'a> = dyn for<'source> FnMut(&str, &[GraphLiteral<'source>]) -> Result<ResolvedOperation, IoError> + 'a;
pub struct ResolvedOperation {
    pub operation: OperationIdentity,
    pub arguments: Vec<FilterArgument>,
}
#[derive(Clone, Copy)]
enum Endpoint<'a> {
    Node(GraphNodeId),
    Reference(&'a str),
}
struct PendingNode<'a> {
    id: GraphNodeId,
    identity: &'a str,
    properties: Vec<GraphLiteral<'a>>,
}
struct PendingEdge<'a> {
    from: Endpoint<'a>,
    to: GraphNodeId,
    port: &'static str,
}
struct Builder<'a, 'c, 'i> {
    source: &'a str,
    ctl: &'c Interrupt<'i>,
    limits: GraphMappingLimits,
    charge: &'c mut dyn FnMut(usize) -> Result<(), IoError>,
    fresh: &'c mut dyn FnMut() -> GraphNodeId,
    input: GraphNodeId,
    nodes: Vec<PendingNode<'a>>,
    edges: Vec<PendingEdge<'a>>,
    aliases: Vec<(&'a str, GraphNodeId)>,
    entries: usize,
    resident: usize,
}
fn unsupported(message: &str) -> IoError {
    IoError::Unsupported(format!("XCF stored graph: {message}"))
}
fn validate_number(number: &photocraft_doc::xcf_filters::ExactNumber) -> Result<(), IoError> {
    use photocraft_doc::xcf_filters::ExactNumber;
    match number {
        ExactNumber::Signed { width, value } => {
            if ![8, 16, 32, 64].contains(width) {
                return Err(unsupported("integer width"));
            }
            if *width < 64 {
                let bound = 1i64.checked_shl(u32::from(*width) - 1).ok_or_else(|| unsupported("integer width"))?;
                if *value < -bound || *value >= bound {
                    return Err(unsupported("signed integer range"));
                }
            }
        }
        ExactNumber::Unsigned { width, value } => {
            if ![8, 16, 32, 64].contains(width) || (*width < 64 && *value >= 1u64.checked_shl(u32::from(*width)).ok_or_else(|| unsupported("integer width"))?) {
                return Err(unsupported("unsigned integer range"));
            }
        }
        ExactNumber::Float32Bits(_) | ExactNumber::Float64Bits(_) => {}
    }
    Ok(())
}
impl<'a> Builder<'a, '_, '_> {
    fn poll(&self) -> Result<(), IoError> {
        self.ctl.check().map_err(|_| IoError::Cancelled)
    }
    fn whitespace(&self, text: &str) -> Result<bool, IoError> {
        for chunk in text.as_bytes().chunks(4096) {
            self.poll()?;
            if !chunk.iter().all(u8::is_ascii_whitespace) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn token(&self, text: &str) -> Result<bool, IoError> {
        if text.is_empty() || text.len() > 1 << 20 {
            return Ok(false);
        }
        for chunk in text.as_bytes().chunks(4096) {
            self.poll()?;
            if !chunk.iter().all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'-' | b':' | b'.')) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn account(&mut self, bytes: usize) -> Result<(), IoError> {
        self.poll()?;
        let total = self.resident.checked_add(bytes).ok_or_else(|| unsupported("resident overflow"))?;
        if total > self.limits.resident_bytes {
            return Err(unsupported("resident limit"));
        }
        (self.charge)(bytes)?;
        self.resident = total;
        Ok(())
    }
    fn reserve<T>(&mut self, out: &mut Vec<T>, count: usize) -> Result<(), IoError> {
        self.account(count.checked_mul(std::mem::size_of::<T>()).ok_or_else(|| unsupported("allocation extent"))?)?;
        out.try_reserve_exact(count).map_err(|_| unsupported("allocation failed"))
    }
    fn tick(&mut self) -> Result<(), IoError> {
        self.poll()?;
        self.entries = self.entries.checked_add(1).ok_or_else(|| unsupported("entry overflow"))?;
        if self.entries > self.limits.entries {
            return Err(unsupported("entry limit"));
        }
        Ok(())
    }
    fn span(&self, start: u64, end: u64) -> Result<&str, IoError> {
        let start = usize::try_from(start).map_err(|_| unsupported("source span address width"))?;
        let end = usize::try_from(end).map_err(|_| unsupported("source span address width"))?;
        self.source.get(start..end).ok_or_else(|| unsupported("source UTF8 span"))
    }
    fn fresh(&mut self) -> Result<GraphNodeId, IoError> {
        self.poll()?;
        let id = (self.fresh)();
        if id.0 == 0 || id == self.input || self.nodes.iter().any(|n| n.id == id) {
            return Err(unsupported("fresh node identity is zero or duplicated"));
        }
        Ok(id)
    }
    fn edge(&mut self, from: Endpoint<'a>, to: GraphNodeId, port: &'static str) -> Result<(), IoError> {
        self.tick()?;
        if self.edges.len() >= self.limits.edges {
            return Err(unsupported("edge limit"));
        }
        if self.edges.iter().any(|e| e.to == to && e.port == port) {
            return Err(unsupported("duplicate input port"));
        }
        self.account(std::mem::size_of::<PendingEdge>())?;
        self.edges.try_reserve_exact(1).map_err(|_| unsupported("edge allocation"))?;
        self.edges.push(PendingEdge { from, to, port });
        Ok(())
    }
    fn chain(&mut self, entries: &'a [GraphEntry], depth: usize, nested: bool, begin: u64, finish: u64) -> Result<Endpoint<'a>, IoError> {
        self.poll()?;
        if depth > self.limits.depth {
            return Err(unsupported("chain depth"));
        }
        let mut current = Endpoint::Node(self.input);
        let mut owner = None;
        let mut reference = false;
        let mut previous_end = begin;
        for entry in entries {
            self.tick()?;
            let (start, end) = match entry {
                GraphEntry::Operation { start, end, .. } | GraphEntry::Property { start, end, .. } => (*start, *end),
            };
            if end > finish || start < previous_end {
                return Err(unsupported("overlapping or reordered source spans"));
            }
            if !self.whitespace(self.span(previous_end, start)?)? {
                return Err(unsupported("omitted required source token"));
            }
            previous_end = end;
            let span = self.span(start, end)?;
            match entry {
                GraphEntry::Operation { identity, .. } => {
                    if !self.token(identity)? || span != identity {
                        return Err(unsupported("operation source span mismatch"));
                    }
                    if self.nodes.len() >= self.limits.nodes {
                        return Err(unsupported("node limit"));
                    }
                    let id = self.fresh()?;
                    self.edge(current, id, "input")?;
                    self.account(std::mem::size_of::<PendingNode>())?;
                    self.nodes.try_reserve_exact(1).map_err(|_| unsupported("node allocation"))?;
                    self.nodes.push(PendingNode { id, identity, properties: Vec::new() });
                    owner = Some(self.nodes.len() - 1);
                    current = Endpoint::Node(id);
                }
                GraphEntry::Property { name, value, .. } => {
                    if !self.token(name)? || !span.starts_with(name) || span.as_bytes().get(name.len()) != Some(&b'=') {
                        return Err(unsupported("property source span mismatch"));
                    }
                    match value {
                        GraphValue::Literal { source, numeric_bits } => {
                            if span.get(name.len() + 1..).map(str::trim_start) != Some(source.as_str()) || source.is_empty() || source.contains('\0') {
                                return Err(unsupported("literal source mismatch"));
                            }
                            let expected_bits = if source.starts_with(['\"', '\'']) {
                                if source.as_bytes().first() != source.as_bytes().last() || source.len() < 2 {
                                    return Err(unsupported("quoted literal delimiter"));
                                }
                                None
                            } else {
                                match source.parse::<f64>() {
                                    Ok(n) if n.is_finite() => Some(n.to_bits()),
                                    Ok(_) => return Err(unsupported("nonfinite literal")),
                                    Err(_) => None,
                                }
                            };
                            if expected_bits != *numeric_bits {
                                return Err(unsupported("numeric source bits mismatch"));
                            }
                            if name == "id" || name == "ref" {
                                if !self.token(source)? {
                                    return Err(unsupported("graph identifier form"));
                                }
                                if name == "ref" {
                                    if !nested || owner.is_some() || reference {
                                        return Err(unsupported("reference must precede nested operations"));
                                    }
                                    current = Endpoint::Reference(source);
                                    reference = true;
                                } else {
                                    let Endpoint::Node(id) = current else {
                                        return Err(unsupported("reference alias identity"));
                                    };
                                    if self.aliases.iter().any(|(alias, _)| *alias == source) {
                                        return Err(unsupported("duplicate source identity"));
                                    }
                                    self.account(std::mem::size_of::<(&str, GraphNodeId)>())?;
                                    self.aliases.try_reserve_exact(1).map_err(|_| unsupported("alias allocation"))?;
                                    self.aliases.push((source, id));
                                }
                            } else {
                                let index = owner.ok_or_else(|| unsupported("property has no operation owner"))?;
                                if self.nodes.get(index).is_some_and(|n| n.properties.iter().any(|p| p.name == name)) {
                                    return Err(unsupported("duplicate operation property requires explicit last-value semantics"));
                                }
                                self.account(std::mem::size_of::<GraphLiteral>())?;
                                let node = self.nodes.get_mut(index).ok_or_else(|| unsupported("operation owner index"))?;
                                node.properties.try_reserve_exact(1).map_err(|_| unsupported("parameter allocation"))?;
                                node.properties.push(GraphLiteral { name, source, numeric_bits: *numeric_bits });
                            }
                        }
                        GraphValue::Chain(chain) => {
                            if name != "aux" {
                                return Err(unsupported("unknown required input port"));
                            }
                            let index = owner.ok_or_else(|| unsupported("auxiliary chain has no owner"))?;
                            let id = self.nodes.get(index).ok_or_else(|| unsupported("auxiliary owner index"))?.id;
                            if !span.trim_end().ends_with(']') || !span.get(name.len() + 1..).is_some_and(|s| s.trim_start().starts_with('[')) {
                                return Err(unsupported("auxiliary source delimiters"));
                            }
                            for child in chain {
                                let (child_start, child_end) = match child {
                                    GraphEntry::Operation { start, end, .. } | GraphEntry::Property { start, end, .. } => (*start, *end),
                                };
                                if child_start <= start || child_end >= end {
                                    return Err(unsupported("auxiliary child outside source span"));
                                }
                            }
                            let open = span.find('[').ok_or_else(|| unsupported("auxiliary opening delimiter"))?;
                            let close = span.rfind(']').ok_or_else(|| unsupported("auxiliary closing delimiter"))?;
                            let inner_begin =
                                start.checked_add(open as u64).and_then(|n| n.checked_add(1)).ok_or_else(|| unsupported("auxiliary source extent"))?;
                            let inner_end = start.checked_add(close as u64).ok_or_else(|| unsupported("auxiliary source extent"))?;
                            let output =
                                self.chain(chain, depth.checked_add(1).ok_or_else(|| unsupported("chain depth overflow"))?, true, inner_begin, inner_end)?;
                            self.edge(output, id, "aux")?;
                        }
                    }
                }
            }
        }
        if !self.whitespace(self.span(previous_end, finish)?)? {
            return Err(unsupported("omitted required source token"));
        }
        if nested && owner.is_none() && !reference {
            return Err(unsupported("empty auxiliary chain"));
        }
        Ok(current)
    }
    fn endpoint(&self, endpoint: Endpoint<'a>) -> Result<GraphNodeId, IoError> {
        self.poll()?;
        match endpoint {
            Endpoint::Node(id) => Ok(id),
            Endpoint::Reference(alias) => {
                self.aliases.iter().find(|(a, _)| *a == alias).map(|(_, id)| *id).ok_or_else(|| unsupported("dangling source reference"))
            }
        }
    }
    fn check_string(&mut self, s: &str) -> Result<(), IoError> {
        if s.len() > 1 << 20 || s.contains('\0') {
            return Err(unsupported("string extent or NUL"));
        }
        self.account(s.len())?;
        for _ in s.as_bytes().chunks(4096) {
            self.poll()?;
        }
        Ok(())
    }
    fn copy(&mut self, s: &str) -> Result<String, IoError> {
        self.check_string(s)?;
        let mut result = String::new();
        result.try_reserve_exact(s.len()).map_err(|_| unsupported("string allocation"))?;
        result.push_str(s);
        Ok(result)
    }
    fn arguments(&mut self, args: &[FilterArgument], depth: usize) -> Result<(), IoError> {
        if depth > self.limits.depth {
            return Err(unsupported("typed parameter depth"));
        }
        self.account(args.len().checked_mul(std::mem::size_of::<FilterArgument>()).ok_or_else(|| unsupported("typed parameter extent"))?)?;
        for (index, arg) in args.iter().enumerate() {
            self.tick()?;
            self.account(arg.name.len())?;
            if !self.token(&arg.name)? || args.get(..index).is_some_and(|before| before.iter().any(|a| a.name == arg.name)) {
                return Err(unsupported("duplicate or invalid typed parameter"));
            }
            match &arg.value {
                FilterValue::Number(number) => validate_number(number)?,
                FilterValue::Config(args) => self.arguments(args, depth + 1)?,
                FilterValue::Graph(_) => return Err(unsupported("resolver supplied unqualified nested graph")),
                FilterValue::StoredColor(_) => return Err(unsupported("resolver stored color requires an exact literal binding")),
                FilterValue::Resource(id) if id.0 == 0 => return Err(unsupported("zero resource reference")),
                FilterValue::String(s) => {
                    self.check_string(s)?;
                }
                FilterValue::Enum { domain, .. } => {
                    if domain.is_empty() {
                        return Err(unsupported("missing enumeration domain"));
                    }
                    self.check_string(domain)?;
                }
                FilterValue::Color { components, profile, .. } => {
                    if components.is_empty() || components.len() > 16 {
                        return Err(unsupported("color component count"));
                    }
                    for component in components {
                        self.poll()?;
                        validate_number(component)?;
                    }
                    if profile.is_some_and(|id| id.0 == 0) {
                        return Err(unsupported("zero profile reference"));
                    }
                    self.account(
                        components
                            .len()
                            .checked_mul(std::mem::size_of::<photocraft_doc::xcf_filters::ExactNumber>())
                            .ok_or_else(|| unsupported("color extent"))?,
                    )?;
                }
                FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => {
                    if points_f64_bits.len() > 100000
                        || samples_f64_bits.len() > 100000
                        || (*free && (!points_f64_bits.is_empty() || !point_types.is_empty()))
                        || (!*free && (points_f64_bits.len() != point_types.len() || !samples_f64_bits.is_empty()))
                    {
                        return Err(unsupported("curve shape"));
                    }
                    for bits in points_f64_bits.iter().flatten().chain(samples_f64_bits) {
                        self.poll()?;
                        if !f64::from_bits(*bits).is_finite() {
                            return Err(unsupported("curve nonfinite"));
                        }
                    }

                    self.account(
                        points_f64_bits
                            .len()
                            .checked_mul(16)
                            .and_then(|n| point_types.len().checked_mul(4).and_then(|p| n.checked_add(p)))
                            .and_then(|n| samples_f64_bits.len().checked_mul(8).and_then(|p| n.checked_add(p)))
                            .ok_or_else(|| unsupported("curve extent"))?,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
/// Resolver results must come from a qualified schema, include every ordinary property
/// exactly once, and preserve requested resource references inertly. The resolver itself
/// must bound its own allocations; this helper checks returned DTOs before publication.
pub fn map_graph<'a>(
    graph: &'a StoredGraph,
    original: &'a str,
    limits: GraphMappingLimits,
    ctl: &Interrupt<'_>,
    fresh: &mut dyn FnMut() -> GraphNodeId,
    resolve: &mut GraphSchemaResolver<'_>,
    charge: &mut dyn FnMut(usize) -> Result<(), IoError>,
) -> Result<FilterGraph, IoError> {
    if limits.nodes > 4096 || limits.edges > 16384 || limits.depth > 24 || limits.entries > 100000 || limits.resident_bytes > 32 << 20 {
        return Err(unsupported("mapping limits may only be lowered"));
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let input = fresh();
    if input.0 == 0 {
        return Err(unsupported("zero input identity"));
    }
    let mut b =
        Builder { source: original, ctl, limits, charge, fresh, input, nodes: Vec::new(), edges: Vec::new(), aliases: Vec::new(), entries: 0, resident: 0 };
    if original.len() > 1 << 20 {
        return Err(unsupported("original pipeline string extent"));
    }
    let output = b.chain(&graph.entries, 0, false, 0, original.len() as u64)?;
    if graph.nodes != b.nodes.len() as u64 || graph.edges != b.edges.len() as u64 {
        return Err(unsupported("stored topology counts mismatch"));
    }
    let output = b.endpoint(output)?;
    let mut edges = Vec::new();
    b.reserve(&mut edges, b.edges.len())?;
    let mut topology = Vec::new();
    b.reserve(&mut topology, b.edges.len())?;
    for index in 0..b.edges.len() {
        b.poll()?;
        let edge = b.edges.get(index).ok_or_else(|| unsupported("edge index"))?;
        let from = b.endpoint(edge.from)?;
        let to = edge.to;
        let port = edge.port;
        let from_index = if from == input { 0 } else { b.nodes.iter().position(|n| n.id == from).ok_or_else(|| unsupported("edge source identity"))? + 1 };
        let to_index = b.nodes.iter().position(|n| n.id == to).ok_or_else(|| unsupported("edge destination identity"))? + 1;
        topology.push((from_index, to_index));
        edges.push(GraphEdge { from, to, output_port: b.copy("output")?, input_port: b.copy(port)? });
    }
    topology.sort_unstable();
    let count = b.nodes.len().checked_add(1).ok_or_else(|| unsupported("topology count"))?;
    let mut degrees = Vec::new();
    let mut queue = Vec::new();
    b.reserve(&mut degrees, count)?;
    b.reserve(&mut queue, count)?;
    degrees.resize(count, 0usize);
    for (_, to) in &topology {
        b.poll()?;
        let d = degrees.get_mut(*to).ok_or_else(|| unsupported("topology destination"))?;
        *d = d.checked_add(1).ok_or_else(|| unsupported("topology indegree"))?;
    }
    queue.extend(degrees.iter().enumerate().filter_map(|(i, n)| (*n == 0).then_some(i)));
    let mut cursor = 0;
    while let Some(node) = queue.get(cursor).copied() {
        b.poll()?;
        cursor += 1;
        let start = topology.partition_point(|(from, _)| *from < node);
        for (from, to) in topology.get(start..).ok_or_else(|| unsupported("topology span"))? {
            if *from != node {
                break;
            }
            b.poll()?;
            let d = degrees.get_mut(*to).ok_or_else(|| unsupported("topology destination"))?;
            *d = d.checked_sub(1).ok_or_else(|| unsupported("topology indegree underflow"))?;
            if *d == 0 {
                queue.push(*to);
            }
        }
    }
    if cursor != count {
        return Err(unsupported("cycle in graph"));
    }
    let mut nodes = Vec::new();
    b.reserve(&mut nodes, b.nodes.len())?;
    for index in 0..b.nodes.len() {
        b.poll()?;
        let node = b.nodes.get(index).ok_or_else(|| unsupported("node index"))?;
        let resolved = resolve(node.identity, &node.properties)?;
        let identity = &resolved.operation;
        if identity.name != node.identity && identity.name.strip_prefix("gegl:") != Some(node.identity) {
            return Err(unsupported("schema operation identity mismatch"));
        }
        if identity.stored_version.is_empty() || identity.schema_epoch.is_empty() {
            return Err(unsupported("missing operation version/schema epoch"));
        }
        if resolved.arguments.len() != node.properties.len() || node.properties.iter().any(|p| !resolved.arguments.iter().any(|a| a.name == p.name)) {
            return Err(unsupported("schema omitted or introduced required property"));
        }
        let id = node.id;
        b.check_string(&identity.name)?;
        b.check_string(&identity.stored_version)?;
        b.check_string(&identity.schema_epoch)?;
        b.arguments(&resolved.arguments, 0)?;
        nodes.push(GraphNode { id, operation: resolved.operation, arguments: resolved.arguments, version_origin: GraphVersionOrigin::SchemaResolved });
    }
    let original_pipeline = Some(b.copy(original)?);
    b.poll()?;
    Ok(FilterGraph { input: Some(input), nodes, edges, output, original_pipeline })
}
