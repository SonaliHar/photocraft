//! Frozen channel presentation options. Native coverage/source bytes never change.
use super::{ChanRef, QuickMaskOptions, bad, source_reference};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_doc::{
    Color, ColorIndicates, Document,
    item_identity::{ChannelId, ItemRef},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
const CMD: &str = "channel.options";
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(e: impl std::fmt::Display, ctl: &Interrupt<'_>) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(CMD, e.to_string()) }
}
fn exact(value: &Value, nonzero: bool) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(CMD, "canonical decimal captured IDs required"))?;
    let n = s.parse::<u64>().map_err(|_| bad(CMD, "ID exceeds range"))?;
    if nonzero && n == 0 {
        return Err(bad(CMD, "ID must be nonzero"));
    }
    Ok(n)
}
#[derive(Clone, Copy)]
enum Target {
    Saved(ChannelId),
    Quick,
}
#[derive(Clone, Copy)]
enum Kind {
    Spot,
    Masked,
    Selected,
}
struct Request<'a> {
    target: Target,
    name: Option<&'a str>,
    color: Option<Color>,
    opacity: Option<f32>,
    solidity: Option<f32>,
    kind: Option<Kind>,
}
struct Owned {
    target: Target,
    name: Option<String>,
    color: Option<Color>,
    opacity: Option<f32>,
    solidity: Option<f32>,
    kind: Option<Kind>,
}
#[derive(Clone)]
struct Fields {
    name: String,
    color: Color,
    opacity: f32,
    indicates: ColorIndicates,
    spot: Option<(Color, f32)>,
}
fn color(value: &Value) -> Result<Color> {
    let rgba = match value {
        Value::String(s) if s.len() <= 9 => crate::commands::parse_hex(s).ok_or_else(|| bad(CMD, "color must be RGB hex"))?,
        Value::Array(values) if (3..=4).contains(&values.len()) => {
            let mut out = [0.0, 0.0, 0.0, 1.0];
            for (target, value) in out.iter_mut().zip(values) {
                *target = value
                    .as_f64()
                    .filter(|n| n.is_finite() && n.abs() <= f64::from(f32::MAX))
                    .ok_or_else(|| bad(CMD, "color components must be finite f32 values"))? as f32;
            }
            out
        }
        _ => return Err(bad(CMD, "color must be RGB hex or three/four finite components")),
    };
    Ok(Color::rgb(rgba[0], rgba[1], rgba[2]))
}
fn percent(value: Option<&Value>) -> Result<Option<f32>> {
    value
        .map(|v| {
            let v = v
                .as_f64()
                .filter(|n| n.is_finite() && n.abs() <= f64::from(f32::MAX))
                .ok_or_else(|| bad(CMD, "opacity/solidity must be finite percentages"))?;
            Ok((v as f32 / 100.0).clamp(0.0, 1.0))
        })
        .transpose()
}
fn request<'a>(session: &Session, value: &'a Value, allow_normalize: bool) -> Result<Request<'a>> {
    let object = value
        .as_object()
        .filter(|p| {
            p.len() <= 9
                && p.keys().all(|k| {
                    matches!(k.as_str(), "channel" | "channelId" | "name" | "color" | "opacity" | "solidity" | "indicates" | "documentId" | "expectedRevision")
                })
        })
        .ok_or_else(|| bad(CMD, "expected bounded Channel Options fields"))?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.channels.len() > 10000 || state.channel_view.alpha_visible.len() > 10000 || state.channel_view.color_hidden.len() > 5 {
        return Err(bad(CMD, "channel/view count limit"));
    }
    match (value.get("documentId"), value.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id, true)? == state.doc.id.0 && exact(rev, false)? == state.revision => {}
        _ => return Err(bad(CMD, "captured target changed or incomplete")),
    }
    if state.revision == u64::MAX
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
        || (state.floating.is_some() && (!allow_normalize || value.get("documentId").is_some() || state.doc.xcf_state.is_some()))
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || state.doc.accounted().is_some()
    {
        return Err(bad(CMD, "finish previews/gestures/jobs before Channel Options"));
    }
    if object.contains_key("channel") && object.contains_key("channelId") {
        return Err(bad(CMD, "supply one channel selector"));
    }
    let reference = if let Some(id) = value.get("channelId") {
        let id = super::channel_identity(id).ok_or_else(|| bad(CMD, "invalid channelId"))?;
        let mut found = None;
        for (index, channel) in state.doc.channels.iter().enumerate() {
            if channel.id.0 == id && found.replace(index).is_some() {
                return Err(bad(CMD, "ambiguous channelId"));
            }
        }
        ChanRef::Alpha(found.ok_or_else(|| bad(CMD, "unknown channelId"))?)
    } else {
        source_reference(value.get("channel").ok_or_else(|| bad(CMD, "missing channel"))?, &state.doc, CMD)?
    };
    let target = match reference {
        ChanRef::Alpha(index) => {
            let channel = state.doc.channels.get(index).ok_or_else(|| bad(CMD, "missing saved channel"))?;
            if channel.id.0 == 0 || state.doc.channels.iter().filter(|ch| ch.id == channel.id).count() != 1 {
                return Err(bad(CMD, "ambiguous saved channel"));
            }
            Target::Saved(channel.id)
        }
        ChanRef::QuickMask => Target::Quick,
        _ => return Err(bad(CMD, "channel must be saved alpha/spot or quickMask")),
    };
    let name = value
        .get("name")
        .map(|v| v.as_str().filter(|s| s.len() <= 4096 && !s.contains('\0')).ok_or_else(|| bad(CMD, "name must be bounded UTF8")))
        .transpose()?
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let color = value.get("color").map(color).transpose()?;
    let opacity = percent(value.get("opacity"))?;
    let solidity = percent(value.get("solidity"))?;
    let kind = value
        .get("indicates")
        .map(|v| match v.as_str() {
            Some("spot") => Ok(Kind::Spot),
            Some("masked" | "maskedAreas") => Ok(Kind::Masked),
            Some("selected" | "selectedAreas") => Ok(Kind::Selected),
            _ => Err(bad(CMD, "indicates must be masked, selected or spot")),
        })
        .transpose()?;
    if matches!(target, Target::Quick) && matches!(kind, Some(Kind::Spot)) {
        return Err(bad(CMD, "Quick Mask cannot be a spot channel"));
    }
    Ok(Request { target, name, color, opacity, solidity, kind })
}

