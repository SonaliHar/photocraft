//! Runtime witnesses for the existing Duplicate Layer destination form.
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelTarget,mask_view_cmds::LayerMaskView};
use serde_json::{Map,Value};
#[derive(Clone,Debug)]
struct Document {id:DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for Document {
    fn eq(&self,other:&Self)->bool {self.id==other.id && self.revision==other.revision && match(self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}}
}
#[derive(Clone,Debug,PartialEq)]
pub(crate) struct Origin {pub(crate) dialog:u64,source:Document,selected:Vec<LayerId>,layer:Option<LayerId>,channel:ChannelTarget,editing_mask:Option<LayerId>,shown_mask:Option<LayerMaskView>,shell_mask:bool,destinations:Vec<Document>}
impl Origin {pub(crate) fn selected(&self)->&[LayerId] {&self.selected}}
fn stale()->String {tl!("The document or selected layers changed. Choose Duplicate Layer again.").into()}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let state=app.session.active().ok_or_else(stale)?;
    if app.ui.transform.is_some() || app.session.has_jobs() || state.floating.is_some() || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before duplicating layers.").into());
    }
    Ok(())
}
pub(crate) fn capture(app:&PhotocraftApp)->Result<Origin,String> {
    idle(app)?;
    let state=app.session.active().ok_or_else(stale)?;
    if state.selected_layers.len()>10_000 || app.session.documents().len()>4096 || state.revision==u64::MAX {return Err(tl!("Too many targets for this duplicate operation.").into());}
    let selected=state.selected_layers().map_err(|error|error.to_string())?;
    let mut destinations=Vec::new();destinations.try_reserve_exact(app.session.documents().len()).map_err(|_|tl!("Cannot prepare the duplicate operation.").to_owned())?;
    for destination in app.session.documents() {destinations.push(Document {id:destination.doc.id,revision:destination.revision,snapshot:destination.doc.downgrade()});}
    Ok(Origin {dialog:0,source:Document {id:state.doc.id,revision:state.revision,snapshot:state.doc.downgrade()},selected,layer:state.active_layer,channel:state.channel_view.target,editing_mask:state.channel_view.editing_mask,shown_mask:state.channel_view.layer_mask,shell_mask:app.layer_mask_targeted(),destinations})
}
pub(crate) fn validate(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>)->Result<(),String> {
    idle(app)?;
    let origin=app.ui.duplicate_layer_origin.as_ref().filter(|origin|origin.dialog==dialog).ok_or_else(stale)?;
    let state=app.session.active().ok_or_else(stale)?;
    let snapshot=origin.source.snapshot.upgrade().ok_or_else(stale)?;
    if state.doc.id!=origin.source.id || state.revision!=origin.source.revision || !state.doc.shares_state(&snapshot)
        || state.active_layer!=origin.layer || state.channel_view.target!=origin.channel || state.channel_view.editing_mask!=origin.editing_mask || state.channel_view.layer_mask!=origin.shown_mask || app.layer_mask_targeted()!=origin.shell_mask {return Err(stale());}
    if state.selected_layers.len()>10_000 {return Err(stale());}
    let selected=state.selected_layers().map_err(|error|error.to_string())?;
    if selected!=origin.selected {return Err(stale());}
    if fields.len()>16 || fields.get("document").and_then(Value::as_u64)!=Some(origin.source.id.0)
        || fields.get("selected").and_then(Value::as_array).is_none_or(|values|values.len()!=origin.selected.len() || values.iter().zip(&origin.selected).any(|(value,id)|value.as_u64()!=Some(id.0))) {return Err(stale());}
    for key in ["name","documentName"] {if fields.get(key).is_some_and(|value|value.as_str().is_none_or(|name|name.len()>4096 || name.contains('\0'))) {return Err(tl!("Enter a shorter valid name for the duplicate.").into());}}
    if fields.get("destination").and_then(Value::as_str)==Some("new") {return Ok(());}
    let index=fields.get("destination").and_then(Value::as_u64).and_then(|index|usize::try_from(index).ok()).ok_or_else(stale)?;
    let captured=origin.destinations.get(index).ok_or_else(stale)?;
    let destination=app.session.documents().get(index).ok_or_else(stale)?;
    let snapshot=captured.snapshot.upgrade().ok_or_else(stale)?;
    if destination.doc.id!=captured.id || destination.revision!=captured.revision || !destination.doc.shares_state(&snapshot) {return Err(tl!("The destination changed. Choose Duplicate Layer again.").into());}
    if destination.floating.is_some() || destination.coalesce.is_some() || destination.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {return Err(tl!("Finish pending edits in the destination before copying layers.").into());}
    Ok(())
}
pub(crate) fn source_pair(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,params:&mut Map<String,Value>)->Result<(),String> {
    validate(app,dialog,fields)?;
    let origin=app.ui.duplicate_layer_origin.as_ref().ok_or_else(stale)?;
    params.insert("documentId".into(),Value::String(origin.source.id.0.to_string()));
    params.insert("expectedRevision".into(),Value::String(origin.source.revision.to_string()));
    Ok(())
}
