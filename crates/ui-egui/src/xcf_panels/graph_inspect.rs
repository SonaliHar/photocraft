//! On-demand structural metadata pages, never graph editing or qualification.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{item_identity::FilterId, xcf_filters::FilterValue};
use photocraft_engine::document_snapshot::{DocumentSnapshot, WeakDocumentSnapshot};
use serde_json::{Value, json};
use std::sync::Arc;

const COMMAND: &str = "xcf.filter.graph.inspectPlan";
const STALE: &str = "xcf.graph.ui.stale-target";
const INVALID: &str = "xcf.graph.ui.invalid-page";
const PAGE: u32 = 32;
#[derive(Clone)]
struct Target { document: photocraft_doc::DocId, revision: u64, filter: FilterId, argument: String, snapshot: WeakDocumentSnapshot }
#[derive(Clone)]
struct DisplayPage { target: Target, offset: u32, more: bool, nodes: Vec<String>, edges: Vec<String> }
fn stable(app: &PhotocraftApp, target: &Target) -> bool {
    app.session.active().is_some_and(|active| active.doc.id == target.document && active.revision == target.revision
        && target.snapshot.upgrade().is_some_and(|snapshot| snapshot.shares_state(&active.doc)))
}
fn target(snapshot: &DocumentSnapshot, revision: u64, filter: FilterId, argument: &str) -> Result<Target, String> {
    if filter.0 == 0 || argument.is_empty() || argument.len() > 128 || argument.contains('\0') { return Err(INVALID.into()); }
    let mut owned = String::new(); owned.try_reserve_exact(argument.len()).map_err(|_| INVALID)?; owned.push_str(argument);
    Ok(Target {document:snapshot.id,revision,filter,argument:owned,snapshot:snapshot.downgrade()})
}
fn current_graph(app: &PhotocraftApp, target: &Target) -> Result<(),String> {
    if !stable(app,target) { return Err(STALE.into()); }
    let active = app.session.active().ok_or(STALE)?;
    let state = active.doc.xcf_state.as_ref().ok_or(STALE)?;
    if state.filter_stacks.len() > 10_000 {return Err(INVALID.into());}
    let mut count=0usize; let mut found=None;
    for stack in &state.filter_stacks {
        count=count.checked_add(stack.nodes.len()).filter(|count|*count<=10_000).ok_or(INVALID)?;
        for node in &stack.nodes {if node.id==target.filter {if found.is_some() {return Err(STALE.into());} found=Some(node);}}
    }
    let node=found.ok_or(STALE)?;
    if node.arguments.len()>64 {return Err(INVALID.into());}
    let mut arguments=node.arguments.iter().filter(|argument|argument.name==target.argument);
    let argument=arguments.next().ok_or(STALE)?;
    if arguments.next().is_some() || !matches!(argument.value,FilterValue::Graph(_)) {return Err(STALE.into());}
    Ok(())
}
fn text(value:&Value,key:&str,max:usize)->Result<String,String> {
    let text=value.get(key).and_then(Value::as_str).filter(|text|text.len()<=max && !text.contains('\0')).ok_or(INVALID)?;
    let mut out=String::new();out.try_reserve_exact(text.len()).map_err(|_|INVALID)?;
    for ch in text.chars() {out.push(if ch.is_control() {' '} else {ch});} Ok(out)
}
fn page_rows<'a>(value:&'a Value,key:&str,offset:u32,max_total:u64)->Result<(&'a [Value],bool),String> {
    let page=value.get(key).ok_or(INVALID)?;
    if page.get("offset").and_then(Value::as_u64)!=Some(u64::from(offset)) || page.get("limit").and_then(Value::as_u64)!=Some(u64::from(PAGE))
        || page.get("total").and_then(Value::as_u64).is_none_or(|total|total>max_total) {return Err(INVALID.into());}
    let rows=page.get("entries").and_then(Value::as_array).filter(|rows|rows.len()<=PAGE as usize).ok_or(INVALID)?;
    let more=page.get("hasMore").and_then(Value::as_bool).ok_or(INVALID)?;
    Ok((rows,more))
}
fn decode(value:&Value,target:Target,offset:u32)->Result<DisplayPage,String> {
    if value.get("documentId").and_then(Value::as_str)!=Some(target.document.0.to_string().as_str())
        || value.get("revisionString").and_then(Value::as_str)!=Some(target.revision.to_string().as_str())
        || value.get("filterId").and_then(Value::as_str)!=Some(target.filter.0.to_string().as_str())
        || value.get("argument").and_then(Value::as_str)!=Some(target.argument.as_str())
        || value.get("metadataOnly")!=Some(&Value::Bool(true)) || value.get("structuralPlanValid")!=Some(&Value::Bool(true))
        || value.get("renderQualified")!=Some(&Value::Bool(false)) || value.get("operationSemanticsQualified")!=Some(&Value::Bool(false))
        || value.get("portSemanticsQualified")!=Some(&Value::Bool(false)) || value.get("mutated")!=Some(&Value::Bool(false)) {return Err(INVALID.into());}
    let (declared,more_nodes)=page_rows(value,"declarationNodes",offset,4096)?;
    let (connections,more_edges)=page_rows(value,"declarationEdges",offset,16384)?;
    let mut nodes=Vec::new();nodes.try_reserve_exact(declared.len()).map_err(|_|INVALID)?;
    for row in declared {
        let id=text(row,"id",20)?;
        let operation=row.get("operation").ok_or(INVALID)?;
        let caption=text(operation,"name",256)?;
        let version=text(operation,"stored_version",64)?;
        nodes.push(format!("{id}: {caption} ({version})"));
    }
    let mut edges=Vec::new();edges.try_reserve_exact(connections.len()).map_err(|_|INVALID)?;
    for row in connections {edges.push(format!("{}:{} → {}:{}",text(row,"from",20)?,text(row,"outputPort",256)?,text(row,"to",20)?,text(row,"inputPort",256)?));}
    Ok(DisplayPage {target,offset,more:more_nodes||more_edges,nodes,edges})
}
fn inspect(app:&mut PhotocraftApp,target:Target,offset:u32)->Result<DisplayPage,String> {
    if offset>16384 {return Err(INVALID.into());}
    current_graph(app,&target)?;
    let result=app.run(COMMAND,json!({"filterId":target.filter.0.to_string(),"argument":&target.argument,"offset":offset,"limit":PAGE}))?;
    if !stable(app,&target) {return Err(STALE.into());}
    decode(&result,target,offset)
}
/// Called only for the current filter's actual Graph argument. Retained metadata
/// is copied into bounded display rows only after an explicit query click.
pub(super) fn properties(app:&mut PhotocraftApp,ui:&mut egui::Ui,snapshot:&DocumentSnapshot,revision:u64,filter:FilterId,argument:&str) {
    let key=egui::Id::new("xcf_graph_inspection_page");
    let cached=ui.data(|data|data.get_temp::<Arc<DisplayPage>>(key)).filter(|page|page.target.filter==filter && page.target.argument==argument && stable(app,&page.target));
    let valid_draft=app.ui.xcf_graph_page_draft.as_ref().filter(|draft|draft.document==snapshot.id.0 && draft.revision==revision && draft.filter==filter.0 && draft.argument==argument && draft.argument.len()<=128 && draft.offset<=16384);
    let offset=cached.as_ref().map(|page|page.offset).or_else(||valid_draft.map(|draft|draft.offset)).unwrap_or(0);
    ui.label(egui::RichText::new(tl!("Graph structure")).color(Tokens::get(ui.ctx()).text));
    ui.label(tl!("Retained structure only; operation behavior and rendering are not qualified by this inspection."));
    let mut query=None;
    ui.horizontal(|ui| {
        if ui.button(tl!("Inspect")).clicked() {query=Some(offset);}
        if ui.add_enabled(offset>=PAGE,egui::Button::new(tl!("Previous"))).clicked() {query=Some(offset.saturating_sub(PAGE));}
        if ui.add_enabled(cached.as_ref().is_some_and(|page|page.more) && offset<=16384-PAGE,egui::Button::new(tl!("Next"))).clicked() {query=offset.checked_add(PAGE);}
    });
    if let Some(page)=cached {
        ui.label(tl!("Nodes"));for row in &page.nodes {ui.label(row);}
        ui.label(tl!("Connections"));for row in &page.edges {ui.label(row);}
    }
    if let Some(offset)=query {
        let result=target(snapshot,revision,filter,argument).and_then(|target|inspect(app,target,offset));
        match result {
            Ok(page)=>{
                app.ui.xcf_graph_page_draft=Some(crate::state::XcfGraphPageDraft {document:page.target.document.0,revision:page.target.revision,filter:filter.0,argument:page.target.argument.clone(),offset});
                ui.data_mut(|data|data.insert_temp(key,Arc::new(page)));
            }
            Err(error)=>{app.ui.status=error;app.ui.status_error=true;ui.data_mut(|data|data.remove::<Arc<DisplayPage>>(key));}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{item_identity::{GraphNodeId,ItemRef},xcf_filters::{FilterArgument,FilterGraph,FilterNode,FilterStack,GraphEdge,GraphNode,OperationIdentity}};
    fn fixture()->PhotocraftApp {
        let mut document=photocraft_doc::Document::with_background("Own graph UI",photocraft_doc::Size::new(1,1),photocraft_doc::ColorMode::Rgb,photocraft_doc::SampleType::U8,photocraft_doc::Color::WHITE);
        let layer=document.layers[0].id;
        let operation=OperationIdentity {name:"own:metadata-only".into(),stored_version:"0:0".into(),schema_epoch:"unqualified".into()};
        let graph=FilterGraph {input:Some(GraphNodeId(1)),output:GraphNodeId(71),original_pipeline:Some("private-not-displayed".into()),
            nodes:(2..=71).map(|id|GraphNode {id:GraphNodeId(id),operation:operation.clone(),version_origin:Default::default(),arguments:vec![]}).collect(),
            edges:(2..=71).map(|id|GraphEdge {from:GraphNodeId(1),to:GraphNodeId(id),output_port:"output".into(),input_port:"input".into()}).collect()};
        let mut state=photocraft_doc::XcfState::default();state.reconcile_layers(&document).unwrap();
        state.filter_stacks.push(FilterStack {owner:ItemRef::Layer(layer),revision:0,nodes:vec![FilterNode {id:FilterId(77),operation,
            source_display_name:None,source_icon:None,arguments:vec![FilterArgument {name:"graph".into(),value:FilterValue::Graph(graph)}],enabled:false,
            opacity_f64_bits:1f64.to_bits(),source_clipping:None,source_region:None,mask:None,auxiliary:vec![],composition:None}]});
        document.xcf_state=Some(state);
        let mut session=photocraft_engine::Session::new();session.open_document(document,None).unwrap();
        PhotocraftApp::new(session,crate::Services::default())
    }
    #[test]
    fn actual_registered_graph_query_pages_are_inert_and_bounded() {
        let mut app=fixture();let active=app.session.active().unwrap();let before=active.doc.clone();let revision=active.revision;
        let native=photocraft_format::save_to_bytes(&before,&Default::default()).unwrap();
        let first=inspect(&mut app,target(&before,revision,FilterId(77),"graph").unwrap(),0).unwrap();
        assert_eq!((first.nodes.len(),first.edges.len()),(32,32));assert!(first.more);
        assert!(first.nodes.iter().all(|line|!line.contains("private-not-displayed")));
        let last=inspect(&mut app,target(&before,revision,FilterId(77),"graph").unwrap(),64).unwrap();
        assert_eq!((last.nodes.len(),last.edges.len()),(6,6));assert!(!last.more);
        assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().revision,revision);
        assert_eq!(photocraft_format::save_to_bytes(&app.session.active().unwrap().doc,&Default::default()).unwrap(),native);
    }
    #[test]
    fn stale_weak_query_and_hostile_cursor_refuse_before_registered_action() {
        let mut app=fixture();let snapshot=app.session.active().unwrap().doc.clone();let rev=app.session.active().unwrap().revision;
        let captured=target(&snapshot,rev,FilterId(77),"graph").unwrap();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().name="Own replacement".into();
        let before=app.session.active().unwrap().doc.clone();
        assert_eq!(inspect(&mut app,captured,0).err().unwrap(),STALE);
        let captured=target(&before,rev,FilterId(77),"graph").unwrap();
        assert_eq!(inspect(&mut app,captured,16385).err().unwrap(),INVALID);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert!(target(&before,rev,FilterId(0),"graph").is_err());assert!(target(&before,rev,FilterId(77),&"x".repeat(129)).is_err());
    }
    #[test]
    fn restored_page_draft_only_roundtrips_cursor_not_query_authority() {
        let draft=crate::state::XcfGraphPageDraft {document:1,revision:2,filter:77,argument:"graph".into(),offset:64};
        assert_eq!(serde_json::from_value::<crate::state::XcfGraphPageDraft>(serde_json::to_value(&draft).unwrap()).unwrap(),draft);
        let mut app=fixture();app.ui.xcf_graph_page_draft=Some(draft);
        let before=app.session.active().unwrap().doc.clone();let rev=app.session.active().unwrap().revision;
        let mut harness=egui_kittest::Harness::builder().with_size(egui::vec2(420.,360.)).build_ui_state(move |ui,app|properties(app,ui,&before,rev,FilterId(77),"graph"),app);
        harness.run();
        assert!(harness.ctx.data(|data|data.get_temp::<Arc<DisplayPage>>(egui::Id::new("xcf_graph_inspection_page"))).is_none());
        assert_eq!(harness.state().session.active().unwrap().history.past_len(),0);
    }
}