pub(super) fn dispatch_preflight(session: &Session, value: &Value, _background: bool) -> Result<()> {
    request(session, value, true).map(|_| ())
}
fn text(value: &str) -> Result<String> {
    let mut out = String::new();
    out.try_reserve_exact(value.len()).map_err(|_| bad(CMD, "name allocation"))?;
    if out.capacity() > 8192 {
        return Err(bad(CMD, "name capacity limit"));
    }
    out.push_str(value);
    Ok(out)
}
fn owned(p: Request<'_>) -> Result<Owned> {
    Ok(Owned { target: p.target, name: p.name.map(text).transpose()?, color: p.color, opacity: p.opacity, solidity: p.solidity, kind: p.kind })
}
fn same_color(a: Color, b: Color) -> bool {
    a.mode == b.mode && a.alpha.to_bits() == b.alpha.to_bits() && a.c.iter().zip(b.c).all(|(a, b)| a.to_bits() == b.to_bits())
}
fn same_spot(a: Option<(Color, f32)>, b: Option<(Color, f32)>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some((ac, as_)), Some((bc, bs))) => same_color(ac, bc) && as_.to_bits() == bs.to_bits(),
        _ => false,
    }
}
fn same_options(a: QuickMaskOptions, b: QuickMaskOptions) -> bool {
    same_color(a.color, b.color) && a.opacity.to_bits() == b.opacity.to_bits() && a.indicates == b.indicates
}
fn same_fields(a: &Fields, b: &Fields) -> bool {
    a.name == b.name && same_color(a.color, b.color) && a.opacity.to_bits() == b.opacity.to_bits() && a.indicates == b.indicates && same_spot(a.spot, b.spot)
}
fn fields(channel: &photocraft_doc::AlphaChannel) -> Result<Fields> {
    if channel.name.len() > 4096
        || channel.name.contains('\0')
        || !channel.opacity.is_finite()
        || !channel.color.alpha.is_finite()
        || channel.color.c.iter().any(|n| !n.is_finite())
        || channel.spot.is_some_and(|(c, s)| !s.is_finite() || !c.alpha.is_finite() || c.c.iter().any(|n| !n.is_finite()))
    {
        return Err(bad(CMD, "malformed current presentation fields"));
    }
    Ok(Fields { name: text(&channel.name)?, color: channel.color, opacity: channel.opacity, indicates: channel.indicates, spot: channel.spot })
}
fn apply(before: &Fields, p: &Owned) -> Result<Fields> {
    let mut next = Fields {
        name: if let Some(name) = &p.name { text(name)? } else { text(&before.name)? },
        color: before.color,
        opacity: before.opacity,
        indicates: before.indicates,
        spot: before.spot,
    };
    match p.kind {
        Some(Kind::Spot) => {
            let (c, s) = before.spot.unwrap_or((before.color, 0.0));
            next.spot = Some((p.color.unwrap_or(c), p.solidity.unwrap_or(s)));
            return Ok(next);
        }
        Some(Kind::Masked) => {
            next.indicates = ColorIndicates::MaskedAreas;
            next.spot = None
        }
        Some(Kind::Selected) => {
            next.indicates = ColorIndicates::SelectedAreas;
            next.spot = None
        }
        None => {}
    }
    if let Some((color, solidity)) = &mut next.spot {
        *color = p.color.unwrap_or(*color);
        *solidity = p.solidity.or(p.opacity).unwrap_or(*solidity);
    } else {
        next.color = p.color.unwrap_or(next.color);
        next.opacity = p.opacity.unwrap_or(next.opacity);
    }
    Ok(next)
}
fn index(doc: &Document, id: ChannelId, ctl: &Interrupt<'_>) -> Result<usize> {
    let mut found = None;
    for (index, channel) in doc.channels.iter().enumerate() {
        poll(ctl)?;
        if channel.id == id && found.replace(index).is_some() {
            return Err(bad(CMD, "ambiguous channel identity"));
        }
    }
    found.ok_or_else(|| bad(CMD, "channel disappeared"))
}
fn imported(doc: &Document, id: ChannelId, before: &Fields, next: &Fields, ctl: &Interrupt<'_>) -> Result<Option<usize>> {
    let Some(state) = &doc.xcf_state else { return Ok(None) };
    if state.items.len() > 10000 || doc.selection_channel_id == Some(id) {
        return Err(bad(CMD, "unsupported imported selection channel"));
    }
    let mut found = None;
    for (position, item) in state.items.iter().enumerate() {
        poll(ctl)?;
        if item.item == ItemRef::Channel(id) && found.replace(position).is_some() {
            return Err(bad(CMD, "ambiguous current channel item"));
        }
    }
    let position = found.ok_or_else(|| bad(CMD, "missing current channel item"))?;
    let item = state.items.get(position).ok_or_else(|| bad(CMD, "missing current item"))?;
    let native = doc.channels.get(index(doc, id, ctl)?).ok_or_else(|| bad(CMD, "missing saved channel"))?;
    if native.name != item.name
        || item.mask.is_some()
        || item.raster_bounds.is_none_or(|b| b.is_empty())
        || native.surface.format().mode != photocraft_doc::ColorMode::Grayscale
        || native.surface.format().alpha
        || native.surface.format().sample != doc.depth
    {
        return Err(bad(CMD, "unsupported current channel domain"));
    }
    if !same_spot(before.spot, next.spot) {
        return Err(bad(CMD, "imported spot conversion/ink semantics are not qualified"));
    }
    if before.spot.is_some()
        && (!same_color(before.color, next.color) || before.opacity.to_bits() != next.opacity.to_bits() || before.indicates != next.indicates)
    {
        return Err(bad(CMD, "imported spot presentation is unsupported"));
    }
    if before.indicates != next.indicates && item.source_flags != 0 {
        return Err(bad(CMD, "unknown source flags for indicates change"));
    }
    if before.spot.is_none() {
        for color in [before.color, next.color] {
            if color.mode != photocraft_doc::ColorMode::Rgb
                || color.alpha.to_bits() != 1.0f32.to_bits()
                || color.c.get(3).is_none_or(|v| v.to_bits() != 0.0f32.to_bits())
                || color.c.iter().any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
            {
                return Err(bad(CMD, "unsupported imported RGB overlay domain"));
            }
        }
        if [before.opacity, next.opacity].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(bad(CMD, "unsupported imported opacity domain"));
        }
    }
    doc.validate_xcf_state_with(ctl).map_err(|e| error(e, ctl))?;
    crate::xcf_item_locks::require_channel_content(doc, id)?;
    poll(ctl)?;
    Ok(Some(position))
}
fn headroom(snapshot: &DocumentSnapshot, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    snapshot.validate_resident()?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, ctl).map_err(|e| error(e, ctl))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, ctl).map_err(|e| error(e, ctl))? as u64;
    let footprint = snapshot.resident_raster_footprint_with(ctl)?;
    let work = model
        .checked_mul(8)
        .and_then(|n| n.checked_add((snapshot.channels.len() as u64).checked_mul(64)?))
        .ok_or_else(|| bad(CMD, "presentation metadata work overflow"))?;
    if work > 1_000_000_000
        || model
            .checked_mul(4)
            .and_then(|n| n.checked_add(payload))
            .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|n| n.checked_add(65536))
            .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad(CMD, "retained presentation/COW headroom limit"));
    }
    poll(ctl)
}
fn install(doc: &mut Document, id: ChannelId, fields: Fields, item: Option<usize>, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let index = index(doc, id, ctl)?;
    let channel = doc.channels.get_mut(index).ok_or_else(|| bad(CMD, "missing channel"))?;
    let renamed = channel.name != fields.name;
    channel.name = fields.name;
    channel.color = fields.color;
    channel.opacity = fields.opacity;
    channel.indicates = fields.indicates;
    channel.spot = fields.spot;
    if let Some(position) = item {
        let item = doc
            .xcf_state
            .as_mut()
            .and_then(|s| s.items.get_mut(position))
            .filter(|item| item.item == ItemRef::Channel(id))
            .ok_or_else(|| bad(CMD, "canonical channel changed"))?;
        if renamed {
            item.name = text(&channel.name)?;
            item.custom_name = true;
        }
    }
    poll(ctl)
}
pub(super) fn run(session: &mut Session, value: &Value) -> Result<Value> {
    let parsed = request(session, value, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    headroom(&state.doc, &Interrupt::NONE)?;
    let parameters = owned(parsed)?;
    let baseline = state.doc.clone();
    let weak = baseline.downgrade();
    let revision = state.revision;
    let view = state.channel_view.clone();
    let options = session.quick_mask_options;
    let (before, quick) = match parameters.target {
        Target::Saved(id) => {
            let channel = baseline.channels.get(index(&baseline, id, &Interrupt::NONE)?).ok_or_else(|| bad(CMD, "missing channel"))?;
            (fields(channel)?, false)
        }
        Target::Quick => {
            if options.color.c.iter().any(|n| !n.is_finite()) || !options.opacity.is_finite() {
                return Err(bad(CMD, "invalid Quick Mask options"));
            }
            (Fields { name: "Quick Mask".into(), color: options.color, opacity: options.opacity, indicates: options.indicates, spot: None }, true)
        }
    };
    let mut next = apply(&before, &parameters)?;
    if quick {
        next.name = text(&before.name)?;
    }
    let item = match parameters.target {
        Target::Saved(id) => imported(&baseline, id, &before, &next, &Interrupt::NONE)?,
        Target::Quick => {
            if baseline.xcf_state.is_some() {
                return Err(bad(CMD, "imported Quick Mask policy is not qualified"));
            }
            None
        }
    };
    let quick_next = QuickMaskOptions { color: next.color, opacity: next.opacity, indicates: next.indicates };
    let document_changed = if quick {
        baseline
            .quick_mask
            .as_ref()
            .is_some_and(|q| !same_color(q.color, next.color) || q.opacity.to_bits() != next.opacity.to_bits() || q.indicates != next.indicates)
    } else {
        !same_fields(&before, &next)
    };
    if !document_changed
        && (!quick
            || same_color(options.color, quick_next.color)
                && options.opacity.to_bits() == quick_next.opacity.to_bits()
                && options.indicates == quick_next.indicates)
    {
        return Ok(super::list(session));
    }
    let work = baseline.clone();
    crate::jobs::run(
        session,
        if quick { "Quick Mask Options" } else { "Channel Options" },
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Channel Options", |ctl| {
                headroom(&work, ctl)?;
                poll(ctl)?;
                let replacement = if quick { super::quick_mask_controlled::reorient_untouched(&work, quick_next.indicates, ctl)? } else { None };
                let mut doc = if document_changed { Some((*work).clone()) } else { None };
                if let Some(doc) = &mut doc {
                    match parameters.target {
                        Target::Saved(id) => install(doc, id, next, item, ctl)?,
                        Target::Quick => {
                            let q = doc.quick_mask.as_mut().ok_or_else(|| bad(CMD, "live Quick Mask disappeared"))?;
                            if let Some(surface) = replacement {
                                q.surface = surface;
                            }
                            q.color = quick_next.color;
                            q.opacity = quick_next.opacity;
                            q.indicates = quick_next.indicates;
                        }
                    }
                }
                poll(ctl)?;
                Ok((doc, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.channel_view != view
                || !same_options(session.quick_mask_options, options)
                || current.floating.is_some()
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || session.command_has_job_on(current.doc.id)
            {
                return Err(bad(CMD, "document/view/options changed while preparing Channel Options"));
            }
            ctx.stage(0.95, 1.0, "Channel Options", |ctl| {
                poll(ctl)?;
                if let Some(document) = document {
                    session.edit_with_interrupt(if quick { "Quick Mask Options" } else { "Channel Options" }, ctl, move |doc, _| {
                        poll(ctl)?;
                        *doc = document;
                        Ok(())
                    })?;
                }
                // No cancellation failure can follow a successful history commit.
                // Session-only settings publish only after that commit succeeded.
                if quick {
                    session.quick_mask_options = quick_next;
                }
                Ok(super::list(session))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, ColorMode, Size};
    use photocraft_raster::Surface;
    use serde_json::json;

    #[test]
    fn saved_options_preserve_wide_native_samples_and_noop_redo() {
        let mut doc = Document::new("Options", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let surface = Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap();
        doc.channels.push(AlphaChannel::new("Native", surface));
        let original = doc.channels[0].surface.clone();
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let params = json!({"channel":0,"name":"Renamed","color":[0.2,0.3,0.4],"opacity":25});
        run(&mut session, &params).unwrap();
        assert_eq!(session.active().unwrap().doc.channels[0].surface, original);
        assert_eq!(session.active().unwrap().doc.channels[0].name, "Renamed");
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc.channels[0].name, "Native");
        assert!(session.redo());
        session
            .edit("Redo seed", |doc, _| {
                doc.name.push_str(" edited");
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let revision = session.active().unwrap().revision;
        run(&mut session, &params).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
    }

    #[test]
    fn quick_options_without_live_mask_are_session_only_and_spot_refusal_is_atomic() {
        let mut session = Session::new();
        session.add_document(Document::new("Quick", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        run(&mut session, &json!({"channel":"quickMask","opacity":37,"indicates":"selected"})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert_eq!(session.quick_mask_options.opacity, 0.37);
        let options = session.quick_mask_options;
        assert!(run(&mut session, &json!({"channel":"quickMask","indicates":"spot","color":"#ffffff"})).is_err());
        assert!(same_options(options, session.quick_mask_options));
    }

    #[test]
    fn spot_defaults_and_hostile_envelopes_preserve_legacy_order() {
        let before = Fields {
            name: "Spot".into(),
            color: Color::rgb(1.0, 0.0, 0.0),
            opacity: 0.5,
            indicates: ColorIndicates::MaskedAreas,
            spot: Some((Color::rgb(0.0, 0.0, 1.0), 0.2)),
        };
        let params = Owned { target: Target::Quick, name: None, color: None, opacity: Some(0.8), solidity: None, kind: Some(Kind::Spot) };
        let result = apply(&before, &params).unwrap();
        assert!(same_spot(result.spot, before.spot)); // explicit spot keeps prior solidity, ignoring opacity
        assert_eq!(result.opacity.to_bits(), before.opacity.to_bits());
        assert!(color(&json!([0.0, {}, 1.0])).is_err());
        assert!(percent(Some(&json!("50"))).is_err());
        assert!(exact(&json!("01"), true).is_err());
    }
}
