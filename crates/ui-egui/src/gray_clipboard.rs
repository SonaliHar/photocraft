//! Frame-local native Gray clipboard target capture. Ordinary image/text routes remain unchanged.
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId,item_identity::ChannelId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelTarget};
use serde_json::{Value,json};
#[derive(Clone,Copy,PartialEq,Eq)]
enum Owner {Channel(usize,ChannelId),Mask(LayerId)}
pub(crate) struct Target {document:DocId,revision:u64,snapshot:WeakDocumentSnapshot,owner:Owner,command:String}
fn stale()->String {tl!("The clipboard target changed. Select the channel or mask again.").into()}
fn owner(app:&PhotocraftApp,command:&str,params:&Value)->Result<Option<Owner>,String> {
    let Some(active)=app.session.active() else {return Ok(None)};
    if params.get("target").is_none() && photocraft_engine::channel_cmds::follows_target(command) && app.layer_mask_targeted() {
        return Ok(Some(Owner::Mask(active.active_layer.ok_or_else(stale)?)));
    }
    let selected=match params.get("target") {
        Some(Value::String(name)) if name=="mask"=>Some(Owner::Mask(active.active_layer.ok_or_else(stale)?)),
        Some(Value::Object(object)) if object.len()==1 && object.contains_key("channel")=> {
            let index=object.get("channel").and_then(Value::as_u64).and_then(|i|usize::try_from(i).ok()).ok_or_else(stale)?;
            Some(Owner::Channel(index,active.doc.channels.get(index).ok_or_else(stale)?.id))
        }
        Some(_)=>None,
        None=>match active.channel_view.target {
            ChannelTarget::Alpha(index)=>Some(Owner::Channel(index,active.doc.channels.get(index).ok_or_else(stale)?.id)),
            ChannelTarget::LayerMask=>Some(Owner::Mask(active.active_layer.ok_or_else(stale)?)),
            _=>None,
        },
    };
    Ok(selected)
}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(stale)?;
    if app.ui.transform.is_some() || active.floating.is_some() || active.coalesce.is_some() || app.session.has_jobs()
        || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {
        return Err(tl!("Finish pending edits before using the channel or mask clipboard.").into());
    }
    Ok(())
}
pub(crate) fn capture(app:&PhotocraftApp,command:&str,params:&Value)->Result<Option<Target>,String> {
    if !matches!(command,"edit.copy"|"edit.cut"|"edit.paste") || app.ui.text_edit.is_some() {return Ok(None)}
    let Some(owner)=owner(app,command,params)? else {return Ok(None)};
    let active=app.session.active().ok_or_else(stale)?;
    let surface=match owner {
        Owner::Channel(index,_)=>&active.doc.channels.get(index).ok_or_else(stale)?.surface,
        Owner::Mask(layer)=>active.doc.layer(layer).and_then(|layer|layer.mask.as_ref()).map(|mask|mask.surface.as_ref()).ok_or_else(stale)?,
    };
    let format=surface.format();
    if format.mode!=photocraft_doc::ColorMode::Grayscale || format.alpha
        || !matches!(format.sample,photocraft_doc::SampleType::U32|photocraft_doc::SampleType::F64) {return Ok(None)}
    if active.doc.channels.len()>10_000 {return Err(tl!("Too many channels for this clipboard action.").into());}
    idle(app)?;
    // This is target capture only. The registered backend independently validates read
    // storage or the stronger current write, source, render, lock and budget restrictions.
    Ok(Some(Target {document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade(),owner,command:command.to_owned()}))
}
/// Call immediately before registered dispatch, after any synchronous OS clipboard read.
pub(crate) fn request(app:&PhotocraftApp,target:&Target,mut params:Value)->Result<Value,String> {
    idle(app)?;
    let active=app.session.active().ok_or_else(stale)?;
    let snapshot=target.snapshot.upgrade().ok_or_else(stale)?;
    if active.doc.id!=target.document || active.revision!=target.revision || !active.doc.shares_state(&snapshot)
        || owner(app,&target.command,&params)?!=Some(target.owner) {return Err(stale())}
    if params.is_null() {params=json!({});}
    let object=params.as_object_mut().filter(|object|object.len()<=14).ok_or_else(||tl!("Invalid clipboard options.").to_owned())?;
    object.insert("documentId".into(),json!(target.document.0.to_string()));
    object.insert("expectedRevision".into(),json!(target.revision.to_string()));
    match target.owner {Owner::Mask(_)=>{object.insert("target".into(),json!("mask"));},Owner::Channel(index,_)=>{object.insert("target".into(),json!({"channel":index}));}}
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_clipboard_does_not_acquire_gray_capture() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        assert!(capture(&app,"edit.copy",&json!({})).unwrap().is_none());
    }
    #[test]
    fn narrow_imported_mask_retains_ordinary_clipboard_route() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let layer=app.session.active().unwrap().active_layer.unwrap();
        let document=app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        document.layer_mut(layer).unwrap().mask=Some(photocraft_doc::LayerMask::hide_all());
        document.xcf_state=Some(Default::default());
        app.ui.mask_target=true;
        for command in ["edit.copy","edit.cut","edit.paste"] {
            assert!(capture(&app,command,&json!({})).unwrap().is_none());
        }
    }
    #[test]
    fn mask_capture_is_frame_local_and_keeps_lossless_target_pair() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let layer=app.session.active().unwrap().active_layer.unwrap();
        let mut mask=photocraft_doc::LayerMask::hide_all();
        mask.surface=std::sync::Arc::new(photocraft_raster::Surface::new(photocraft_doc::PixelFormat {mode:photocraft_doc::ColorMode::Grayscale,sample:photocraft_doc::SampleType::F64,alpha:false}));
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().mask=Some(mask);
        app.ui.mask_target=true;

        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state=Some(Default::default());
        assert!(capture(&app,"edit.copy",&json!({})).unwrap().is_some());
        // Upstream Cut does not implicitly follow the shell mask target in Composite view.
        assert!(capture(&app,"edit.cut",&json!({})).unwrap().is_none());
        assert!(capture(&app,"edit.cut",&json!({"target":"mask"})).unwrap().is_some());
        assert!(capture(&app,"edit.paste",&json!({})).unwrap().is_some());
        // Candidate capture does not establish an editable canonical source; that remains
        // a registered backend decision for this deliberately unbound synthetic state.
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state=None;
        let target=capture(&app,"edit.paste",&json!({})).unwrap().unwrap();
        let params=request(&app,&target,json!({"center":[1,1]})).unwrap();
        assert_eq!(params["documentId"],json!(app.session.active().unwrap().doc.id.0.to_string()));
        assert_eq!(params["target"],json!("mask"));
        app.session.active_mut().unwrap().revision+=1;
        assert!(request(&app,&target,json!({})).is_err());
    }
}
