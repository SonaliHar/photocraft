//! Own current saved-channel content/visibility locks; inherited locks remain effective.
use crate::{DocumentSnapshot, EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

const ID: &str = "xcf.channel.locks.set";
const MAX_ITEMS: usize = 10_000;
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(message) }
}
struct Request {
    document: u64,
    revision: u64,
    channel: ChannelId,
    content: Option<bool>,
    visibility: Option<bool>,
}
fn decimal(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("canonical string identities required"))?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("noncanonical decimal identity"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad("identity overflow"))?;
    if raw == 0 && !zero {
        return Err(bad("identity must be nonzero"));
    }
    Ok(raw)
}
fn request(params: &Value) -> Result<Request> {
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "channelId", "content", "visibility"];
    let object = params.as_object().ok_or_else(|| bad("explicit channel-lock fields required"))?;
    if object.len() < 4 || object.len() > 5 || object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("unknown or missing channel-lock field"));
    }
    let flag = |key: &str| -> Result<Option<bool>> {
        object.get(key).map(|value| value.as_bool().ok_or_else(|| bad("lock flags must be explicit booleans"))).transpose()
    };
    let content = flag("content")?;
    let visibility = flag("visibility")?;
    if content.is_none() && visibility.is_none() {
        return Err(bad("at least one supported lock flag required"));
    }
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        channel: ChannelId(decimal(params, "channelId", false)?),
        content,
        visibility,
    })
}
fn context(session: &Session, background: bool) -> Result<()> {
    if background
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(bad("synchronous idle uncoalesced context required"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() || state.coalesce.is_some() || state.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_some()) {
        return Err(bad("complete floating/attachment/coalesced context first"));
    }
    if state.doc.resident_arc().is_none() || state.revision == u64::MAX {
        return Err(bad("resident revision carrier required"));
    }
    Ok(())
}
/// Root dispatch calls before float normalization, jobs or params cloning.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    context(session, background)?;
    let p = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id.0 != p.document || state.revision != p.revision {
        return Err(bad("stale document or revision"));
    }
    if state.doc.channels.len() > MAX_ITEMS || state.doc.xcf_state.as_ref().is_none_or(|xcf| xcf.items.len() > MAX_ITEMS) {
        return Err(bad("current saved-channel metadata limit"));
    }
    Ok(())
}
fn relation(doc: &Document, id: ChannelId, control: &Interrupt<'_>) -> Result<(usize, usize)> {
    if doc.channels.len() > MAX_ITEMS || doc.selection_channel_id == Some(id) {
        return Err(bad("current saved alpha/spot channel required"));
    }
    let xcf = doc.xcf_state.as_ref().ok_or_else(|| bad("missing current XCF state"))?;
    if xcf.items.len() > MAX_ITEMS {
        return Err(bad("current item limit"));
    }
    if [xcf.resources.len(), xcf.source_items.len(), xcf.filter_stacks.len(), xcf.filter_masks.len(), xcf.item_sets.len(), xcf.editable_indexed.len()]
        .into_iter()
        .any(|n| n > MAX_ITEMS)
        || xcf.opaque.len() > 100_000
    {
        return Err(bad("retained metadata count limit"));
    }
    let mut references = xcf.selection.selected.len().checked_add(xcf.selection.selected_paths.len()).ok_or_else(|| bad("reference work limit"))?;
    for set in &xcf.item_sets {
        poll(control)?;
        references = references.checked_add(set.members.len()).ok_or_else(|| bad("reference work limit"))?;
        if references > 100_000 {
            return Err(bad("reference work limit"));
        }
    }
    if references > 100_000 {
        return Err(bad("reference work limit"));
    }
    let mut channels = HashSet::new();
    channels.try_reserve(doc.channels.len()).map_err(|_| bad("channel identity allocation"))?;
    let mut native = None;
    for (index, channel) in doc.channels.iter().enumerate() {
        poll(control)?;
        if channel.id.0 == 0 || !channels.insert(channel.id) {
            return Err(bad("zero or duplicate native channel identity"));
        }
        if channel.id == id {
            native = Some(index);
        }
    }
    let mut items = HashSet::new();
    items.try_reserve(xcf.items.len()).map_err(|_| bad("item identity allocation"))?;
    let mut metadata = None;
    for (index, item) in xcf.items.iter().enumerate() {
        poll(control)?;
        if item.item.raw() == 0 || !items.insert(item.item) {
            return Err(bad("zero or duplicate canonical item identity"));
        }
        if item.item == ItemRef::Channel(id) {
            metadata = Some(index);
        }
    }
    let native = native.ok_or_else(|| bad("missing native saved channelId"))?;
    let metadata = metadata.ok_or_else(|| bad("missing canonical saved channelId"))?;
    let channel = doc.channels.get(native).ok_or_else(|| bad("missing saved channel"))?;
    let item = xcf.items.get(metadata).ok_or_else(|| bad("missing canonical channel"))?;
    if channel.name != item.name {
        return Err(bad("current native and canonical channel names disagree"));
    }
    doc.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid current XCF state"))?;
    // Setting only own lock bits does not require permission to change content.
    // This query validates the typed ancestry; parent/native flags are never cleared.
    crate::xcf_item_locks::effective_item_locks(doc, ItemRef::Channel(id))?;
    poll(control)?;
    Ok((native, metadata))
}
fn headroom(doc: &DocumentSnapshot, control: &Interrupt<'_>) -> Result<()> {
    let footprint = doc.resident_raster_footprint_with(control).map_err(|_| controlled(control, "retained raster envelope"))?;
    if footprint.retained_bytes.checked_add(footprint.clone_metadata_bytes).is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
        return Err(bad("raster/COW peak limit"));
    }
    // Measured public payload/capacity slice, not a whole-document RSS promise.
    let mut limits = photocraft_io::NativeSaveLimits::default().preparation;
    limits.resident_bytes = limits.resident_bytes.min(128 << 20);
    limits.object_bytes = limits.object_bytes.min(128 << 20);
    limits.index_bytes = limits.index_bytes.min(4 << 20);
    limits.manifest_bytes = limits.manifest_bytes.min(16 << 20);
    let retained = photocraft_io::retained_native_input_bytes(doc, limits, control).map_err(|_| controlled(control, "retained metadata/payload headroom"))?;
    if u64::try_from(retained).ok().and_then(|n| n.checked_mul(2)).is_none_or(|n| n > 128 << 20) {
        return Err(bad("measured retained/staged metadata peak limit"));
    }
    Ok(())
}
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    poll(control)?;
    dispatch_preflight(session, params, false)?;
    let p = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    headroom(&state.doc, control)?;
    let (_, metadata) = relation(&state.doc, p.channel, control)?;
    let before = state.doc.clone();
    let snapshot = before.downgrade();
    let item = before.xcf_state.as_ref().and_then(|xcf| xcf.items.get(metadata)).ok_or_else(|| bad("missing current channel"))?;
    let mut locks = item.locks;
    if let Some(value) = p.content {
        locks.content = value;
    }
    if let Some(value) = p.visibility {
        locks.visibility = value;
    }
    let changed = locks != item.locks;
    let response = json!({"channelId":p.channel.0.to_string(),"changed":changed,"locks":{"content":locks.content,"visibility":locks.visibility},"revisionString":if changed {state.revision+1} else {state.revision}.to_string(),"visibilityScope":"persistedItemStateOnly"});
    if !changed {
        poll(control)?;
        return Ok(response);
    }
    let mut candidate = (*before).clone();
    let item = candidate
        .xcf_state
        .as_mut()
        .and_then(|xcf| xcf.items.get_mut(metadata))
        .filter(|item| item.item == ItemRef::Channel(p.channel))
        .ok_or_else(|| bad("stale canonical channel"))?;
    item.locks = locks;
    // Native AlphaChannel has no Locks field. Preserve it and all other current
    // metadata, including unrequested position/alpha bits and every parent lock.
    candidate.validate_xcf_state_with(control).map_err(|_| controlled(control, "invalid staged XCF state"))?;
    let candidate = DocumentSnapshot::from_resident(Arc::new(candidate))?;
    let target = || -> Result<photocraft_ops::LayerTarget> {
        if state.selected_layers.len() > MAX_ITEMS {
            return Err(bad("runtime selection limit"));
        }
        let mut selected = Vec::new();
        selected.try_reserve_exact(state.selected_layers.len()).map_err(|_| bad("runtime selection allocation"))?;
        selected.extend_from_slice(&state.selected_layers);
        Ok(photocraft_ops::LayerTarget { active: state.active_layer, selected })
    };
    let prior = target()?;
    let layers = target()?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let mut history = state.history.clone();
    history.set_current_layers(prior, &read)?;
    history.record("Set Current XCF Saved Channel Locks", before, layers, &read)?;
    history.trim(&candidate, &read)?;
    dispatch_preflight(session, params, false)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if !snapshot.upgrade().is_some_and(|original| original.shares_state(&current.doc)) {
        return Err(bad("stale channel snapshot"));
    }
    poll(control)?;
    let state = session.active_mut().ok_or(EngineError::NoDocument)?;
    state.doc = candidate;
    state.history = history;
    state.revision += 1;
    state.last_damage = None;
    Ok(response)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Set Current XCF Saved Channel Locks",
        menu: &[],
        shortcut: None,
        params: r#"{documentId:canonical nonzero U64 string,expectedRevision:canonical U64 string,channelId:canonical nonzero U64 string,content?:bool,visibility?:bool}; at least one explicit own current channel lock; preserve ancestor/native locks, source flags, samples/overlay/spot, selection and runtime focus. Visibility protects persisted ItemState.visible, not ChannelView."#,
        enabled: |s| {
            context(s, false)
                .map_err(|error| error.to_string())
                .and_then(|_| s.active().and_then(|d| d.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no retained XCF channel state".into()))
        },
        journal: true,
        run: |s, p| set_with_interrupt(s, p, &Interrupt::NONE),
    }]
}

