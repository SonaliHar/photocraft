//! Captured Refine Edge forms, including the controlled brush outputs.
//! Source qualification and output preparation remain engine authority.
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
pub(crate) fn handles(command:&str)->bool {command=="select.refineEdge"}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {fields.get("__command").and_then(Value::as_str).is_some_and(handles)}
fn stale()->String {tl!("The selection target changed. Reopen the dialog.").into()}
fn invalid()->String {tl!("Check the selection settings and try again.").into()}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(||tl!("Open a document before refining an edge.").to_owned())?;
    if active.doc.id.0==0 || active.revision==u64::MAX || app.ui.transform.is_some() || app.session.has_jobs()
        || active.floating.is_some() || active.coalesce.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before creating this selection.").into());
    }
    Ok(())
}
pub(crate) fn clear(app:&mut PhotocraftApp) {app.ui.selection_refine_origin=None;}
pub(crate) fn capture_open(app:&mut PhotocraftApp,dialog:u64,command:&str) {
    if !handles(command) {return;}
    clear(app);
    let capture=(|| {
        idle(app)?;let active=app.session.active().ok_or_else(stale)?;
        if active.channel_view.color_hidden.len()>16 || active.channel_view.alpha_visible.len()>10_000 {return Err(invalid());}
        Ok::<_,String>(Origin {dialog,command:command.to_owned(),document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade(),
            layer:active.active_layer,view:copy_view(&active.channel_view)?,shell_mask:app.layer_mask_targeted()})
    })();
    match capture {Ok(origin)=>app.ui.selection_refine_origin=Some(origin),Err(error)=>{app.ui.status=error;app.ui.status_error=true;}}
}
fn copy_view(view:&ChannelView)->Result<ChannelView,String> {
    fn copy(values:&[bool],cap:usize)->Result<Vec<bool>,String> {
        if values.len()>cap {return Err(invalid());}
        let mut output=Vec::new();output.try_reserve_exact(values.len()).map_err(|_|invalid())?;output.extend_from_slice(values);Ok(output)
    }
    Ok(ChannelView {target:view.target,color_hidden:copy(&view.color_hidden,16)?,alpha_visible:copy(&view.alpha_visible,10_000)?,
        quick_mask_hidden:view.quick_mask_hidden,layer_mask:view.layer_mask,editing_mask:view.editing_mask})
}
/// Borrowed stroke census before the generic dialog snapshot or params clone.
/// It accepts the existing bounded brush view data but never qualifies its output.
fn strokes(value:&Value)->bool {
    let Some(strokes)=value.as_array().filter(|values|values.len()<=photocraft_engine::smartselect_cmds::MAX_STROKES) else {return false;};
    let mut total=0usize;let mut bytes=0usize;
    for stroke in strokes {
        let Some(object)=stroke.as_object().filter(|object|object.len()<=3) else {return false;};
        if object.keys().any(|key|!matches!(key.as_str(),"points"|"size"|"mode")) {return false;}
        let Some(points)=object.get("points").and_then(Value::as_array) else {return false;};
        let Some(next)=total.checked_add(points.len()).filter(|sum|*sum<=photocraft_engine::smartselect_cmds::MAX_STROKE_POINTS) else {return false;};total=next;
        let Some(cost)=points.len().checked_mul(3*std::mem::size_of::<Value>()).and_then(|cost|cost.checked_add(512)) else {return false;};
        let Some(next)=bytes.checked_add(cost).filter(|sum|*sum<=16<<20) else {return false;};bytes=next;
        if !points.iter().all(|point|point.as_array().is_some_and(|pair|pair.len()==2 && pair.iter().all(|v|v.as_f64().is_some_and(|n|n.is_finite() && n.abs()<=1e7)))) {return false;}
        if object.get("size").is_some_and(|v|!v.as_f64().is_some_and(|n|n.is_finite() && (1.0..=5000.0).contains(&n))) {return false;}
        if object.get("mode").is_some_and(|v|!matches!(v.as_str(),Some("add"|"subtract"))) {return false;}
    }
    true
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
fn number(value:&Value,min:f64,max:f64)->bool {value.as_f64().is_some_and(|n|n.is_finite() && (min..=max).contains(&n))}
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
            "sampleAllLayers"|"smartRadius"|"decontaminate"=>value.is_boolean(),
            "radius"=>number(value,0.,250.),"smooth"|"contrast"|"amount"=>number(value,0.,100.),
            "feather"=>number(value,0.,1000.),"shiftEdge"=>number(value,-100.,100.),
            "output"=>matches!(value.as_str(),Some("selection"|"layerMask"|"newLayer"|"newLayerWithMask"|"newDocument"|"newDocumentWithMask")),
            "target"=>matches!(value.as_str(),Some(""|"mask")),
            "refineStrokes"=>strokes(value),
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
    idle(app)?;let origin=app.ui.selection_refine_origin.as_ref().ok_or_else(stale)?;
    let active=app.session.active().ok_or_else(stale)?;
    if origin.dialog!=dialog || origin.command!=command || active.doc.id!=origin.document || active.revision!=origin.revision
        || !origin.snapshot.is_current(&active.doc) || active.active_layer!=origin.layer || active.channel_view!=origin.view || app.layer_mask_targeted()!=origin.shell_mask {return Err(stale());}Ok(())
}
pub(crate) fn prepare(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,mut params:Value)->Result<Value,String> {
    validate(app,dialog,fields)?;
    if !fields.get("__command").and_then(Value::as_str).is_some_and(handles) {return Ok(params);}
    let origin=app.ui.selection_refine_origin.as_ref().ok_or_else(stale)?;
    let object=params.as_object_mut().filter(|object|object.len()<=14).ok_or_else(invalid)?;
    object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;use serde_json::json;
    fn app()->PhotocraftApp {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();
        app.session.execute("select.all",json!({})).unwrap();app
    }
    fn settings()->Map<String,Value> {json!({"__command":"select.refineEdge","__filter":true,"__preview":true,
        "radius":0,"smartRadius":false,"smooth":0,"feather":0,"contrast":0,"shiftEdge":0,
        "decontaminate":false,"amount":100,"output":"selection","sampleAllLayers":false,"refineStrokes":[]}).as_object().unwrap().clone()}
    #[test]
    fn editable_options_keep_capture_and_preserve_mask_and_decontamination_params() {
        let mut app=app();capture_open(&mut app,7,"select.refineEdge");let mut fields=settings();
        fields.insert("output".into(),json!("layerMask"));fields.insert("decontaminate".into(),json!(true));fields.insert("target".into(),json!("mask"));
        fields.insert("__view".into(),json!("overlay"));
        let params=crate::filter_dialog::params_of(&fields);let request=prepare(&app,7,&fields,params).unwrap();
        assert_eq!(request["documentId"],app.session.active().unwrap().doc.id.0.to_string());
        assert_eq!(request["expectedRevision"],app.session.active().unwrap().revision.to_string());
        assert_eq!(request["target"],"mask");assert_eq!(request["decontaminate"],true);assert_eq!(request["output"],"layerMask");
    }
    #[test]
    fn restored_or_changed_view_target_cannot_reuse_dialog_authority() {
        let mut app=app();capture_open(&mut app,7,"select.refineEdge");let fields=settings();
        app.session.active_mut().unwrap().channel_view.target=photocraft_engine::channel_cmds::ChannelTarget::Color(0);
        assert!(validate(&app,7,&fields).is_err());
        app.session.active_mut().unwrap().channel_view.target=photocraft_engine::channel_cmds::ChannelTarget::Composite;
        assert!(validate(&app,7,&fields).is_ok());
        app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(app.ui.selection_refine_origin.is_none());assert!(validate(&app,7,&fields).is_err());
    }
    #[test]
    fn nonempty_brush_keeps_exact_points_order_and_captured_pair_for_each_output() {
        for output in ["selection","layerMask","newLayer","newLayerWithMask","newDocument","newDocumentWithMask"] {
            for decontaminate in [false,true] {
                let mut app=app();let mut fields=settings();
                let brush=json!([{"points":[[1,1],[2,1]],"size":3,"mode":"add"},{"points":[[2,2]],"size":2,"mode":"subtract"}]);
                fields.insert("refineStrokes".into(),brush.clone());fields.insert("output".into(),json!(output));fields.insert("decontaminate".into(),json!(decontaminate));
                capture_open(&mut app,7,"select.refineEdge");assert!(validate(&app,7,&fields).is_ok());
                let request=prepare(&app,7,&fields,crate::filter_dialog::params_of(&fields)).unwrap();
                assert_eq!(request["refineStrokes"],brush);assert_eq!(request["output"],output);assert_eq!(request["decontaminate"],decontaminate);
                assert_eq!(request["documentId"],app.session.active().unwrap().doc.id.0.to_string());
            }
        }
    }
    #[test]
    fn stale_brush_confirmation_keeps_form_document_and_history() {
        let mut app=app();let mut fields=settings();fields.insert("refineStrokes".into(),json!([{"points":[[1,1]],"size":3,"mode":"add"}]));
        let id=app.ui.open_dialog(crate::state::DialogKind::Command,fields);capture_open(&mut app,id,"select.refineEdge");
        app.session.active_mut().unwrap().channel_view.target=photocraft_engine::channel_cmds::ChannelTarget::Color(0);
        let before=app.session.active().unwrap().doc.clone();let history=app.session.active().unwrap().history.past_len();
        assert!(crate::dialogs::confirm(&mut app,id).is_err());assert!(app.ui.dialogs.iter().any(|dialog|dialog.id==id));
        assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().history.past_len(),history);
    }
    #[test]
    fn malformed_arrays_and_stale_revision_refuse_before_params_copy() {
        let mut app=app();capture_open(&mut app,7,"select.refineEdge");let mut malformed=settings();
        malformed.insert("refineStrokes".into(),json!([{"points":[[1]],"mode":"add"}]));assert!(fields(&malformed).is_err());
        malformed=settings();malformed.insert("__spec".into(),json!("x".repeat(8193)));assert!(fields(&malformed).is_err());
        let original=settings();app.session.edit("Own redo seed",|document,_|{document.name="Own change".into();Ok(())}).unwrap();
        app.session.execute("edit.undo",json!({})).unwrap();assert!(prepare(&app,7,&original,json!({})).is_err());assert!(app.session.active().unwrap().history.can_redo());
    }
}
