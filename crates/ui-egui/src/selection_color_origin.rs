//! Captured generated Focus Area/Sky dialogs; fields never grant source authority.
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelView};
use serde_json::{Map,Value};
#[derive(Clone,Debug)]
pub(crate) struct Origin {
    dialog:u64,command:String,document:DocId,revision:u64,snapshot:WeakDocumentSnapshot,
    layer:Option<LayerId>,view:ChannelView,shell_mask:bool,
}
impl PartialEq for Origin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.command==other.command && self.document==other.document && self.revision==other.revision
            && self.layer==other.layer && self.view==other.view && self.shell_mask==other.shell_mask
            && match(self.snapshot.upgrade(),other.snapshot.upgrade()){(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
pub(crate) fn handles(command:&str)->bool {matches!(command,"select.focusArea"|"select.sky")}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {fields.get("__command").and_then(Value::as_str).is_some_and(handles)}
fn stale()->String {tl!("The selection target changed. Reopen the dialog.").into()}
fn invalid()->String {tl!("Check the selection settings and try again.").into()}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(||tl!("Open a document before selecting image colors.").to_owned())?;
    if active.doc.id.0==0 || active.revision==u64::MAX || app.ui.transform.is_some() || app.session.has_jobs()
        || active.floating.is_some() || active.coalesce.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before creating this selection.").into());
    }
    Ok(())
}
pub(crate) fn clear(app:&mut PhotocraftApp) {app.ui.selection_color_origin=None;}
pub(crate) fn capture_open(app:&mut PhotocraftApp,dialog:u64,command:&str) {
    if !handles(command) {return;}
    clear(app);
    let capture=(|| {
        idle(app)?;let active=app.session.active().ok_or_else(stale)?;
        if active.channel_view.color_hidden.len()>16 || active.channel_view.alpha_visible.len()>10_000 {return Err(invalid());}
        Ok::<_,String>(Origin {dialog,command:command.to_owned(),document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade(),
            layer:active.active_layer,view:active.channel_view.clone(),shell_mask:app.layer_mask_targeted()})
    })();
    match capture {Ok(origin)=>app.ui.selection_color_origin=Some(origin),Err(error)=>{app.ui.status=error;app.ui.status_error=true;}}
}
fn decimal(value:&Value,nonzero:bool)->bool {
    value.as_str().is_some_and(|text|!text.is_empty() && text.len()<=20 && (text.len()==1 || !text.starts_with('0')) && text.bytes().all(|byte|byte.is_ascii_digit())
        && text.parse::<u64>().is_ok_and(|number|!nonzero || number!=0))
}
/// Generated forms may carry bounded schema/view metadata; it is never a target witness.
fn metadata(value:&Value,depth:usize,nodes:&mut usize,bytes:&mut usize)->bool {
    if depth>8 {return false;}
    let Some(count)=nodes.checked_add(1).filter(|count|*count<=1024) else {return false;};*nodes=count;
    match value {
        Value::String(text)=>{let Some(total)=bytes.checked_add(text.len()).filter(|total|*total<=65536) else {return false;};*bytes=total;text.len()<=8192},
        Value::Array(values)=>values.len()<=1024 && values.iter().all(|value|metadata(value,depth+1,nodes,bytes)),
        Value::Object(values)=>values.len()<=128 && values.iter().all(|(key,value)|{
            if key.len()>128 {return false;}let Some(total)=bytes.checked_add(key.len()).filter(|total|*total<=65536) else {return false;};*bytes=total;
            metadata(value,depth+1,nodes,bytes)
        }),
        Value::Number(number)=>number.as_f64().is_some_and(f64::is_finite),
        Value::Bool(_)|Value::Null=>true,
    }
}
pub(crate) fn fields(fields:&Map<String,Value>)->Result<(),String> {
    let command=fields.get("__command").and_then(Value::as_str).ok_or_else(invalid)?;
    if !handles(command) {return Ok(());}
    if fields.len()>32 {return Err(invalid());}
    let mut nodes=0usize;let mut bytes=0usize;
    for (key,value) in fields {
        if key.len()>128 {return Err(invalid());}
        let valid=match key.as_str() {
            "__command"=>value.as_str()==Some(command),
            "__smartFilter"=>false,
            "__filter"|"__preview"=>value.is_boolean(),
            "__label"=>value.as_str().is_some_and(|text|text.len()<=4096),
            "sampleAllLayers"=>value.is_boolean(),
            "mode"=>value.as_str().is_some_and(|text|text.len()<=32 && !text.contains('\0')),
            "range"|"noise" if command=="select.focusArea"=>value.as_f64().is_some_and(|number|number.is_finite() && (number as f32).is_finite()),
            "threshold"|"softness" if command=="select.sky"=>value.as_f64().is_some_and(|number|number.is_finite() && (number as f32).is_finite()),
            "documentId"=>decimal(value,true),"expectedRevision"=>decimal(value,false),
            key if key.starts_with("__")=>metadata(value,0,&mut nodes,&mut bytes),
            _=>false,
        };
        if !valid {return Err(invalid());}
    }
    if fields.contains_key("documentId")!=fields.contains_key("expectedRevision") {return Err(invalid());}Ok(())
}
pub(crate) fn validate(app:&PhotocraftApp,dialog:u64,fields_value:&Map<String,Value>)->Result<(),String> {
    fields(fields_value)?;
    let command=fields_value.get("__command").and_then(Value::as_str).ok_or_else(invalid)?;
    if !handles(command) {return Ok(());}
    idle(app)?;let origin=app.ui.selection_color_origin.as_ref().ok_or_else(stale)?;
    let active=app.session.active().ok_or_else(stale)?;
    if origin.dialog!=dialog || origin.command!=command || active.doc.id!=origin.document || active.revision!=origin.revision
        || !origin.snapshot.is_current(&active.doc) || active.active_layer!=origin.layer || active.channel_view!=origin.view || app.layer_mask_targeted()!=origin.shell_mask {return Err(stale());}Ok(())
}
pub(crate) fn prepare(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,mut params:Value)->Result<Value,String> {
    validate(app,dialog,fields)?;
    if !fields.get("__command").and_then(Value::as_str).is_some_and(handles) {return Ok(params);}
    let origin=app.ui.selection_color_origin.as_ref().ok_or_else(stale)?;
    let object=params.as_object_mut().filter(|object|object.len()<=8).ok_or_else(invalid)?;
    object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));Ok(params)
}
#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn app()->PhotocraftApp {let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();app}
    fn settings(command:&str)->Map<String,Value> {
        let value=if command=="select.sky" {json!({"__command":command,"threshold":50,"softness":50,"sampleAllLayers":true,"mode":"replace"})}else {json!({"__command":command,"range":0.5,"noise":0,"sampleAllLayers":true,"mode":"replace"})};value.as_object().unwrap().clone()
    }
    #[test]
    fn options_retry_preserves_origin_but_stale_restored_views_refuse() {
        for command in ["select.focusArea","select.sky"] {
            let mut app=app();capture_open(&mut app,7,command);let mut fields=settings(command);
            fields.insert("sampleAllLayers".into(),json!(false));fields.insert("mode".into(),json!("add"));assert!(validate(&app,7,&fields).is_ok());
            let params=crate::filter_dialog::params_of(&fields);let request=prepare(&app,7,&fields,params).unwrap();
            assert_eq!(request["documentId"],app.session.active().unwrap().doc.id.0.to_string());
            app.session.active_mut().unwrap().channel_view.target=photocraft_engine::channel_cmds::ChannelTarget::Color(0);assert!(validate(&app,7,&fields).is_err());
            app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.selection_color_origin.is_none());assert!(validate(&app,7,&fields).is_err());
        }
    }
    #[test]
    fn malformed_params_and_nested_metadata_refuse_without_target_mutation() {
        let mut app=app();capture_open(&mut app,8,"select.focusArea");let original=settings("select.focusArea");
        let before=app.session.active().unwrap().doc.clone();
        let mut malformed=original.clone();malformed.insert("range".into(),json!([1,2]));assert!(validate(&app,8,&malformed).is_err());
        let mut bounded=original.clone();bounded.insert("__spec".into(),json!("{range:0..1=0.5}"));assert!(validate(&app,8,&bounded).is_ok());
        bounded.insert("__extra".into(),json!(vec![0;1025]));assert!(fields(&bounded).is_err());
        app.session.active_mut().unwrap().revision+=1;assert!(prepare(&app,8,&original,json!({})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        clear(&mut app);assert!(app.ui.selection_color_origin.is_none());
    }
}
