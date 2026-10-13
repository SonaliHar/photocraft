//! New Guide dialog origin fencing. Actual unit/axis conversion belongs to the engine.
use crate::PhotocraftApp;
use photocraft_engine::document_snapshot::WeakDocumentSnapshot;
use serde_json::{Map,Value,json};
const COMMAND:&str="view.newGuide";
#[derive(Clone,Debug)]
pub(crate) struct GuideOrigin {dialog:u64,document:photocraft_doc::DocId,revision:u64,snapshot:WeakDocumentSnapshot}
impl PartialEq for GuideOrigin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.document==other.document && self.revision==other.revision
            && match(self.snapshot.upgrade(),other.snapshot.upgrade()){(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or(tl!("Open a document before adding a guide."))?;
    if active.doc.id.0==0 || app.ui.transform.is_some() || app.session.has_jobs() || active.coalesce.is_some() || active.floating.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {return Err(tl!("Finish pending edits before adding a guide.").into());}Ok(())
}
pub(crate) fn open(app:&mut PhotocraftApp)->Option<u64> {
    if let Err(error)=idle(app) {app.ui.status=error;app.ui.status_error=true;return None;}
    Some(crate::new_guide_ui::open(app))
}
pub(crate) fn capture_open(app:&mut PhotocraftApp,id:u64) {
    app.ui.guide_origin=None;
    if idle(app).is_err() {return;}
    if let Some(active)=app.session.active() {
        app.ui.guide_origin=Some(GuideOrigin{dialog:id,document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade()});
    }
}

pub(crate) fn owns(fields:&Map<String,Value>)->bool {
    fields.get("__command").and_then(Value::as_str)==Some(COMMAND) || fields.get("__guideOrigin").and_then(Value::as_bool)==Some(true)
}
pub(crate) fn body(ui:&mut egui::Ui,fields:&mut Map<String,Value>) {crate::new_guide_ui::body(ui,fields);}
pub(crate) fn confirm(app:&mut PhotocraftApp,id:u64,fields:&Map<String,Value>)->Result<Value,String> {
    let target=app.ui.guide_origin.take().ok_or(tl!("The document changed. Reopen New Guide."))?;idle(app)?;
    let active=app.session.active().ok_or(tl!("The document changed. Reopen New Guide."))?;
    let snapshot=target.snapshot.upgrade().ok_or(tl!("The document changed. Reopen New Guide."))?;
    if target.dialog!=id || fields.get("__command").and_then(Value::as_str)!=Some(COMMAND) || active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot) {
        return Err(tl!("The document changed. Reopen New Guide.").into());
    }
    if fields.len()>16 || fields.keys().any(|key|!key.starts_with("__") && !matches!(key.as_str(),"orientation"|"position"|"color"|"customColor"|"documentId"|"expectedRevision"))
        || ["orientation","position","color","customColor"].iter().any(|key| fields.get(*key).is_none_or(|value|value.as_str().is_none_or(|text|text.len()>256 || text.contains('\0')))) {
        return Err(tl!("Check the guide settings and try again.").into());
    }
    let mut fields=fields.clone();
    fields.insert("documentId".into(),json!(target.document.0.to_string()));
    fields.insert("expectedRevision".into(),json!(target.revision.to_string()));
    crate::new_guide_ui::confirm(app,&fields)

}
#[cfg(test)]
mod tests {
    use super::*;
    fn app()->PhotocraftApp {let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());app.session.execute("file.new",json!({"width":4,"height":3})).unwrap();app}
    #[test]
    fn stale_tab_edit_and_restored_dialog_never_add_guides() {
        for change in 0..3 {
            let mut app=app();let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            match change {0=>{app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();},1=>{app.session.execute("image.canvasSize",json!({"width":5,"height":3})).unwrap();},_=>{app.ui=serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();assert!(app.ui.guide_origin.is_none());}}
            let before=app.session.active().unwrap().doc.clone();let revision=app.session.active().unwrap().revision;assert!(confirm(&mut app,id,&fields).is_err());assert!(app.session.active().unwrap().doc.shares_state(&before));assert_eq!(app.session.active().unwrap().revision,revision);
        }
    }
    #[test]
    fn original_defaults_and_actual_engine_per_axis_conversion_remain() {
        let mut app=app();app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().physical_resolution=Some(photocraft_doc::PhysicalResolution {x_dpi:144.,y_dpi:288.,unit:photocraft_doc::PhysicalUnit::Inch});
        for orientation in ["vertical","horizontal"] {
            let id=open(&mut app).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();
            assert_eq!(fields["color"],"cyan");assert!(fields["position"].is_string());assert!(!fields.contains_key("__preview"));assert!(!fields.contains_key("documentId"));
            fields.insert("orientation".into(),json!(orientation));fields.insert("position".into(),json!("1 in"));
            fields.insert("__guideDpiX".into(),json!(999.));fields.insert("__guideDpiY".into(),json!(999.));
            confirm(&mut app,id,&fields).unwrap();
        }
        let guides=&app.session.active().unwrap().doc.guides;assert_eq!(guides.vertical,vec![144.]);assert_eq!(guides.horizontal,vec![288.]);
    }
    #[test]
    fn malformed_position_or_live_gesture_refuses_without_source_change() {
        let mut app=app();let id=open(&mut app).unwrap();let mut fields=app.ui.dialog_mut(id).unwrap().fields.clone();fields.insert("position".into(),Value::Null);let before=app.session.active().unwrap().doc.clone();
        assert!(confirm(&mut app,id,&fields).is_err());assert!(app.session.active().unwrap().doc.shares_state(&before));
        let id=open(&mut app).unwrap();let fields=app.ui.dialog_mut(id).unwrap().fields.clone();app.session.active_mut().unwrap().coalesce=Some("Own gesture".into());
        assert_eq!(confirm(&mut app,id,&fields).unwrap_err(),tl!("Finish pending edits before adding a guide."));assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
