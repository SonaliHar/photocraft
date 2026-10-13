//! Conservative current empty-selection filter-mask controls; engine admission remains authoritative.
use crate::{PhotocraftApp,theme::Tokens};
use photocraft_doc::{DocId,LayerId,item_identity::{ItemRef,MaskId}};
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::json;
fn stale()->String {tl!("The filter mask changed. Select it again before editing.").into()}
struct Target {document:DocId,revision:u64,layer:LayerId,mask:MaskId,enabled:bool,snapshot:WeakDocumentSnapshot}
fn capture(app:&PhotocraftApp,layer:LayerId,mask:MaskId)->Option<Target> {
    let active=app.session.active()?;
    let enabled=photocraft_engine::xcf_filter_mask_cmds::zero_selection_candidate(&active.doc,layer,mask)?;
    Some(Target {document:active.doc.id,revision:active.revision,layer,mask,enabled,snapshot:active.doc.downgrade()})
}
fn idle(app:&PhotocraftApp)->bool {
    app.ui.transform.is_none() && !app.session.has_jobs() && app.session.active().is_some_and(|active|
        active.coalesce.is_none() && active.floating.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state|state.floating.is_none()))
}
fn context_error()->String {tl!("Finish pending edits and unlock the filter mask before editing.").into()}
fn permitted(app:&PhotocraftApp,target:&Target)->bool {
    let Some(active)=app.session.active() else {return false;};
    if !idle(app) {return false;}
    photocraft_engine::xcf_item_locks::effective_item_locks(&active.doc,ItemRef::Mask(target.mask)).is_ok_and(|locks|!locks.native.all && !locks.imported.content)
}
fn apply(app:&mut PhotocraftApp,target:&Target,enabled:bool)->Result<serde_json::Value,String> {
    if !idle(app) {return Err(context_error());}
    let active=app.session.active().ok_or_else(stale)?;
    let snapshot=target.snapshot.upgrade().ok_or_else(stale)?;
    if active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot)
        || photocraft_engine::xcf_filter_mask_cmds::zero_selection_candidate(&active.doc,target.layer,target.mask)!=Some(target.enabled) {return Err(stale());}
    if !permitted(app,target) {return Err(context_error());}
    app.run("xcf.filter.mask.enabled.set",json!({"maskId":target.mask.0.to_string(),"enabled":enabled}))
}
pub(crate) fn properties(app:&mut PhotocraftApp,ui:&mut egui::Ui,layer:LayerId) {
    let Some(active)=app.session.active() else {return;};
    let Some(state)=active.doc.xcf_state.as_ref() else {return;};
    if state.filter_masks.len()>10_000 || state.filter_stacks.len()>10_000 {return;}
    // Only bounded declared candidates are listed. No raster samples, CRC or
    // per-frame renderer evaluation establish eligibility on this UI path.
    let mut rows=Vec::new();
    if rows.try_reserve_exact(16).is_err() {return;}
    for mask in state.filter_masks.iter().take(64) {
        if rows.len()==16 {break;}
        if let Some(target)=capture(app,layer,mask.id) {rows.push(target);}
    }
    if rows.is_empty() {return;}
    let tokens=Tokens::get(ui.ctx());
    ui.separator();
    ui.label(egui::RichText::new(tl!("GIMP Filters")).color(tokens.text));
    for target in rows {
        let editable=permitted(app,&target);
        let mut disabled=!target.enabled;
        let changed=ui.push_id(("xcf-filter-mask",target.document.0,target.mask.0),|ui| {
            ui.add_enabled_ui(editable,|ui|crate::widgets::checkbox(ui,&mut disabled,tl!("Disable Filter Mask")).changed()).inner
        }).inner;
        if changed {
            if let Err(error)=apply(app,&target,!disabled) {app.ui.status=error;app.ui.status_error=true;}
            break;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalescing_filter_mask_context_is_refused_before_registered_dispatch() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let active=app.session.active().unwrap();
        let target=Target {document:active.doc.id,revision:active.revision,layer:active.active_layer.unwrap(),mask:MaskId(1),enabled:true,snapshot:active.doc.downgrade()};
        let before=active.doc.clone();
        let journal=app.session.journal.clone();
        app.session.active_mut().unwrap().coalesce=Some("Own pending filter gesture".into());
        assert!(!permitted(&app,&target));
        assert_eq!(apply(&mut app,&target,false).unwrap_err(),context_error());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.journal,journal);
        assert!(app.session.active().unwrap().coalesce.is_some());
    }
    #[test]
    fn no_canonical_filter_masks_never_offer_control() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":8,"height":6})).unwrap();
        let layer=app.session.active().unwrap().doc.layers.first().unwrap().id;
        assert!(capture(&app,layer,MaskId(1)).is_none());
    }
}
