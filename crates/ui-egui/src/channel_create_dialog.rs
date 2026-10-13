//! Origin-fenced creation forms for existing channel commands, not a new pixel policy.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map,Value,json};
const ALPHA:&str="channel.new";
const SPOT:&str="channel.newSpot";
const ALPHA_SPEC:&str=r#"{"name":text,"fill":"black|white|selection"="black","color":text,"opacity":0..100=50,"indicates":"masked|selected"="masked"}"#;
const SPOT_SPEC:&str=r#"{"name":text,"color":text,"solidity":0..100=0,"fromSelection":bool=true}"#;
#[derive(Clone,Debug)]
pub(crate) struct ChannelCreateOrigin {dialog:u64,command:String,document:photocraft_doc::DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for ChannelCreateOrigin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.command==other.command && self.document==other.document && self.revision==other.revision
            && match (self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
fn supported(command:&str)->bool {matches!(command,ALPHA|SPOT)}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or(tl!("Open a document before creating a channel."))?;
    if active.doc.id.0==0 || app.ui.transform.is_some() || app.session.has_jobs() || active.coalesce.is_some() || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before creating a channel.").into());
    }
    Ok(())
}
pub(crate) fn open(app:&mut PhotocraftApp,command:&str)->Option<u64> {
    let prepared=(|| {
        if !supported(command) {return Err(tl!("This channel action is unavailable.").into());}
        idle(app)?;
        let spec=photocraft_engine::commands::find(command).ok_or(tl!("This channel action is unavailable."))?;
        let active=app.session.active().ok_or(tl!("Open a document before creating a channel."))?;
        Ok::<_,String>((ChannelCreateOrigin{dialog:0,command:command.into(),document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade()},spec.label))
    })();
    let (mut target,label)=match prepared {Ok(value)=>value,Err(error)=>{app.ui.status=error;app.ui.status_error=true;return None;}};
    let mut fixed=Map::new();fixed.insert("__channelCreate".into(),json!(true));fixed.insert("name".into(),json!(""));fixed.insert("color".into(),json!("#ff0000"));
    let id=crate::filter_dialog::open_with_spec(app,command,label,if command==ALPHA {ALPHA_SPEC}else {SPOT_SPEC},fixed);
    if let Some(dialog)=app.ui.dialog_mut(id) {dialog.fields.remove("__preview");}
    target.dialog=id;app.ui.channel_create_origin=Some(target);Some(id)
}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {
    fields.get("__command").and_then(Value::as_str).is_some_and(supported) || fields.get("__channelCreate").and_then(Value::as_bool)==Some(true)
}
pub(crate) fn body(ui:&mut egui::Ui,fields:&mut Map<String,Value>) {crate::filter_dialog::body(ui,fields);}
fn invalid()->String {tl!("Check the channel settings and try again.").into()}
pub(crate) fn confirm(app:&mut PhotocraftApp,id:u64,fields:&Map<String,Value>)->Result<Value,String> {
    let target=app.ui.channel_create_origin.take().ok_or(tl!("The document changed. Reopen the channel dialog."))?;idle(app)?;
    let active=app.session.active().ok_or(tl!("The document changed. Reopen the channel dialog."))?;
    let snapshot=target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen the channel dialog."))?;
    if target.dialog!=id || fields.get("__command").and_then(Value::as_str)!=Some(target.command.as_str()) || !supported(&target.command)
        || active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot) {
        return Err(tl!("The document changed. Reopen the channel dialog.").into());
    }
    let allowed=|key:&str|matches!(key,"name"|"color"|"documentId"|"expectedRevision") || if target.command==ALPHA {matches!(key,"fill"|"opacity"|"indicates")}else {matches!(key,"solidity"|"fromSelection")};
    if fields.len()>16 || fields.keys().any(|key|!key.starts_with("__") && !allowed(key)) {return Err(invalid());}
    let name=fields.get("name").and_then(Value::as_str).filter(|name|name.len()<=2048 && !name.contains('\0')).ok_or_else(invalid)?;
    let color=fields.get("color").ok_or_else(invalid)?;
    let bounded=match color {
        Value::String(text)=>text.len()<=9 && !text.contains('\0'),
        Value::Array(values)=>(3..=4).contains(&values.len()) && values.iter().all(|value|value.as_f64().is_some_and(|value|value.is_finite() && (value as f32).is_finite())),_=>false,
    };
    if !bounded {return Err(invalid());}
    let mut params=Map::new();params.insert("name".into(),json!(name));params.insert("color".into(),color.clone());
    let percentage=if target.command==ALPHA {"opacity"}else {"solidity"};
    let amount=fields.get(percentage).and_then(Value::as_f64).filter(|number|number.is_finite()).ok_or_else(invalid)?;
    params.insert(percentage.into(),json!(amount));
    if target.command==ALPHA {
        let fill=fields.get("fill").and_then(Value::as_str).filter(|fill|matches!(*fill,"black"|"white"|"selection")).ok_or_else(invalid)?;
        let indicates=fields.get("indicates").and_then(Value::as_str).filter(|indicates|matches!(*indicates,"masked"|"selected")).ok_or_else(invalid)?;
        params.insert("fill".into(),json!(fill));params.insert("indicates".into(),json!(indicates));
    }else {params.insert("fromSelection".into(),json!(fields.get("fromSelection").and_then(Value::as_bool).ok_or_else(invalid)?));}
    params.insert("documentId".into(),json!(target.document.0.to_string()));params.insert("expectedRevision".into(),json!(target.revision.to_string()));
    app.run(&target.command,Value::Object(params))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn app()->PhotocraftApp {let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());app.session.execute("file.new",json!({"width":3,"height":2})).unwrap();app}
    #[test]
    fn both_creation_forms_refuse_stale_tabs_revisions_and_restored_fields() {
        for command in [ALPHA,SPOT] {for change in 0..3 {
            let mut app=app();let id=open(&mut app,command).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {0=>{app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();},1=>{app.session.execute("image.canvasSize",json!({"width":4,"height":2})).unwrap();},_=>{app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.channel_create_origin.is_none());}}
            let before=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;assert!(confirm(&mut app,id,&fields).is_err());assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().revision,revision);
        }}
    }
    #[test]
    fn original_defaults_are_explicit_without_preview_or_target_widgets() {
        for command in [ALPHA,SPOT] {
            let mut app=app();let id=open(&mut app,command).unwrap();let fields=&app.ui.dialog_mut(id).unwrap().fields;
            assert_eq!(fields["name"],"");assert_eq!(fields["color"],"#ff0000");assert!(!fields.contains_key("__preview"));assert!(!fields.contains_key("documentId"));assert!(!fields.contains_key("expectedRevision"));
            if command==ALPHA {assert_eq!(fields["fill"],"black");assert_eq!(fields["opacity"],50.);assert_eq!(fields["indicates"],"masked");}
            else {assert_eq!(fields["solidity"],0.);assert_eq!(fields["fromSelection"],true);}
        }
    }
    #[test]
    fn high_id_confirm_uses_captured_pair_and_malformed_values_leave_source() {
        let mut app=app();app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().id=photocraft_doc::DocId(9_007_199_254_740_995);
        let id=open(&mut app,ALPHA).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();fields.insert("documentId".into(),json!("wrong-selector"));fields.insert("expectedRevision".into(),json!("wrong-selector"));
        confirm(&mut app,id,&fields).unwrap();assert_eq!(app.session.active().unwrap().doc.channels.len(),1);assert_eq!(app.session.active().unwrap().doc.id.0,9_007_199_254_740_995);
        let id=open(&mut app,SPOT).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();fields.insert("color".into(),json!([0.,1.]));let before=app.session.active().unwrap().doc.clone();assert!(confirm(&mut app,id,&fields).is_err());assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
