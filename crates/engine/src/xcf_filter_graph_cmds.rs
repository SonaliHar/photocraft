//! Inert structural planning for retained Graph arguments. No evaluator/grant.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::gimp::graph_plan::{Endpoint, GraphPlanError, plan_graph};
use photocraft_doc::{
    item_identity::FilterId,
    xcf_filters::{FilterGraph, FilterValue},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

const ID: &str = "xcf.filter.graph.inspectPlan";
const ROWS: usize = 64;
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn check(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
struct Params<'a> {
    filter: FilterId,
    argument: &'a str,
    depth: usize,
    offset: usize,
    limit: usize,
}
fn params(value: &Value) -> Result<Params<'_>> {
    let object = value.as_object().ok_or_else(|| bad("filterId and argument are required"))?;
    if object.len() > 5 || object.keys().any(|key| !matches!(key.as_str(), "filterId" | "argument" | "maxDepth" | "offset" | "limit")) {
        return Err(bad("unknown graph query field"));
    }
    let id = object.get("filterId").and_then(Value::as_str).ok_or_else(|| bad("filterId must be a canonical nonzero decimal string"))?;
    if id.is_empty() || id.len() > 20 || id.starts_with('0') || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("invalid canonical filterId"));
    }
    let filter = FilterId(id.parse::<u64>().map_err(|_| bad("filterId overflow"))?);
    let argument = object
        .get("argument")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= 128 && !name.contains('\0'))
        .ok_or_else(|| bad("argument must be a bounded exact name"))?;
    let depth = match object.get("maxDepth") {
        None => 32,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=4096).contains(n))
            .ok_or_else(|| bad("maxDepth must be an integer in 1..4096"))?,
    };
    let offset = match object.get("offset") {
        None => 0,
        Some(value) => {
            value.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n <= 16384).ok_or_else(|| bad("offset must be an unsigned integer in 0..16384"))?
        }
    };
    let limit = match object.get("limit") {
        None => ROWS,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=ROWS).contains(n))
            .ok_or_else(|| bad("limit must be an unsigned integer in 1..64"))?,
    };
    Ok(Params { filter, argument, depth, offset, limit })
}
/// Root dispatch calls this before cloning params or normalizing a float preview.
pub(crate) fn dispatch_preflight(value: &Value) -> Result<()> {
    params(value).map(|_| ())
}
fn graph_envelope(graph: &FilterGraph, control: &Interrupt<'_>) -> Result<()> {
    if graph.nodes.len() > 4096 || graph.edges.len() > 16384 {
        return Err(bad("graph node/edge limit"));
    }
    let mut bytes = 0usize;
    for node in &graph.nodes {
        check(control)?;
        for (text, limit) in [(&node.operation.name, 256), (&node.operation.stored_version, 64), (&node.operation.schema_epoch, 256)] {
            if text.len() > limit || text.contains('\0') {
                return Err(bad("graph operation metadata limit"));
            }
            bytes = bytes.checked_add(text.len()).filter(|n| *n <= 4 << 20).ok_or_else(|| bad("graph metadata limit"))?;
        }
    }
    let mut port_bytes = 0usize;
    for edge in &graph.edges {
        check(control)?;
        for port in [&edge.input_port, &edge.output_port] {
            if port.is_empty() || port.len() > 256 || port.contains('\0') {
                return Err(bad("graph port metadata limit"));
            }
            port_bytes = port_bytes.checked_add(port.len()).filter(|n| *n <= 64 << 10).ok_or_else(|| bad("graph port aggregate limit"))?;
        }
    }
    Ok(())
}
fn rows<T>(values: &[T], offset: usize, limit: usize, control: &Interrupt<'_>, map: impl Fn(&T, usize) -> Value) -> Result<Value> {
    check(control)?;
    if offset > 16384 || !(1..=ROWS).contains(&limit) {
        return Err(bad("graph page limit"));
    }
    let mut entries = Vec::new();
    entries.try_reserve_exact(values.len().saturating_sub(offset).min(limit)).map_err(|_| bad("graph query allocation"))?;
    for (relative, value) in values.iter().skip(offset).take(limit).enumerate() {
        check(control)?;
        let index = offset.checked_add(relative).ok_or_else(|| bad("graph page index overflow"))?;
        entries.push(map(value, index));
    }
    let end = offset.checked_add(entries.len()).ok_or_else(|| bad("graph page index overflow"))?;
    let has_more = end < values.len();
    let truncated = entries.len() < values.len();
    Ok(json!({"total":values.len(),"offset":offset,"limit":limit,"entries":entries,"truncated":truncated,
        "hasMore":has_more,"nextOffset":has_more.then_some(end)}))
}
pub fn inspect_with_interrupt(session: &Session, value: &Value, control: &Interrupt<'_>) -> Result<Value> {
    let params = params(value)?;
    check(control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.resident_arc().is_none() || state.doc.id.0 == 0 {
        return Err(bad("graph inspection requires a resident canonical metadata carrier"));
    }
    let xcf = state.doc.xcf_state.as_ref().ok_or_else(|| bad("missing retained XCF state"))?;
    if xcf.filter_stacks.len() > 10_000 {
        return Err(bad("filter stack census limit"));
    }
    let mut total = 0usize;
    for stack in &xcf.filter_stacks {
        check(control)?;
        total = total.checked_add(stack.nodes.len()).filter(|n| *n <= 10_000).ok_or_else(|| bad("filter census limit"))?;
    }
    let mut found = None;
    for stack in &xcf.filter_stacks {
        for node in &stack.nodes {
            check(control)?;
            if node.id == params.filter {
                if found.is_some() {
                    return Err(bad("duplicate current filterId"));
                }
                found = Some((stack.owner, node));
            }
        }
    }
    let (owner, node) = found.ok_or_else(|| bad("unknown current filterId"))?;
    if owner.raw() == 0 || node.arguments.len() > 64 {
        return Err(bad("filter owner/argument census limit"));
    }
    let mut graph = None;
    for argument in &node.arguments {
        check(control)?;
        if argument.name == params.argument {
            if graph.is_some() {
                return Err(bad("duplicate current argument name"));
            }
            let FilterValue::Graph(value) = &argument.value else {
                return Err(bad("selected argument is not a retained Graph"));
            };
            graph = Some(value);
        }
    }
    let graph = graph.ok_or_else(|| bad("no retained graph with the exact argument name"))?;
    graph_envelope(graph, control)?;
    // Only structural port identity is admitted here. Passing stored names to the
    // DAG planner does not verify a stock operation's port schema or semantics.
    let plan = plan_graph(graph, params.depth, control, |_, _, _| true)
        .map_err(|error| if matches!(error, GraphPlanError::Cancelled) || control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) })?;
    let declared = rows(&graph.nodes, params.offset, params.limit, control, |node, index| {
        json!({"id":node.id.0.to_string(),"declarationIndex":index,
        "operation":node.operation,"versionOrigin":node.version_origin,"argumentCount":node.arguments.len(),"argumentsInspected":false})
    })?;
    let edges = rows(
        &graph.edges,
        params.offset,
        params.limit,
        control,
        |edge, index| json!({"declarationIndex":index,"from":edge.from.0.to_string(),"to":edge.to.0.to_string(),"outputPort":edge.output_port,"inputPort":edge.input_port}),
    )?;
    let ordered = rows(
        plan.nodes(),
        params.offset,
        params.limit,
        control,
        |node, index| json!({"planIndex":index,"id":node.id().0.to_string(),"declarationIndex":node.index(),"dependencyDepth":node.depth()}),
    )?;
    let output = match plan.output() {
        Endpoint::ExternalInput(id) => json!({"kind":"externalInput","id":id.0.to_string()}),
        Endpoint::Node { id, index } => json!({"kind":"node","id":id.0.to_string(),"declarationIndex":index}),
    };
    check(control)?;
    let owner_kind = match owner {
        photocraft_doc::item_identity::ItemRef::Layer(_) => "layer",
        photocraft_doc::item_identity::ItemRef::Channel(_) => "channel",
        photocraft_doc::item_identity::ItemRef::Mask(_) => "mask",
        photocraft_doc::item_identity::ItemRef::Path(_) => "path",
    };
    Ok(
        json!({"documentId":state.doc.id.0.to_string(),"revisionString":state.revision.to_string(),"filterId":params.filter.0.to_string(),"argument":params.argument,
        "owner":{"kind":owner_kind,"id":owner.raw().to_string()},"inputId":graph.input.map(|id|id.0.to_string()),"output":output,
        "declarationNodes":declared,"declarationEdges":edges,"topologicalNodes":ordered,"maxDepth":params.depth,
        "originalPipelineRetained":graph.original_pipeline.is_some(),"originalPipelineBytes":graph.original_pipeline.as_ref().map(String::len),
        "structuralPlanValid":true,"operationSemanticsQualified":false,"portSemanticsQualified":false,"argumentsSemanticsQualified":false,
        "renderQualified":false,"metadataOnly":true,"mutated":false}),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Inspect Retained Filter Graph Plan",
        menu: &[],
        shortcut: None,
        params: r#"{filterId:canonical nonzero decimal U64 string,argument:exact name<=128bytes,maxDepth?:integer1..4096=32,offset?:unsigned integer0..16384=0,limit?:integer1..64=64} -> structural DAG plan; offset/limit apply independently to declaration-node/edge/topological pages, absolute indexes retained, stored ports unchanged; no operation/port/render qualification, mutation or grants"#,
        enabled: |s| s.active().and_then(|d| d.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no retained XCF state".into()),
        journal: false,
        run: |s, p| inspect_with_interrupt(s, p, &Interrupt::NONE),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        item_identity::{GraphNodeId, ItemRef},
        xcf_filters::{FilterArgument, FilterNode, FilterStack, GraphEdge, GraphNode, OperationIdentity},
    };
    fn graph() -> FilterGraph {
        FilterGraph {
            input: Some(GraphNodeId(1)),
            nodes: vec![GraphNode {
                id: GraphNodeId(2),
                operation: OperationIdentity { name: "own:unknown".into(), stored_version: "0:0".into(), schema_epoch: "unqualified".into() },
                version_origin: Default::default(),
                arguments: vec![],
            }],
            edges: vec![GraphEdge { from: GraphNodeId(1), output_port: "unaltered-output".into(), to: GraphNodeId(2), input_port: "unaltered-input".into() }],
            output: GraphNodeId(2),
            original_pipeline: Some("own retained text".into()),
        }
    }
    #[test]
    fn exact_borrowed_query_fields_reject_hostile_or_rounded_id_and_depth() {
        assert!(dispatch_preflight(&json!({"filterId":"18446744073709551615","argument":"graph","maxDepth":4096})).is_ok());
        assert!(dispatch_preflight(&json!({"filterId":"1","argument":"graph","offset":16384,"limit":64})).is_ok());
        for value in [
            Value::Null,
            json!({"filterId":1,"argument":"graph"}),
            json!({"filterId":"01","argument":"graph"}),
            json!({"filterId":"1","argument":"graph","maxDepth":1.0}),
            json!({"filterId":"1","argument":"graph","maxDepth":0}),
            json!({"filterId":"1","argument":"graph","ignored":{}}),
        ] {
            assert!(dispatch_preflight(&value).is_err());
        }
        for patch in [
            json!({"offset":-1}),
            json!({"offset":16385}),
            json!({"offset":1.0}),
            json!({"offset":"01"}),
            json!({"limit":0}),
            json!({"limit":65}),
            json!({"limit":1.0}),
            json!({"limit":null}),
        ] {
            let mut query = json!({"filterId":"1","argument":"graph"});
            query.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
            assert!(dispatch_preflight(&query).is_err());
        }
    }
    #[test]
    fn pagination_reaches_late_metadata_with_absolute_indexes_and_bounded_empty_pages() {
        let values: Vec<_> = (0..70).collect();
        let first = rows(&values, 0, 64, &Interrupt::NONE, |value, index| json!({"value":value,"declarationIndex":index})).unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), 64);
        assert_eq!(first["nextOffset"], 64);
        let last = rows(&values, 64, 64, &Interrupt::NONE, |value, index| json!({"value":value,"declarationIndex":index})).unwrap();
        assert_eq!(last["entries"].as_array().unwrap().len(), 6);
        assert_eq!(last["entries"][0]["declarationIndex"], 64);
        assert_eq!(last["entries"][5]["value"], 69);
        assert_eq!(last["hasMore"], false);
        assert_eq!(last["nextOffset"], Value::Null);
        assert_eq!(last["truncated"], true);
        let empty = rows(&values, 16384, 1, &Interrupt::NONE, |value, index| json!({"value":value,"planIndex":index})).unwrap();
        assert_eq!(empty["entries"].as_array().unwrap().len(), 0);
        assert_eq!(empty["total"], 70);
        assert_eq!(empty["offset"], 16384);
        let stop = || true;
        assert!(matches!(rows(&values, 64, 1, &Interrupt::cancel_only(&stop), |value, _| json!(value)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn structural_planner_preserves_declared_unknown_ports_but_refuses_cycles_and_duplicates() {
        let original = graph();
        graph_envelope(&original, &Interrupt::NONE).unwrap();
        let plan = plan_graph(&original, 32, &Interrupt::NONE, |_, _, _| true).unwrap();
        assert_eq!(plan.nodes()[0].id(), GraphNodeId(2));
        assert_eq!(original.edges[0].input_port, "unaltered-input");
        let mut duplicate = original.clone();
        duplicate.nodes.push(duplicate.nodes[0].clone());
        assert!(plan_graph(&duplicate, 32, &Interrupt::NONE, |_, _, _| true).is_err());
        let mut cycle = original.clone();
        cycle.edges.push(GraphEdge { from: GraphNodeId(2), output_port: "output".into(), to: GraphNodeId(2), input_port: "other".into() });
        assert!(plan_graph(&cycle, 32, &Interrupt::NONE, |_, _, _| true).is_err());
        let stop = || true;
        assert!(graph_envelope(&original, &Interrupt::cancel_only(&stop)).is_err());
    }
    #[test]
    fn actual_retained_graph_query_is_inert_native_exact_and_keeps_real_redo() {
        let mut document = photocraft_doc::Document::with_background(
            "own graph",
            photocraft_doc::Size::new(1, 1),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let owner = document.layers.first().unwrap().id;
        let mut xcf = photocraft_doc::XcfState::default();
        xcf.reconcile_layers(&document).unwrap();
        xcf.filter_stacks.push(FilterStack {
            owner: ItemRef::Layer(owner),
            revision: 0,
            nodes: vec![FilterNode {
                id: FilterId(77),
                operation: OperationIdentity {
                    name: "own:retained-unqualified".into(),
                    stored_version: "0:0".into(),
                    schema_epoch: "own-metadata-only".into(),
                },
                source_display_name: None,
                source_icon: None,
                arguments: vec![FilterArgument { name: "graph".into(), value: FilterValue::Graph(graph()) }],
                enabled: false,
                opacity_f64_bits: 1.0_f64.to_bits(),
                source_clipping: None,
                source_region: None,
                mask: None,
                auxiliary: vec![],
                composition: None,
            }],
        });
        document.xcf_state = Some(xcf);
        let mut session = Session::new();
        session.open_document(document, None).unwrap();
        session
            .edit("Real redo", |document, _| {
                document.name = "redo".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let query = json!({"filterId":"77","argument":"graph"});
        let result = inspect_with_interrupt(&session, &query, &Interrupt::NONE).unwrap();
        assert_eq!(result["structuralPlanValid"], true);
        assert_eq!(result["renderQualified"], false);
        assert_eq!(result["portSemanticsQualified"], false);
        assert_eq!(result["declarationEdges"]["entries"][0]["inputPort"], "unaltered-input");
        let stop = || true;
        assert!(matches!(inspect_with_interrupt(&session, &query, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo");
    }
}