// Implementation-first regression drafts; not executed in this increment.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{AlphaChannel, Color, ColorMode, SampleType, Size, XcfState};
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn fixture() -> (Session, ChannelId) {
        let mut doc = Document::with_background("own channel locks", Size::new(2, 2), ColorMode::Rgb, SampleType::U16, Color::WHITE);
        let mut channel = AlphaChannel::new(
            "own saved channel",
            photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false)),
        );
        channel.spot = Some((Color::rgb(0.25, 0.5, 0.75), f32::from_bits(0x3f000001)));
        let id = channel.id;
        doc.channels.push(channel);
        let mut xcf = XcfState::default();
        xcf.reconcile_layers(&doc).unwrap();
        let item = xcf.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.visible = true;
        item.source_flags = 0x12345678;
        item.locks.position = true;
        item.locks.alpha = true;
        xcf.selection.selected = vec![ItemRef::Channel(id)];
        xcf.selection.active = Some(ItemRef::Channel(id));
        doc.xcf_state = Some(xcf);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session.active_mut().unwrap().set_import_origin(photocraft_io::ImportOrigin::xcf(), "own synthetic channel source", Some("own-channel.xcf".into()));
        (session, id)
    }
    fn params(session: &Session, id: ChannelId, content: bool, visibility: bool) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"channelId":id.0.to_string(),"content":content,"visibility":visibility})
    }
    #[test]
    fn own_lock_bits_block_real_consumers_and_preserve_native_view_source_undo() {
        let (mut session, id) = fixture();
        let before = session.active().unwrap().doc.clone();
        let view = session.active().unwrap().channel_view.clone();
        let source = session.active().unwrap().source.clone();
        let p = params(&session, id, true, true);
        assert_eq!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap()["changed"], true);
        let mut expected = (*before).clone();
        let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.locks.content = true;
        item.locks.visibility = true;
        assert_eq!(*session.active().unwrap().doc, expected);
        assert_eq!(session.active().unwrap().channel_view, view);
        assert_eq!(session.active().unwrap().source, source);
        assert!(crate::xcf_item_locks::require_channel_content(&session.active().unwrap().doc, id).is_err());
        let visibility = json!({"items":[{"kind":"channel","id":id.0.to_string()}],"visible":false});
        assert!(crate::xcf_selection_cmds::visibility_with_interrupt(&mut session, &visibility, &Interrupt::NONE).is_err());
        let native = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
        let noop = params(&session, id, true, true);
        let revision = session.active().unwrap().revision;
        assert_eq!(set_with_interrupt(&mut session, &noop, &Interrupt::NONE).unwrap()["changed"], false);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, expected);
        let unlock = params(&session, id, false, false);
        set_with_interrupt(&mut session, &unlock, &Interrupt::NONE).unwrap();
        crate::xcf_item_locks::require_channel_content(&session.active().unwrap().doc, id).unwrap();
        crate::xcf_selection_cmds::visibility_with_interrupt(&mut session, &visibility, &Interrupt::NONE).unwrap();
        assert_eq!(session.active().unwrap().channel_view, view);
        assert_eq!(session.active().unwrap().doc.channels, before.channels);
    }
    #[test]
    fn own_unlock_keeps_typed_parent_locks_effective_and_unrequested_bits_exact() {
        let (mut session, id) = fixture();
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        // Canonical ancestry must name a real group, not the background raster.
        let children = std::mem::take(&mut doc.layers);
        let group = photocraft_doc::Layer::group("own channel lock ancestor", children);
        let parent = group.id;
        doc.layers.push(group);
        let mut xcf = doc.xcf_state.take().unwrap();
        xcf.reconcile_layers(doc).unwrap();
        xcf.items.iter_mut().find(|item| item.item == ItemRef::Layer(parent)).unwrap().locks.content = true;
        let item = xcf.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.parent = Some(ItemRef::Layer(parent));
        item.locks.content = true;
        doc.xcf_state = Some(xcf);
        doc.validate_xcf_state().unwrap();
        let before = session.active().unwrap().doc.clone();
        let p = params(&session, id, false, false);
        set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let mut expected = (*before).clone();
        let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.locks.content = false;
        item.locks.visibility = false;
        assert_eq!(*session.active().unwrap().doc, expected);
        assert!(crate::xcf_item_locks::require_channel_content(&session.active().unwrap().doc, id).is_err());
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
    }
    #[test]
    fn hostile_fields_and_cancelled_publication_preserve_real_redo() {
        let (mut session, id) = fixture();
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo sentinel".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let p = params(&session, id, true, true);
        for (key, value) in [
            ("channelId", json!(id.0)),
            ("channelId", json!("01")),
            ("expectedRevision", json!("18446744073709551615")),
            ("content", json!(null)),
            ("position", json!(false)),
            ("alpha", json!(false)),
        ] {
            let mut bad = p.clone();
            bad[key] = value;
            assert!(set_with_interrupt(&mut session, &bad, &Interrupt::NONE).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
        }
        let calls = AtomicUsize::new(0);
        let stop = || calls.fetch_add(1, Ordering::Relaxed) >= 12;
        assert!(matches!(set_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop)), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo sentinel");
    }
}
