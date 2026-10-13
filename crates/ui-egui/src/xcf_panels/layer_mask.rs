//! Current canonical layer-mask flag, dispatched without changing shell selection.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{DocId, LayerId, item_identity::{ItemRef,MaskId}};
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::json;
const STALE:&str="xcf.mask.enabled.stale-target";
struct Target {document:DocId,revision:u64,layer:LayerId,mask:MaskId,enabled:bool,snapshot:WeakDocumentSnapshot}
fn association(app:&PhotocraftApp,layer:LayerId)->Option<(MaskId,bool)> {
    let active=app.session.active()?;
    let state=active.doc.xcf_state.as_ref()?;
    if state.items.len()>10_000 || state.filter_masks.len()>10_000 {return None;}
    let native=active.doc.layer(layer)?.mask.as_ref()?;
    let mut filters=std::collections::HashSet::new();
    filters.try_reserve(state.filter_masks.len()).ok()?;
    for mask in &state.filter_masks {filters.insert(mask.id);}
    let mut target=None;
    for item in &state.items {
        let Some(metadata)=&item.mask else {continue;};
        if metadata.owner!=ItemRef::Layer(layer) {continue;}
        let ItemRef::Mask(id)=item.item else {return None;};
        if filters.contains(&id) {continue;}
        if id.0==0 || target.is_some() || metadata.enabled!=native.enabled || metadata.linked!=native.linked
            || metadata.bounds.is_empty() || metadata.offset!=[metadata.bounds.x0,metadata.bounds.y0]
            || item.raster_bounds.is_some_and(|bounds|bounds!=metadata.bounds) {return None;}
        target=Some((id,metadata.enabled));
    }
    target
}
fn capture(app:&PhotocraftApp,layer:LayerId)->Option<Target> {
    let (mask,enabled)=association(app,layer)?;
    let active=app.session.active()?;
    Some(Target {document:active.doc.id,revision:active.revision,layer,mask,enabled,snapshot:active.doc.downgrade()})
}
fn permitted(app:&PhotocraftApp,target:&Target)->bool {
    let Some(active)=app.session.active() else {return false;};
    if active.floating.is_some() || app.session.has_jobs() {return false;}
    let Ok(locks)=photocraft_engine::xcf_item_locks::effective_item_locks(&active.doc,ItemRef::Mask(target.mask)) else {return false;};
    // Pixels-only native locks do not prohibit the separate mask operation.
    !locks.native.all && !locks.imported.content
        && photocraft_engine::xcf_item_locks::require_layer_mask_content(&active.doc,target.layer).is_ok()
}
fn apply(app:&mut PhotocraftApp,target:&Target,enabled:bool)->Result<serde_json::Value,String> {
    let active=app.session.active().ok_or(STALE)?;
    let snapshot=target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot)
        || association(app,target.layer)!=Some((target.mask,target.enabled)) {return Err(STALE.into());}
    if !permitted(app,target) {return Err("xcf.mask.enabled.context-or-lock".into());}
    // Immediate registered dispatch after revalidation: no refocus, preview drop or direct edit.
    app.run("xcf.mask.enabled.set",json!({"maskId":target.mask.0.to_string(),"enabled":enabled}))
}
pub(crate) fn properties(app:&mut PhotocraftApp,ui:&mut egui::Ui,layer:LayerId) {
    let Some(target)=capture(app,layer) else {return;};
    let tokens=Tokens::get(ui.ctx());
    let editable=permitted(app,&target);
    let mut enabled=target.enabled;
    ui.separator();
    ui.label(egui::RichText::new(tl!("Layer Mask")).color(tokens.text));
    let changed=ui.push_id(("xcf-current-layer-mask",target.document.0,target.mask.0),|ui| {
        ui.add_enabled_ui(editable,|ui|crate::widgets::checkbox(ui,&mut enabled,tl!("Enable Layer Mask")).changed()).inner
    }).inner;
    if changed && let Err(error)=apply(app,&target,enabled) {
        app.ui.status=error;
        app.ui.status_error=true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(PhotocraftApp,LayerId) {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let doc=app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let layer=doc.layers.first().unwrap().id;
        doc.layer_mut(layer).unwrap().mask=Some(photocraft_doc::LayerMask::hide_all());
        doc.sample_encoding=Some(photocraft_doc::SampleEncoding::NonlinearProfile);
        doc.icc_profile=Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
        let mut state=photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).unwrap();
        let bounds=doc.bounds();
        state.items.push(photocraft_doc::xcf_state::ItemState {item:ItemRef::Mask(MaskId(1)),name:"Own mask".into(),custom_name:true,visible:true,locks:Default::default(),parent:None,source_tattoo:None,source_flags:0,raster_bounds:Some(bounds),composition:None,
            mask:Some(photocraft_doc::xcf_state::MaskState {owner:ItemRef::Layer(layer),bounds,offset:[bounds.x0,bounds.y0],outside_f64_bits:0.0f64.to_bits(),enabled:true,shown:false,editing:false,linked:true})});
        doc.xcf_state=Some(state);
        (app,layer)
    }
    #[test]
    fn guarded_mask_control_changes_only_flags_and_preserves_focus_undo() {
        let (mut app,layer)=fixture();
        let target=capture(&app,layer).unwrap();
        let before=app.session.active().unwrap().doc.clone();
        let focus=app.session.active().unwrap().active_layer;
        apply(&mut app,&target,false).unwrap();
        let mut expected=before.as_ref().clone();
        expected.layer_mut(layer).unwrap().mask.as_mut().unwrap().enabled=false;
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item|item.item==ItemRef::Mask(target.mask)).unwrap().mask.as_mut().unwrap().enabled=false;
        assert_eq!(app.session.active().unwrap().doc.as_ref(),&expected);
        assert_eq!(app.session.active().unwrap().active_layer,focus);
        app.session.execute("edit.undo",json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc,before);
    }
    #[test]
    fn stale_and_lock_refusal_never_dispatches() {
        let (mut app,layer)=fixture();
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().locks.pixels=true;
        let target=capture(&app,layer).unwrap();
        assert!(permitted(&app,&target));
        app.session.active_mut().unwrap().revision+=1;
        let before=app.session.active().unwrap().doc.clone();
        assert_eq!(apply(&mut app,&target,false).unwrap_err(),STALE);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().locks.all=true;
        let target=capture(&app,layer).unwrap();
        assert!(!permitted(&app,&target));
        let before=app.session.active().unwrap().doc.clone();
        assert!(apply(&mut app,&target,false).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
