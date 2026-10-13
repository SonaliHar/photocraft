//! Captured-origin channel forms. Presentation labels never become channel IDs.
use crate::{PhotocraftApp,state::DialogKind};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map,Value,json};
const DUPLICATE:&str="channel.duplicate";
const LOAD:&str="select.loadSelection";
const SAVE:&str="select.saveSelection";
#[derive(Clone,Debug)]
struct Target {document:photocraft_doc::DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for Target {
    fn eq(&self,b:&Self)->bool {self.document==b.document && self.revision==b.revision && match(self.snapshot.upgrade(),b.snapshot.upgrade()){(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}}
}
#[derive(Clone,Debug,PartialEq)]
struct Choice {key:String,label:String,selector:Value}
#[derive(Clone,Debug,PartialEq)]
pub(crate) struct ChannelActionOrigin {dialog:u64,command:String,source:Target,choices:Vec<Choice>,fixed:Option<(String,Value)>,layer:Option<Value>,destinations:Vec<Target>}
fn supported(command:&str)->bool {matches!(command,DUPLICATE|LOAD|SAVE)}
fn stale()->String {tl!("The document changed. Reopen the channel dialog.").into()}
fn invalid()->String {tl!("Check the channel settings and try again.").into()}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or(tl!("Open a document before using this channel action."))?;
    if active.doc.id.0==0 || app.ui.transform.is_some() || app.session.has_jobs() || active.coalesce.is_some() || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {return Err(tl!("Finish pending edits before using this channel action.").into());}Ok(())
}
fn selector(value:&Value)->bool {
    match value {
        Value::String(text)=>!text.is_empty() && text.len()<=4096 && !text.contains('\0'),
        Value::Number(_)=>value.as_u64().is_some(),
        Value::Object(object)=>!object.is_empty() && object.len()<=2 && object.iter().all(|(key,value)|match key.as_str(){"channelId"=>identity(value),"alpha"|"color"=>value.as_u64().is_some(),_=>false}),_=>false,
    }
}
fn identity(value:&Value)->bool {
    if value.as_u64().is_some_and(|id|id!=0) {return true;}
    value.as_str().is_some_and(|s|!s.is_empty() && s.len()<=20 && !s.starts_with('0') && s.bytes().all(|b|b.is_ascii_digit()) && s.parse::<u64>().is_ok())
}
fn bounded_label(text:&str,total:&mut usize)->Result<(),String> {
    if text.len()>4096 || text.contains('\0') {return Err(invalid());}
    *total=total.checked_add(text.len()).filter(|n|*n<=4<<20).ok_or_else(invalid)?;Ok(())
}
/// `initial` preserves an explicit context-menu channel/channelId and optional
/// layer; no source locator, nativeCoverage policy or resource read is introduced.
pub(crate) fn open(app:&mut PhotocraftApp,command:&str,initial:&Value)->Option<u64> {
    let prepared=(|| {
        idle(app)?;if !supported(command) {return Err(invalid());}
        let initial=if initial.is_null() {None}else {Some(initial.as_object().ok_or_else(invalid)?)};
        if initial.is_some_and(|p|p.len()>7 || p.keys().any(|k|!matches!(k.as_str(),"channel"|"channelId"|"layer"|"name"|"invert"|"operation"|"document"))) {return Err(invalid());}
        if initial.is_some_and(|p|if command==DUPLICATE {p.contains_key("layer") || p.contains_key("operation")}else if command==LOAD {p.contains_key("name")}else {p.contains_key("layer") || p.contains_key("invert")}) {return Err(invalid());}
        let active=app.session.active().ok_or_else(stale)?;
        let doc=&active.doc;let count=doc.channels.len();
        if count>10000 || app.session.documents().len()>4096 {return Err(invalid());}
        let mut total=0usize;let mut choices=Vec::new();choices.try_reserve_exact(count.checked_add(12).ok_or_else(invalid)?).map_err(|_|invalid())?;
        let mode=doc.pixel_format().mode;
        let composite=photocraft_engine::channel_cmds::composite_name(mode);
        let mut add=|key:String,label:&str,selector:Value|->Result<(),String>{bounded_label(label,&mut total)?;choices.push(Choice{key,label:label.into(),selector});Ok(())};
        if command==SAVE {add("new".into(),"new",json!("new"))?;}else {
            add("composite".into(),composite,json!("composite"))?;
            if mode.color_channels()>1 {for name in photocraft_engine::channel_cmds::color_names(mode) {add((*name).into(),name,json!(name.to_lowercase()))?;}}
        }
        for channel in &doc.channels {if channel.id.0==0 {return Err(invalid());}add(format!("alpha:{}",channel.id.0),&channel.name,json!({"channelId":channel.id.0.to_string()}))?;}
        for name in if command==LOAD {&["transparency","mask","selection"][..]}else if command==DUPLICATE {&["quickMask"][..]}else {&[][..]} {add((*name).into(),name,json!(name))?;}
        let fixed=if let Some(params)=initial {
            if params.contains_key("channel") && params.contains_key("channelId") {return Err(invalid());}
            if let Some(value)=params.get("channelId") {if !identity(value) {return Err(invalid());}Some(("channelId".into(),value.clone()))}
            else if let Some(value)=params.get("channel") {if !selector(value) {return Err(invalid());}Some(("channel".into(),value.clone()))}else {None}
        }else {None};
        let layer=initial.and_then(|p|p.get("layer")).map(|value|if identity(value) {Ok(value.clone())}else {Err(invalid())}).transpose()?;
        let source=Target{document:doc.id,revision:active.revision,snapshot:doc.downgrade()};
        let selected=if command==SAVE {"new".into()}else if command==LOAD {doc.channels.first().map(|channel|format!("alpha:{}",channel.id.0)).unwrap_or_else(||"composite".into())}else {"composite".into()};
        let mut fields=Map::new();fields.insert("__command".into(),json!(command));fields.insert("__label".into(),json!(if command==LOAD {tl!("Load Selection…")}else if command==SAVE {tl!("Save Selection…")}else {tl!("Duplicate Channel…")}));fields.insert("__channelAction".into(),json!(true));
        fields.insert("channel".into(),json!(selected));if command!=SAVE {fields.insert("invert".into(),json!(initial.and_then(|p|p.get("invert")).and_then(Value::as_bool).unwrap_or(false)));}
        if initial.and_then(|p|p.get("invert")).is_some_and(|v|!v.is_boolean()) {return Err(invalid());}
        let mut display=Vec::new();display.try_reserve_exact(choices.len()).map_err(|_|invalid())?;
        for choice in &choices {display.push(json!({"key":choice.key,"label":choice.label}));}
        fields.insert("__channels".into(),Value::Array(display));fields.insert("__fixedChannel".into(),json!(fixed.is_some()));
        let mut destinations=Vec::new();
        if command==DUPLICATE {
            let name=initial.and_then(|p|p.get("name")).and_then(Value::as_str).unwrap_or("");
            if name.len()>2048 || name.contains('\0') || initial.and_then(|p|p.get("name")).is_some_and(|v|!v.is_string()) {return Err(invalid());}
            fields.insert("name".into(),json!(name));
            let mut options=Vec::new();options.try_reserve_exact(app.session.documents().len()+2).map_err(|_|invalid())?;
            options.push(json!({"key":"active","label":tl!("Current Document")}));options.push(json!({"key":"new","label":tl!("New")}));
            destinations.try_reserve_exact(app.session.documents().len()).map_err(|_|invalid())?;
            for (index,state) in app.session.documents().iter().enumerate() {bounded_label(&state.doc.name,&mut total)?;options.push(json!({"key":index.to_string(),"label":state.doc.name}));destinations.push(Target{document:state.doc.id,revision:state.revision,snapshot:state.doc.downgrade()});}
            let destination=match initial.and_then(|p|p.get("document")) {None|Some(Value::Null)=>"active".into(),Some(Value::String(s)) if s=="new"=>"new".into(),Some(value)=>value.as_u64().and_then(|n|usize::try_from(n).ok()).filter(|n|*n<destinations.len()).map(|n|n.to_string()).ok_or_else(invalid)?};
            fields.insert("document".into(),json!(destination));fields.insert("__destinations".into(),Value::Array(options));
        }else {
            // Preserve Load Selection's original active-document form choices.
            if initial.and_then(|p|p.get("document")).is_some() {return Err(invalid());}
            let operation=match initial.and_then(|p|p.get("operation")) {None=>"new",Some(value)=>value.as_str().ok_or_else(invalid)?};
            if !matches!(operation,"new"|"add"|"subtract"|"intersect") && !(command==SAVE && operation=="replace") {return Err(invalid());}
            fields.insert("operation".into(),json!(operation));
            if command==SAVE {
                let name=match initial.and_then(|p|p.get("name")) {None=>"",Some(value)=>value.as_str().ok_or_else(invalid)?};
                if name.len()>4096 || name.contains('\0') {return Err(invalid());}
                fields.insert("name".into(),json!(name));
            }
        }
        Ok::<_,String>((ChannelActionOrigin{dialog:0,command:command.into(),source,choices,fixed,layer,destinations},fields))
    })();
    let (mut origin,fields)=match prepared {Ok(value)=>value,Err(error)=>{app.ui.status=error;app.ui.status_error=true;return None;}};
    let id=app.ui.open_dialog(DialogKind::Command,fields);origin.dialog=id;app.ui.channel_action_origin=Some(origin);Some(id)
}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {fields.get("__command").and_then(Value::as_str).is_some_and(supported) || fields.get("__channelAction").and_then(Value::as_bool)==Some(true)}
fn dropdown(ui:&mut egui::Ui,fields:&mut Map<String,Value>,key:&str,presentation:&str,label:&str) {
    let mut selected=fields.get(key).and_then(Value::as_str).filter(|s|s.len()<=64).unwrap_or("").to_owned();
    let rows=fields.get(presentation).and_then(Value::as_array).filter(|v|v.len()<=10012);
    let mut options=Vec::new();let mut total=0usize;
    if let Some(rows)=rows {if options.try_reserve_exact(rows.len()).is_err() {ui.label(tl!("There is not enough memory to display these choices."));return;}
        for row in rows {if let (Some(key),Some(label))=(row.get("key").and_then(Value::as_str),row.get("label").and_then(Value::as_str)) {
            if key.len()>64 || label.len()>4096 || total.checked_add(label.len()).is_none_or(|n|n>4<<20) {ui.label(tl!("These channel choices are too large to display."));return;}
            total+=label.len();options.push((key.to_owned(),label.to_owned()));
        }}}
    let mut refs=Vec::new();if refs.try_reserve_exact(options.len()).is_err() {ui.label(tl!("There is not enough memory to display these choices."));return;}
    for (key,label) in &options {refs.push((key.clone(),label.as_str()));}
    ui.horizontal(|ui|{ui.label(label);if crate::widgets::dropdown(ui,key,&mut selected,&refs,200.) {fields.insert(key.into(),json!(selected));}});
}
pub(crate) fn body(ui:&mut egui::Ui,fields:&mut Map<String,Value>) {
    let command=fields.get("__command").and_then(Value::as_str).filter(|command|supported(command)).unwrap_or("").to_owned();
    if fields.get("__fixedChannel").and_then(Value::as_bool)!=Some(true) {dropdown(ui,fields,"channel","__channels",tl!("Channel"));}
    if command==DUPLICATE || command==SAVE {
        let limit=if command==SAVE {4096}else {2048};
        let mut name=fields.get("name").and_then(Value::as_str).filter(|name|name.len()<=limit).unwrap_or("").to_owned();
        ui.horizontal(|ui|{ui.label(tl!("Name"));if ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.).char_limit(limit)).changed() {fields.insert("name".into(),json!(name));}});
    }
    if command==DUPLICATE {dropdown(ui,fields,"document","__destinations",tl!("Document"));}
    else {
        let mut operation=fields.get("operation").and_then(Value::as_str).filter(|op|op.len()<=16).unwrap_or("new").to_owned();
        let options=if command==SAVE {vec![("new".into(),tl!("New")),("replace".into(),tl!("Replace")),("add".into(),tl!("Add")),("subtract".into(),tl!("Subtract")),("intersect".into(),tl!("Intersect"))]}
            else {vec![("new".into(),tl!("New")),("add".into(),tl!("Add")),("subtract".into(),tl!("Subtract")),("intersect".into(),tl!("Intersect"))]};
        ui.horizontal(|ui|{ui.label(tl!("Operation"));if crate::widgets::dropdown(ui,"channel-selection-operation",&mut operation,&options,200.) {fields.insert("operation".into(),json!(operation));}});
    }
    if command!=SAVE {let mut invert=fields.get("invert").and_then(Value::as_bool).unwrap_or(false);if crate::widgets::checkbox(ui,&mut invert,tl!("Invert")).changed(){fields.insert("invert".into(),json!(invert));}}
}
fn matches_target(target:&Target,state:&photocraft_engine::DocState)->bool {
    target.document==state.doc.id && target.revision==state.revision && target.snapshot.upgrade().is_some_and(|snapshot|state.doc.shares_state(&snapshot))
}
pub(crate) fn confirm(app:&mut PhotocraftApp,id:u64,fields:&Map<String,Value>)->Result<Value,String> {
    let origin=app.ui.channel_action_origin.take().ok_or_else(stale)?;idle(app)?;
    let active=app.session.active().ok_or_else(stale)?;
    if origin.dialog!=id || fields.get("__command").and_then(Value::as_str)!=Some(origin.command.as_str()) || !matches_target(&origin.source,active) {return Err(stale());}
    let allowed=|key:&str|matches!(key,"channel"|"documentId"|"expectedRevision") || match origin.command.as_str() {
        DUPLICATE=>matches!(key,"name"|"document"|"invert"),LOAD=>matches!(key,"invert"|"operation"),SAVE=>matches!(key,"name"|"operation"),_=>false,
    };
    if fields.len()>16 || fields.keys().any(|key|!key.starts_with("__") && !allowed(key)) {return Err(invalid());}
    let mut params=Map::new();
    if let Some((key,value))=&origin.fixed {params.insert(key.clone(),value.clone());}
    else {let key=fields.get("channel").and_then(Value::as_str).ok_or_else(invalid)?;let choice=origin.choices.iter().find(|choice|choice.key==key).ok_or_else(invalid)?;params.insert("channel".into(),choice.selector.clone());}
    if let Some(layer)=origin.layer {params.insert("layer".into(),layer);}
    if origin.command!=SAVE {params.insert("invert".into(),json!(fields.get("invert").and_then(Value::as_bool).ok_or_else(invalid)?));}
    if origin.command==DUPLICATE {
        let name=fields.get("name").and_then(Value::as_str).filter(|n|n.len()<=2048 && !n.contains('\0')).ok_or_else(invalid)?;
        // Empty untouched name retains the backend's default copy naming.
        if !name.is_empty() {params.insert("name".into(),json!(name));}
        let destination=fields.get("document").and_then(Value::as_str).ok_or_else(invalid)?;
        if destination=="new" {params.insert("document".into(),json!("new"));}
        else if destination!="active" {
            let index=destination.parse::<usize>().ok().filter(|i|i.to_string()==destination).ok_or_else(invalid)?;
            let target=origin.destinations.get(index).ok_or_else(invalid)?;let state=app.session.documents().get(index).ok_or_else(stale)?;
            if !matches_target(target,state) || state.floating.is_some() || state.coalesce.is_some() || state.doc.xcf_state.as_ref().is_some_and(|s|s.floating.is_some()) {return Err(stale());}
            params.insert("document".into(),json!(index));
        }
    }else {
        let operation=fields.get("operation").and_then(Value::as_str).filter(|op|matches!(*op,"new"|"add"|"subtract"|"intersect") || (origin.command==SAVE && *op=="replace")).ok_or_else(invalid)?;params.insert("operation".into(),json!(operation));
        if origin.command==SAVE {let name=fields.get("name").and_then(Value::as_str).filter(|name|name.len()<=4096 && !name.contains('\0')).ok_or_else(invalid)?;params.insert("name".into(),json!(name));}
    }
    params.insert("documentId".into(),json!(origin.source.document.0.to_string()));params.insert("expectedRevision".into(),json!(origin.source.revision.to_string()));
    app.run(&origin.command,Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app()->PhotocraftApp {let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());app.session.execute("file.new",json!({"width":3,"height":2})).unwrap();app}
    #[test]
    fn both_actions_refuse_stale_source_tab_edits_and_restored_forms() {
        for command in [DUPLICATE,LOAD,SAVE] {for change in 0..3 {
            let mut app=app();let id=open(&mut app,command,&Value::Null).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {0=>{app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();},1=>{app.session.execute("image.canvasSize",json!({"width":4,"height":2})).unwrap();},_=>{app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.channel_action_origin.is_none());}}
            let before=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;let count=app.session.documents().len();
            assert!(confirm(&mut app,id,&fields).is_err());assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().revision,revision);assert_eq!(app.session.documents().len(),count);
        }}
    }
    #[test]
    fn explicit_typed_context_and_destination_are_captured_without_name_identity() {
        let mut app=app();app.session.execute("select.saveSelection",json!({"name":"Same"})).unwrap();
        let channel=app.session.active().unwrap().doc.channels[0].id;
        let initial=json!({"channelId":channel.0.to_string(),"document":"new"});
        let id=open(&mut app,DUPLICATE,&initial).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["__fixedChannel"],true);assert_eq!(fields["document"],"new");assert!(!fields.contains_key("nativeCoverage"));
        fields.insert("channel".into(),json!("unrelated-label"));fields.insert("documentId".into(),json!("wrong-restored-target"));
        let source=app.session.active().unwrap().doc.clone();let count=app.session.documents().len();
        confirm(&mut app,id,&fields).unwrap();assert_eq!(app.session.documents().len(),count+1);assert!(app.session.documents().first().unwrap().doc.shares_state(&source));
    }
    #[test]
    fn load_defaults_and_malformed_initial_params_are_bounded() {
        let mut app=app();let id=open(&mut app,LOAD,&Value::Null).unwrap();let fields=&app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(fields["operation"],"new");assert_eq!(fields["invert"],false);assert!(!fields.contains_key("document"));assert!(!fields.contains_key("documentId"));
        assert!(open(&mut app,LOAD,&json!({"channel":"red","invert":"wrong"})).is_none());
        assert!(open(&mut app,DUPLICATE,&json!({"channelId":"1","channel":0})).is_none());
        assert!(open(&mut app,DUPLICATE,&json!({"channel":{"unknownGrant":true}})).is_none());
    }
    #[test]
    fn save_selection_defaults_and_duplicate_alpha_labels_keep_typed_choices() {
        let mut app=app();app.session.execute("select.all",json!({})).unwrap();
        app.session.execute(SAVE,json!({"name":"Own duplicate label"})).unwrap();app.session.execute(SAVE,json!({"name":"Own duplicate label"})).unwrap();
        let second=app.session.active().unwrap().doc.channels[1].id;
        let id=open(&mut app,SAVE,&Value::Null).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["channel"],"new");assert_eq!(fields["name"],"");assert_eq!(fields["operation"],"new");assert!(!fields.contains_key("document"));assert!(!fields.contains_key("invert"));assert!(!fields.contains_key("nativeCoverage"));
        fields.insert("channel".into(),json!(format!("alpha:{}",second.0)));fields.insert("operation".into(),json!("replace"));
        let count=app.session.active().unwrap().doc.channels.len();confirm(&mut app,id,&fields).unwrap();assert_eq!(app.session.active().unwrap().doc.channels.len(),count);assert_eq!(app.session.active().unwrap().doc.channels[1].id,second);
    }

}
