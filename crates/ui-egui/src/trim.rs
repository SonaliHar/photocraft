//! Origin-fenced Trim dialog, using the existing generated controls unchanged.
use crate::{PhotocraftApp,state::DialogKind};
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map,Value,json};
const COMMAND:&str="image.trim";
const DISPLAY_SPEC:&str=r#"{"basedOn":"transparent|topLeft|bottomRight"="transparent","top":bool=true,"bottom":bool=true,"left":bool=true,"right":bool=true}"#;
#[derive(Clone,Debug)]
pub(crate) struct TrimOrigin {dialog:u64,document:photocraft_doc::DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for TrimOrigin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.document==other.document && self.revision==other.revision
            && match (self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or(tl!("Open a document before trimming."))?;
    if active.doc.id.0==0 || app.ui.transform.is_some() || app.session.has_jobs() || active.coalesce.is_some()
        || active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before trimming.").into());
    }
    Ok(())
}
pub(crate) fn open(app:&mut PhotocraftApp)->Option<u64> {
    if let Err(error)=idle(app) {app.ui.status=error;app.ui.status_error=true;return None;}
    let spec=photocraft_engine::commands::find(COMMAND)?;
    let active=app.session.active()?;
    let mut target=TrimOrigin {dialog:0,document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade()};
    let mut fields=Map::new();
    fields.insert("__command".into(),json!(COMMAND));fields.insert("__label".into(),json!(spec.label));
    fields.insert("__filter".into(),json!(true));fields.insert("__trim".into(),json!(true));
    // Stable existing form; canonical selectors are not editable controls.
    fields.insert("__spec".into(),json!(DISPLAY_SPEC));
    fields.insert("basedOn".into(),json!("transparent"));
    for side in ["top","bottom","left","right"] {fields.insert(side.into(),json!(true));}
    let id=app.ui.open_dialog(DialogKind::Command,fields);target.dialog=id;
    app.ui.trim_origin=Some(target);Some(id)
}
pub(crate) fn owns(fields:&Map<String,Value>)->bool {
    // All restored trim-command dialogs must reach this guard, even without marker.
    fields.get("__command").and_then(Value::as_str)==Some(COMMAND)
        || fields.get("__trim").and_then(Value::as_bool)==Some(true)
}
pub(crate) fn confirm(app:&mut PhotocraftApp,id:u64,fields:&Map<String,Value>)->Result<Value,String> {
    let target=app.ui.trim_origin.take().ok_or(tl!("The document changed. Reopen the Trim dialog."))?;
    idle(app)?;
    let active=app.session.active().ok_or(tl!("The document changed. Reopen the Trim dialog."))?;
    let snapshot=target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen the Trim dialog."))?;
    if target.dialog!=id || fields.get("__command").and_then(Value::as_str)!=Some(COMMAND)
        || active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot) {
        return Err(tl!("The document changed. Reopen the Trim dialog.").into());
    }
    if fields.len()>16 || fields.keys().any(|key|!key.starts_with("__") && !matches!(key.as_str(),"basedOn"|"top"|"bottom"|"left"|"right"|"documentId"|"expectedRevision")) {
        return Err(tl!("Check the Trim values and try again.").into());
    }
    let based=fields.get("basedOn").and_then(Value::as_str).filter(|value|matches!(*value,"transparent"|"topLeft"|"bottomRight")).ok_or(tl!("Check the Trim values and try again."))?;
    let mut params=Map::new();params.insert("basedOn".into(),json!(based));
    for side in ["top","bottom","left","right"] {
        let value=fields.get(side).and_then(Value::as_bool).ok_or(tl!("Check the Trim values and try again."))?;
        params.insert(side.into(),json!(value));
    }
    params.insert("documentId".into(),json!(target.document.0.to_string()));
    params.insert("expectedRevision".into(),json!(target.revision.to_string()));
    app.run(COMMAND,Value::Object(params))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn app()->PhotocraftApp {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":3})).unwrap();app
    }
    #[test]
    fn stale_tab_revision_and_restored_origin_refuse_without_document_edits() {
        for change in 0..3 {
            let mut app=app();let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {
                0=>{app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();},
                1=>{app.session.execute("image.canvasSize",json!({"width":6,"height":3})).unwrap();},
                _=>{app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.trim_origin.is_none());},
            }
            let source=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;
            assert!(confirm(&mut app,id,&fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&source));assert_eq!(app.session.active().unwrap().revision,revision);
        }
    }
    #[test]
    fn exact_high_id_origin_preserves_existing_form_and_coalesce_refuses() {
        let mut app=app();app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().id=photocraft_doc::DocId(9_007_199_254_740_995);
        let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(fields["basedOn"],"transparent");for side in ["top","bottom","left","right"] {assert_eq!(fields[side],true);}
        let target=app.ui.trim_origin.as_ref().unwrap();assert_eq!(target.document.0,9_007_199_254_740_995);
        let mut supplied=fields.clone();supplied.insert("documentId".into(),json!("wrong-restored-target"));supplied.insert("expectedRevision".into(),json!("wrong-restored-revision"));
        confirm(&mut app,id,&supplied).unwrap();
        assert_eq!(app.session.active().unwrap().doc.id.0,9_007_199_254_740_995);
        let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
        let before=app.session.active().unwrap().doc.clone();app.session.active_mut().unwrap().coalesce=Some("Own live gesture".into());
        assert_eq!(confirm(&mut app,id,&fields).unwrap_err(),tl!("Finish pending edits before trimming."));assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
