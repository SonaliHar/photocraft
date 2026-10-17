//! Bounded metadata-only saved-channel management. Pixels remain COW owners.
use super::{ChannelTarget, ChannelView};
use crate::{EngineError, Result, Session};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
const MAX_CHANNELS: usize = 10_000;
fn error(command: &str, message: impl std::fmt::Display) -> EngineError {
    super::bad(command, message.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
#[derive(Clone, Copy)]
enum Edit<'a> {
    Rename(&'a str),
    Move(usize),
    Delete,
}
struct Request<'a> {
    index: usize,
    id: ChannelId,
    edit: Edit<'a>,
}
fn request<'a>(session: &Session, command: &str, params: &'a Value) -> Result<Request<'a>> {
    if !matches!(command, "channel.rename" | "channel.move" | "channel.delete") {
        return Err(error(command, "unknown channel management command"));
    }
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 5
                && object.keys().all(|key| {
                    matches!(key.as_str(), "channel" | "channelId" | "documentId" | "expectedRevision")
                        || key
                            == if command == "channel.rename" {
                                "name"
                            } else if command == "channel.move" {
                                "to"
                            } else {
                                "channel"
                            }
                })
        })
    {
        return Err(error(command, "expected bounded channel management parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let document = &state.doc;
    if document.channels.len() > MAX_CHANNELS || state.channel_view.alpha_visible.len() > MAX_CHANNELS || state.channel_view.color_hidden.len() > 16 {
        return Err(error(command, "channel/view count limit"));
    }
    if document.xcf_state.as_ref().is_some_and(|state| state.items.len() > MAX_CHANNELS) {
        return Err(error(command, "current item count limit"));
    }
    if params.get("channel").is_some() && params.get("channelId").is_some() {
        return Err(error(command, "supply exactly one channel selector"));
    }
    if let Some(value) = params.get("channel") {
        let valid = match value {
            Value::Number(number) => number.as_u64().is_some(),
            Value::String(text) => text.len() <= 4096 && !text.contains('\0'),
            Value::Object(object) => {
                object.len() == 1
                    && object.iter().all(|(key, value)| match key.as_str() {
                        "channelId" => super::channel_identity(value).is_some(),
                        "alpha" => value.as_u64().is_some(),
                        _ => false,
                    })
            }
            _ => false,
        };
        if !valid {
            return Err(error(command, "invalid bounded channel selector"));
        }
    }
    if params.get("channelId").is_some_and(|value| super::channel_identity(value).is_none()) {
        return Err(error(command, "invalid stable channelId"));
    }
    let index = if super::parameter_ref(params, "channel").is_some() {
        super::alpha_index(document, params, "channel", command)?
    } else if command == "channel.delete" {
        match state.channel_view.target {
            ChannelTarget::Alpha(index) => index,
            _ => return Err(error(command, "missing channel")),
        }
    } else {
        return Err(error(command, "missing channel"));
    };
    let channel = document.channels.get(index).ok_or_else(|| error(command, "channel index out of range"))?;
    if channel.id.0 == 0 || document.channels.iter().filter(|value| value.id == channel.id).count() != 1 {
        return Err(error(command, "zero/ambiguous channel identity"));
    }
    let edit = match command {
        "channel.rename" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| name.len() <= 4096 && !name.contains('\0'))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| error(command, "missing or oversized name"))?;
            Edit::Rename(name)
        }
        "channel.move" => {
            let to = params.get("to").and_then(Value::as_u64).ok_or_else(|| error(command, "missing unsigned destination index"))?;
            Edit::Move(usize::try_from(to).unwrap_or(usize::MAX).min(document.channels.len().saturating_sub(1)))
        }
        _ => Edit::Delete,
    };
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(document.id.0) && document.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error(command, "captured document/revision changed or incomplete")),
    }
    Ok(Request { index, id: channel.id, edit })
}
pub(crate) fn dispatch_preflight(session: &Session, command: &str, params: &Value, _background: bool) -> Result<()> {
    let _ = request(session, command, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || (command != "channel.rename" && state.revision >= u64::MAX - 1)
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_some())
    {
        return Err(error(command, "idle uncoalesced channel management context required"));
    }
    Ok(())
}
// Ordinary synthetic/current canonical rows may be deleted with their real channel.
// A retained original source row is not permission for this narrower lane.
fn ordinary_current_channel_row(document: &Document, id: ChannelId, control: &Interrupt<'_>) -> Result<Option<usize>> {
    let Some(state) = &document.xcf_state else { return Ok(None) };
    if document.source_color_context.is_some()
        || state.provenance != Default::default()
        || !state.source_items.is_empty()
        || !state.resources.is_empty()
        || !state.opaque.is_empty()
    {
        return Ok(None);
    }
    let mut found = None;
    for (at, item) in state.items.iter().enumerate() {
        poll(control)?;
        if item.item == ItemRef::Channel(id) {
            if found.is_some()
                || item.parent.is_some()
                || item.mask.is_some()
                || item.composition.is_some()
                || item.source_tattoo.is_some()
                || item.source_flags != 0
            {
                return Ok(None);
            }
            found = Some(at);
        }
    }
    Ok(found)
}
fn relation(document: &Document, id: ChannelId, edit: Edit<'_>, command: &str, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if document.selection_channel_id == Some(id) {
        return Err(error(command, "selection channel is not a saved channel"));
    }
    crate::xcf_item_locks::require_channel_content(document, id)?;
    let Some(state) = &document.xcf_state else {
        return Ok(());
    };
    // Bound every retained relation before checking inherited locks or cloning.
    if [
        state.items.len(),
        state.source_items.len(),
        state.filter_stacks.len(),
        state.resources.len(),
        state.item_sets.len(),
        state.filter_masks.len(),
        state.editable_indexed.len(),
    ]
    .into_iter()
    .any(|count| count > MAX_CHANNELS)
        || state.opaque.len() > 100_000
    {
        return Err(error(command, "retained channel relation limit"));
    }
    let owner = ItemRef::Channel(id);
    let deleting = matches!(edit, Edit::Delete);
    let mut nodes = 0usize;
    for stack in &state.filter_stacks {
        poll(control)?;
        nodes = nodes.checked_add(stack.nodes.len()).filter(|nodes| *nodes <= 100_000).ok_or_else(|| error(command, "filter node work limit"))?;
    }
    let mut visits = 0usize;
    let mut reference = |value: ItemRef| -> Result<()> {
        poll(control)?;
        visits = visits.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| error(command, "reference work limit"))?;
        if deleting && value == owner {
            return Err(error(command, "deleting referenced imported channel metadata is unsupported; retained source and relationships must remain intact"));
        }
        Ok(())
    };
    let own_row = if deleting { ordinary_current_channel_row(document, id, control)? } else { None };
    for (at, item) in state.items.iter().enumerate() {
        if Some(at) != own_row {
            reference(item.item)?;
        }
        if let Some(parent) = item.parent {
            reference(parent)?;
        }
        if let Some(mask) = &item.mask {
            reference(mask.owner)?;
        }
    }
    for source in &state.source_items {
        reference(source.owner)?;
    }
    for stack in &state.filter_stacks {
        reference(stack.owner)?;
        for node in &stack.nodes {
            poll(control)?;
            if let Some(mask) = node.mask {
                reference(mask)?;
            }
            for (_, input) in &node.auxiliary {
                reference(*input)?;
            }
        }
    }
    for value in state.selection.selected.iter().chain(&state.selection.selected_paths) {
        reference(*value)?;
    }
    for value in [state.selection.active, state.selection.mask_target].into_iter().flatten() {
        reference(value)?;
    }
    for set in &state.item_sets {
        for value in &set.members {
            reference(*value)?;
        }
    }
    for opaque in &state.opaque {
        if let Some(owner) = opaque.owner {
            reference(owner)?;
        }
    }
    if let Some(floating) = &state.floating {
        reference(floating.target)?;
    }
    if let Some(item) = state.items.iter().find(|item| item.item == owner) {
        let channel = document.channels.iter().find(|channel| channel.id == id).ok_or_else(|| error(command, "missing current channel"))?;
        if item.name != channel.name {
            return Err(error(command, "native and canonical channel names disagree"));
        }
        let locks = state.effective_locks(owner).map_err(|e| error(command, e))?;
        if matches!(edit, Edit::Move(_)) && locks.position {
            return Err(error(command, "imported channel position locked"));
        }
    }
    document.validate_xcf_state_with(control).map_err(|e| error(command, e))?;
    poll(control)
}
fn owned(name: &str, command: &str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve_exact(name.len()).map_err(|_| error(command, "name allocation failed"))?;
    output.push_str(name);
    Ok(output)
}
fn view_copy(view: &ChannelView, command: &str) -> Result<ChannelView> {
    let copy = |values: &[bool]| -> Result<Vec<bool>> {
        let mut output = Vec::new();
        output.try_reserve_exact(values.len()).map_err(|_| error(command, "channel view allocation failed"))?;
        output.extend_from_slice(values);
        Ok(output)
    };
    Ok(ChannelView {
        target: view.target,
        color_hidden: copy(&view.color_hidden)?,
        alpha_visible: copy(&view.alpha_visible)?,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
pub(crate) fn run(session: &mut Session, command: &str, params: &Value) -> Result<Value> {
    dispatch_preflight(session, command, params, false)?;
    let request = request(session, command, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let before = state.doc.clone();
    let revision = state.revision;
    let noop = match request.edit {
        Edit::Move(to) => to == request.index,
        Edit::Rename(name) => before.channels.get(request.index).is_some_and(|channel| channel.name == name),
        Edit::Delete => false,
    };
    if noop {
        return Ok(match request.edit {
            Edit::Move(_) => json!({"channel":request.index}),
            _ => Value::Null,
        });
    }
    let name = match request.edit {
        Edit::Rename(name) => Some(owned(name, command)?),
        _ => None,
    };
    let edit = match request.edit {
        Edit::Move(to) => Some(to),
        _ => None,
    };
    let deleting = matches!(request.edit, Edit::Delete);
    let response = if let Some(to) = edit { json!({"channel":to}) } else { Value::Null };
    let index = request.index;
    let id = request.id;
    let prior_view = view_copy(&state.channel_view, command)?;
    let work_view = view_copy(&prior_view, command)?;
    let baseline = before.clone();
    let command = owned(command, command)?;
    let label = if deleting {
        "Delete Channel"
    } else if edit.is_some() {
        "Channel Order"
    } else {
        "Rename Channel"
    };
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                let edit = if deleting {
                    Edit::Delete
                } else if let Some(to) = edit {
                    Edit::Move(to)
                } else {
                    Edit::Rename(name.as_deref().ok_or_else(|| error(&command, "missing prepared name"))?)
                };
                relation(&before, id, edit, &command, control)?;
                let limits = photocraft_io::NativeSaveLimits::default().preparation;
                let model = photocraft_io::retained_native_model_bytes(&before, limits, control).map_err(|e| error(&command, e))? as u64;
                let payload = photocraft_io::retained_native_payload_bytes(&before, limits, control).map_err(|e| error(&command, e))? as u64;
                let raster = before.resident_raster_footprint_with(control)?;
                let peak = model
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(payload))
                    .and_then(|n| n.checked_add(raster.retained_bytes))
                    .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
                    .and_then(|n| n.checked_add(128 * 1024))
                    .ok_or_else(|| error(&command, "metadata residency overflow"))?;
                if peak > MAX_CHECKED_RASTER_BYTES {
                    return Err(error(&command, "channel metadata/source residency limit"));
                }
                poll(control)?;
                let mut candidate = (*before).clone();
                if candidate.channels.get(index).is_none_or(|channel| channel.id != id) {
                    return Err(error(&command, "stale channel identity"));
                }
                if deleting {
                    let own_row = ordinary_current_channel_row(&before, id, control)?;
                    candidate.channels.remove(index);
                    if let Some(at) = own_row {
                        let state = candidate.xcf_state.as_mut().ok_or_else(|| error(&command, "missing current channel metadata"))?;
                        if state.items.get(at).is_none_or(|item| item.item != ItemRef::Channel(id)) {
                            return Err(error(&command, "stale current channel metadata"));
                        }
                        poll(control)?;
                        state.items.remove(at);
                        poll(control)?;
                    }
                } else if let Some(to) = edit_index(edit) {
                    let channel = candidate.channels.remove(index);
                    candidate.channels.insert(to, channel);
                } else if let Edit::Rename(name) = edit {
                    candidate.channels.get_mut(index).ok_or_else(|| error(&command, "missing current channel"))?.name = owned(name, &command)?;
                    if let Some(xcf) = &mut candidate.xcf_state {
                        for item in &mut xcf.items {
                            poll(control)?;
                            if item.item == ItemRef::Channel(id) {
                                item.name = owned(name, &command)?;
                                item.custom_name = true;
                            }
                        }
                    }
                }
                candidate.validate_xcf_state_with(control).map_err(|e| error(&command, e))?;
                let mut view = view_copy(&work_view, &command)?;
                if deleting {
                    if index < view.alpha_visible.len() {
                        view.alpha_visible.remove(index);
                    }
                    view.target = match view.target {
                        ChannelTarget::Alpha(target) if target == index => {
                            view.color_hidden.clear();
                            ChannelTarget::Composite
                        }
                        ChannelTarget::Alpha(target) if target > index => ChannelTarget::Alpha(target - 1),
                        target => target,
                    };
                } else if let Some(to) = edit_index(edit) {
                    view.alpha_visible
                        .try_reserve(before.channels.len().saturating_sub(view.alpha_visible.len()))
                        .map_err(|_| error(&command, "channel view allocation failed"))?;
                    view.alpha_visible.resize(before.channels.len(), false);
                    let visible = view.alpha_visible.remove(index);
                    view.alpha_visible.insert(to, visible);
                    if let ChannelTarget::Alpha(target) = view.target {
                        view.target = ChannelTarget::Alpha(if target == index {
                            to
                        } else if index < target && target <= to {
                            target - 1
                        } else if to <= target && target < index {
                            target + 1
                        } else {
                            target
                        });
                    }
                }
                poll(control)?;
                Ok((candidate, view, ctx.clone()))
            })
        },
        move |session, (candidate, view, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id || state.revision != revision || !state.doc.shares_state(&baseline) || state.channel_view != prior_view {
                return Err(EngineError::Other("channel target/view changed during preparation".into()));
            }
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                session.edit_with_interrupt(label, control, move |document, _| {
                    *document = candidate;
                    Ok(())
                })?;
                if let Some(state) = session.active_mut() {
                    state.channel_view = view;
                    if deleting {
                        super::fix_view_now(state);
                    } else if edit.is_some() {
                        super::touch(state);
                    }
                }
                Ok(response)
            })
        },
    )
}
fn edit_index(edit: Edit<'_>) -> Option<usize> {
    if let Edit::Move(index) = edit { Some(index) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Size, XcfState};
    use photocraft_raster::Surface;
    fn fixture(imported: bool) -> Session {
        let mut document = Document::with_background("channel metadata", Size::new(2, 1), ColorMode::Rgb, SampleType::F64, Color::WHITE);
        for (name, values) in [("First", [0.123456789012345f64, 0.987654321098765]), ("Second", [0.25, 0.75])] {
            let mut surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
            surface
                .try_write_interleaved_with(document.bounds(), &values.into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(), &Interrupt::NONE)
                .unwrap();
            document.channels.push(AlphaChannel::new(name, surface));
        }
        if imported {
            let mut xcf = XcfState::default();
            xcf.reconcile_layers(&document).unwrap();
            document.xcf_state = Some(xcf);
        }
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        let state = session.active_mut().unwrap();
        state.channel_view.target = ChannelTarget::Alpha(0);
        state.channel_view.alpha_visible = vec![true, false];
        session
    }
    #[test]
    fn wide_move_and_trimmed_rename_keep_words_order_and_single_history_steps() {
        let mut session = fixture(false);
        let before = session.active().unwrap().doc.clone();
        let id = before.channels[0].id;
        let history = session.active().unwrap().history.past_len();
        let native = before.channels[0].surface.try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap();
        assert_eq!(run(&mut session, "channel.move", &json!({"channelId":id.0.to_string(),"to":u64::MAX})).unwrap(), json!({"channel":1}));
        let state = session.active().unwrap();
        assert_eq!(state.channel_view.target, ChannelTarget::Alpha(1));
        assert_eq!(state.channel_view.alpha_visible, vec![false, true]);
        assert_eq!(state.doc.channels[1].id, id);
        assert_eq!(state.doc.channels[1].surface.try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), native);
        run(&mut session, "channel.rename", &json!({"channel":{"channelId":id.0.to_string()},"name":"  Native name  "})).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[1].name, "Native name");
        assert_eq!(session.active().unwrap().history.past_len(), history + 2);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[1].name, "First");
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[0].id, id);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[1].id, id);
    }
    #[test]
    fn imported_name_sync_and_referenced_delete_refusal_preserve_metadata() {
        let mut session = fixture(true);
        let id = session.active().unwrap().doc.channels[0].id;
        session
            .edit("Retain saved channel membership", |document, _| {
                document.xcf_state.as_mut().unwrap().item_sets.push(photocraft_doc::xcf_editing::ItemSet {
                    id: photocraft_doc::item_identity::ItemSetId(701),
                    name: "Retained channel membership".into(),
                    family: photocraft_doc::xcf_editing::ItemFamily::Channel,
                    rule: photocraft_doc::xcf_editing::ItemSetRule::Named,
                    members: vec![ItemRef::Channel(id)],
                });
                Ok(())
            })
            .unwrap();
        session.active().unwrap().doc.validate_xcf_state().unwrap();
        run(&mut session, "channel.rename", &json!({"channelId":id.0.to_string(),"name":"  Current name "})).unwrap();
        let state = session.active().unwrap();
        let xcf = state.doc.xcf_state.as_ref().unwrap();
        let item = xcf.items.iter().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        assert_eq!(item.name, "Current name");
        assert!(item.custom_name);
        assert_eq!(xcf.item_sets[0].members, vec![ItemRef::Channel(id)]);
        let before = state.doc.clone();
        let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
        let view = state.channel_view.clone();
        let history = state.history.past_len();
        assert!(run(&mut session, "channel.delete", &json!({"channelId":id.0.to_string()})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().channel_view, view);
        assert_eq!(session.active().unwrap().history.past_len(), history);
        assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
        session.active().unwrap().doc.validate_xcf_state().unwrap();
    }
    #[test]
    fn targeted_delete_and_noop_keep_legacy_view_rules() {
        let mut session = fixture(false);
        let history = session.active().unwrap().history.past_len();
        assert_eq!(run(&mut session, "channel.move", &json!({"channel":0,"to":0})).unwrap(), json!({"channel":0}));
        assert_eq!(session.active().unwrap().history.past_len(), history);
        run(&mut session, "channel.delete", &Value::Null).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.doc.channels.len(), 1);
        assert_eq!(state.doc.channels[0].name, "Second");
        assert_eq!(state.channel_view.target, ChannelTarget::Composite);
        assert_eq!(state.channel_view.alpha_visible, vec![false]);
    }
    #[test]
    fn stale_malformed_and_cancelled_reference_checks_are_atomic() {
        let session = fixture(true);
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let history = state.history.past_len();
        let id = before.channels[0].id;
        for params in [
            json!({"channel":999999,"name":"x"}),
            json!({"channel":0,"name":"x".repeat(4097)}),
            json!({"channelId":id.0.to_string(),"name":"x","documentId":before.id.0.to_string(),"expectedRevision":state.revision.saturating_add(1).to_string()}),
            json!({"channel":0,"channelId":id.0.to_string(),"name":"x"}),
        ] {
            assert!(dispatch_preflight(&session, "channel.rename", &params, false).is_err());
        }
        let cancel = || true;
        assert!(matches!(relation(&before, id, Edit::Rename("x"), "channel.rename", &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
}
