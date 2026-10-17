//! Runtime-only Fill origin; restored view fields never authorize document edits.
use crate::PhotocraftApp;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::{WeakDocumentSnapshot, channel_cmds::ChannelTarget};
use serde_json::{Map, Value};
#[derive(Clone, Debug)]
pub(crate) struct FillOrigin {
    pub(super) dialog: u64,
    document: DocId,
    revision: u64,
    snapshot: WeakDocumentSnapshot,
    layer: Option<LayerId>,
    channel: ChannelTarget,
    mask: bool,
}
impl PartialEq for FillOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.dialog == other.dialog
            && self.document == other.document
            && self.revision == other.revision
            && self.layer == other.layer
            && self.channel == other.channel
            && self.mask == other.mask
            && match (self.snapshot.upgrade(), other.snapshot.upgrade()) {
                (Some(a), Some(b)) => a.shares_state(&b),
                (None, None) => true,
                _ => false,
            }
    }
}
fn idle(app: &PhotocraftApp) -> Result<(), String> {
    let state = app.session.active().ok_or_else(|| tl!("Open a document before filling.").to_owned())?;
    if app.ui.transform.is_some()
        || app.session.has_jobs()
        || state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_some())
    {
        return Err(tl!("Finish pending edits before filling.").into());
    }
    Ok(())
}
pub(super) fn capture(app: &PhotocraftApp, dialog: u64) -> Result<FillOrigin, String> {
    idle(app)?;
    let state = app.session.active().ok_or_else(|| tl!("Open a document before filling.").to_owned())?;
    Ok(FillOrigin {
        dialog,
        document: state.doc.id,
        revision: state.revision,
        snapshot: state.doc.downgrade(),
        layer: state.active_layer,
        channel: state.channel_view.target,
        mask: app.layer_mask_targeted(),
    })
}
pub(super) fn validate(app: &PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<Value, String> {
    let origin =
        app.ui.fill_origin.as_ref().filter(|origin| origin.dialog == dialog).ok_or_else(|| tl!("Reopen the Fill dialog before applying.").to_owned())?;
    idle(app)?;
    let state = app.session.active().ok_or_else(|| tl!("Open a document before filling.").to_owned())?;
    let snapshot = origin.snapshot.upgrade().ok_or_else(|| tl!("The Fill target changed. Reopen the dialog.").to_owned())?;
    if state.doc.id != origin.document
        || state.revision != origin.revision
        || !state.doc.shares_state(&snapshot)
        || state.active_layer != origin.layer
        || state.channel_view.target != origin.channel
        || app.layer_mask_targeted() != origin.mask
    {
        return Err(tl!("The Fill target changed. Reopen the dialog.").into());
    }
    if fields.len() > 32
        || fields.iter().any(|(key, value)| key.len() > 128 || (!key.starts_with("__") && matches!(value,Value::String(text) if text.len()>4096)))
    {
        return Err(tl!("Invalid Fill options.").into());
    }
    for key in super::KEYS {
        if let Some(value) = fields.get(key)
            && !super::valid(app, key, value)
        {
            return Err(tl!("Invalid Fill options.").into());
        }
    }
    let mut params = super::params(fields);
    // Only the newly controlled wide Gray lane accepts these runtime selector keys.
    let surface = if origin.mask || origin.channel == ChannelTarget::LayerMask {
        origin.layer.and_then(|id| state.doc.layer(id)).and_then(|layer| layer.mask.as_ref()).map(|mask| mask.surface.as_ref())
    } else {
        match origin.channel {
            ChannelTarget::Alpha(index) => state.doc.channels.get(index).map(|channel| &channel.surface),
            ChannelTarget::Composite => state.doc.quick_mask.as_ref().map(|channel| &channel.surface),
            _ => None,
        }
    };
    if surface.is_some_and(|surface| matches!(surface.format().sample, photocraft_doc::SampleType::U32 | photocraft_doc::SampleType::F64))
        && let Some(object) = params.as_object_mut()
    {
        object.insert("documentId".into(), Value::String(origin.document.0.to_string()));
        object.insert("expectedRevision".into(), Value::String(origin.revision.to_string()));
    }
    Ok(params)
}
