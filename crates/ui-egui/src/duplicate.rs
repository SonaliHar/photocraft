//! Duplicate dialog with runtime-only source identity and unchanged copy semantics.
use crate::{PhotocraftApp,state::DialogKind};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map,Value,json};
const COMMAND:&str="image.duplicate";
const DISPLAY_SPEC:&str=r#"{"name":text,"mergedOnly":bool=false}"#;
const NAME_BYTES:usize=2048;
#[derive(Clone,Debug)]
pub(crate) struct DuplicateOrigin {dialog:u64,document:photocraft_doc::DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for DuplicateOrigin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.document==other.document && self.revision==other.revision
            && match (self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or(tl!("Open a document before duplicating it."))?;
    if active.doc.id.0==0 || app.ui.transform.is_some() || app.session.has_jobs() || active.coalesce.is_some()
        || active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before duplicating the document.").into());
    }
    Ok(())
}
fn default_name(name:&str)->Result<String,String> {
    let size=name.len().checked_add(5).filter(|size|*size<=NAME_BYTES).ok_or(tl!("Choose a shorter document name before duplicating it."))?;
    if name.contains('\0') {return Err(tl!("Check the duplicate name and try again.").into());}
    let mut output=String::new();output.try_reserve_exact(size).map_err(|_|tl!("There is not enough memory to open Duplicate."))?;
    output.push_str(name);output.push_str(" copy");Ok(output)
}
pub(crate) fn open(app:&mut PhotocraftApp)->Option<u64> {
    let prepared=(|| {
        idle(app)?;
        let spec=photocraft_engine::commands::find(COMMAND).ok_or(tl!("Duplicate is unavailable."))?;
        let active=app.session.active().ok_or(tl!("Open a document before duplicating it."))?;
        let name=default_name(&active.doc.name)?;
        let origin=DuplicateOrigin {dialog:0,document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade()};
        Ok::<_,String>((origin,name,spec.label))
    })();
    let (mut origin,name,label)=match prepared {Ok(value)=>value,Err(error)=>{app.ui.status=error;app.ui.status_error=true;return None;}};
    let mut fields=Map::new();fields.insert("__command".into(),json!(COMMAND));fields.insert("__label".into(),json!(label));
    fields.insert("__filter".into(),json!(true));fields.insert("__duplicate".into(),json!(true));
    fields.insert("__spec".into(),json!(DISPLAY_SPEC));fields.insert("name".into(),json!(name));fields.insert("mergedOnly".into(),json!(false));
    let id=app.ui.open_dialog(DialogKind::Command,fields);origin.dialog=id;app.ui.duplicate_origin=Some(origin);Some(id)
}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {
    fields.get("__command").and_then(Value::as_str)==Some(COMMAND) || fields.get("__duplicate").and_then(Value::as_bool)==Some(true)
}
pub(crate) fn confirm(app:&mut PhotocraftApp,id:u64,fields:&Map<String,Value>)->Result<Value,String> {
    let target=app.ui.duplicate_origin.take().ok_or(tl!("The source document changed. Reopen Duplicate."))?;
    idle(app)?;
    let active=app.session.active().ok_or(tl!("The source document changed. Reopen Duplicate."))?;
    let snapshot=target.snapshot.upgrade().ok_or(tl!("The source document changed. Reopen Duplicate."))?;
    if target.dialog!=id || fields.get("__command").and_then(Value::as_str)!=Some(COMMAND)
        || active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot) {
        return Err(tl!("The source document changed. Reopen Duplicate.").into());
    }
    if fields.len()>12 || fields.keys().any(|key|!key.starts_with("__") && !matches!(key.as_str(),"name"|"mergedOnly"|"documentId"|"expectedRevision")) {
        return Err(tl!("Check the Duplicate values and try again.").into());
    }
    let name=fields.get("name").and_then(Value::as_str).filter(|name|name.len()<=NAME_BYTES && !name.contains('\0')).ok_or(tl!("Check the duplicate name and try again."))?;
    let merged=fields.get("mergedOnly").and_then(Value::as_bool).ok_or(tl!("Check the Duplicate values and try again."))?;
    app.run(COMMAND,json!({"name":name,"mergedOnly":merged,"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string()}))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn app()->PhotocraftApp {let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());app.session.execute("file.new",json!({"width":3,"height":2})).unwrap();app}
    #[test]
    fn tab_source_edit_and_restored_dialog_do_not_duplicate_another_target() {
        for change in 0..3 {
            let mut app=app();let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {0=>{app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();},1=>{app.session.execute("image.canvasSize",json!({"width":4,"height":2})).unwrap();},_=>{app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.duplicate_origin.is_none());}}
            let before=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;let count=app.session.documents().len();
            assert!(confirm(&mut app,id,&fields).is_err());assert_eq!(app.session.documents().len(),count);assert_eq!(app.session.active().unwrap().revision,revision);assert!(app.session.active().unwrap().doc.shares_state(&before));
        }
    }
    #[test]
    fn high_id_pair_replaces_restored_selectors_and_copy_defaults_are_exact() {
        let mut app=app();app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().id=photocraft_doc::DocId(9_007_199_254_740_995);
        let source=app.session.active().unwrap().doc.clone();let count=app.session.documents().len();let id=open(&mut app).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["name"],format!("{} copy",source.name));assert_eq!(fields["mergedOnly"],false);
        fields.insert("documentId".into(),json!("wrong-restored-id"));fields.insert("expectedRevision".into(),json!("wrong-restored-revision"));
        confirm(&mut app,id,&fields).unwrap();assert_eq!(app.session.documents().len(),count+1);assert_eq!(app.session.active().unwrap().doc.name,format!("{} copy",source.name));assert!(app.session.documents().first().unwrap().doc.shares_state(&source));
    }
    #[test]
    fn bounded_names_and_live_coalesce_leave_source_unchanged() {
        assert!(default_name(&"x".repeat(NAME_BYTES-5)).is_ok());assert!(default_name(&"x".repeat(NAME_BYTES-4)).is_err());assert!(default_name("own\0name").is_err());
        let mut app=app();let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();let before=app.session.active().unwrap().doc.clone();app.session.active_mut().unwrap().coalesce=Some("Own gesture".into());
        assert_eq!(confirm(&mut app,id,&fields).unwrap_err(),tl!("Finish pending edits before duplicating the document."));assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
