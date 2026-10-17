//! Explicit current saved-channel names; samples, source records and UI focus stay.
use crate::{DocumentSnapshot, EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::{Interrupt, backing::ReadControl};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

const ID: &str = "xcf.channel.name.set";
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
struct Request<'a> {
    document: u64,
    revision: u64,
    channel: ChannelId,
    name: &'a str,
    custom: bool,
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
fn request(params: &Value) -> Result<Request<'_>> {
    const KEYS: [&str; 5] = ["documentId", "expectedRevision", "channelId", "name", "customName"];
    let object = params.as_object().ok_or_else(|| bad("five explicit fields required"))?;
    if object.len() != KEYS.len() || object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(bad("unknown or missing channel-name field"));
    }
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= 4096 && !name.contains('\0'))
        .ok_or_else(|| bad("name must be exact nonempty UTF8<=4096 without NUL"))?;
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        channel: ChannelId(decimal(params, "channelId", false)?),
        name,
        custom: params.get("customName").and_then(Value::as_bool).ok_or_else(|| bad("customName must be boolean"))?,
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
    crate::xcf_item_locks::require_channel_content(doc, id)?;
    poll(control)?;
    Ok((native, metadata))
}
fn owned(text: &str) -> Result<String> {
    let mut output = String::new();
    output.try_reserve_exact(text.len()).map_err(|_| bad("name allocation"))?;
    output.push_str(text);
    Ok(output)
}
fn headroom(doc: &DocumentSnapshot, control: &Interrupt<'_>, name_bytes: usize) -> Result<()> {
    let footprint = doc.resident_raster_footprint_with(control).map_err(|_| controlled(control, "retained raster envelope"))?;
    let names = u64::try_from(name_bytes).ok().and_then(|n| n.checked_mul(2)).ok_or_else(|| bad("name allocation limit"))?;
    if footprint
        .retained_bytes
        .checked_add(footprint.clone_metadata_bytes)
        .and_then(|n| n.checked_add(names))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("raster/COW/name peak limit"));
    }
    // Measured public payload/capacity slice, not a whole-document RSS promise.
    let mut limits = photocraft_io::NativeSaveLimits::default().preparation;
    limits.resident_bytes = limits.resident_bytes.min(128 << 20);
    limits.object_bytes = limits.object_bytes.min(128 << 20);
    limits.index_bytes = limits.index_bytes.min(4 << 20);
    limits.manifest_bytes = limits.manifest_bytes.min(16 << 20);
    let retained = photocraft_io::retained_native_input_bytes(doc, limits, control).map_err(|_| controlled(control, "retained metadata/payload headroom"))?;
    if u64::try_from(retained).ok().and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(names)).is_none_or(|n| n > 128 << 20) {
        return Err(bad("measured retained/staged metadata peak limit"));
    }
    Ok(())
}
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    poll(control)?;
    dispatch_preflight(session, params, false)?;
    let p = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    headroom(&state.doc, control, p.name.len())?;
    let (native, metadata) = relation(&state.doc, p.channel, control)?;
    let before = state.doc.clone();
    let snapshot = before.downgrade();
    let item = before.xcf_state.as_ref().and_then(|xcf| xcf.items.get(metadata)).ok_or_else(|| bad("missing current channel"))?;
    if item.name == p.name && item.custom_name == p.custom {
        poll(control)?;
        return Ok(json!({"channelId":p.channel.0.to_string(),"changed":false,"revisionString":state.revision.to_string()}));
    }
    let channel_name = owned(p.name)?;
    let item_name = owned(p.name)?;
    let mut candidate = (*before).clone();
    candidate.channels.get_mut(native).filter(|channel| channel.id == p.channel).ok_or_else(|| bad("stale native channel"))?.name = channel_name;
    let item = candidate
        .xcf_state
        .as_mut()
        .and_then(|xcf| xcf.items.get_mut(metadata))
        .filter(|item| item.item == ItemRef::Channel(p.channel))
        .ok_or_else(|| bad("stale canonical channel"))?;
    item.name = item_name;
    item.custom_name = p.custom;
    // Both explicit mappings are already synchronized. Do not reconcile unrelated
    // names/flags or normalize layer/channel/path focus during this metadata edit.
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
    history.record("Set Current XCF Saved Channel Name", before, layers, &read)?;
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
    Ok(json!({"channelId":p.channel.0.to_string(),"changed":true,"customName":p.custom,"revisionString":state.revision.to_string()}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Set Current XCF Saved Channel Name",
        menu: &[],
        shortcut: None,
        params: r#"Exact {documentId:canonical nonzero U64 string,expectedRevision:canonical U64 string,channelId:canonical nonzero U64 string,name:exact nonempty UTF8<=4096 without NUL,customName:bool}; synchronize current AlphaChannel/ItemState name and explicit custom flag. Preserve samples/spot/overlay/source flags/provenance/semantic selection/runtime focus; no automatic naming."#,
        enabled: |s| {
            context(s, false)
                .map_err(|error| error.to_string())
                .and_then(|_| s.active().and_then(|d| d.doc.xcf_state.as_ref()).map(|_| ()).ok_or_else(|| "no retained XCF channel state".into()))
        },
        journal: true,
        run: |s, p| set_with_interrupt(s, p, &Interrupt::NONE),
    }]
}

