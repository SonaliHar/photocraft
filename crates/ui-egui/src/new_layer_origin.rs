//! Runtime-only insertion target for the existing New Layer dialog.
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelTarget,mask_view_cmds::LayerMaskView};
use serde_json::{Map,Value};
#[derive(Clone,Debug)]
pub(crate) struct Origin {pub(crate) dialog:u64,document:DocId,revision:u64,snapshot:WeakDocumentSnapshot,active:Option<LayerId>,selected:Vec<LayerId>,channel:ChannelTarget,editing_mask:Option<LayerId>,shown_mask:Option<LayerMaskView>,shell_mask:bool}
impl PartialEq for Origin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.document==other.document && self.revision==other.revision && self.active==other.active && self.selected==other.selected
            && self.channel==other.channel && self.editing_mask==other.editing_mask && self.shown_mask==other.shown_mask && self.shell_mask==other.shell_mask
            && match(self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
fn stale()->String {tl!("The layer target changed. Reopen New Layer.").into()}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let state=app.session.active().ok_or_else(||tl!("Open a document before creating a layer.").to_owned())?;
    if app.ui.transform.is_some() || app.session.has_jobs() || state.floating.is_some() || state.coalesce.is_some() {
        return Err(tl!("Finish pending edits before creating a layer.").into());
    }
    Ok(())
}
pub(crate) fn capture(app:&PhotocraftApp)->Result<Origin,String> {
    idle(app)?;
    let state=app.session.active().ok_or_else(stale)?;
    if state.doc.id.0==0 || state.revision==u64::MAX || state.selected_layers.len()>10_000 {return Err(stale());}
    let selected=state.selected_layers().map_err(|error|error.to_string())?;
    Ok(Origin {dialog:0,document:state.doc.id,revision:state.revision,snapshot:state.doc.downgrade(),active:state.active_layer,selected,channel:state.channel_view.target,editing_mask:state.channel_view.editing_mask,shown_mask:state.channel_view.layer_mask,shell_mask:app.layer_mask_targeted()})
}
pub(crate) fn validate(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>)->Result<(),String> {
    idle(app)?;
    let origin=app.ui.new_layer_origin.as_ref().filter(|origin|origin.dialog==dialog).ok_or_else(stale)?;
    let state=app.session.active().ok_or_else(stale)?;
    let snapshot=origin.snapshot.upgrade().ok_or_else(stale)?;
    if state.doc.id!=origin.document || state.revision!=origin.revision || !state.doc.shares_state(&snapshot) || state.active_layer!=origin.active
        || state.channel_view.target!=origin.channel || state.channel_view.editing_mask!=origin.editing_mask || state.channel_view.layer_mask!=origin.shown_mask || app.layer_mask_targeted()!=origin.shell_mask {return Err(stale());}
    if state.selected_layers.len()>10_000 || state.selected_layers().map_err(|error|error.to_string())?!=origin.selected {return Err(stale());}
    let invalid=||tl!("Check the new layer settings and try again.").to_owned();
    if fields.len()>16 || fields.get("__command").and_then(Value::as_str)!=Some(crate::new_layer_ui::COMMAND)
        || fields.keys().any(|key|key.len()>128 || (!key.starts_with("__") && !matches!(key.as_str(),"name"|"clipToPrevious"|"color"|"mode"|"opacity"|"fillNeutral"))) {return Err(invalid());}
    for key in ["name","color","mode"] {if fields.get(key).is_some_and(|value|value.as_str().is_none_or(|text|text.len()>4096 || text.contains('\0'))) {return Err(invalid());}}
    for key in ["clipToPrevious","fillNeutral"] {if fields.get(key).is_some_and(|value|!value.is_boolean()) {return Err(invalid());}}
    if fields.get("opacity").is_some_and(|value|value.as_f64().is_none_or(|n|!n.is_finite())) {return Err(invalid());}
    Ok(())
}

/// Call after borrowed-field validation and parameter construction, immediately before dispatch.
pub(crate) fn prepare(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,mut params:Value)->Result<Value,String> {
    validate(app,dialog,fields)?;
    let origin=app.ui.new_layer_origin.as_ref().ok_or_else(stale)?;
    let object=params.as_object_mut().filter(|object|object.len()<=7).ok_or_else(||tl!("Check the new layer settings and try again.").to_owned())?;
    object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));
    Ok(params)
}
