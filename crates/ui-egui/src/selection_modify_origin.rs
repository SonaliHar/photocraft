//! Runtime origin for existing generated Select > Modify dialogs.
//! Serialized fields keep their upstream view/settings behavior, never mutation authority.
use crate::PhotocraftApp;
use photocraft_doc::DocId;
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::{Map,Value};
#[derive(Clone,Debug)]
pub(crate) struct Origin {dialog:u64,command:String,document:DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for Origin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.command==other.command && self.document==other.document && self.revision==other.revision
            && match(self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
pub(crate) fn handles(command:&str)->bool {
    matches!(command,"select.modify.border"|"select.modify.smooth"|"select.modify.expand"|"select.modify.contract"|"select.modify.feather")
}
fn canonical(value:&Value,nonzero:bool)->bool {
    value.as_str().is_some_and(|text|!text.is_empty() && text.len()<=20 && text.bytes().all(|byte|byte.is_ascii_digit())
        && (text.len()==1 || !text.starts_with('0')) && text.parse::<u64>().is_ok_and(|id|!nonzero || id!=0))
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(||tl!("Open a document before modifying a selection.").to_owned())?;
    if active.doc.id.0==0 || active.revision==u64::MAX || app.ui.transform.is_some() || app.session.has_jobs()
        || active.floating.is_some() || active.coalesce.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before modifying a selection.").into());
    }
    Ok(())
}
pub(crate) fn capture_open(app:&mut PhotocraftApp,dialog:u64,command:&str) {
    if !handles(command) {return;}
    app.ui.selection_modify_origin=None;
    if let Err(error)=idle(app) {app.ui.status=error;app.ui.status_error=true;return;}
    if let Some(active)=app.session.active() {
        app.ui.selection_modify_origin=Some(Origin {dialog,command:command.to_owned(),document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade()});
    }
}
/// Check borrowed fields before the generic dialog parser clones command values.
pub(crate) fn validate(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>)->Result<(),String> {
    let command=fields.get("__command").and_then(Value::as_str).ok_or_else(||tl!("Reopen the selection dialog before applying.").to_owned())?;
    if !handles(command) {return Ok(());}
    let stale=||tl!("The selection changed. Reopen the Modify dialog.").to_owned();
    let origin=app.ui.selection_modify_origin.as_ref().ok_or_else(stale)?;
    idle(app)?;
    let active=app.session.active().ok_or_else(stale)?;
    let snapshot=origin.snapshot.upgrade().ok_or_else(stale)?;
    if origin.dialog!=dialog || origin.command!=command || active.doc.id!=origin.document || active.revision!=origin.revision || !active.doc.shares_state(&snapshot) {
        return Err(stale());
    }
    if fields.len()>32 || fields.keys().any(|key|key.len()>128 || (!key.starts_with("__") && !matches!(key.as_str(),"radius"|"applyAtCanvasBounds"|"documentId"|"expectedRevision")))
        || fields.get("radius").is_some_and(|value|value.as_f64().is_none_or(|radius|!radius.is_finite()))
        || fields.get("applyAtCanvasBounds").is_some_and(|value|!value.is_boolean()) {
        return Err(tl!("Check the selection settings and try again.").into());
    }
    match(fields.get("documentId"),fields.get("expectedRevision")) {
        (None,None)=>{},
        (Some(document),Some(revision)) if canonical(document,true) && canonical(revision,false)=>{},
        _=>return Err(tl!("Check the selection settings and try again.").into()),
    }
    Ok(())
}
/// Preserve the normal generated-dialog confirmation tail; only replace its final params.
pub(crate) fn prepare(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,mut params:Value)->Result<Value,String> {
    validate(app,dialog,fields)?;
    let command=fields.get("__command").and_then(Value::as_str).ok_or_else(||tl!("Reopen the selection dialog before applying.").to_owned())?;
    if !handles(command) {return Ok(params);}
    let origin=app.ui.selection_modify_origin.as_ref().ok_or_else(||tl!("The selection changed. Reopen the Modify dialog.").to_owned())?;
    let invalid=||tl!("Check the selection settings and try again.").to_owned();
    let object=params.as_object_mut().filter(|object|object.len()<=4 && object.keys().all(|key|matches!(key.as_str(),"radius"|"applyAtCanvasBounds"|"documentId"|"expectedRevision"))).ok_or_else(invalid)?;
    if object.get("radius").is_some_and(|value|value.as_f64().is_none_or(|radius|!radius.is_finite()))
        || object.get("applyAtCanvasBounds").is_some_and(|value|!value.is_boolean()) {return Err(invalid());}
    object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));
    Ok(params)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn modified_revision_or_restored_dialog_cannot_gain_authority_or_prefs() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();
        let fields=json!({"__command":"select.modify.feather"}).as_object().unwrap().clone();
        capture_open(&mut app,7,"select.modify.feather");
        let before=app.session.active().unwrap().doc.clone();
        let prefs=app.session.prefs().dialogs.clone();
        let request=prepare(&app,7,&fields,json!({"radius":3,"applyAtCanvasBounds":false})).unwrap();
        assert_eq!(request["documentId"],json!(before.id.0.to_string()));
        app.session.active_mut().unwrap().revision+=1;
        assert!(prepare(&app,7,&fields,json!({"radius":3})).is_err());
        app.ui.selection_modify_origin=None;
        assert!(prepare(&app,7,&fields,json!({"radius":3})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.prefs().dialogs,prefs);
    }
}
