//! Explicit saved selection actions; runtime layer/channel/path focus is separate.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, Document, LayerId, MAX_GROUP_DEPTH, item_identity::ItemRef, xcf_editing::EditingSelection};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Value,json};
use std::{collections::HashSet,fmt::Write};

const COMMAND: &str="xcf.items.select";
const LIMIT: &str="xcf.items.select.ui.limit";
const STALE: &str="xcf.items.select.ui.stale-target";
const PREVIEW: &str="xcf.items.select.ui.preview";
const MAX_REFS: usize=1000;
const MAX_ITEMS: usize=10_000;

struct Target { document:DocId,revision:u64,layer:LayerId,snapshot:WeakDocumentSnapshot }
struct View { target:Target,selected:bool,active:bool,count:usize,add_allowed:bool,replace_allowed:bool }
#[derive(Clone,Copy)]
enum Action { Replace,Select,Deselect }

fn total(selection:&EditingSelection)->Result<usize,String> {
    selection.selected.len().checked_add(selection.selected_paths.len())
        .and_then(|n|n.checked_add(usize::from(selection.active.is_some())+usize::from(selection.mask_target.is_some())))
        .filter(|n|*n<=MAX_REFS).ok_or_else(||LIMIT.into())
}
fn current_selection(document:&Document,layer:LayerId)->Result<&EditingSelection,String> {
    let xcf=document.xcf_state.as_ref().ok_or(STALE)?;
    if layer.0==0 || document.id.0==0 || xcf.items.len()>MAX_ITEMS { return Err(LIMIT.into()); }
    total(&xcf.selection)?;
    // Bound native identity discovery before any recursive layer lookup.
    let mut frames:[Option<std::slice::Iter<'_,photocraft_doc::Layer>>;MAX_GROUP_DEPTH+1]=std::array::from_fn(|_|None);
    let mut depth=0usize;*frames.first_mut().ok_or(LIMIT)?=Some(document.layers.iter());
    let mut native_ids=HashSet::new();
    let mut visits=0usize;let mut found=false;
    loop {
        let next=frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(current)=next else { if depth==0 { break; }depth=depth.checked_sub(1).ok_or(LIMIT)?;continue; };
        visits=visits.checked_add(1).filter(|n|*n<=MAX_ITEMS).ok_or(LIMIT)?;
        native_ids.try_reserve(1).map_err(|_|LIMIT)?;
        if current.id.0==0 || !native_ids.insert(current.id) { return Err(STALE.into()); }
        if current.id==layer { if found { return Err(STALE.into()); }found=true; }
        if let Some(children)=current.children() { depth=depth.checked_add(1).filter(|n|*n<=MAX_GROUP_DEPTH).ok_or(LIMIT)?;*frames.get_mut(depth).ok_or(LIMIT)?=Some(children.iter()); }
    }
    if !found { return Err(STALE.into()); }
    let mut available=HashSet::new();
    for item in &xcf.items { available.try_reserve(1).map_err(|_|LIMIT)?;if item.item.raw()==0 || !available.insert(item.item) { return Err(STALE.into()); } }
    if !available.contains(&ItemRef::Layer(layer)) { return Err(STALE.into()); }
    let selection=&xcf.selection;
    let mut selected=HashSet::new();selected.try_reserve(selection.selected.len()).map_err(|_|LIMIT)?;
    for item in &selection.selected { if !available.contains(item) || !selected.insert(*item) { return Err(STALE.into()); } }
    if selection.active.is_some_and(|active|!selected.contains(&active)) { return Err(STALE.into()); }
    let mut paths=HashSet::new();paths.try_reserve(selection.selected_paths.len()).map_err(|_|LIMIT)?;
    for item in &selection.selected_paths {
        if !matches!(item,ItemRef::Path(_)) || !available.contains(item) || !paths.insert(*item) { return Err(STALE.into()); }
    }
    if selection.mask_target.is_some_and(|mask|!matches!(mask,ItemRef::Mask(_)) || !available.contains(&mask)) { return Err(STALE.into()); }
    Ok(selection)
}
fn view(app:&PhotocraftApp,layer:LayerId)->Result<Option<View>,String> {
    let Some(state)=app.session.active() else { return Ok(None) };
    if state.doc.xcf_state.is_none() { return Ok(None); }
    if state.active_layer!=Some(layer) { return Ok(None); }
    if state.doc.resident_arc().is_none() { return Err(LIMIT.into()); }
    let selection=current_selection(&state.doc,layer)?;
    let item=ItemRef::Layer(layer);let selected=selection.selected.contains(&item);let active=selection.active==Some(item);
    let retained=selection.selected_paths.len()+usize::from(selection.mask_target.is_some());
    let add_count=selection.selected.len()+usize::from(!selected)+retained+1;
    let replace_count=retained+2;
    Ok(Some(View { target:Target {document:state.doc.id,revision:state.revision,layer,snapshot:state.doc.downgrade()},
        selected,active,count:selection.selected.len(),add_allowed:add_count<=MAX_REFS,replace_allowed:replace_count<=MAX_REFS }))
}
fn validate<'a>(app:&'a PhotocraftApp,target:&Target)->Result<&'a EditingSelection,String> {
    let state=app.session.active().ok_or(STALE)?;
    if state.floating.is_some() || app.ui.transform.is_some() { return Err(PREVIEW.into()); }
    if app.session.has_jobs() || state.coalesce.is_some() { return Err("xcf.items.select.ui.busy".into()); }
    let snapshot=target.snapshot.upgrade().ok_or(STALE)?;
    if state.doc.id!=target.document || state.revision!=target.revision || state.active_layer!=Some(target.layer) || !snapshot.shares_state(&state.doc) { return Err(STALE.into()); }
    current_selection(&state.doc,target.layer)
}
fn copy(values:&[ItemRef])->Result<Vec<ItemRef>,String> {
    if values.len()>MAX_REFS { return Err(LIMIT.into()); }
    let mut output=Vec::new();output.try_reserve_exact(values.len()).map_err(|_|LIMIT)?;output.extend_from_slice(values);Ok(output)
}
fn wire(item:ItemRef)->Result<Value,String> {
    if item.raw()==0 { return Err(STALE.into()); }
    let kind=match item { ItemRef::Layer(_)=>"layer",ItemRef::Channel(_)=>"channel",ItemRef::Mask(_)=>"mask",ItemRef::Path(_)=>"path" };
    let mut id=String::new();id.try_reserve_exact(20).map_err(|_|LIMIT)?;write!(&mut id,"{}",item.raw()).map_err(|_|LIMIT)?;
    Ok(json!({"kind":kind,"id":id}))
}
fn wire_list(values:&[ItemRef])->Result<Vec<Value>,String> {
    let mut output=Vec::new();output.try_reserve_exact(values.len()).map_err(|_|LIMIT)?;
    for item in values { output.push(wire(*item)?); }Ok(output)
}
fn run(app:&mut PhotocraftApp,target:&Target,action:Action)->Result<Value,String> {
    let selection=validate(app,target)?;
    let layer=ItemRef::Layer(target.layer);
    let mut selected=match action { Action::Replace=>Vec::new(),_=>copy(&selection.selected)? };
    match action {
        Action::Replace=>{selected.try_reserve_exact(1).map_err(|_|LIMIT)?;selected.push(layer);},
        Action::Select=>if !selected.contains(&layer) { selected.try_reserve(1).map_err(|_|LIMIT)?;selected.push(layer); },
        Action::Deselect=>selected.retain(|item|*item!=layer),
    }
    let active=match action { Action::Replace|Action::Select=>Some(layer),Action::Deselect=>selection.active.filter(|item|*item!=layer) };
    // These independent typed relationships are retained only after bounded
    // current-membership/coherence checks; no name/index/cache identity inference.
    let requested=EditingSelection {selected,active,selected_paths:copy(&selection.selected_paths)?,mask_target:selection.mask_target};
    total(&requested)?;
    let params=json!({"selected":wire_list(&requested.selected)?,"active":requested.active.map(wire).transpose()?,
        "selectedPaths":wire_list(&requested.selected_paths)?,"maskTarget":requested.mask_target.map(wire).transpose()?});
    // Recheck the captured snapshot after preparation, immediately before the
    // common registered engine route performs its authoritative validation.
    validate(app,target)?;
    app.run(COMMAND,params)
}

