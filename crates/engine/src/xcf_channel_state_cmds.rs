//! Lossless current saved-channel overlay metadata; original source records stay inert.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::{
    Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::Interrupt;
use serde_json::{Value, json};
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("xcf.channel.overlay.{code}"))
}
#[derive(Clone, Copy)]
struct Parameters {
    id: ChannelId,
    color: [f32; 3],
    opacity: f32,
}
fn bits(value: &Value) -> Result<f32> {
    let text = value.as_str().ok_or_else(|| bad("bits"))?;
    if text.len() != 8 || !text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(bad("bits"));
    }
    let bits = u32::from_str_radix(text, 16).map_err(|_| bad("bits"))?;
    let number = f32::from_bits(bits);
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(bad("range"));
    }
    Ok(number)
}
fn envelope(params: &Value) -> Result<Parameters> {
    let object = params.as_object().ok_or_else(|| bad("params"))?;
    if object.len() != 3 || object.keys().any(|key| !matches!(key.as_str(), "channelId" | "colorF32Bits" | "opacityF32Bits")) {
        return Err(bad("params"));
    }
    let text = object.get("channelId").and_then(Value::as_str).ok_or_else(|| bad("identity"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bad("identity"));
    }
    let id = text.parse::<u64>().map_err(|_| bad("identity"))?;
    if id == 0 || id.to_string() != text {
        return Err(bad("identity"));
    }
    let values = object.get("colorF32Bits").and_then(Value::as_array).filter(|values| values.len() == 3).ok_or_else(|| bad("bits"))?;
    let mut color = [0.0; 3];
    for (out, value) in color.iter_mut().zip(values) {
        *out = bits(value)?;
    }
    let opacity = bits(object.get("opacityF32Bits").ok_or_else(|| bad("bits"))?)?;
    Ok(Parameters { id: ChannelId(id), color, opacity })
}
/// Root invokes before implicit floating/coalescing/background normalization.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if params.as_object().is_some_and(|object| object.contains_key("coalesce")) || session.coalesce_request.is_some() {
        return Err(bad("coalescing"));
    }
    envelope(params)?;
    if active.floating.is_some() {
        return Err(bad("floating"));
    }
    if background || session.command_is_background() || session.command_has_pending_job() || session.has_jobs() || session.color_restrict.is_some() {
        return Err(bad("context"));
    }
    Ok(())
}
fn relation(doc: &Document, id: ChannelId, control: &Interrupt<'_>) -> Result<usize> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if doc.channels.len() > 10_000 {
        return Err(bad("limit"));
    }
    if doc.selection_channel_id == Some(id) {
        return Err(bad("unsupported-selection-channel"));
    }
    doc.validate_xcf_state_with(control).map_err(|error| if error.code == "cancelled" { EngineError::Cancelled } else { bad("invalid-state") })?;
    let state = doc.xcf_state.as_ref().ok_or_else(|| bad("unsupported"))?;
    if state.items.len() > 10_000 {
        return Err(bad("limit"));
    }
    let mut target = None;
    for item in &state.items {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if item.item == ItemRef::Channel(id) && target.replace(item).is_some() {
            return Err(bad("identity"));
        }
    }
    let item = target.ok_or_else(|| bad("missing-channel"))?;
    let bounds = item.raster_bounds.filter(|bounds| !bounds.is_empty()).ok_or_else(|| bad("unsupported-bounds"))?;
    if item.mask.is_some() || [bounds.x0, bounds.y0, bounds.x1, bounds.y1].into_iter().any(|coordinate| i64::from(coordinate).abs() > 1 << 29) {
        return Err(bad("unsupported-bounds"));
    }
    let mut index = None;
    for (ordinal, channel) in doc.channels.iter().enumerate() {
        control.check().map_err(|_| EngineError::Cancelled)?;
        if channel.id == id && index.replace(ordinal).is_some() {
            return Err(bad("identity"));
        }
    }
    let index = index.ok_or_else(|| bad("missing-native-channel"))?;
    let channel = doc.channels.get(index).ok_or_else(|| bad("missing-native-channel"))?;
    let format = channel.surface.format();
    if channel.spot.is_some()
        || format.mode != photocraft_doc::ColorMode::Grayscale
        || format.alpha
        || format.sample != doc.depth
        || channel.color.mode != photocraft_doc::ColorMode::Rgb
        || channel.color.alpha.to_bits() != 1.0f32.to_bits()
        || channel.color.c.get(3).is_none_or(|value| value.to_bits() != 0.0f32.to_bits())
        || channel.color.c.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || !channel.opacity.is_finite()
        || !(0.0..=1.0).contains(&channel.opacity)
    {
        return Err(bad("unsupported-overlay-domain"));
    }
    channel.surface.try_validate_tile_origins_with(control)?;
    crate::xcf_item_locks::require_channel_content(doc, id)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    Ok(index)
}
fn same(channel: &photocraft_doc::AlphaChannel, p: Parameters) -> bool {
    channel.color.c.iter().take(3).zip(p.color).all(|(old, new)| old.to_bits() == new.to_bits()) && channel.opacity.to_bits() == p.opacity.to_bits()
}
fn install(doc: &mut Document, index: usize, p: Parameters) -> Result<()> {
    let channel = doc.channels.get_mut(index).filter(|channel| channel.id == p.id).ok_or_else(|| bad("stale-target"))?;
    channel.color = photocraft_doc::Color::rgb(
        *p.color.first().ok_or_else(|| bad("bits"))?,
        *p.color.get(1).ok_or_else(|| bad("bits"))?,
        *p.color.get(2).ok_or_else(|| bad("bits"))?,
    );
    channel.opacity = p.opacity;
    Ok(())
}
pub fn set_overlay_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let p = envelope(params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    active.doc.validate_resident().map_err(|_| bad("unsupported-accounted"))?;
    let index = relation(&active.doc, p.id, control)?;
    let changed = !same(active.doc.channels.get(index).ok_or_else(|| bad("missing-native-channel"))?, p);
    let response = json!({"channelId":p.id.0.to_string(),"colorF32Bits":p.color.map(|number|format!("{:08x}",number.to_bits())),"opacityF32Bits":format!("{:08x}",p.opacity.to_bits()),"changed":changed});
    if !changed {
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
    install(doc, index, p)?;
    relation(doc, p.id, control)?;
    control.check().map_err(|_| EngineError::Cancelled)?;
    dispatch_preflight(session, params, false)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    if active.revision != revision || !active.doc.shares_state(&baseline) {
        return Err(bad("stale-target"));
    }
    session.edit_with_interrupt("Set XCF Channel Overlay", control, move |doc, _| {
        let index = relation(doc, p.id, control)?;
        install(doc, index, p)
    })?;
    Ok(response)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    set_overlay_with_interrupt(session, params, &Interrupt::NONE)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "xcf.channel.overlay.set",
        label: "Set XCF Channel Overlay",
        menu: &[],
        shortcut: None,
        params: r#"{"channelId":"canonical decimal U64","colorF32Bits":["three lower-case eight-digit IEEE f32 hex strings"],"opacityF32Bits":"lower-case eight-digit IEEE f32 hex"} → finite 0..1 RGB display overlay on current saved alpha channel; spot/selection channels unsupported; raw samples and source records unchanged"#,
        enabled,
        run,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params(id: ChannelId) -> Value {
        json!({"channelId":id.0.to_string(),"colorF32Bits":["3e800001","80000000","3f800000"],"opacityF32Bits":"3f000001"})
    }
    fn fixture(depth: photocraft_doc::SampleType) -> (Session, ChannelId) {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        let doc = session.active_mut().unwrap().doc.try_resident_mut().unwrap();
        doc.depth = depth;
        let channel = photocraft_doc::AlphaChannel::new(
            "Own exact overlay",
            photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, depth, false)),
        );
        let id = channel.id;
        doc.channels.push(channel);
        doc.xcf_state = Some(photocraft_doc::XcfState {
            items: vec![photocraft_doc::xcf_state::ItemState {
                item: ItemRef::Channel(id),
                name: "Own exact overlay".into(),
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
        (session, id)
    }
    #[test]
    fn exact_overlay_bits_native_history_noop_and_samples_are_preserved() {
        let (mut session, id) = fixture(photocraft_doc::SampleType::U8);
        let before = session.active().unwrap().doc.clone();
        let p = params(id);
        set_overlay_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap();
        let mut expected = before.as_ref().clone();
        install(&mut expected, 0, envelope(&p).unwrap()).unwrap();
        assert_eq!(session.active().unwrap().doc.as_ref(), &expected);
        let native = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&native).unwrap();
        let channel = reopened.channels.first().unwrap();
        assert_eq!(channel.color.c.get(1).unwrap().to_bits(), 0x80000000);
        assert_eq!(channel.opacity.to_bits(), 0x3f000001);
        assert_eq!(channel.surface, before.channels.first().unwrap().surface);
        let revision = session.active().unwrap().revision;
        assert_eq!(set_overlay_with_interrupt(&mut session, &p, &Interrupt::NONE).unwrap()["changed"], false);
        assert_eq!(session.active().unwrap().revision, revision);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc, before);
    }
    #[test]
    fn invalid_bits_and_imported_content_lock_refuse_without_mutation() {
        let (mut session, id) = fixture(photocraft_doc::SampleType::U8);
        for invalid in ["7fc00000", "7f800000", "bf000000", "3f800001", "3F000000", "0"] {
            let mut p = params(id);
            p["opacityF32Bits"] = json!(invalid);
            let before = session.active().unwrap().doc.clone();
            assert!(set_overlay_with_interrupt(&mut session, &p, &Interrupt::NONE).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
        }
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state.as_mut().unwrap().items.first_mut().unwrap().locks.content = true;
        let before = session.active().unwrap().doc.clone();
        assert!(set_overlay_with_interrupt(&mut session, &params(id), &Interrupt::NONE).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
}
