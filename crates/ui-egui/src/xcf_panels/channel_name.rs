//! Explicit saved-channel name/custom flag draft, separate from channel focus.
use crate::{PhotocraftApp, state::XcfChannelNameDraft, theme::Tokens};
use photocraft_doc::{
    DocId,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::json;
const STALE: &str = "xcf.channel.name.ui.stale-target";
struct Target {
    document: DocId,
    revision: u64,
    channel: ChannelId,
    snapshot: WeakDocumentSnapshot,
}
struct View {
    name: Option<String>,
    custom: bool,
}
fn bounded(name: &str) -> bool {
    name.len() <= 4096 && !name.contains('\0')
}
fn view(app: &PhotocraftApp, id: ChannelId) -> Option<View> {
    let active = app.session.active()?;
    let state = active.doc.xcf_state.as_ref()?;
    if id.0 == 0 || active.doc.channels.len() > 10_000 || state.items.len() > 10_000 || active.doc.selection_channel_id == Some(id) {
        return None;
    }
    let mut channels = active.doc.channels.iter().filter(|channel| channel.id == id);
    let channel = channels.next()?;
    if channels.next().is_some() || channel.name.len() > 1 << 20 {
        return None;
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Channel(id));
    let item = items.next()?;
    if items.next().is_some() || item.name.len() > 1 << 20 || item.name != channel.name {
        return None;
    }
    // A long retained source name is never silently truncated into an Apply value.
    let name = if bounded(&channel.name) {
        let mut name = String::new();
        name.try_reserve_exact(channel.name.len()).ok()?;
        name.push_str(&channel.name);
        Some(name)
    } else {
        None
    };
    Some(View { name, custom: item.custom_name })
}
fn capture(app: &PhotocraftApp, id: ChannelId) -> Option<Target> {
    view(app, id)?;
    let active = app.session.active()?;
    Some(Target { document: active.doc.id, revision: active.revision, channel: id, snapshot: active.doc.downgrade() })
}
fn permitted(app: &PhotocraftApp, id: ChannelId) -> bool {
    let Some(active) = app.session.active() else {
        return false;
    };
    if active.floating.is_some()
        || active.coalesce.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || app.session.has_jobs()
    {
        return false;
    }
    photocraft_engine::xcf_item_locks::require_channel_content(&active.doc, id).is_ok()
}
fn apply(app: &mut PhotocraftApp, target: &Target, draft: &XcfChannelNameDraft) -> Result<serde_json::Value, String> {
    if !bounded(&draft.name) || draft.name.is_empty() {
        return Err("xcf.channel.name.ui.name-limit".into());
    }
    let active = app.session.active().ok_or(STALE)?;
    let snapshot = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document
        || active.revision != target.revision
        || !active.doc.shares_state(&snapshot)
        || draft.document != target.document.0
        || draft.revision != target.revision
        || draft.channel != target.channel.0
    {
        return Err(STALE.into());
    }
    view(app, target.channel).ok_or(STALE)?;
    if !permitted(app, target.channel) {
        return Err("xcf.channel.name.ui.context-or-lock".into());
    }
    app.run(
        "xcf.channel.name.set",
        json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),
        "channelId":target.channel.0.to_string(),"name":draft.name,"customName":draft.custom_name}),
    )
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: ChannelId) {
    let Some(target) = capture(app, id) else {
        return;
    };
    let Some(current) = view(app, id) else {
        return;
    };
    if app
        .ui
        .xcf_channel_name_draft
        .as_ref()
        .is_none_or(|draft| draft.document != target.document.0 || draft.revision != target.revision || draft.channel != id.0 || !bounded(&draft.name))
    {
        app.ui.xcf_channel_name_draft = Some(XcfChannelNameDraft {
            document: target.document.0,
            revision: target.revision,
            channel: id.0,
            name: current.name.clone().unwrap_or_default(),
            custom_name: current.custom,
        });
    }
    let editable = permitted(app, id);
    let tokens = Tokens::get(ui.ctx());
    let mut submit = false;
    let mut reset = false;
    ui.push_id(("xcf-current-channel-name", target.document.0, id.0), |ui| {
        ui.separator();
        let label = ui.label(egui::RichText::new(tl!("Name:")).color(tokens.text));
        if current.name.is_none() {
            ui.label(tl!("Enter a replacement name for this long saved name."));
        }
        if let Some(draft) = app.ui.xcf_channel_name_draft.as_mut() {
            ui.add_enabled(editable, egui::TextEdit::singleline(&mut draft.name).char_limit(4096).desired_width(ui.available_width())).labelled_by(label.id);
            ui.add_enabled_ui(editable, |ui| {
                let _ = crate::widgets::checkbox(ui, &mut draft.custom_name, tl!("Custom"));
            });
            let changed = current.name.as_deref() != Some(draft.name.as_str()) || current.custom != draft.custom_name;
            ui.horizontal_wrapped(|ui| {
                submit = ui.add_enabled(editable && changed && bounded(&draft.name) && !draft.name.is_empty(), egui::Button::new(tl!("Apply"))).clicked();
                reset = ui.button(tl!("Reset")).clicked();
            });
        }
    });
    if reset {
        app.ui.xcf_channel_name_draft = Some(XcfChannelNameDraft {
            document: target.document.0,
            revision: target.revision,
            channel: id.0,
            name: current.name.unwrap_or_default(),
            custom_name: current.custom,
        });
    } else if submit && let Some(draft) = app.ui.xcf_channel_name_draft.clone() {
        match apply(app, &target, &draft) {
            Ok(_) => app.ui.xcf_channel_name_draft = None,
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PhotocraftApp, ChannelId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let doc = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let channel = photocraft_doc::AlphaChannel::new("Own retained name", photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8));
        let id = channel.id;
        doc.channels.push(channel);
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).unwrap();
        let bounds = doc.bounds();
        let item = state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.custom_name = false;
        item.raster_bounds = Some(bounds);
        assert_eq!(state.items.iter().filter(|item| item.item == ItemRef::Channel(id)).count(), 1);
        doc.xcf_state = Some(state);
        (app, id)
    }
    #[test]
    fn guarded_name_apply_changes_only_current_name_flag_and_preserves_channel_view() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let view = app.session.active().unwrap().channel_view.clone();
        let draft =
            XcfChannelNameDraft { document: target.document.0, revision: target.revision, channel: id.0, name: "Own edited name".into(), custom_name: true };
        apply(&mut app, &target, &draft).unwrap();
        let mut expected = before.as_ref().clone();
        expected.channels.first_mut().unwrap().name = draft.name.clone();
        let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.name = draft.name;
        item.custom_name = true;
        assert_eq!(app.session.active().unwrap().doc.as_ref(), &expected);
        assert_eq!(app.session.active().unwrap().channel_view, view);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
    #[test]
    fn stale_and_bad_restored_name_refuse_without_dispatch() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        let mut draft =
            XcfChannelNameDraft { document: target.document.0, revision: target.revision, channel: id.0, name: "Own stale".into(), custom_name: false };
        app.session.active_mut().unwrap().revision += 1;
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(apply(&mut app, &target, &draft).unwrap_err(), STALE);
        draft.name = "a".repeat(4097);
        assert!(apply(&mut app, &target, &draft).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
