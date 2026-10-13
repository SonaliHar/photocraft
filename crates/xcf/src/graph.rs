//! Inert subset of the official GEGL chain grammar, no operation/runtime lookup.
//! https://www.gegl.org/gegl-chain.html
use crate::{reader::Reader, *};
struct Parser<'a, 'b, 'o, 's> {
    r: &'a mut Reader<'b, 'o>,
    source: &'s str,
    pos: usize,
    nodes: u64,
    edges: u64,
    entries: u64,
}
impl Parser<'_, '_, '_, '_> {
    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }
    fn ws(&mut self) -> Result<(), XcfError> {
        while self.byte().is_some_and(|b| b.is_ascii_whitespace()) {
            if self.pos.is_multiple_of(4096) {
                self.r.poll()?;
            }
            self.pos += 1;
        }
        Ok(())
    }
    fn copy(&mut self, start: usize, end: usize) -> Result<String, XcfError> {
        let s = self.source.get(start..end).ok_or_else(|| self.r.error(ErrorKind::Structure, "graph UTF8 span"))?;
        if s.len() > self.r.limits.string_bytes {
            return Err(self.r.error(ErrorKind::Limit, "graph token bytes"));
        }
        self.r.charge(s.len())?;
        let mut out = String::new();
        out.try_reserve_exact(s.len()).map_err(|_| self.r.error(ErrorKind::Allocation, "graph token"))?;
        out.push_str(s);
        Ok(out)
    }
    fn limit(&mut self, kind: BudgetKind, value: u64, fallback: u64) -> Result<(), XcfError> {
        self.r.poll()?;
        if let Some(b) = self.r.budget {
            b.check(kind, value, "stored graph")?;
        } else if value > fallback {
            return Err(self.r.error(ErrorKind::Limit, "graph structure limit"));
        }
        Ok(())
    }
    fn edge(&mut self) -> Result<(), XcfError> {
        self.edges = self.edges.checked_add(1).ok_or_else(|| self.r.error(ErrorKind::Limit, "graph edge overflow"))?;
        self.limit(BudgetKind::GraphEdges, self.edges, 20000)
    }
    fn chain(&mut self, depth: u64, nested: bool) -> Result<Vec<GraphEntry>, XcfError> {
        self.limit(BudgetKind::ConfigDepth, depth, 64)?;
        let mut out = Vec::new();
        let mut preceding = false;
        loop {
            self.ws()?;
            if self.byte() == Some(b']') {
                if !nested {
                    return Err(self.r.error(ErrorKind::Structure, "unexpected graph close"));
                }
                self.pos += 1;
                break;
            }
            if self.byte().is_none() {
                if nested {
                    return Err(self.r.error(ErrorKind::Truncated, "unclosed graph chain"));
                }
                break;
            }
            self.r.poll()?;
            let start = self.pos;
            while self.byte().is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.')) {
                if self.pos.is_multiple_of(4096) {
                    self.r.poll()?;
                }
                self.pos += 1;
            }
            if self.pos == start {
                return Err(self.r.error(ErrorKind::Unsupported, "unqualified graph token syntax"));
            }
            let identity = self.copy(start, self.pos)?;
            let entry;
            if self.byte() == Some(b'=') {
                self.pos += 1;
                self.ws()?;
                if identity == "ref" {
                    self.edge()?;
                }
                let value = if self.byte() == Some(b'[') {
                    self.pos += 1;
                    self.edge()?;
                    GraphValue::Chain(self.chain(depth + 1, true)?)
                } else {
                    let at = self.pos;
                    let quote = self.byte().filter(|b| matches!(b, b'\'' | b'"'));
                    if let Some(quote) = quote {
                        self.pos += 1;
                        let mut escaped = false;
                        loop {
                            if self.pos.is_multiple_of(4096) {
                                self.r.poll()?;
                            }
                            let b = self.byte().ok_or_else(|| self.r.error(ErrorKind::Truncated, "graph quoted value"))?;
                            self.pos += 1;
                            if b == 0 {
                                return Err(self.r.error(ErrorKind::Structure, "graph NUL"));
                            }
                            if escaped {
                                escaped = false;
                            } else if b == b'\\' {
                                escaped = true;
                            } else if b == quote {
                                break;
                            }
                        }
                        if self.byte().is_some_and(|b| !b.is_ascii_whitespace() && b != b']') {
                            return Err(self.r.error(ErrorKind::Unsupported, "unqualified graph quoted delimiter"));
                        }
                    } else {
                        while self.byte().is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'[' | b']' | b'=')) {
                            if self.pos.is_multiple_of(4096) {
                                self.r.poll()?;
                            }
                            if self.byte() == Some(0) {
                                return Err(self.r.error(ErrorKind::Structure, "graph NUL"));
                            }
                            self.pos += 1;
                        }
                    }
                    if self.pos == at {
                        return Err(self.r.error(ErrorKind::Structure, "missing graph property value"));
                    }
                    let source = self.copy(at, self.pos)?;
                    let numeric_bits = if quote.is_none() {
                        match source.parse::<f64>() {
                            Ok(n) if n.is_finite() => Some(n.to_bits()),
                            Ok(_) => return Err(self.r.error(ErrorKind::Unsupported, "nonfinite graph value")),
                            Err(_) => None,
                        }
                    } else {
                        None
                    };
                    GraphValue::Literal { source, numeric_bits }
                };
                entry = GraphEntry::Property { name: identity, value, start: start as u64, end: self.pos as u64 };
            } else {
                self.nodes = self.nodes.checked_add(1).ok_or_else(|| self.r.error(ErrorKind::Limit, "graph node overflow"))?;
                self.limit(BudgetKind::GraphNodes, self.nodes, 10000)?;
                if preceding {
                    self.edge()?;
                }
                preceding = true;
                entry = GraphEntry::Operation { identity, start: start as u64, end: self.pos as u64 };
            }
            self.entries = self.entries.checked_add(1).ok_or_else(|| self.r.error(ErrorKind::Limit, "graph entry overflow"))?;
            self.limit(BudgetKind::ConfigEntries, self.entries, 100000)?;
            self.r.charge(std::mem::size_of::<GraphEntry>())?;
            out.try_reserve_exact(1).map_err(|_| self.r.error(ErrorKind::Allocation, "graph entries"))?;
            out.push(entry);
        }
        Ok(out)
    }
}
pub(crate) fn parse(r: &mut Reader<'_, '_>, source: &str) -> Result<StoredGraph, XcfError> {
    let nodes_before = r.graph_nodes;
    let edges_before = r.graph_edges;
    let entries = r.config_entries;
    let mut p = Parser { r, source, pos: 0, nodes: nodes_before, edges: edges_before, entries };
    let entries = p.chain(0, false)?;
    let actual_edges = validate_topology(p.r, &entries)?;
    p.edges = edges_before.checked_add(actual_edges).ok_or_else(|| p.r.error(ErrorKind::Limit, "graph topology edge overflow"))?;
    p.r.graph_nodes = p.nodes;
    p.r.graph_edges = p.edges;
    p.r.config_entries = p.entries;
    Ok(StoredGraph { entries, nodes: p.nodes - nodes_before, edges: p.edges - edges_before })
}

