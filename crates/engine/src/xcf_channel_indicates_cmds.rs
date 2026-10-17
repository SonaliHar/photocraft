//! Current saved alpha-channel overlay interpretation, consumed by AlphaChannel::overlay_coverage.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    ColorIndicates, Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
const ID: &str = "xcf.channel.indicates.set";
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{ID}.{code}"))
}
fn decimal(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("identity"))?;
    if text.is_empty() || text.len() > 20 || text.len() > 1 && text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("identity"));
    }
    let raw = text.parse::<u64>().map_err(|_| bad("identity"))?;
    if raw == 0 && !zero {
        return Err(bad("identity"));
    }
    Ok(raw)
}
#[derive(Clone, Copy)]
struct Request {
    document: u64,
    revision: u64,
    channel: ChannelId,
    indicates: ColorIndicates,
}
fn envelope(params: &Value) -> Result<Request> {
    let object = params.as_object().ok_or_else(|| bad("params"))?;
    if object.len() != 4 || object.keys().any(|key| !matches!(key.as_str(), "documentId" | "expectedRevision" | "channelId" | "indicates")) {
        return Err(bad("params"));
    }
    let indicates = match params.get("indicates").and_then(Value::as_str) {
        Some("masked") => ColorIndicates::MaskedAreas,
        Some("selected") => ColorIndicates::SelectedAreas,
        _ => return Err(bad("indicates")),
    };
    Ok(Request {
        document: decimal(params, "documentId", false)?,
        revision: decimal(params, "expectedRevision", true)?,
        channel: ChannelId(decimal(params, "channelId", false)?),
        indicates,
    })
}
/// Root invokes before implicit floating/job/coalescing normalization or parameter copying.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if params.as_object().is_some_and(|object| object.contains_key("coalesce")) || session.coalesce_request.is_some() || active.coalesce.is_some() {
        return Err(bad("coalescing"));
    }
    let request = envelope(params)?;
    if request.document != active.doc.id.0 || request.revision != active.revision {
        return Err(bad("stale-target"));
    }
    if active.floating.is_some() || active.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some()) {
        return Err(bad("floating"));
    }
    if background
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
        || session.color_restrict.is_some()
        || active.revision == u64::MAX
    {
        return Err(bad("context"));
    }
    active.doc.validate_resident().map_err(|_| bad("unsupported-accounted"))?;
    Ok(())
}
fn relation(doc: &Document, id: ChannelId, control: &Interrupt<'_>) -> Result<usize> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if doc.channels.len() > 10_000 || doc.selection_channel_id == Some(id) {
        return Err(bad("unsupported-channel"));
    }
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("missing-state"))?;
    if state.items.len() > 10_000 {
        return Err(bad("limit"));
    }
    let mut native = None;
    let mut ids = std::collections::HashSet::new();
    ids.try_reserve(doc.channels.len()).map_err(|_| bad("allocation"))?;
    for (index, channel) in doc.channels.iter().enumerate() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if channel.id.0 == 0 || !ids.insert(channel.id) {
            return Err(bad("identity"));
        }
        if channel.id == id {
            native = Some(index);
        }
    }
    let index = native.ok_or_else(|| bad("missing-native-channel"))?;
    let mut metadata = None;
    let mut items = std::collections::HashSet::new();
    items.try_reserve(state.items.len()).map_err(|_| bad("allocation"))?;
    for item in &state.items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if item.item.raw() == 0 || !items.insert(item.item) {
            return Err(bad("identity"));
        }
        if item.item == ItemRef::Channel(id) {
            metadata = Some(item);
        }
    }
    let item = metadata.ok_or_else(|| bad("missing-canonical-channel"))?;
    let channel = doc.channels.get(index).ok_or_else(|| bad("missing-native-channel"))?;
    let bounds = item.raster_bounds.filter(|bounds| !bounds.is_empty()).ok_or_else(|| bad("unsupported-bounds"))?;
    let format = channel.surface.format();
    // Spot channels use ink coverage regardless of indicates. Refuse that ineffective
    // metadata edit instead of claiming a mapped source behavior for it.
    if channel.spot.is_some()
        || item.mask.is_some()
        || format.mode != photocraft_doc::ColorMode::Grayscale
        || format.alpha
        || format.sample != doc.depth
        || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].into_iter().any(|coordinate| i64::from(coordinate).abs() > 1 << 29)
    {
        return Err(bad("unsupported-domain"));
    }
    doc.validate_xcf_state_with(control)
        .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("{ID}.invalid-state: {error}")) })?;
    crate::xcf_item_locks::require_channel_content(doc, id)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(index)
}
fn install(doc: &mut Document, index: usize, request: Request) -> Result<()> {
    doc.channels.get_mut(index).filter(|channel| channel.id == request.channel).ok_or_else(|| bad("stale-target"))?.indicates = request.indicates;
    Ok(())
}
pub fn set_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    let request = envelope(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    let index = relation(&active.doc, request.channel, control)?;
    let changed = active.doc.channels.get(index).ok_or_else(|| bad("missing-native-channel"))?.indicates != request.indicates;
    let response = json!({"channelId":request.channel.0.to_string(),"indicates":if request.indicates==ColorIndicates::MaskedAreas {"masked"}else{"selected"},"changed":changed});
    if !changed {
        control.check().map_err(|_| EngineError::Cancelled)?;
        return Ok(response);
    }
    let baseline = active.doc.clone();
    let revision = active.revision;
    let footprint = baseline.resident_raster_footprint_with(control)?;
    if footprint
        .clone_metadata_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(footprint.retained_bytes))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("cow-limit"));
    }
    let mut staged = baseline.clone();
    let doc = staged.try_resident_mut_with_raster_limit(photocraft_raster::MAX_CHECKED_RASTER_BYTES, control)?;
    install(doc, index, request)?;
    relation(doc, request.channel, control)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(session, params, false)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(&baseline) {
        return Err(bad("stale-target"));
    }
    session.edit_with_interrupt("Set XCF Channel Color Indicates", control, move |doc, _| {
        let index = relation(doc, request.channel, control)?;
        install(doc, index, request)
    })?;
    Ok(response)
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Set XCF Channel Color Indicates",
        menu: &[],
        shortcut: None,
        params: r#"Exact {documentId:canonical nonzero U64 string,expectedRevision:canonical U64 string,channelId:canonical nonzero U64 string,indicates:masked|selected}; current saved alpha channel overlay coverage interpretation only, preserving samples/color/opacity/source records. Spot/selection channels unsupported; synchronous uncoalesced/no preview."#,
        enabled: |session| session.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run,
        journal: false,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Session, ChannelId) {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        let channel = photocraft_doc::AlphaChannel::new("Own channel", photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8));
        let id = channel.id;
        doc.channels.push(channel);
        let mut state = photocraft_doc::XcfState::default();
        state.reconcile_layers(doc).unwrap();
        *state.items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap() = photocraft_doc::xcf_state::ItemState {
            item: ItemRef::Channel(id),
            name: "Own channel".into(),
            custom_name: false,
            visible: false,
            locks: Default::default(),
            parent: None,
            source_tattoo: None,
            source_flags: 0,
            raster_bounds: Some(doc.bounds()),
            mask: None,
            composition: None,
        };
        doc.xcf_state = Some(state);
        (session, id)
    }
    fn params(session: &Session, id: ChannelId) -> Value {
        let active = session.active().unwrap();
        json!({"documentId":active.doc.id.0.to_string(),"expectedRevision":active.revision.to_string(),"channelId":id.0.to_string(),"indicates":"selected"})
    }
    #[test]
    fn selected_interpretation_only_native_undo_samples_and_coverage() {
        let (mut session, id) = fixture();
        let before = session.active().unwrap().doc.clone();
        let request = params(&session, id);
        set_with_interrupt(&mut session, &request, &Interrupt::NONE).unwrap();
        let mut expected = before.as_ref().clone();
        expected.channels.first_mut().unwrap().indicates = ColorIndicates::SelectedAreas;
        assert_eq!(session.active().unwrap().doc.as_ref(), &expected);
        let channel = session.active().unwrap().doc.channels.first().unwrap();
        assert_eq!(channel.overlay_coverage(0.25), 0.25);
        assert_eq!(before.channels.first().unwrap().overlay_coverage(0.25), 0.75);
        let native = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), expected);
        let revision = session.active().unwrap().revision;
        let request = params(&session, id);
        assert_eq!(set_with_interrupt(&mut session, &request, &Interrupt::NONE).unwrap()["changed"], false);
        assert_eq!(session.active().unwrap().revision, revision);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
    }
    #[test]
    fn stale_unknown_enum_spot_and_locked_requests_refuse() {
        let (mut session, id) = fixture();
        let mut request = params(&session, id);
        request["indicates"] = json!("automatic");
        let before = session.active().unwrap().doc.clone();
        assert!(set_with_interrupt(&mut session, &request, &Interrupt::NONE).is_err());
        // Duplicate canonical identity remains invalid after correcting the positive fixture.
        let mut duplicate = before.as_ref().clone();
        let state = duplicate.xcf_state.as_mut().unwrap();
        let row = state.items.iter().find(|item| item.item == ItemRef::Channel(id)).unwrap().clone();
        state.items.push(row);
        assert!(relation(&duplicate, id, &Interrupt::NONE).is_err());
        request = params(&session, id);
        session.active_mut().unwrap().revision += 1;
        assert!(set_with_interrupt(&mut session, &request, &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().channels.first_mut().unwrap().spot =
            Some((photocraft_doc::Color::rgb(1.0, 0.0, 0.0), 0.5));
        let request = params(&session, id);
        let before = session.active().unwrap().doc.clone();
        assert!(set_with_interrupt(&mut session, &request, &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        {
            let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
            doc.channels.first_mut().unwrap().spot = None;
            doc.xcf_state.as_mut().unwrap().items.iter_mut().find(|item| item.item == ItemRef::Channel(id)).unwrap().locks.content = true;
        }
        let request = params(&session, id);
        let before = session.active().unwrap().doc.clone();
        assert!(set_with_interrupt(&mut session, &request, &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
