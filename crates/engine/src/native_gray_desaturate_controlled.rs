//! Controlled closed-unit native Gray Desaturate; no display/color conversion.
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, item_identity::ChannelId};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
const ID: &str = "image.adjustments.desaturate";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Mask(LayerId),
    Channel(usize, ChannelId),
    QuickMask(ChannelId),
}
fn resolve(session: &Session, p: &Value) -> Result<Option<Target>> {
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.doc.channels.len() > 10_000 {
        return Err(bad("channel count limit"));
    }
    let layer = || crate::commands::layer_param(session, p);
    let channel = |index: usize| -> Result<Target> {
        let id = st.doc.channels.get(index).ok_or_else(|| bad("no such channel"))?.id;
        if id.0 == 0 || st.doc.channels.iter().filter(|c| c.id == id).count() != 1 {
            return Err(bad("ambiguous channel identity"));
        }
        Ok(Target::Channel(index, id))
    };
    let quick = || -> Result<Target> {
        let id = st.doc.quick_mask.as_ref().ok_or_else(|| bad("not in Quick Mask mode"))?.id;
        if id.0 == 0 {
            return Err(bad("invalid Quick Mask identity"));
        }
        Ok(Target::QuickMask(id))
    };
    if let Some(value) = p.get("layer")
        && value.as_u64().is_none_or(|v| v == 0)
    {
        return Err(bad("layer must be a nonzero unsigned integer"));
    }
    if let Some(value) = p.get("target") {
        return match value {
            Value::String(s) if s == "mask" => Ok(Some(Target::Mask(layer()?))),
            Value::String(s) if s == "quickMask" => Ok(Some(quick()?)),
            Value::String(s) if s == "pixels" => Ok(None),
            Value::Object(map) if map.len() == 1 && map.contains_key("channel") => {
                let index = map
                    .get("channel")
                    .and_then(Value::as_u64)
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| bad("channel must be a bounded unsigned index"))?;
                Ok(Some(channel(index)?))
            }
            _ => Err(bad("invalid explicit target")),
        };
    }
    Ok(match st.channel_view.target {
        ChannelTarget::Alpha(index) => Some(channel(index)?),
        ChannelTarget::LayerMask => Some(Target::Mask(layer()?)),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => Some(quick()?),
        ChannelTarget::Composite if crate::mask_view_cmds::current(st).is_some() => Some(Target::Mask(layer()?)),
        _ => None,
    })
}
fn surface(doc: &Document, t: Target) -> Result<&Surface> {
    match t {
        Target::Mask(id) => doc.layer(id).and_then(|l| l.mask.as_ref()).map(|m| m.surface.as_ref()).ok_or_else(|| bad("layer has no mask")),
        Target::Channel(index, id) => doc.channels.get(index).filter(|c| c.id == id).map(|c| &c.surface).ok_or_else(|| bad("channel identity changed")),
        Target::QuickMask(id) => doc.quick_mask.as_ref().filter(|c| c.id == id).map(|c| &c.surface).ok_or_else(|| bad("Quick Mask identity changed")),
    }
}
fn locks(doc: &Document, t: Target) -> Result<()> {
    match t {
        Target::Channel(_, id) => crate::xcf_item_locks::require_channel_content(doc, id),
        Target::QuickMask(_) => Ok(()),
        Target::Mask(id) => {
            let locks = crate::xcf_item_locks::effective_layer_locks(doc, id)?;
            if locks.native.all || locks.imported.content {
                return Err(bad("mask content locked"));
            }
            if doc
                .xcf_state
                .as_ref()
                .is_some_and(|s| s.items.iter().any(|i| i.mask.as_ref().is_some_and(|m| m.owner == photocraft_doc::item_identity::ItemRef::Layer(id))))
            {
                crate::xcf_item_locks::require_layer_mask_content(doc, id)?;
            }
            Ok(())
        }
    }
}
fn context(session: &Session) -> Result<()> {
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || st.floating.is_some()
        || st.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return Err(bad("finish transient work before native Gray editing"));
    }
    Ok(())
}

