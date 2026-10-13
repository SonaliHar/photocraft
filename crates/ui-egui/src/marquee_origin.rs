//! Runtime-only origin of a newly drawn marquee. Geometry remains owned by canvas.
use crate::PhotocraftApp;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::{WeakDocumentSnapshot, channel_cmds::ChannelTarget};
use serde_json::Value;
#[derive(Clone, Debug)]
pub(super) struct Origin {
    document: DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    layer: Option<LayerId>,
    channel: ChannelTarget,
    mask: bool,
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let active = app.session.active().ok_or_else(|| tl!("Open a document before making a selection.").to_owned())?;
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || active.floating.is_some()
        || active.coalesce.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(tl!("Finish pending edits before making a selection.").into());
    }
    Ok(())
}
pub(super) fn capture(app: &PhotocraftApp) -> Result<Origin, String> {
    idle(app)?;
    let active = app.session.active().ok_or_else(|| tl!("Open a document before making a selection.").to_owned())?;
    Ok(Origin {
        document: active.doc.id,
        revision: active.revision,
        snapshot: active.doc.downgrade(),
        layer: active.active_layer,
        channel: active.channel_view.target,
        mask: app.layer_mask_targeted(),
    })
}
pub(super) fn validate(app: &PhotocraftApp, origin: Option<&Origin>) -> Result<(), String> {
    let stale = || tl!("The selection target changed. Draw the selection again.").to_owned();
    let origin = origin.ok_or_else(stale)?;
    idle(app)?;
    let active = app.session.active().ok_or_else(stale)?;
    let snapshot = origin.snapshot.upgrade().ok_or_else(stale)?;
    if active.doc.id != origin.document
        || active.revision != origin.revision
        || !active.doc.shares_state(&snapshot)
        || active.active_layer != origin.layer
        || active.channel_view.target != origin.channel
        || app.layer_mask_targeted() != origin.mask
    {
        return Err(stale());
    }
    Ok(())
}
pub(super) fn dispatch(app: &mut PhotocraftApp, origin: Option<&Origin>, params: Value) -> Result<Value, String> {
    dispatch_for(app, origin, "select.rect", params)
}
pub(super) fn dispatch_for(app: &mut PhotocraftApp, origin: Option<&Origin>, command: &str, mut params: Value) -> Result<Value, String> {
    if !matches!(command, "select.rect" | "select.lasso") {
        return Err(tl!("Invalid selection command.").into());
    }
    validate(app, origin)?;
    let origin = origin.ok_or_else(|| tl!("Draw the selection again before applying.").to_owned())?;
    let object = params.as_object_mut().ok_or_else(|| tl!("Invalid selection options.").to_owned())?;
    object.insert("documentId".into(), Value::String(origin.document.0.to_string()));
    object.insert("expectedRevision".into(), Value::String(origin.revision.to_string()));
    app.run(command, params)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn stale_revision_and_changed_channel_do_not_commit_marquee() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        let origin = capture(&app).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
        assert!(dispatch(&mut app, Some(&origin), json!({"x":0,"y":0,"width":2,"height":2})).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        app.session.active_mut().unwrap().revision += 1;
        assert!(validate(&app, Some(&origin)).is_err());
        assert!(validate(&app, None).is_err());
    }
}
