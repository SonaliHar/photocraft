//! Explicit retained-chain to typed-graph bridge. Structural mapping only;
//! source admission, evaluator selection and resource authority remain external.
use crate::{IoError,xcf_graphs::{GraphMappingLimits,GraphSchemaResolver,map_graph}};
use photocraft_doc::{item_identity::{GraphNodeId,ResourceId},xcf_filters::{FilterGraph,FilterValue,GraphNode,GraphEdge}};
use photocraft_algo::gimp::{typed_schema::PreparedSchemaIndex,graph_plan::{self,PortDirection,GraphPlan}};
use photocraft_raster::Interrupt;
use photocraft_xcf::StoredGraph;

#[derive(Clone,Copy,Debug)]
pub struct Limits {pub mapping:GraphMappingLimits,pub work_units:u64,pub dependency_depth:usize}
impl Default for Limits {fn default()->Self{Self{mapping:GraphMappingLimits::default(),work_units:1_000_000_000,dependency_depth:32}}}
/// Caller-owned exact port schema, not operation-name dispatch. None is the
/// explicit external image endpoint. Callbacks must be bounded/cooperative and
/// prepay captured backing/allocations outside this public model census.
pub type PortValidator<'a> = dyn FnMut(Option<&GraphNode>,PortDirection,&str)->bool+'a;
pub type AuxiliaryValidator<'a> = dyn FnMut(&GraphNode,&GraphEdge)->bool+'a;
pub type ResourceValidator<'a> = dyn FnMut(ResourceId)->bool+'a;
/// Immutable source/typed relationship. Original AST spans/declaration order are
/// borrowed unchanged, and native graph DTO retains exact original pipeline text.
pub struct GraphCandidate<'a>{source:&'a StoredGraph,original:&'a str,graph:FilterGraph}
impl<'a> GraphCandidate<'a>{
    pub fn source(&self)->&'a StoredGraph{self.source}
    pub fn original(&self)->&'a str{self.original}
    pub fn graph(&self)->&FilterGraph{&self.graph}
    pub fn into_value(self)->FilterValue{FilterValue::Graph(self.graph)}
    /// Existing planner/scheduler input; ports are explicitly checked again by
    /// the requesting caller, never retained as an opaque capability token.
    pub fn plan<'g>(&'g self,depth:usize,ports:&mut PortValidator<'_>,control:&Interrupt<'_>)->Result<GraphPlan<'g>,IoError>{
        graph_plan::plan_graph(&self.graph,depth,control,|id,direction,name|port(&self.graph,id,direction,name,ports))
            .map_err(|e|if control.cancelled(){IoError::Cancelled}else{unsupported(e.to_string())})
    }
}
fn unsupported(message:impl std::fmt::Display)->IoError{IoError::Unsupported(format!("XCF graph candidate: {message}"))}
fn poll(c:&Interrupt<'_>)->Result<(),IoError>{c.check().map_err(|_|IoError::Cancelled)}
fn add(a:u64,b:u64)->Result<u64,IoError>{a.checked_add(b).ok_or_else(||unsupported("accounting overflow"))}
fn mul(a:u64,b:u64)->Result<u64,IoError>{a.checked_mul(b).ok_or_else(||unsupported("accounting overflow"))}
/// Prospective owned mapper/topology/planner index reservation. Counts come from
/// the borrowed AST; this is a conservative private-map reserve, not heap/RSS.
/// Resolver-returned nested argument buffers and opaque validator captures must
/// be admitted by their provider before its callback; they are not inferred here.
#[derive(Clone, Copy, Debug)]
pub struct GraphIndexRequirements {
    pub known_index_bytes: u64,
    pub work_units: u64,
    pub requires_provider_index_admission: bool,
}
/// Allocation-free preparation used by explicit aggregate callers. Legacy
/// map_with_schemas remains unchanged. Caller prepays work before this traversal.
pub fn prospective_index_requirements(
    source: &StoredGraph,
    schemas: &PreparedSchemaIndex<'_, '_>,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<GraphIndexRequirements, IoError> {
    use photocraft_xcf::{GraphEntry, GraphValue};
    let max = Limits::default();
    let m = limits.mapping;
    let d = max.mapping;
    if m.nodes > d.nodes
        || m.edges > d.edges
        || m.depth > d.depth
        || m.entries > d.entries
        || m.resident_bytes > d.resident_bytes
        || limits.work_units > max.work_units
    {
        return Err(unsupported("index limits may only be lowered"));
    }
    fn walk(
        entries: &[GraphEntry],
        depth: usize,
        limits: GraphMappingLimits,
        count: &mut [u64; 5],
        used: &mut u64,
        remaining: &mut u64,
        control: &Interrupt<'_>,
    ) -> Result<(), IoError> {
        poll(control)?;
        if depth > limits.depth {
            return Err(unsupported("index source depth"));
        }
        for entry in entries {
            poll(control)?;
            // Constant-time enum/name shape inspection determines the exact
            // entry + node + derived-edge debit BEFORE its count/child traversal.
            let debit = match entry {
                GraphEntry::Operation { .. } => 128,
                GraphEntry::Property { value: GraphValue::Chain(_), .. } => 96,
                GraphEntry::Property { name, .. } if name == "ref" => 96,
                _ => 64,
            };
            *remaining = remaining.checked_sub(debit).ok_or_else(|| unsupported("index preparation work"))?;
            *used = add(*used, debit)?;
            count[0] = add(count[0], 1)?;
            if count[0] > limits.entries as u64 {
                return Err(unsupported("index source entries"));
            }
            match entry {
                GraphEntry::Operation { .. } => count[1] = add(count[1], 1)?,
                GraphEntry::Property { name, value, .. } => match value {
                    GraphValue::Literal { .. } => {
                        if name == "id" {
                            count[3] = add(count[3], 1)?;
                        } else if name == "ref" {
                            count[4] = add(count[4], 1)?;
                        } else {
                            count[2] = add(count[2], 1)?;
                        }
                    }
                    GraphValue::Chain(children) => {
                        count[4] = add(count[4], 1)?;
                        walk(children, depth + 1, limits, count, used, remaining, control)?;
                    }
                },
            }
        }
        Ok(())
    }
    let mut count = [0; 5];
    let mut used = 0;
    let mut remaining = limits.work_units;
    walk(&source.entries, 0, m, &mut count, &mut used, &mut remaining, control)?;
    let [entries, nodes, properties, aliases, connections] = count;
    if nodes != source.nodes || nodes > m.nodes as u64 || source.edges > m.edges as u64 {
        return Err(unsupported("index source node/edge declaration"));
    }
    // Each operation/aux/ref can create at most one edge in the existing mapper.
    // Use this counted upper even before the later exact topology proof.
    let edges = add(nodes, connections)?;
    let node_slots =
        mul(nodes, (std::mem::size_of::<(GraphNodeId, &str, Vec<crate::xcf_graphs::GraphLiteral<'_>>)>() + std::mem::size_of::<GraphNode>()) as u64)?;
    let property_slots = mul(
        properties,
        (std::mem::size_of::<crate::xcf_graphs::GraphLiteral<'_>>() + std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>()) as u64,
    )?;
    let alias_slots = mul(aliases, std::mem::size_of::<(&str, GraphNodeId)>() as u64)?;
    // Pending enum endpoints, final edges and simultaneous topology pair vectors.
    let edge_slots = mul(edges, 128)?;
    let queues = mul(add(nodes, 1)?, (2 * std::mem::size_of::<usize>()) as u64)?;
    // Reuses the bridge's existing planner/hash/index envelope verbatim.
    let planner = add(mul(add(add(nodes, edges)?, 1)?, 512)?, 2 << 20)?;
    let schema = schemas.requirements();
    let held = add(schema.owned_index_bytes, schema.held_schema_index_bytes)?;
    let known = add(add(add(add(add(node_slots, property_slots)?, alias_slots)?, edge_slots)?, queues)?, add(planner, held)?)?;
    let work = add(mul(entries, 64)?, add(mul(nodes, 32)?, mul(edges, 32)?)?)?;
    if work != used {
        return Err(unsupported("index preparation debit differs from counted formula"));
    }
    poll(control)?;
    Ok(GraphIndexRequirements { known_index_bytes: known, work_units: work, requires_provider_index_admission: true })
}
fn port(graph:&FilterGraph,id:GraphNodeId,direction:PortDirection,name:&str,allowed:&mut PortValidator<'_>)->bool{
    if graph.input==Some(id){return direction==PortDirection::Output&&allowed(None,direction,name);}
    graph.nodes.iter().find(|node|node.id==id).is_some_and(|node|allowed(Some(node),direction,name))
}
fn resources(value:&FilterValue,depth:usize,visits:&mut usize,allowed:&mut ResourceValidator<'_>,c:&Interrupt<'_>)->Result<(),IoError>{
    poll(c)?;*visits=visits.checked_add(1).filter(|n|*n<=100_000).ok_or_else(||unsupported("resource/value traversal limit"))?;
    if depth>32{return Err(unsupported("resource nesting limit"));}
    match value{
        FilterValue::Resource(id)=>if id.0==0||!allowed(*id){return Err(unsupported("unbound explicit resource"));},
        FilterValue::Color{profile:Some(id),..}=>if id.0==0||!allowed(*id){return Err(unsupported("unbound explicit color profile"));},
        FilterValue::Config(arguments)=>for argument in arguments{resources(&argument.value,depth+1,visits,allowed,c)?;},
        FilterValue::Graph(_)=>return Err(unsupported("resolver nested graph lacks retained chain binding")),
        _=>{},
    }Ok(())
}
/// The literal resolver must bind exact source spellings to caller-authorized
/// version/schema identity and native typed values. It supplies no implicit
/// defaults: the existing mapper requires every explicit property exactly once.
/// Each node then passes the prepared schema's ordered/width/domain validator.
/// Aux/resource validators are mandatory, including when the caller rejects all.
#[allow(clippy::too_many_arguments)]
pub fn map_with_schemas<'a>(source:&'a StoredGraph,original:&'a str,schemas:&PreparedSchemaIndex<'_,'_>,limits:Limits,control:&Interrupt<'_>,fresh:&mut dyn FnMut()->GraphNodeId,resolve:&mut GraphSchemaResolver<'_>,ports:&mut PortValidator<'_>,auxiliary:&mut AuxiliaryValidator<'_>,resource:&mut ResourceValidator<'_>,charge:&mut dyn FnMut(usize)->Result<(),IoError>)->Result<GraphCandidate<'a>,IoError>{
    poll(control)?;let maximum=Limits::default();let m=limits.mapping;let d=maximum.mapping;
    if m.nodes>d.nodes||m.edges>d.edges||m.depth>d.depth||m.entries>d.entries||m.resident_bytes>d.resident_bytes||limits.work_units>maximum.work_units||limits.dependency_depth==0||limits.dependency_depth>maximum.dependency_depth{return Err(unsupported("limits may only be lowered"));}
    if source.nodes>m.nodes as u64||source.edges>m.edges as u64||original.len()>1<<20{return Err(unsupported("source graph count/text bounds"));}
    let held=schemas.requirements();let schema_bytes=add(held.owned_index_bytes,held.known_borrowed_bytes)?;
    // Conservative existing planner hash/queue/sort scratch plus linear searches
    // in callbacks. All are prospective, before mapping or planner allocation.
    let plan_bytes=add(mul(add(add(source.nodes,source.edges)?,1)?,512)?,2<<20)?;
    let validation_reserve=photocraft_algo::gimp::typed_schema::Limits::default().retained_bytes as u64;
    let outside=add(add(schema_bytes,plan_bytes)?,validation_reserve)?;
    let mapped_limit=(m.resident_bytes as u64).checked_sub(outside).ok_or_else(||unsupported("schema/planner headroom"))?;
    let work=add(add(mul(m.entries as u64,64)?,mul(original.len() as u64,32)?)?,add(mul(mul(source.nodes,source.nodes)?,16)?,mul(mul(source.edges,add(source.nodes,1)?)?,16)?)?)?;
    let mut remaining=limits.work_units.checked_sub(add(work,100_000*256)?).ok_or_else(||unsupported("mapping/planner/resource work limit"))?;
    charge(usize::try_from(outside).map_err(|_|unsupported("resident address width"))?)?;
    let reduced=GraphMappingLimits{resident_bytes:usize::try_from(mapped_limit).map_err(|_|unsupported("resident address width"))?,..m};
    // Schema order must not silently reorder the retained declarations.
    let mut exact_resolve=|identity:&str,literals:&[crate::xcf_graphs::GraphLiteral<'_>]|{
        poll(control)?;let result=resolve(identity,literals)?;
        if result.arguments.len()!=literals.len()||result.arguments.iter().zip(literals).any(|(argument,literal)|argument.name!=literal.name){return Err(unsupported("resolver changed source property declaration order"));}
        Ok(result)
    };
    let graph=map_graph(source,original,reduced,control,fresh,&mut exact_resolve,charge)?;
    let mut values=0usize;let mut validation_peak=0u64;
    for node in &graph.nodes{
        poll(control)?;remaining=remaining.checked_sub(held.lookup_work_units).ok_or_else(||unsupported("schema lookup work"))?;
        let schema=schemas.lookup(node,control).map_err(|e|if control.cancelled(){IoError::Cancelled}else{unsupported(e)})?;
        let req=schema.requirements();validation_peak=validation_peak.max(req.validation_retained_limit);
        if req.validation_work_limit>remaining{return Err(unsupported("declared schema validation work"));}
        if schema_bytes.checked_add(plan_bytes).and_then(|v|v.checked_add(validation_peak)).is_none_or(|v|v>m.resident_bytes as u64){return Err(unsupported("schema validation resident headroom"));}
        let(_,usage)=schema.validate_node_with_usage(node,control).map_err(|e|if control.cancelled(){IoError::Cancelled}else{unsupported(e)})?;
        remaining=remaining.checked_sub(usage.work_units).ok_or_else(||unsupported("schema validation work"))?;
        for argument in &node.arguments{resources(&argument.value,0,&mut values,resource,control)?;}
    }
    let _=remaining; // declared aggregate resource traversal was prepaid above
    for edge in &graph.edges{poll(control)?;if edge.input_port=="aux"{let owner=graph.nodes.iter().find(|node|node.id==edge.to).ok_or_else(||unsupported("auxiliary owner"))?;if !auxiliary(owner,edge){return Err(unsupported("unbound auxiliary edge"));}}}
    let candidate=GraphCandidate{source,original,graph};
    let _=candidate.plan(limits.dependency_depth,ports,control)?;
    poll(control)?;Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::xcf_filters::{OperationIdentity,GraphVersionOrigin};
    use photocraft_algo::gimp::typed_schema::{PreparedSchema,Schema,Limits as SchemaLimits};
    fn source()->StoredGraph{StoredGraph{entries:vec![photocraft_xcf::GraphEntry::Operation{identity:"gegl:nop".into(),start:0,end:8}],nodes:1,edges:1}}
    #[test]
    fn prospective_index_uses_counted_structures_not_full_resident_quota() {
        let index=PreparedSchemaIndex::prepare(&[],SchemaLimits::default(),&Interrupt::NONE).unwrap();
        let req=prospective_index_requirements(&source(),&index,Limits::default(),&Interrupt::NONE).unwrap();
        assert!(req.known_index_bytes<16<<20);
        assert!(req.requires_provider_index_admission);
        assert_eq!(req.work_units,128);
        let mut low=Limits::default();low.work_units=127;
        assert!(prospective_index_requirements(&source(),&index,low,&Interrupt::NONE).is_err());
        low.work_units=0;
        assert!(prospective_index_requirements(&source(),&index,low,&Interrupt::NONE).is_err());
        let stop=||true;
        assert!(matches!(prospective_index_requirements(&source(),&index,Limits::default(),&Interrupt::cancel_only(&stop)),Err(IoError::Cancelled)));
        let mut false_count=source();false_count.nodes=2;
        assert!(prospective_index_requirements(&false_count,&index,Limits::default(),&Interrupt::NONE).is_err());
    }
    #[test]
    fn explicit_schema_and_ports_preserve_pipeline_spans_and_native_graph_value() {
        let identity=OperationIdentity{name:"gegl:nop".into(),stored_version:"caller-version".into(),schema_epoch:"caller-epoch".into()};
        let schema=PreparedSchema::prepare(Schema{identity:&identity,version_origin:GraphVersionOrigin::SchemaResolved,properties:&[]},SchemaLimits::default(),&Interrupt::NONE).unwrap();
        let references=[&schema];let schemas=PreparedSchemaIndex::prepare(&references,SchemaLimits::default(),&Interrupt::NONE).unwrap();let source=source();let original="gegl:nop";let mut id=0;
        let mut fresh=||{id+=1;GraphNodeId(id)};let mut resolve=|_:&str,_:&[crate::xcf_graphs::GraphLiteral<'_>]|Ok(crate::xcf_graphs::ResolvedOperation{operation:identity.clone(),arguments:Vec::new()});
        let mut ports=|node:Option<&GraphNode>,direction,name:&str|match(node,direction){(None,PortDirection::Output)=>name=="output",(Some(_),PortDirection::Input)=>name=="input",(Some(_),PortDirection::Output)=>name=="output",_=>false};
        let mut auxiliary=|_:&GraphNode,_:&GraphEdge|false;let mut resource=|_:ResourceId|false;let mut charged=0usize;let mut charge=|bytes|{charged+=bytes;Ok(())};
        let candidate=map_with_schemas(&source,original,&schemas,Limits::default(),&Interrupt::NONE,&mut fresh,&mut resolve,&mut ports,&mut auxiliary,&mut resource,&mut charge).unwrap();
        assert_eq!(candidate.source(),&source);assert_eq!(candidate.original(),original);assert_eq!(candidate.graph().original_pipeline.as_deref(),Some(original));assert_eq!(candidate.graph().nodes[0].version_origin,GraphVersionOrigin::SchemaResolved);assert_eq!(candidate.plan(32,&mut ports,&Interrupt::NONE).unwrap().nodes().len(),1);
        let value=candidate.into_value();let encoded=serde_json::to_string(&value).unwrap();let restored:FilterValue=serde_json::from_str(&encoded).unwrap();assert_eq!(restored,value);assert!(charged<=Limits::default().mapping.resident_bytes);
    }
    #[test]
    fn resource_closure_refusal_and_cancel_never_return_partial_mapping() {
        let stop=||true;assert!(matches!(resources(&FilterValue::Resource(ResourceId(7)),0,&mut 0,&mut |_|true,&Interrupt::cancel_only(&stop)),Err(IoError::Cancelled)));
        assert!(resources(&FilterValue::Config(vec![photocraft_doc::xcf_filters::FilterArgument{name:"resource".into(),value:FilterValue::Resource(ResourceId(7))}]),0,&mut 0,&mut |_|false,&Interrupt::NONE).is_err());
    }
}