struct Request {
    target: Target,
}
fn envelope(params: &Value) -> Result<()> {
    if !params.is_null() {
        let object = params.as_object().filter(|object| object.len() <= 6).ok_or_else(|| bad("bounded native Gray Desaturate parameters required"))?;
        if object.keys().any(|key| !matches!(key.as_str(), "target" | "layer" | "documentId" | "expectedRevision" | "opacity" | "__kind")) {
            return Err(bad("unsupported native Gray Desaturate parameter"));
        }
    }
    if params.get("__kind").is_some_and(|value| value.as_str() != Some("desaturate")) {
        return Err(bad("native Gray identity requires Desaturate"));
    }
    if let Some(value) = params.get("opacity")
        && !value.is_null()
        && value.as_f64().is_none_or(|value| !value.is_finite())
    {
        return Err(bad("opacity must be finite"));
    }
    Ok(())
}
fn metadata(doc: &Document, target: Target, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if doc.xcf_state.as_ref().is_some_and(|state| {
        state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.filter_stacks.len() > 10_000 || state.filter_masks.len() > 10_000
    }) {
        return Err(bad("typed current metadata limit"));
    }
    locks(doc, target)?;
    if let Target::Mask(id) = target {
        let mut count = 0usize;
        photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
            if layer.id == id {
                count = count.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
            }
            Ok(true)
        })?;
        if id.0 == 0 || count != 1 {
            return Err(bad("missing or ambiguous current mask owner"));
        }
    }
    let source = surface(doc, target)?;
    let format = source.format();
    if format.mode != ColorMode::Grayscale || format.alpha || !matches!(format.sample, SampleType::U32 | SampleType::F64) {
        return Err(bad("native Gray identity requires U32/F64 without alpha"));
    }
    if source.tile_count() > 100_000 || source.default_bytes().len() != format.bytes_per_pixel() {
        return Err(bad("native Gray storage shape limit"));
    }
    source.try_validate_tile_origins_with(control).map_err(|error| super_error(control, error))?;
    for (coordinate, _) in source.tiles() {
        poll(control)?;
        if coordinate.tx.checked_mul(photocraft_geom::TILE_SIZE).and_then(|origin| origin.checked_add(photocraft_geom::TILE_SIZE)).is_none()
            || coordinate.ty.checked_mul(photocraft_geom::TILE_SIZE).and_then(|origin| origin.checked_add(photocraft_geom::TILE_SIZE)).is_none()
        {
            return Err(bad("native Gray tile extent overflow"));
        }
    }
    // Existing typed lock/relationship helpers are authoritative for this
    // selected target. No unrelated source payload CRC/hash/sample traversal:
    // nothing is transformed, requalified or installed by this identity.
    poll(control)
}
fn super_error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error.to_string()) }
}
fn request(session: &Session, params: &Value) -> Result<Option<Request>> {
    let Some(target) = resolve(session, params)? else { return Ok(None) };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let format = surface(&state.doc, target)?.format();
    if !matches!(format.sample, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    envelope(params)?;
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if state.doc.id.0 != 0 && decimal(id) == Some(state.doc.id.0) && decimal(revision) == Some(state.revision) => {}
        _ => return Err(bad("captured Gray identity target changed or incomplete")),
    }
    context(session)?;
    metadata(&state.doc, target, &Interrupt::NONE)?;
    Ok(Some(Request { target }))
}
pub(crate) fn dispatch_preflight(session: &Session, id: &str, params: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    request(session, params).map(|request| request.is_some())
}
pub(crate) fn try_run(session: &mut Session, params: &Value) -> Result<Option<Value>> {
    let Some(request) = request(session, params)? else { return Ok(None) };
    // Borrowed synchronous identity: no job, owned plane, document transaction,
    // focus change, journal mutation or redo invalidation is necessary.
    let state = session.active().ok_or(EngineError::NoDocument)?;
    context(session)?;
    metadata(&state.doc, request.target, &Interrupt::NONE)?;
    Ok(Some(Value::Null))
}

// Meaningful identity/selector drafts, intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer, LayerMask, Size};
    use photocraft_geom::Rect;
    use serde_json::json;
    use std::sync::Arc;
    fn fixture(sample: SampleType) -> (Session, LayerId) {
        let mut doc = Document::new("Native Gray identity", Size::new(1, 1), ColorMode::Rgb, sample);
        let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
        let defaults = if sample == SampleType::F64 { (-0.0f64).to_ne_bytes().to_vec() } else { 1u32.to_ne_bytes().to_vec() };
        let raw = if sample == SampleType::F64 { 0x7ff8_0000_0000_0073u64.to_ne_bytes().to_vec() } else { 7u32.to_ne_bytes().to_vec() };
        let mut surface = Surface::try_with_default_bytes(format, &defaults).unwrap();
        surface.try_write_interleaved_with(Rect::new(-1000000, 0, -999999, 1), &raw, &Interrupt::NONE).unwrap();
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        layer.mask = Some(LayerMask { surface: Arc::new(surface.clone()), enabled: false, linked: false, density: 0.7, feather: 3.0 });
        doc.layers.push(layer);
        doc.channels.push(AlphaChannel::new("Saved", surface.clone()));
        doc.quick_mask = Some(AlphaChannel::new("Quick", surface));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, id)
    }
    #[test]
    fn every_native_gray_target_keeps_raw_words_defaults_offcanvas_metadata_and_redo() {
        for sample in [SampleType::U32, SampleType::F64] {
            let (mut session, id) = fixture(sample);
            session
                .edit("Seed", |doc, _| {
                    doc.name = "Seed".into();
                    Ok(())
                })
                .unwrap();
            session.execute("edit.undo", Value::Null).unwrap();
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let active = session.active().unwrap().active_layer;
            for params in [json!({"target":"mask","layer":id.0}), json!({"target":{"channel":0}}), json!({"target":"quickMask"})] {
                let r = request(&session, &params).unwrap().unwrap();
                let expected = surface(&before, r.target).unwrap();
                let defaults = expected.default_bytes().to_vec();
                let words = expected.to_interleaved(Rect::new(-1000000, 0, -999999, 1));
                assert_eq!(try_run(&mut session, &params).unwrap(), Some(Value::Null));
                let state = session.active().unwrap();
                assert!(state.doc.shares_state(&before));
                assert_eq!(state.revision, revision);
                assert_eq!(state.active_layer, active);
                assert!(state.history.can_redo());
                let current = surface(&state.doc, r.target).unwrap();
                assert_eq!(current.default_bytes(), defaults);
                assert_eq!(current.to_interleaved(Rect::new(-1000000, 0, -999999, 1)), words);
            }
        }
    }
    #[test]
    fn stale_hostile_selectors_and_context_refuse_without_history_or_focus_mutation() {
        let (mut session, id) = fixture(SampleType::F64);
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        for params in [
            json!({"target":"mask","documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
            json!({"target":{"channel":-1}}),
            json!({"target":{"channel":0,"extra":1}}),
            json!({"target":"mask","layer":0}),
            json!({"target":"mask","opacity":{}}),
        ] {
            assert!(try_run(&mut session, &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().active_layer, Some(id));
        }
        let cancel = || true;
        assert!(matches!(metadata(&before, Target::Mask(id), &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    }
}
