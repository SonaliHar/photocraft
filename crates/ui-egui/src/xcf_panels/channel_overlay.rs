//! Current canonical channel display overlay, separate from runtime eye/target state.
use crate::{PhotocraftApp, theme::Tokens};
use photocraft_doc::{
    DocId,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_engine::WeakDocumentSnapshot;
use serde_json::json;
const STALE: &str = "xcf.channel.overlay.stale-target";
struct Target {
    document: DocId,
    revision: u64,
    channel: ChannelId,
    snapshot: WeakDocumentSnapshot,
}
fn values(app: &PhotocraftApp, id: ChannelId) -> Option<([f32; 3], f32)> {
    let active = app.session.active()?;
    let doc = &active.doc;
    let state = doc.xcf_state.as_ref()?;
    if id.0 == 0 || doc.channels.len() > 10_000 || state.items.len() > 10_000 || doc.selection_channel_id == Some(id) {
        return None;
    }
    let mut metadata = state.items.iter().filter(|item| item.item == ItemRef::Channel(id));
    let item = metadata.next()?;
    if metadata.next().is_some() || item.mask.is_some() || item.raster_bounds.is_none_or(|bounds| bounds.is_empty()) {
        return None;
    }
    let mut channels = doc.channels.iter().filter(|channel| channel.id == id);
    let channel = channels.next()?;
    if channels.next().is_some()
        || channel.spot.is_some()
        || channel.color.mode != photocraft_doc::ColorMode::Rgb
        || channel.color.alpha.to_bits() != 1.0f32.to_bits()
        || channel.color.c.get(3)?.to_bits() != 0.0f32.to_bits()
        || channel.color.c.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || !channel.opacity.is_finite()
        || !(0.0..=1.0).contains(&channel.opacity)
        || channel.surface.format().mode != photocraft_doc::ColorMode::Grayscale
        || channel.surface.format().alpha
        || channel.surface.format().sample != doc.depth
    {
        return None;
    }
    Some(([*channel.color.c.first()?, *channel.color.c.get(1)?, *channel.color.c.get(2)?], channel.opacity))
}
fn capture(app: &PhotocraftApp, id: ChannelId) -> Option<Target> {
    values(app, id)?;
    let active = app.session.active()?;
    Some(Target { document: active.doc.id, revision: active.revision, channel: id, snapshot: active.doc.downgrade() })
}
fn apply(app: &mut PhotocraftApp, target: &Target, color: [f32; 3], opacity: f32) -> Result<(), String> {
    let active = app.session.active().ok_or(STALE)?;
    let snapshot = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !active.doc.shares_state(&snapshot) {
        return Err(STALE.into());
    }
    if active.floating.is_some() {
        return Err("xcf.channel.overlay.floating".into());
    }
    if app.session.has_jobs() {
        return Err("xcf.channel.overlay.context".into());
    }
    values(app, target.channel).ok_or(STALE)?;
    if color.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) || !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err("xcf.channel.overlay.range".into());
    }
    // Revalidation and registered dispatch are immediate: no refocus or async gap.
    app.run(
        "xcf.channel.overlay.set",
        json!({"channelId":target.channel.0.to_string(),
        "colorF32Bits":color.map(|number|format!("{:08x}",number.to_bits())),"opacityF32Bits":format!("{:08x}",opacity.to_bits())}),
    )
    .map(|_| ())
}
fn apply_indicates(app: &mut PhotocraftApp, target: &Target, indicates: photocraft_doc::ColorIndicates) -> Result<(), String> {
    let active = app.session.active().ok_or(STALE)?;
    let snapshot = target.snapshot.upgrade().ok_or(STALE)?;
    if active.doc.id != target.document || active.revision != target.revision || !active.doc.shares_state(&snapshot) {
        return Err(STALE.into());
    }
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err("xcf.channel.indicates.set.floating".into());
    }
    if app.session.has_jobs() || active.coalesce.is_some() {
        return Err("xcf.channel.indicates.set.context".into());
    }
    values(app, target.channel).ok_or(STALE)?;
    photocraft_engine::xcf_item_locks::require_channel_content(&active.doc, target.channel).map_err(|error| error.to_string())?;
    app.run(
        "xcf.channel.indicates.set",
        json!({"documentId":target.document.0.to_string(),"expectedRevision":target.revision.to_string(),
        "channelId":target.channel.0.to_string(),"indicates":if indicates==photocraft_doc::ColorIndicates::MaskedAreas {"masked"}else{"selected"}}),
    )
    .map(|_| ())
}
fn parse(text: &str) -> Option<f64> {
    if text.len() > 64 {
        return None;
    }
    let value = text.trim().parse::<f32>().ok()?;
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(f64::from(value))
}
fn field(ui: &mut egui::Ui, value: &mut f32, label: &str, enabled: bool) -> bool {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        ui.add_enabled(
            enabled,
            egui::DragValue::new(value)
                .range(0.0..=1.0)
                .clamp_existing_to_range(false)
                .speed(0.01)
                .custom_formatter(|value, _| (value as f32).to_string())
                .custom_parser(parse)
                .update_while_editing(false),
        )
        .labelled_by(label.id)
        .changed()
    })
    .inner
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: ChannelId) {
    let Some(target) = capture(app, id) else {
        return;
    };
    let Some((mut color, mut opacity)) = values(app, id) else {
        return;
    };
    let active = app.session.active();
    let editable = active.is_some_and(|active| active.floating.is_none())
        && !app.session.has_jobs()
        && active.is_some_and(|active| {
            photocraft_engine::xcf_item_locks::effective_item_locks(&active.doc, ItemRef::Channel(id))
                .is_ok_and(|locks| locks.require(photocraft_engine::xcf_item_locks::EditKind::Content).is_ok())
        });
    let Some(current_indicates) = active.and_then(|active| active.doc.channels.iter().find(|channel| channel.id == id).map(|channel| channel.indicates)) else {
        return;
    };
    let mut indicates = if current_indicates == photocraft_doc::ColorIndicates::MaskedAreas { "masked".to_owned() } else { "selected".to_owned() };
    let indicates_editable =
        editable && active.is_some_and(|active| active.coalesce.is_none() && active.doc.xcf_state.as_ref().is_none_or(|state| state.floating.is_none()));
    let mut indicates_changed = false;
    let tokens = Tokens::get(ui.ctx());
    let changed = ui
        .push_id(("xcf-channel-overlay", target.document.0, id.0), |ui| {
            ui.separator();
            ui.label(egui::RichText::new(tl!("Overlay Color")).color(tokens.text));
            let mut changed = false;
            for (value, label) in color.iter_mut().zip([tl!("Red"), tl!("Green"), tl!("Blue")]) {
                changed |= field(ui, value, label, editable);
            }
            changed |= field(ui, &mut opacity, &format!("{} (0–1)", tl!("Overlay Opacity")), editable);
            // Reuse the native Channel Options labels and translation lookup behavior.
            // These describe interpretation, never modify the stored grayscale samples.
            let masked_label = "Color Indicates Masked Areas";
            let selected_label = "Color Indicates Selected Areas";
            let options = [("masked".to_owned(), tl!(masked_label)), ("selected".to_owned(), tl!(selected_label))];
            let width = ui.available_width().min(260.0);
            indicates_changed =
                ui.add_enabled_ui(indicates_editable, |ui| crate::widgets::dropdown(ui, "xcf-channel-indicates", &mut indicates, &options, width)).inner;
            changed
        })
        .inner;
    if changed && let Err(error) = apply(app, &target, color, opacity) {
        app.ui.status = error;
        app.ui.status_error = true;
        return;
    }
    if indicates_changed {
        // Two independent explicit edits in one event batch remain two commands.
        // A successful color edit advances revision; recapture that same document
        // and channel rather than issuing a knowingly stale interpretation request.
        let refreshed = if changed { capture(app, id).filter(|new| new.document == target.document) } else { Some(target) };
        let result = refreshed.ok_or_else(|| STALE.to_owned()).and_then(|target| {
            apply_indicates(
                app,
                &target,
                if indicates == "masked" { photocraft_doc::ColorIndicates::MaskedAreas } else { photocraft_doc::ColorIndicates::SelectedAreas },
            )
        });
        if let Err(error) = result {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_parser_preserves_f32_bits_and_refuses_hostile_input() {
        for bits in [0u32, 0x80000000, 0x3e800001, 0x3f000001, 0x3f800000] {
            let value = f32::from_bits(bits);
            assert_eq!((parse(&value.to_string()).unwrap() as f32).to_bits(), bits);
        }
        for text in ["NaN", "inf", "-0.01", "1.00001", ""] {
            assert!(parse(text).is_none());
        }
        assert!(parse(&"1".repeat(65)).is_none());
    }
    fn fixture() -> (PhotocraftApp, ChannelId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let doc = app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let channel = photocraft_doc::AlphaChannel::new("Own channel", photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8));
        let id = channel.id;
        doc.channels.push(channel);
        doc.xcf_state = Some(photocraft_doc::XcfState {
            items: vec![photocraft_doc::xcf_state::ItemState {
                item: ItemRef::Channel(id),
                name: "Own channel".into(),
                custom_name: true,
                visible: false,
                locks: Default::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(doc.bounds()),
                mask: None,
                composition: None,
            }],
            ..Default::default()
        });
        let mut state = doc.xcf_state.take().unwrap();
        state.reconcile_layers(doc).unwrap();
        doc.xcf_state = Some(state);
        (app, id)
    }
    #[test]
    fn captured_overlay_apply_changes_only_current_bits_without_refocus() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let view = app.session.active().unwrap().channel_view.clone();
        let color = [f32::from_bits(0x3e800001), f32::from_bits(0x80000000), 1.0];
        let opacity = f32::from_bits(0x3f000001);
        apply(&mut app, &target, color, opacity).unwrap();
        let mut expected = before.as_ref().clone();
        let channel = expected.channels.first_mut().unwrap();
        channel.color = photocraft_doc::Color::rgb(color[0], color[1], color[2]);
        channel.opacity = opacity;
        assert_eq!(app.session.active().unwrap().doc.as_ref(), &expected);
        assert_eq!(app.session.active().unwrap().channel_view, view);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
    #[test]
    fn stale_overlay_target_never_dispatches_or_changes_document() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        app.session.active_mut().unwrap().revision += 1;
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        assert_eq!(apply(&mut app, &target, [0.0, 1.0, 0.0], 0.75).unwrap_err(), STALE);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
        assert_eq!(app.session.active().unwrap().revision, revision);
    }

    #[test]
    fn indicates_control_preserves_overlay_samples_and_view_one_undo() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let view = app.session.active().unwrap().channel_view.clone();
        apply_indicates(&mut app, &target, photocraft_doc::ColorIndicates::SelectedAreas).unwrap();
        let mut expected = before.as_ref().clone();
        expected.channels.first_mut().unwrap().indicates = photocraft_doc::ColorIndicates::SelectedAreas;
        assert_eq!(app.session.active().unwrap().doc.as_ref(), &expected);
        assert_eq!(app.session.active().unwrap().channel_view, view);
        assert_eq!(app.session.active().unwrap().doc.channels.first().unwrap().overlay_coverage(0.25), 0.25);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
    #[test]
    fn indicates_stale_target_refuses_before_registered_dispatch() {
        let (mut app, id) = fixture();
        let target = capture(&app, id).unwrap();
        app.session.active_mut().unwrap().revision += 1;
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(apply_indicates(&mut app, &target, photocraft_doc::ColorIndicates::SelectedAreas).unwrap_err(), STALE);
        assert!(app.session.active().unwrap().doc.shares_state(&before));
    }
}
