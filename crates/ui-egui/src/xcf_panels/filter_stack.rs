//! Captured metadata-only stack controls; checked engine rendering occurs on action.
use crate::{PhotocraftApp,theme::Tokens};
use photocraft_doc::{DocId,Document,LayerId,LayerContent,item_identity::{FilterId,ItemRef},xcf_filters::FilterStack};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Value,json};
use std::{collections::HashSet,fmt::Write};

const MOVE:&str="xcf.filter.move";
const REMOVE:&str="xcf.filter.remove";
const LIMIT:&str="xcf.filter.stack.ui.limit";
const STALE:&str="xcf.filter.stack.ui.stale-target";
const CONTEXT:&str="xcf.filter.stack.ui.context";
struct Target {document:DocId,revision:u64,owner:LayerId,filter:FilterId,index:usize,stack_revision:u64,snapshot:WeakDocumentSnapshot}
struct Row {target:Target,caption:String,candidate:bool}
struct View {rows:Vec<Row>,len:usize,unlocked:bool}
#[derive(Clone,Copy)]
enum Action {Move(usize),Remove}

fn stack(document:&Document,owner:LayerId)->Result<&FilterStack,String> {
    let state=document.xcf_state.as_ref().ok_or(STALE)?;
    if owner.0==0 || state.items.len()>10_000 || state.filter_stacks.len()>10_000 {return Err(LIMIT.into());}
    let mut owners=HashSet::new();let mut filters=HashSet::new();let mut visits=0usize;let mut found=None;
    for stack in &state.filter_stacks {
        if stack.owner.raw()==0 || stack.nodes.len()>1024 {return Err(LIMIT.into());}
        owners.try_reserve(1).map_err(|_|LIMIT)?;if !owners.insert(stack.owner) {return Err(STALE.into());}
        visits=visits.checked_add(stack.nodes.len()).filter(|n|*n<=10_000).ok_or(LIMIT)?;
        for node in &stack.nodes {
            filters.try_reserve(1).map_err(|_|LIMIT)?;
            if node.id.0==0 || !filters.insert(node.id) || node.arguments.len()>64 {return Err(STALE.into());}
        }
        if stack.owner==ItemRef::Layer(owner) {found=Some(stack);}
    }
    found.ok_or_else(||STALE.into())
}
fn unlocked(document:&Document,owner:LayerId)->Result<bool,String> {
    let locks=photocraft_engine::xcf_item_locks::effective_layer_locks(document,owner).map_err(|error|error.to_string())?;
    Ok(!locks.native.all && !locks.native.pixels && !locks.imported.content
        && document.layer(owner).is_some_and(|layer|matches!(layer.content,LayerContent::Raster(_))))
}
fn caption(node:&photocraft_doc::xcf_filters::FilterNode)->Result<String,String> {
    let source=node.source_display_name.as_deref().filter(|name|!name.is_empty()).unwrap_or(&node.operation.name);
    let mut output=String::new();output.try_reserve_exact(516).map_err(|_|LIMIT)?;
    for character in source.chars().take(128) {output.push(if character.is_control() {' '}else{character});}
    if source.chars().nth(128).is_some() {output.push('…');}Ok(output)
}
fn metadata_candidate(node:&photocraft_doc::xcf_filters::FilterNode)->bool {
    node.operation.name.len()<=256 && node.operation.stored_version.len()<=64 && node.operation.schema_epoch.len()<=256
        && node.arguments.len()<=64 && photocraft_engine::xcf_filter_cmds::scalar_edit_candidate(node)
}
fn view(app:&PhotocraftApp,owner:LayerId)->Result<Option<View>,String> {
    let Some(active)=app.session.active() else {return Ok(None)};
    let Some(state)=active.doc.xcf_state.as_ref() else {return Ok(None)};
    if active.active_layer!=Some(owner) {return Ok(None);}
    if active.doc.resident_arc().is_none() || active.doc.id.0==0 {return Err(LIMIT.into());}
    if state.filter_stacks.len()>10_000 {return Err(LIMIT.into());}
    if !state.filter_stacks.iter().any(|stack|stack.owner==ItemRef::Layer(owner)) {return Ok(None);}
    let stack=stack(&active.doc,owner)?;if stack.nodes.is_empty() {return Ok(None);}
    let unlocked=unlocked(&active.doc,owner)?;
    let mut rows=Vec::new();rows.try_reserve_exact(stack.nodes.len().min(64)).map_err(|_|LIMIT)?;
    let snapshot=active.doc.downgrade();
    for (index,node) in stack.nodes.iter().take(64).enumerate() {
        rows.push(Row {target:Target {document:active.doc.id,revision:active.revision,owner,filter:node.id,index,stack_revision:stack.revision,snapshot:snapshot.clone()},
            caption:caption(node)?,candidate:metadata_candidate(node)});
    }
    Ok(Some(View {rows,len:stack.nodes.len(),unlocked}))
}
fn validate(app:&PhotocraftApp,target:&Target)->Result<usize,String> {
    let active=app.session.active().ok_or(STALE)?;
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) || app.ui.transform.is_some() || app.session.has_jobs() {return Err(CONTEXT.into());}
    let snapshot=target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id!=target.document || active.revision!=target.revision || active.active_layer!=Some(target.owner) || !snapshot.shares_state(&active.doc) {return Err(STALE.into());}
    let stack=stack(&active.doc,target.owner)?;
    let node=stack.nodes.get(target.index).filter(|node|node.id==target.filter).ok_or(STALE)?;
    if stack.revision!=target.stack_revision || stack.revision==u64::MAX || !metadata_candidate(node) || !unlocked(&active.doc,target.owner)? {return Err(CONTEXT.into());}
    Ok(stack.nodes.len())
}
fn run(app:&mut PhotocraftApp,target:&Target,action:Action)->Result<Value,String> {
    let count=validate(app,target)?;
    let mut id=String::new();id.try_reserve_exact(20).map_err(|_|LIMIT)?;write!(&mut id,"{}",target.filter.0).map_err(|_|LIMIT)?;
    let (command,params)=match action {
        Action::Move(index) if index<count && index<1024=>(MOVE,json!({"filterId":id,"index":index})),
        Action::Move(_)=>return Err(LIMIT.into()),
        Action::Remove=>(REMOVE,json!({"filterId":id})),
    };
    // This public borrowed fence rejects jobs/coalescing/previews before app.run
    // can reach generic command normalization or a background route.
    photocraft_engine::xcf_filter_cmds::pre_dispatch_stack_change(&app.session,command,&params).map_err(|error|error.to_string())?;
    validate(app,target)?;
    app.run(command,params)
}
pub(crate) fn properties(app:&mut PhotocraftApp,ui:&mut egui::Ui,owner:LayerId) {
    let view=match view(app,owner) {Ok(Some(view))=>view,Ok(None)=>return,Err(error)=>{ui.label(error);return;}};
    let context=app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|state|
        state.floating.is_none() && state.revision<u64::MAX && state.doc.xcf_state.as_ref().is_none_or(|xcf|xcf.floating.is_none()));
    let move_enabled=context && photocraft_engine::commands::find(MOVE).is_some_and(|command|(command.enabled)(&app.session).is_ok());
    let remove_enabled=context && photocraft_engine::commands::find(REMOVE).is_some_and(|command|(command.enabled)(&app.session).is_ok());
    ui.separator();ui.label(egui::RichText::new(tl!("GIMP Filters")).color(Tokens::get(ui.ctx()).text));
    let mut action=None;
    for (row_index,row) in view.rows.iter().enumerate() {
        ui.push_id(("xcf_stack_actions",row.target.filter.0),|ui| {
            ui.label(&row.caption);ui.monospace(row.target.filter.0.to_string());
            let enabled=view.unlocked && row.candidate && row.target.stack_revision<u64::MAX;
            ui.horizontal_wrapped(|ui| {
                let up=row.target.index.checked_sub(1);let down=row.target.index.checked_add(1).filter(|index|*index<view.len);
                if ui.add_enabled(enabled && move_enabled && up.is_some(),egui::Button::new(tl!("Move Up"))).clicked() && let Some(index)=up {action=Some((row_index,Action::Move(index)));}
                if ui.add_enabled(enabled && move_enabled && down.is_some(),egui::Button::new(tl!("Move Down"))).clicked() && let Some(index)=down {action=Some((row_index,Action::Move(index)));}
                if ui.add_enabled(enabled && remove_enabled,egui::Button::new(tl!("Remove Filter"))).clicked() {action=Some((row_index,Action::Remove));}
            });
            if !view.unlocked {ui.label(tl!("Layer content is locked"));}else if !row.candidate {ui.label(tl!("Filter editing is unavailable"));}
        });
    }
    if view.len>view.rows.len() {ui.label(format!("{}: {}/{}",tl!("GIMP Filters"),view.rows.len(),view.len));}
    if let Some((index,action))=action {
        let result=view.rows.get(index).ok_or_else(||LIMIT.to_string()).and_then(|row|run(app,&row.target,action));
        if let Err(error)=result {app.ui.status=error;app.ui.status_error=true;}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use egui_kittest::{Harness,kittest::Queryable};
    use photocraft_doc::{ColorMode,Layer,SampleEncoding,SampleType,Size,XcfState,
        import_compositing::{GimpComposition,GimpOperator,GimpSpace,ResolvedGimpComposition},
        xcf_filters::{ExactNumber,FilterArgument,FilterNode,FilterValue,OperationIdentity}};
    fn fixture()->(PhotocraftApp,LayerId) {
        // Own current-model fixture uses an existing scalar template contract;
        // no original GIMP/source qualification is claimed by this UI regression.
        let mut document=Document::new("Own stack controls",Size::new(1,1),ColorMode::Rgb,SampleType::F32);
        document.sample_encoding=Some(SampleEncoding::Linear);document.icc_profile=Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut layer=Layer::raster("Own owner",document.pixel_format());layer.surface_mut().unwrap().try_write_pixel_f64(0,0,&[0.25,0.5,0.75,1.]).unwrap();
        let owner=layer.id;document.layers.push(layer);let mut state=XcfState::default();state.provenance.format_version=22;state.reconcile_layers(&document).unwrap();
        let item=state.items.iter_mut().find(|item|item.item==ItemRef::Layer(owner)).unwrap();item.raster_bounds=Some(document.bounds());item.source_flags=0x12345678;
        item.composition=Some(GimpComposition {compatibility_epoch:"own-current-template-policy".into(),source_mode:28,legacy:false,
            stored_blend_space:1,stored_composite_space:1,stored_operator:1,pass_through:false,
            resolved:Some(ResolvedGimpComposition {blend_space:GimpSpace::LinearRgb,composite_space:GimpSpace::LinearRgb,operator:GimpOperator::Union})});
        let node=|id,exposure:f32|FilterNode {id:FilterId(id),operation:OperationIdentity {name:"gegl:exposure".into(),stored_version:"1:0".into(),schema_epoch:"xcf-22-stock-point-replace".into()},
            source_display_name:Some(format!("Own exposure {id}")),source_icon:None,
            arguments:vec![FilterArgument {name:"exposure".into(),value:FilterValue::Number(ExactNumber::Float32Bits(exposure.to_bits()))},FilterArgument {name:"black-level".into(),value:FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits()))}],
            enabled:true,opacity_f64_bits:1f64.to_bits(),source_clipping:Some(0),source_region:Some(0),mask:None,auxiliary:vec![],
            composition:Some(GimpComposition {compatibility_epoch:"xcf-22-stock-point-replace".into(),source_mode:62,legacy:false,
                stored_blend_space:-261651887,stored_composite_space:-261651887,stored_operator:-261651887,pass_through:false,resolved:None})};
        state.filter_stacks.push(FilterStack {owner:ItemRef::Layer(owner),revision:0,nodes:vec![node(77,0.),node(78,0.5)]});document.xcf_state=Some(state);
        let mut session=photocraft_engine::Session::new();session.open_document(document,None).unwrap();session.select_layer(owner).unwrap();
        session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(),"Own synthetic stack.xcf",None);
        let mut app=PhotocraftApp::new(session,Services::default());app.background_jobs=true;(app,owner)
    }
    #[test]
    fn captured_move_remove_are_synchronous_native_preserving_and_undoable() {
        let (mut app,owner)=fixture();let before=app.session.active().unwrap().doc.clone();let source=app.session.active().unwrap().source.clone();
        let target=view(&app,owner).unwrap().unwrap().rows.remove(0).target;
        let result=run(&mut app,&target,Action::Move(1)).unwrap();assert_ne!(result.get("pending"),Some(&json!(true)));assert!(!app.session.has_jobs());
        let moved=app.session.active().unwrap().doc.clone();let mut expected=(*before).clone();let stack=&mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];stack.nodes.swap(0,1);stack.revision=1;
        assert_eq!(*moved,expected);assert_eq!(app.session.active().unwrap().source,source);
        let bytes=photocraft_format::save_to_bytes(&moved,&Default::default()).unwrap();assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(),expected);
        assert!(app.session.undo());assert_eq!(app.session.active().unwrap().doc,before);assert!(app.session.redo());assert_eq!(app.session.active().unwrap().doc,moved);
        let target=view(&app,owner).unwrap().unwrap().rows.remove(1).target;run(&mut app,&target,Action::Remove).unwrap();
        let removed=app.session.active().unwrap().doc.clone();let mut expected=(*moved).clone();let stack=&mut expected.xcf_state.as_mut().unwrap().filter_stacks[0];stack.nodes.remove(1);stack.revision=2;
        assert_eq!(*removed,expected);assert_eq!(app.session.active().unwrap().source,source);
        assert!(app.session.undo());assert_eq!(app.session.active().unwrap().doc,moved);
    }
    #[test]
    fn actual_widget_move_and_remove_use_captured_registered_routes() {
        let (app,owner)=fixture();let before=app.session.active().unwrap().doc.clone();
        let mut h=Harness::builder().with_size(egui::vec2(600.,380.)).build_ui_state(move|ui,app|properties(app,ui,owner),app);
        h.run();h.query_all_by_label("Move Down").find(|node|!node.accesskit_node().is_disabled()).unwrap().click();h.run();
        assert!(!h.state().ui.status_error);assert!(!h.state().session.has_jobs());
        assert_eq!(h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes[0].id,FilterId(78));
        h.query_all_by_label("Remove Filter").next().unwrap().click();h.run();assert!(!h.state().ui.status_error);
        assert_eq!(h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().filter_stacks[0].nodes.len(),1);
        h.state_mut().run("edit.undo",json!({})).unwrap();h.state_mut().run("edit.undo",json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc,before);
    }
    #[test]
    fn stale_snapshot_locked_owner_and_duplicate_filter_ids_refuse_without_editing() {
        let (mut app,owner)=fixture();let target=view(&app,owner).unwrap().unwrap().rows.remove(0).target;
        app.session.edit("Own metadata change",|document,_|{document.name="changed".into();Ok(())}).unwrap();let changed=app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().revision=target.revision;assert!(run(&mut app,&target,Action::Remove).is_err());assert_eq!(app.session.active().unwrap().doc,changed);
        let (mut app,owner)=fixture();app.session.edit("Own lock",|document,_|{document.layer_mut(owner).unwrap().locks.pixels=true;Ok(())}).unwrap();
        let locked=app.session.active().unwrap().doc.clone();let view=view(&app,owner).unwrap().unwrap();assert!(!view.unlocked);assert!(run(&mut app,&view.rows[0].target,Action::Remove).is_err());assert_eq!(app.session.active().unwrap().doc,locked);
        let mut malformed=(*locked).clone();let stack=&mut malformed.xcf_state.as_mut().unwrap().filter_stacks[0];stack.nodes[1].id=stack.nodes[0].id;
        assert!(super::stack(&malformed,owner).is_err());
    }
    #[test]
    fn actual_and_stale_float_preview_refuse_before_any_drop_or_redo_change() {
        let (mut app,owner)=fixture();app.session.execute("select.all",json!({})).unwrap();
        app.session.edit("Own real redo",|document,_|{document.name="redo".into();Ok(())}).unwrap();assert!(app.session.undo());
        let target=view(&app,owner).unwrap().unwrap().rows.remove(0).target;app.session.execute("select.float",json!({"dx":1,"dy":0,"copy":true})).unwrap();
        for stale in [false,true] {
            if stale {app.session.active_mut().unwrap().floating.as_mut().unwrap().revision=u64::MAX;}
            let state=app.session.active().unwrap();let before=state.doc.clone();let preview=state.floating.clone().unwrap();let redo=state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(run(&mut app,&target,Action::Remove).unwrap_err(),CONTEXT);
            let state=app.session.active().unwrap();assert_eq!(state.doc,before);assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),redo);
            let retained=state.floating.as_ref().unwrap();assert_eq!((retained.layer,retained.offset,retained.revision),(preview.layer,preview.offset,preview.revision));assert!(std::sync::Arc::ptr_eq(&retained.parts,&preview.parts));
        }
    }
}
