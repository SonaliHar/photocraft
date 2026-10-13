//! Bounded, lossless typed source metadata. No locator, payload read, schema
//! resolution, evaluator selection, resource grant or renderer qualification.
use crate::{EngineError, Result};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef, MaskId, PathId},
    xcf_filters::{ExactNumber, FilterArgument, FilterValue, OperationIdentity},
    xcf_items::{EditableSource, TransformState},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
const MAX_REPLY: usize = 64 * 1024;
const MAX_NODES: usize = 1024;
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("xcf.source.inspect: {message}"))
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || text.starts_with('0') || !text.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
/// Exact small selector: {owner:{kind:layer|channel|mask|path,id:canonical string},
/// offset?:0..100000,limit?:1..64}. Root uses before any params clone/float handling.
pub(crate) fn selector(params: &Value) -> Result<(ItemRef, usize, usize)> {
    let object = params
        .as_object()
        .filter(|object| object.len() <= 3 && object.keys().all(|key| matches!(key.as_str(), "owner" | "offset" | "limit")))
        .ok_or_else(|| error("expected owner/page object"))?;
    let owner = object
        .get("owner")
        .and_then(Value::as_object)
        .filter(|owner| owner.len() == 2 && owner.keys().all(|key| matches!(key.as_str(), "kind" | "id")))
        .ok_or_else(|| error("owner requires exact kind/id fields"))?;
    let id = owner.get("id").and_then(decimal).ok_or_else(|| error("owner id must be a canonical nonzero U64 string"))?;
    let owner = match owner.get("kind").and_then(Value::as_str) {
        Some("layer") => ItemRef::Layer(photocraft_doc::LayerId(id)),
        Some("channel") => ItemRef::Channel(ChannelId(id)),
        Some("mask") => ItemRef::Mask(MaskId(id)),
        Some("path") => ItemRef::Path(PathId(id)),
        _ => return Err(error("unknown owner kind")),
    };
    let page = |key: &str, default: u64, max: u64| -> Result<usize> {
        let value = object.get(key).map(|value| value.as_u64().ok_or_else(|| error("pagination must be unsigned integers"))).transpose()?.unwrap_or(default);
        if value > max {
            return Err(error("pagination exceeds bound"));
        }
        usize::try_from(value).map_err(|_| error("pagination overflow"))
    };
    let offset = page("offset", 0, 100_000)?;
    let limit = page("limit", 64, 64)?;
    if limit == 0 {
        return Err(error("limit must be positive"));
    }
    Ok((owner, offset, limit))
}
struct Budget<'a, 'b> {
    control: &'a Interrupt<'b>,
    bytes: usize,
    nodes: usize,
}
impl Budget<'_, '_> {
    fn node(&mut self, depth: usize) -> Result<()> {
        self.control.check().map_err(|_| EngineError::Cancelled)?;
        if depth > 8 {
            return Err(error("nested typed value depth exceeds8"));
        }
        self.nodes = self.nodes.checked_add(1).filter(|nodes| *nodes <= MAX_NODES).ok_or_else(|| error("typed value node limit"))?;
        self.charge(128)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes).filter(|bytes| *bytes <= MAX_REPLY).ok_or_else(|| error("typed inspection reply exceeds64KiB"))?;
        Ok(())
    }
    fn text(&mut self, value: &str) -> Result<String> {
        self.control.check().map_err(|_| EngineError::Cancelled)?;
        if value.len() > 4096 {
            return Err(error("typed string exceeds4096bytes"));
        }
        self.charge(value.len().checked_mul(6).ok_or_else(|| error("escaped string size overflow"))?)?;
        let mut output = String::new();
        output.try_reserve_exact(value.len()).map_err(|_| error("typed string allocation denied"))?;
        output.push_str(value);
        Ok(output)
    }
}
fn number(value: &ExactNumber, budget: &mut Budget<'_, '_>, depth: usize) -> Result<Value> {
    budget.node(depth)?;
    Ok(match value {
        ExactNumber::Signed { width, value } => json!({"Signed":{"width":width,"value":value.to_string()}}),
        ExactNumber::Unsigned { width, value } => json!({"Unsigned":{"width":width,"value":value.to_string()}}),
        ExactNumber::Float32Bits(bits) => json!({"Float32Bits":format!("0x{bits:08x}")}),
        ExactNumber::Float64Bits(bits) => json!({"Float64Bits":format!("0x{bits:016x}")}),
    })
}
fn operation(value: &OperationIdentity, budget: &mut Budget<'_, '_>, depth: usize) -> Result<Value> {
    budget.node(depth)?;
    Ok(json!({"name":budget.text(&value.name)?,"stored_version":budget.text(&value.stored_version)?,"schema_epoch":budget.text(&value.schema_epoch)?}))
}
fn arguments(values: &[FilterArgument], budget: &mut Budget<'_, '_>, depth: usize) -> Result<Value> {
    if values.len() > 64 {
        return Err(error("nested configuration exceeds64members; inspect a smaller typed domain"));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(values.len()).map_err(|_| error("typed argument allocation denied"))?;
    for argument in values {
        budget.node(depth)?;
        output.push(json!({"name":budget.text(&argument.name)?,"value":value(&argument.value,budget,depth+1)?}));
    }
    Ok(Value::Array(output))
}
fn value(input: &FilterValue, budget: &mut Budget<'_, '_>, depth: usize) -> Result<Value> {
    budget.node(depth)?;
    Ok(match input {
        FilterValue::Number(input) => json!({"Number":number(input,budget,depth+1)?}),
        FilterValue::Bool(input) => json!({"Bool":input}),
        FilterValue::String(input) => json!({"String":budget.text(input)?}),
        FilterValue::Enum { domain, value } => json!({"Enum":{"domain":budget.text(domain)?,"value":value}}),
        FilterValue::Resource(id) => {
            if id.0 == 0 {
                return Err(error("zero resource identity"));
            }
            json!({"Resource":id.0.to_string()})
        }
        FilterValue::Color { components, encoding, profile } => {
            if components.len() > 16 || profile.is_some_and(|id| id.0 == 0) {
                return Err(error("color component/resource bound"));
            }
            let mut numbers = Vec::new();
            numbers.try_reserve_exact(components.len()).map_err(|_| error("color allocation denied"))?;
            for component in components {
                numbers.push(number(component, budget, depth + 1)?);
            }
            json!({"Color":{"components":numbers,"encoding":encoding,"profile":profile.map(|id|id.0.to_string())}})
        }
        FilterValue::Config(values) => json!({"Config":arguments(values,budget,depth+1)?}),
        FilterValue::Curve { free, points_f64_bits, point_types, samples_f64_bits } => {
            if points_f64_bits.len() > 128 || point_types.len() > 128 || samples_f64_bits.len() > 1024 {
                return Err(error("curve exceeds bounded exact detail domain"));
            }
            let mut points = Vec::new();
            points.try_reserve_exact(points_f64_bits.len()).map_err(|_| error("curve point allocation denied"))?;
            for [x, y] in points_f64_bits {
                budget.node(depth + 1)?;
                points.push([format!("0x{x:016x}"), format!("0x{y:016x}")]);
            }
            let mut types = Vec::new();
            types.try_reserve_exact(point_types.len()).map_err(|_| error("curve type allocation denied"))?;
            for point in point_types {
                budget.node(depth + 1)?;
                types.push(*point);
            }
            let mut samples = Vec::new();
            samples.try_reserve_exact(samples_f64_bits.len()).map_err(|_| error("curve sample allocation denied"))?;
            for sample in samples_f64_bits {
                budget.node(depth + 1)?;
                samples.push(format!("0x{sample:016x}"));
            }
            json!({"Curve":{"free":free,"points_f64_bits":points,"point_types":types,"samples_f64_bits":samples}})
        }
        FilterValue::Graph(graph) => {
            if graph.nodes.len() > 64 || graph.edges.len() > 128 || graph.output.0 == 0 || graph.input.is_some_and(|id| id.0 == 0) {
                return Err(error("graph exceeds bounded exact detail domain"));
            }
            let mut nodes = Vec::new();
            nodes.try_reserve_exact(graph.nodes.len()).map_err(|_| error("graph node allocation denied"))?;
            for node in &graph.nodes {
                budget.node(depth + 1)?;
                if node.id.0 == 0 {
                    return Err(error("zero graph node identity"));
                }
                nodes.push(json!({"id":node.id.0.to_string(),"operation":operation(&node.operation,budget,depth+2)?,"version_origin":node.version_origin,"arguments":arguments(&node.arguments,budget,depth+2)?}));
            }
            let mut edges = Vec::new();
            edges.try_reserve_exact(graph.edges.len()).map_err(|_| error("graph edge allocation denied"))?;
            for edge in &graph.edges {
                budget.node(depth + 1)?;
                if edge.from.0 == 0 || edge.to.0 == 0 {
                    return Err(error("zero graph edge identity"));
                }
                edges.push(json!({"from":edge.from.0.to_string(),"output_port":budget.text(&edge.output_port)?,"to":edge.to.0.to_string(),"input_port":budget.text(&edge.input_port)?}));
            }
            let pipeline = graph.original_pipeline.as_deref().map(|pipeline| budget.text(pipeline)).transpose()?;
            json!({"Graph":{"input":graph.input.map(|id|id.0.to_string()),"nodes":nodes,"edges":edges,"output":graph.output.0.to_string(),"original_pipeline":pipeline}})
        }
    })
}
fn transform(value: &TransformState) -> Value {
    json!({"matrix_f64_bits":value.matrix_f64_bits.map(|bits|format!("0x{bits:016x}")),"interpolation":value.interpolation,"pre_offset":value.pre_offset})
}
fn preview(text: &str, budget: &mut Budget<'_, '_>) -> Result<Value> {
    let end = text.char_indices().nth(256).map_or(text.len(), |(offset, _)| offset);
    Ok(json!({"text":budget.text(text.get(..end).ok_or_else(||error("UTF8 preview boundary"))?)?,"bytes":text.len().to_string(),"truncated":end<text.len()}))
}
/// Metadata only: owner census<=10000, configuration page<=64, nested reply
/// ledger<=64KiB/depth8/1024nodes. Larger nested values refuse atomically instead
/// of returning a silently lossy value. Cache content is never hashed/read here.
pub fn source_with_interrupt(document: &Document, owner: ItemRef, offset: usize, limit: usize, control: &Interrupt<'_>) -> Result<Value> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if owner.raw() == 0 || offset > 100_000 || limit == 0 || limit > 64 {
        return Err(error("owner/page outside bounded domain"));
    }
    let state = document.xcf_state.as_ref().ok_or_else(|| error("missing retained source metadata"))?;
    if state.source_items.len() > 10_000 {
        return Err(error("source census exceeds10000"));
    }
    let mut found = None;
    for source in &state.source_items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if source.owner == owner {
            if found.is_some() {
                return Err(error("duplicate source owner"));
            }
            found = Some(source);
        }
    }
    let source = found.ok_or_else(|| error("unknown source owner"))?;
    let mut budget = Budget { control, bytes: 4096, nodes: 0 };
    let (metadata, configuration) = match &source.source {
        EditableSource::Text { content, markup, font, direction, source_modified, configuration, current_origin } => {
            if font.0 == 0 {
                return Err(error("zero font resource identity"));
            }
            (
                json!({"kind":"text","content":preview(content,&mut budget)?,"markup":markup.as_deref().map(|markup|preview(markup,&mut budget)).transpose()?,"fontId":font.0.to_string(),"direction":direction,"sourceModified":source_modified,"currentOriginPx":current_origin}),
                configuration,
            )
        }
        EditableSource::Vector { path, source_modified, configuration } => {
            if path.0 == 0 {
                return Err(error("zero path identity"));
            }
            (json!({"kind":"vector","pathId":path.0.to_string(),"sourceModified":source_modified}), configuration)
        }
        EditableSource::Linked { resource, last_verified_hash, configuration, current } => {
            if resource.0 == 0 {
                return Err(error("zero linked resource identity"));
            }
            let verified = last_verified_hash.as_deref().map(|hash| budget.text(hash)).transpose()?;
            (
                json!({"kind":"linked","resourceId":resource.0.to_string(),"lastVerifiedHash":verified,"current":current.as_ref().map(|current|json!({"bounds":current.bounds,"transform":current.transform.as_ref().map(transform)}))}),
                configuration,
            )
        }
    };
    let end = offset.saturating_add(limit).min(configuration.len());
    let entries = if offset >= configuration.len() {
        Value::Array(Vec::new())
    } else {
        arguments(configuration.get(offset..end).ok_or_else(|| error("configuration page bounds"))?, &mut budget, 0)?
    };
    let cache=source.cache.as_ref().map(|cache|->Result<Value>{Ok(json!({"blake3":budget.text(&cache.blake3)?,"width":cache.width,"height":cache.height,"format":cache.format,"bytes":cache.bytes.len().to_string(),"payloadRead":false}))}).transpose()?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(json!({"owner":crate::xcf_inspect_cmds::reference(owner),"source":metadata,"originalTransform":source.transform.as_ref().map(transform),"cache":cache,
        "configuration":{"total":configuration.len(),"offset":offset,"limit":limit,"entries":entries,"truncated":end<configuration.len()},
        "numericEncoding":"integers:decimal strings; F32/F64:fixed-width lowercase hexadecimal bits","accessGranted":false,"qualification":false,"admission":"typed metadata only; no schema/evaluator/recovery authority"}))
}