pub(crate) fn properties(app:&mut PhotocraftApp,ui:&mut egui::Ui,layer:LayerId) {
    let view=match view(app,layer) {Ok(Some(view))=>view,Ok(None)=>return,Err(error)=>{ui.label(error);return;}};
    let enabled=app.ui.transform.is_none() && !app.session.has_jobs()
        && app.session.active().is_some_and(|state|state.floating.is_none() && state.coalesce.is_none() && state.revision<u64::MAX)
        && photocraft_engine::commands::find(COMMAND).is_some_and(|command|(command.enabled)(&app.session).is_ok());
    ui.separator();
    ui.label(egui::RichText::new(format!("{} · {}",tl!("Saved"),tl!("Selection"))).color(Tokens::get(ui.ctx()).text));
    ui.horizontal_wrapped(|ui|{ui.label(tl!("Layer"));ui.monospace(layer.0.to_string());ui.label(format!("{}: {}",tl!("Selection"),view.count));});
    let mut action=None;
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(enabled && view.replace_allowed,egui::Button::new(tl!("New selection"))).clicked() { action=Some(Action::Replace); }
        if ui.add_enabled(enabled && view.add_allowed && (!view.selected || !view.active),egui::Button::new(tl!("Select"))).clicked() { action=Some(Action::Select); }
        if ui.add_enabled(enabled && view.selected,egui::Button::new(tl!("Deselect"))).clicked() { action=Some(Action::Deselect); }
    });
    if let Some(action)=action && let Err(error)=run(app,&view.target,action) {app.ui.status=error;app.ui.status_error=true;}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use egui_kittest::{Harness,kittest::Queryable};
    use photocraft_doc::{ColorMode,Layer,LayerMask,SampleType,Size,XcfState,
        item_identity::MaskId,vector::{NamedPath,Path},xcf_state::{ItemState,MaskState}};

    fn fixture()->(PhotocraftApp,LayerId,LayerId) {
        let mut document=Document::new("Own saved selection",Size::new(2,2),ColorMode::Rgb,SampleType::U8);
        let mut first=Layer::raster("Own first",document.pixel_format());
        first.mask=Some(LayerMask::reveal_all());
        let mut second=Layer::raster("Own second",document.pixel_format());
        second.surface_mut().unwrap().try_write_pixel_f64(0,0,&[0.25,0.5,0.75,1.0]).unwrap();
        let first_id=first.id;let second_id=second.id;document.layers=vec![first,second];
        let path=NamedPath::new("Own saved path",Path::default());let path_id=path.id;document.paths.push(path);
        let mut xcf=XcfState::default();xcf.reconcile_layers(&document).unwrap();
        let mask=ItemRef::Mask(MaskId(9007199254740993));
        xcf.items.push(ItemState {item:mask,name:"Own mask".into(),custom_name:true,visible:true,locks:Default::default(),parent:None,
            source_tattoo:Some(23),source_flags:0x12345678,raster_bounds:Some(photocraft_geom::Rect::new(0,0,2,2)),composition:None,
            mask:Some(MaskState {owner:ItemRef::Layer(first_id),bounds:photocraft_geom::Rect::new(0,0,2,2),offset:[0,0],outside_f64_bits:1.0_f64.to_bits(),enabled:true,shown:false,editing:false,linked:true})});
        xcf.selection=EditingSelection {selected:vec![ItemRef::Layer(first_id)],active:Some(ItemRef::Layer(first_id)),selected_paths:vec![ItemRef::Path(path_id)],mask_target:Some(mask)};
        for item in &mut xcf.items {item.source_flags=0x12345678;}
        document.xcf_state=Some(xcf);document.validate_xcf_state().unwrap();
        let mut session=photocraft_engine::Session::new();session.open_document(document,None).unwrap();session.select_layer(second_id).unwrap();
        session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(),"Own synthetic saved state.xcf",None);
        (PhotocraftApp::new(session,Services::default()),first_id,second_id)
    }
    fn assert_independent(app:&PhotocraftApp,before:&Document,focus:LayerId,source:&Option<photocraft_engine::SourceIdentity>,view:&photocraft_engine::channel_cmds::ChannelView) {
        let state=app.session.active().unwrap();let original=before.xcf_state.as_ref().unwrap();let current=state.doc.xcf_state.as_ref().unwrap();
        assert_eq!(current.selection.selected_paths,original.selection.selected_paths);assert_eq!(current.selection.mask_target,original.selection.mask_target);
        let mut expected=before.clone();expected.xcf_state.as_mut().unwrap().selection=current.selection.clone();assert_eq!(*state.doc,expected);
        assert_eq!(state.active_layer,Some(focus));assert_eq!(state.channel_view,*view);assert_eq!(&state.source,source);
        assert_eq!(app.ui.selected_path,None);
        let bytes=photocraft_format::save_to_bytes(&state.doc,&Default::default()).unwrap();assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(),expected);
    }

    #[test]
    fn actual_helpers_add_deselect_replace_keep_independent_saved_relationships_and_history() {
        let (mut app,first,second)=fixture();let before=app.session.active().unwrap().doc.clone();
        let source=app.session.active().unwrap().source.clone();let channel=app.session.active().unwrap().channel_view.clone();
        let target=view(&app,second).unwrap().unwrap().target;run(&mut app,&target,Action::Select).unwrap();
        let added=app.session.active().unwrap().doc.clone();
        assert_eq!(added.xcf_state.as_ref().unwrap().selection.selected,vec![ItemRef::Layer(first),ItemRef::Layer(second)]);
        assert_eq!(added.xcf_state.as_ref().unwrap().selection.active,Some(ItemRef::Layer(second)));
        assert_independent(&app,&before,second,&source,&channel);
        assert!(app.session.undo());assert_eq!(app.session.active().unwrap().doc,before);
        assert!(app.session.redo());assert_eq!(app.session.active().unwrap().doc,added);
        let target=view(&app,second).unwrap().unwrap().target;run(&mut app,&target,Action::Deselect).unwrap();
        assert_eq!(app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.selected,vec![ItemRef::Layer(first)]);
        assert_eq!(app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.active,None);
        assert_independent(&app,&before,second,&source,&channel);
        let target=view(&app,second).unwrap().unwrap().target;run(&mut app,&target,Action::Replace).unwrap();
        assert_eq!(app.session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.selected,vec![ItemRef::Layer(second)]);
        assert_independent(&app,&before,second,&source,&channel);
    }

    #[test]
    fn actual_widget_buttons_dispatch_the_registered_semantic_transaction() {
        let (app,_,second)=fixture();let before=app.session.active().unwrap().doc.clone();
        let mut h=Harness::builder().with_size(egui::vec2(480.,220.)).build_ui_state(move|ui,app|properties(app,ui,second),app);
        h.run();h.get_by_label("Select").click();h.run();assert!(!h.state().ui.status_error);
        assert_eq!(h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.active,Some(ItemRef::Layer(second)));
        h.get_by_label("Deselect").click();h.run();assert!(!h.state().ui.status_error);
        assert_eq!(h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.active,None);
        h.get_by_label("New selection").click();h.run();assert!(!h.state().ui.status_error);
        assert_eq!(h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap().selection.selected,vec![ItemRef::Layer(second)]);
        let original=before.xcf_state.as_ref().unwrap();let current=h.state().session.active().unwrap().doc.xcf_state.as_ref().unwrap();
        assert_eq!(current.selection.selected_paths,original.selection.selected_paths);assert_eq!(current.selection.mask_target,original.selection.mask_target);
        assert_eq!(h.state().session.active().unwrap().active_layer,Some(second));
    }

    #[test]
    fn stale_revision_or_changed_weak_snapshot_cannot_apply_to_new_current_state() {
        let (mut app,_,second)=fixture();let target=view(&app,second).unwrap().unwrap().target;
        app.session.edit("Own unrelated change",|document,_|{document.name="changed".into();Ok(())}).unwrap();
        let changed=app.session.active().unwrap().doc.clone();assert!(run(&mut app,&target,Action::Replace).is_err());
        app.session.active_mut().unwrap().revision=target.revision;
        assert!(run(&mut app,&target,Action::Replace).is_err());assert_eq!(app.session.active().unwrap().doc,changed);
        let (other,_,_)=fixture();app.session.open_document((*other.session.active().unwrap().doc).clone(),None).unwrap();
        let other=app.session.active().unwrap().doc.clone();assert!(run(&mut app,&target,Action::Select).is_err());assert_eq!(app.session.active().unwrap().doc,other);
    }

    #[test]
    fn changed_runtime_current_layer_is_not_an_ambient_retarget_or_stale_apply() {
        let (mut app,first,second)=fixture();let target=view(&app,second).unwrap().unwrap().target;
        let before=app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().active_layer=Some(first);
        assert!(run(&mut app,&target,Action::Replace).is_err());assert_eq!(app.session.active().unwrap().doc,before);
    }

    #[test]
    fn actual_and_stale_pixel_move_preview_refuse_before_drop_with_real_redo_retained() {
        let (mut app,_,second)=fixture();
        app.session.execute("select.all",json!({})).unwrap();
        app.session.edit("Own real redo",|document,_|{document.name="redo name".into();Ok(())}).unwrap();assert!(app.session.undo());
        let target=view(&app,second).unwrap().unwrap().target;
        app.session.execute("select.float",json!({"dx":1,"dy":0,"copy":true})).unwrap();
        for stale in [false,true] {
            if stale {app.session.active_mut().unwrap().floating.as_mut().unwrap().revision=u64::MAX;}
            let state=app.session.active().unwrap();let before=state.doc.clone();let revision=state.revision;let preview=state.floating.clone().unwrap();
            let redo=state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(run(&mut app,&target,Action::Replace).unwrap_err(),PREVIEW);
            let state=app.session.active().unwrap();assert_eq!(state.doc,before);assert_eq!(state.revision,revision);
            assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(),redo);
            let retained=state.floating.as_ref().unwrap();assert_eq!((retained.layer,retained.offset,retained.revision),(preview.layer,preview.offset,preview.revision));
            assert!(std::sync::Arc::ptr_eq(&retained.parts,&preview.parts));
        }
    }
}
