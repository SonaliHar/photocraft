//! Own saved-channel lock checkboxes, independent of channel visibility/view mode.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    DocId,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_engine::{
    WeakDocumentSnapshot,
    channel_cmds::ChannelTarget,
    xcf_item_locks::{self, EditKind},
};
use serde_json::{Value, json};

const COMMAND: &str = "xcf.channel.locks.set";
const STALE: &str = "xcf.channel.locks.ui.stale-target";
struct Target {
    document: DocId,
    revision: u64,
    channel: ChannelId,
    snapshot: WeakDocumentSnapshot,
}
struct View {
    target: Target,
    content: bool,
    visibility: bool,
    parent: Option<ItemRef>,
    inherited_content: bool,
    inherited_visibility: bool,
}

fn current(app: &PhotocraftApp, id: ChannelId) -> Option<View> {
    let active = app.session.active()?;
    let xcf = active.doc.xcf_state.as_ref()?;
    if id.0 == 0
        || active.doc.id.0 == 0
        || active.doc.resident_arc().is_none()
        || active.doc.channels.len() > 10_000
        || xcf.items.len() > 10_000
        || active.doc.selection_channel_id == Some(id)
    {
        return None;
    }
    let ChannelTarget::Alpha(index) = active.channel_view.target else {
        return None;
    };
    if active.doc.channels.get(index)?.id != id {
        return None;
    }
    let mut native = active.doc.channels.iter().filter(|channel| channel.id == id);
    native.next()?;
    if native.next().is_some() {
        return None;
    }
    let mut items = xcf.items.iter().filter(|item| item.item == ItemRef::Channel(id));
    let item = items.next()?;
    if items.next().is_some() {
        return None;
    }
    // Pure bounded typed ancestry query: no validator payload hashing, samples or render.
    xcf_item_locks::effective_item_locks(&active.doc, ItemRef::Channel(id)).ok()?;
    let inherited = item.parent.map(|parent| xcf_item_locks::effective_item_locks(&active.doc, parent)).transpose().ok()?;
    let inherited_content = inherited.is_some_and(|locks| locks.require(EditKind::Content).is_err());
    let inherited_visibility = inherited.is_some_and(|locks| locks.require(EditKind::Visibility).is_err());
    Some(View {
        target: Target { document: active.doc.id, revision: active.revision, channel: id, snapshot: active.doc.downgrade() },
        content: item.locks.content,
        visibility: item.locks.visibility,
        parent: item.parent,
        inherited_content,
        inherited_visibility,
    })
}
fn idle(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && !app.session.has_jobs()
        && app.session.active().is_some_and(|active| {
            active.floating.is_none()
                && active.coalesce.is_none()
                && active.revision < u64::MAX
                && active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_none())
        })
        && photocraft_engine::commands::find(COMMAND).is_some_and(|command| (command.enabled)(&app.session).is_ok())
}
fn apply(app: &mut PhotocraftApp, target: &Target, key: &str, value: bool) -> Result<Value, String> {
    if !matches!(key, "content" | "visibility") {
        return Err("xcf.channel.locks.ui.unsupported-bit".into());
    }
    let active = app.session.active().ok_or(STALE)?;
    let snapshot = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !active.doc.shares_state(&snapshot) {
        return Err(STALE.into());
    }
    current(app, target.channel).ok_or(STALE)?;
    if !idle(app) {
        return Err("xcf.channel.locks.ui.busy-or-preview".into());
    }
    let mut params =
        json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),"channelId":target.channel.0.to_string()});
    params.as_object_mut().ok_or(STALE)?.insert(key.into(), json!(value));
    app.run(COMMAND, params)
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: ChannelId) {
    let Some(view) = current(app, id) else {
        return;
    };
    let enabled = idle(app);
    let tokens = Tokens::get(ui.ctx());
    let mut action = None;
    ui.push_id(("xcf-current-channel-locks", view.target.document.0, id.0), |ui| {
        ui.separator();
        ui.label(egui::RichText::new(tl!("Lock:")).color(tokens.text));
        // Own flags remain editable even when a parent restricts actual content or
        // saved visibility. Clearing these checkboxes never clears parent flags.
        ui.add_enabled_ui(enabled, |ui| {
            for (key, label, flag) in [("content", tl!("Lock image pixels"), view.content), ("visibility", tl!("Visibility"), view.visibility)] {
                let mut own = flag;
                if crate::widgets::checkbox(ui, &mut own, label).changed() && action.is_none() {
                    action = Some((key, own));
                }
            }
        });
        if (view.inherited_content || view.inherited_visibility)
            && let Some(parent) = view.parent
        {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(tl!("Layer")).color(tokens.text_dim));
                ui.monospace(parent.raw().to_string());
                ui.label(tl!("Lock:"));
                if view.inherited_content {
                    ui.label(tl!("Lock image pixels"));
                }
                if view.inherited_visibility {
                    ui.label(tl!("Visibility"));
                }
            });
        }
    });
    if let Some((key, value)) = action
        && let Err(error) = apply(app, &view.target, key, value)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    fn fixture() -> (PhotocraftApp, ChannelId) {
        let mut doc = photocraft_doc::Document::with_background(
            "Own channel lock UI",
            photocraft_doc::Size::new(2, 2),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U16,
            photocraft_doc::Color::WHITE,
        );
        let channel = photocraft_doc::AlphaChannel::new(
            "own saved channel",
            photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::U16, false)),
        );
        let id = channel.id;
        doc.channels.push(channel);
        let mut xcf = photocraft_doc::XcfState::default();
        xcf.reconcile_layers(&doc).unwrap();
        let item = xcf.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.source_flags = 0x12345678;
        item.visible = true;
        doc.xcf_state = Some(xcf);
        let mut session = photocraft_engine::Session::new();
        session.add_document(doc, None).unwrap();
        session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "Own synthetic channel UI source", None);
        (PhotocraftApp::new(session, Default::default()), id)
    }
    #[test]
    fn checkbox_dispatches_one_undo_and_actual_consumer_respects_content() {
        let (app, id) = fixture();
        let before = app.session.active().unwrap().doc.clone();
        let view = app.session.active().unwrap().channel_view.clone();
        let source = app.session.active().unwrap().source.clone();
        let entries = app.session.active().unwrap().history.entry_labels().count();
        let mut harness = Harness::new_ui_state(|ui, app| show(app, ui, id), app);
        harness.get_by_label("Lock image pixels").click();
        harness.run();
        let mut expected = (*before).clone();
        expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().locks.content = true;
        let active = harness.state().session.active().unwrap();
        assert_eq!(*active.doc, expected);
        assert_eq!(active.channel_view, view);
        assert_eq!(active.source, source);
        assert_eq!(active.history.entry_labels().count(), entries + 1);
        assert!(xcf_item_locks::require_channel_content(&active.doc, id).is_err());
        let native = photocraft_format::save_to_bytes(&active.doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
        assert!(harness.state_mut().session.undo());
        assert_eq!(harness.state().session.active().unwrap().doc, before);
    }
    #[test]
    fn own_unlock_cannot_remove_inherited_content_permission() {
        let (mut app, id) = fixture();
        let doc = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let group = photocraft_doc::Layer::group("Own actual parent", Vec::new());
        let parent = group.id;
        doc.layers.push(group);
        let mut state = doc.xcf_state.take().unwrap();
        state.reconcile_layers(doc).unwrap();
        let parent_position = state.items.iter().position(|item| item.item == ItemRef::Layer(parent)).unwrap();
        let parent_item = state.items.remove(parent_position);
        state.items.insert(0, parent_item);
        doc.xcf_state = Some(state);
        let xcf = doc.xcf_state.as_mut().unwrap();
        xcf.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.content = true;
        let item = xcf.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.parent = Some(ItemRef::Layer(parent));
        item.locks.content = true;
        doc.validate_xcf_state().unwrap();
        let view = current(&app, id).unwrap();
        assert!(view.inherited_content);
        apply(&mut app, &view.target, "content", false).unwrap();
        assert!(xcf_item_locks::require_channel_content(&app.session.active().unwrap().doc, id).is_err());
        assert!(current(&app, id).unwrap().inherited_content);
    }
    #[test]
    fn switched_channel_stale_revision_and_coalescing_refuse_without_history_changes() {
        let (mut app, id) = fixture();
        let target = current(&app, id).unwrap().target;
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        assert!(apply(&mut app, &target, "content", true).is_err());
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        app.session.active_mut().unwrap().revision += 1;
        assert!(apply(&mut app, &target, "visibility", true).is_err());
        app.session.active_mut().unwrap().revision = revision;
        app.session.active_mut().unwrap().coalesce = Some("own in-progress interaction".into());
        assert!(apply(&mut app, &target, "content", true).is_err());
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn real_floating_preview_is_retained_before_any_checkbox_dispatch() {
        let (mut app, id) = fixture();
        let target = current(&app, id).unwrap().target;
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
        app.session.execute("select.all", json!({})).unwrap();
        app.session.execute("select.float", json!({"dx":1,"dy":0,"copy":true})).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let active = app.session.active().unwrap();
        let before = active.doc.clone();
        let preview = active.floating.clone().unwrap();
        let revision = active.revision;
        assert!(apply(&mut app, &target, "visibility", true).is_err());
        let active = app.session.active().unwrap();
        assert!(active.doc.shares_state(&before));
        assert_eq!(active.revision, revision);
        let retained = active.floating.as_ref().unwrap();
        assert_eq!((retained.layer, retained.offset, retained.revision), (preview.layer, preview.offset, preview.revision));
        assert!(std::sync::Arc::ptr_eq(&retained.parts, &preview.parts));
    }
}