#[derive(Clone, Copy)]
enum Endpoint<'a> {
    Node(usize),
    Reference(&'a str, u64),
}
struct Topology<'a, 'b, 'o, 's> {
    r: &'a mut Reader<'b, 'o>,
    ids: std::collections::HashMap<&'s str, usize>,
    edges: Vec<(Endpoint<'s>, usize)>,
    nodes: usize,
}
impl<'s> Topology<'_, '_, '_, 's> {
    fn edge(&mut self, input: Endpoint<'s>, output: usize) -> Result<(), XcfError> {
        self.r.poll()?;
        let count = self
            .r
            .graph_edges
            .checked_add(self.edges.len() as u64)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| self.r.error(ErrorKind::Limit, "graph topology edge overflow"))?;
        if let Some(budget) = self.r.budget {
            budget.check(BudgetKind::GraphEdges, count, "stored graph topology")?;
        } else if count > 20000 {
            return Err(self.r.error(ErrorKind::Limit, "graph topology edge limit"));
        }
        self.r.charge(std::mem::size_of::<(Endpoint<'s>, usize)>())?;
        self.edges.try_reserve_exact(1).map_err(|_| self.r.error(ErrorKind::Allocation, "graph topology edges"))?;
        self.edges.push((input, output));
        Ok(())
    }
    fn chain(&mut self, entries: &'s [GraphEntry], nested: bool) -> Result<Endpoint<'s>, XcfError> {
        let mut current = Endpoint::Node(0);
        let mut operation = false;
        let mut referenced = false;
        let mut aux = false;
        for entry in entries {
            self.r.poll()?;
            match entry {
                GraphEntry::Operation { .. } => {
                    self.nodes = self.nodes.checked_add(1).ok_or_else(|| self.r.error(ErrorKind::Limit, "graph topology nodes"))?;
                    self.edge(current, self.nodes)?;
                    current = Endpoint::Node(self.nodes);
                    operation = true;
                    aux = false;
                }
                GraphEntry::Property { name, value, start, .. } if name == "id" || name == "ref" => {
                    let GraphValue::Literal { source, .. } = value else {
                        return Err(self.r.error(ErrorKind::Structure, format!("graph {name} requires an identifier at span {start}")));
                    };
                    if source.is_empty() || !source.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.')) {
                        return Err(self.r.error(ErrorKind::Unsupported, format!("unqualified graph {name} identifier at span {start}")));
                    }
                    if name == "ref" {
                        if !nested || operation || referenced {
                            return Err(self.r.error(ErrorKind::Structure, format!("graph ref must occur once before nested operations at span {start}")));
                        }
                        current = Endpoint::Reference(source, *start);
                        referenced = true;
                    } else {
                        let Endpoint::Node(node) = current else {
                            return Err(self.r.error(ErrorKind::Unsupported, format!("graph reference alias id at span {start}")));
                        };
                        if self.ids.contains_key(source.as_str()) {
                            return Err(self.r.error(ErrorKind::Structure, format!("duplicate graph id {source} at span {start}")));
                        }
                        self.r.charge(std::mem::size_of::<(&str, usize)>().saturating_mul(4))?;
                        self.ids.try_reserve(1).map_err(|_| self.r.error(ErrorKind::Allocation, "graph identifiers"))?;
                        self.ids.insert(source, node);
                    }
                }
                GraphEntry::Property { name, value: GraphValue::Chain(chain), start, .. } => {
                    if name != "aux" {
                        return Err(self.r.error(ErrorKind::Unsupported, format!("unqualified graph port {name} at span {start}")));
                    }
                    if !operation || aux {
                        return Err(self.r.error(ErrorKind::Structure, format!("missing owner or duplicate graph aux port at span {start}")));
                    }
                    let Endpoint::Node(owner) = current else { return Err(self.r.error(ErrorKind::Structure, "graph port owner")) };
                    let input = self.chain(chain, true)?;
                    self.edge(input, owner)?;
                    aux = true;
                }
                GraphEntry::Property { start, .. } if !operation => {
                    return Err(self.r.error(ErrorKind::Structure, format!("graph parameter without operation at span {start}")));
                }
                _ => {}
            }
        }
        if !operation && !referenced && nested {
            return Err(self.r.error(ErrorKind::Structure, "empty graph aux chain"));
        }
        Ok(current)
    }
}
fn validate_topology(r: &mut Reader<'_, '_>, entries: &[GraphEntry]) -> Result<u64, XcfError> {
    let mut t = Topology { r, ids: std::collections::HashMap::new(), edges: Vec::new(), nodes: 0 };
    t.chain(entries, false)?;
    let count = t.nodes.checked_add(1).ok_or_else(|| t.r.error(ErrorKind::Limit, "graph topology size"))?;
    let mut resolved = Vec::new();
    t.r.charge(t.edges.len().checked_mul(std::mem::size_of::<(usize, usize)>()).ok_or_else(|| t.r.error(ErrorKind::Limit, "graph topology bytes"))?)?;
    resolved.try_reserve_exact(t.edges.len()).map_err(|_| t.r.error(ErrorKind::Allocation, "resolved graph edges"))?;
    for (input, output) in &t.edges {
        t.r.poll()?;
        let source = match input {
            Endpoint::Node(node) => *node,
            Endpoint::Reference(id, span) => {
                *t.ids.get(id).ok_or_else(|| t.r.error(ErrorKind::Structure, format!("dangling graph ref {id} at span {span}")))?
            }
        };
        resolved.push((source, *output));
    }
    resolved.sort_unstable();
    let mut degrees = Vec::new();
    let mut queue = Vec::new();
    t.r.charge(count.checked_mul(std::mem::size_of::<usize>() * 2).ok_or_else(|| t.r.error(ErrorKind::Limit, "graph topology bytes"))?)?;
    degrees.try_reserve_exact(count).map_err(|_| t.r.error(ErrorKind::Allocation, "graph indegrees"))?;
    queue.try_reserve_exact(count).map_err(|_| t.r.error(ErrorKind::Allocation, "graph traversal"))?;
    degrees.resize(count, 0usize);
    for (_, output) in &resolved {
        let degree = degrees.get_mut(*output).ok_or_else(|| t.r.error(ErrorKind::Structure, "graph output index"))?;
        *degree = degree.checked_add(1).ok_or_else(|| t.r.error(ErrorKind::Limit, "graph indegree overflow"))?;
    }
    queue.extend(degrees.iter().enumerate().filter_map(|(i, degree)| (*degree == 0).then_some(i)));
    let mut cursor = 0;
    while let Some(node) = queue.get(cursor).copied() {
        t.r.poll()?;
        cursor += 1;
        let start = resolved.partition_point(|(source, _)| *source < node);
        for (source, output) in resolved.get(start..).ok_or_else(|| t.r.error(ErrorKind::Structure, "graph traversal span"))? {
            if *source != node {
                break;
            }
            t.r.poll()?;
            let degree = degrees.get_mut(*output).ok_or_else(|| t.r.error(ErrorKind::Structure, "graph output index"))?;
            *degree = degree.checked_sub(1).ok_or_else(|| t.r.error(ErrorKind::Structure, "graph indegree underflow"))?;
            if *degree == 0 {
                queue.push(*output);
            }
        }
    }
    if cursor != count {
        return Err(t.r.error(ErrorKind::Structure, "cycle in stored graph id/ref topology"));
    }
    Ok(resolved.len() as u64)
}