// Implementation-first drafts: intentionally unrun until the coherent batch gate.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{AlphaChannel, Color, ColorMode, SampleType, Size, XcfState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture(depth: SampleType) -> (Session, ChannelId) {
        let mut doc = Document::with_background("own saved channel", Size::new(2, 2), ColorMode::Rgb, depth, Color::WHITE);
        let mut channel =
            AlphaChannel::new("exact original name", photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(ColorMode::Grayscale, depth, false)));
        channel.spot = Some((Color::rgb(0.25, 0.5, 0.75), f32::from_bits(0x3f000001)));
        channel.opacity = f32::from_bits(0x3f000001);
        let id = channel.id;
        doc.channels.push(channel);
        let mut xcf = XcfState::default();
        xcf.reconcile_layers(&doc).unwrap();
        let item = xcf.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
        item.source_flags = 0x12345678;
        item.custom_name = false;
        xcf.selection.selected = vec![ItemRef::Channel(id)];
        xcf.selection.active = Some(ItemRef::Channel(id));
        doc.xcf_state = Some(xcf);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session.active_mut().unwrap().set_import_origin(
            photocraft_io::ImportOrigin::xcf(),
            "Own synthetic channel source",
            Some("own-synthetic-channel.xcf".into()),
        );
        (session, id)
    }
    fn params(session: &Session, id: ChannelId, name: &str, custom: bool) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"channelId":id.0.to_string(),"name":name,"customName":custom})
    }

    #[test]
    fn explicit_name_and_custom_flag_preserve_native_samples_semantics_focus_and_undo() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32, SampleType::F64] {
            let (mut session, id) = fixture(depth);
            let before = session.active().unwrap().doc.clone();
            let view = session.active().unwrap().channel_view.clone();
            let focus = session.active().unwrap().selected_layers.clone();
            let active_layer = session.active().unwrap().active_layer;
            let source = session.active().unwrap().source.clone();
            let p = params(&session, id, "  Own α.channel copy  ", false);
            assert_eq!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap()["changed"], true);
            let mut expected = (*before).clone();
            expected.channels.iter_mut().find(|channel| channel.id == id).unwrap().name = "  Own α.channel copy  ".into();
            let item = expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap();
            item.name = "  Own α.channel copy  ".into();
            item.custom_name = false;
            let current = session.active().unwrap();
            assert_eq!(*current.doc, expected);
            assert_eq!(current.channel_view, view);
            assert_eq!(current.active_layer, active_layer);
            assert_eq!(current.selected_layers, focus);
            assert_eq!(current.source, source);
            let native = photocraft_format::save_to_bytes(&current.doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
            let revision = current.revision;
            let noop = params(&session, id, "  Own α.channel copy  ", false);
            assert_eq!(set_with_interrupt(&mut session, &noop, &Interrupt::NONE).unwrap()["changed"], false);
            assert_eq!(session.active().unwrap().revision, revision);
            assert!(session.undo());
            assert_eq!(session.active().unwrap().doc, before);
            assert!(session.redo());
            assert_eq!(*session.active().unwrap().doc, expected);
            let flag = params(&session, id, "  Own α.channel copy  ", true);
            set_with_interrupt(&mut session, &flag, &Interrupt::NONE).unwrap();
            expected.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().custom_name = true;
            assert_eq!(*session.active().unwrap().doc, expected);
        }
    }

    #[test]
    fn hostile_explicit_fields_stale_revision_and_lock_preserve_real_redo() {
        let (mut session, id) = fixture(SampleType::U16);
        session
            .edit("Real redo", |doc, _| {
                doc.name = "redo sentinel".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let p = params(&session, id, "new", true);
        let mut wrongs = Vec::new();
        for (key, value) in [
            ("channelId", json!(id.0)),
            ("channelId", json!("01")),
            ("channelId", json!("0")),
            ("expectedRevision", json!("18446744073709551615")),
            ("name", json!("")),
            ("name", json!("x\0y")),
            ("name", json!("x".repeat(4097))),
            ("customName", json!("true")),
        ] {
            let mut wrong = p.clone();
            wrong[key] = value;
            wrongs.push(wrong);
        }
        let mut extra = p.clone();
        extra["coalesce"] = json!(false);
        wrongs.push(extra);
        for wrong in wrongs {
            assert!(set_with_interrupt(&mut session, &wrong, &Interrupt::NONE).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().revision, revision);
        }
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo sentinel");
        assert!(session.undo());
        session
            .active_mut()
            .unwrap()
            .doc
            .try_resident_mut()
            .unwrap()
            .xcf_state
            .as_mut()
            .unwrap()
            .items
            .iter_mut()
            .find(|item| item.item == ItemRef::Channel(id))
            .unwrap()
            .locks
            .content = true;
        let locked = session.active().unwrap().doc.clone();
        let p = params(&session, id, "locked", true);
        assert!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&locked));
    }

    #[test]
    fn duplicate_current_mapping_and_native_name_disagreement_are_not_repaired() {
        for native_duplicate in [true, false] {
            let (mut session, id) = fixture(SampleType::F32);
            let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
            if native_duplicate {
                doc.channels.push(doc.channels.first().unwrap().clone());
            } else {
                doc.channels.first_mut().unwrap().name = "different native name".into();
            }
            let before = session.active().unwrap().doc.clone();
            let p = params(&session, id, "new", true);
            assert!(set_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
        }
    }

    #[test]
    fn every_observed_cancellation_fence_preserves_native_and_existing_redo() {
        let (mut measured, id) = fixture(SampleType::U8);
        let p = params(&measured, id, "new name", true);
        let count = AtomicUsize::new(0);
        let measure = || {
            count.fetch_add(1, Ordering::Relaxed);
            false
        };
        set_with_interrupt(&mut measured, &p, &Interrupt::cancel_only(&measure)).unwrap();
        let checks = count.load(Ordering::Relaxed);
        assert!(checks > 1);
        for threshold in 0..checks {
            let (mut session, id) = fixture(SampleType::U8);
            session
                .edit("Real redo", |doc, _| {
                    doc.name = "redo sentinel".into();
                    Ok(())
                })
                .unwrap();
            assert!(session.undo());
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let native = photocraft_format::save_to_bytes(&before, &Default::default()).unwrap();
            let p = params(&session, id, "new name", true);
            let calls = AtomicUsize::new(0);
            let stop = || calls.fetch_add(1, Ordering::Relaxed) >= threshold;
            let result = set_with_interrupt(&mut session, &p, &Interrupt::cancel_only(&stop));
            if matches!(result, Err(EngineError::Cancelled)) {
                assert!(session.active().unwrap().doc.shares_state(&before));
                assert_eq!(session.active().unwrap().revision, revision);
                assert_eq!(photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap(), native);
                assert!(session.redo());
                assert_eq!(session.active().unwrap().doc.name, "redo sentinel");
            } else {
                assert!(result.is_ok());
            }
        }
    }
}
