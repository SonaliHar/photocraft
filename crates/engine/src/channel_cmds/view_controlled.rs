//! Bounded pure channel-view publication; no document or history mutation.
use super::{ChanRef, ChannelTarget, ChannelView, bad, color_count};
use crate::{EngineError, Result, Session};
use photocraft_raster::Interrupt;
use serde_json::Value;

const MAX_CHANNELS: usize = 10_000;
const MAX_NAMES: usize = 1 << 20;
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(value: &Value, nonzero: bool, cmd: &str) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(cmd, "captured IDs require canonical decimal strings"))?;
    let n = s.parse::<u64>().map_err(|_| bad(cmd, "captured ID overflow"))?;
    if nonzero && n == 0 {
        return Err(bad(cmd, "zero document ID"));
    }
    Ok(n)
}
fn slot(command: &str) -> Option<usize> {
    match command {
        "channel.target.composite" => Some(2),
        "channel.target.slot3" => Some(3),
        "channel.target.slot4" => Some(4),
        "channel.target.slot5" => Some(5),
        "channel.target.slot6" => Some(6),
        "channel.target.slot7" => Some(7),
        "channel.target.slot8" => Some(8),
        "channel.target.slot9" => Some(9),
        _ => None,
    }
}
fn admission(session: &Session, command: &str, params: &Value, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let parameterized = matches!(command, "channel.target" | "channel.setVisible");
    if !parameterized && slot(command).is_none() {
        return Err(bad(command, "unknown channel view command"));
    }
    if !params.is_null() {
        let map = params.as_object().ok_or_else(|| bad(command, "view parameters must be an object or null"))?;
        if map.len() > 5
            || map.keys().any(|k| {
                !matches!(k.as_str(), "channel" | "channelId" | "visible" | "documentId" | "expectedRevision")
                    || (!parameterized && !matches!(k.as_str(), "documentId" | "expectedRevision"))
                    || (command == "channel.target" && k == "visible")
            })
        {
            return Err(bad(command, "unknown channel view field"));
        }
    }
    if params.get("channel").is_some() && params.get("channelId").is_some() {
        return Err(bad(command, "supply one channel selector"));
    }
    if let Some(value) = params.get("visible")
        && !value.is_boolean()
    {
        return Err(bad(command, "visible must be boolean"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.view_revision == u64::MAX
        || state.doc.channels.len() > MAX_CHANNELS
        || state.channel_view.alpha_visible.len() > MAX_CHANNELS
        || state.channel_view.color_hidden.len() > 5
        || state.channel_view.alpha_visible.capacity() > MAX_CHANNELS * 2
        || state.channel_view.color_hidden.capacity() > 64
    {
        return Err(bad(command, "channel view/revision bound"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) => {
            if decimal(id, true, command)? != state.doc.id.0 || decimal(revision, false, command)? != state.revision {
                return Err(bad(command, "captured channel view changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || session.command_has_job_on(state.doc.id)
            {
                return Err(bad(command, "finish captured target preview/gesture/job"));
            }
        }
        _ => return Err(bad(command, "documentId and expectedRevision must be paired")),
    }
    let mut names = 0usize;
    for channel in &state.doc.channels {
        poll(ctl)?;
        names = names.checked_add(channel.name.len()).ok_or_else(|| bad(command, "channel names overflow"))?;
        if channel.name.len() > 4096 || names > MAX_NAMES {
            return Err(bad(command, "channel output names bound"));
        }
    }
    // Bound metadata traversal used by the existing mask-view reconciliation.
    photocraft_io::retained_native_model_bytes(&state.doc, photocraft_io::NativeSaveLimits::default().preparation, ctl)
        .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { bad(command, e.to_string()) })?;
    poll(ctl)
}
fn reference(session: &Session, command: &str, params: &Value) -> Result<ChanRef> {
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if let Some(value) = params.get("channel").and_then(Value::as_object)
        && value.len() != 1
    {
        return Err(bad(command, "channel reference requires exactly one selector"));
    }
    let identity = params.get("channelId").or_else(|| params.get("channel").and_then(|v| v.as_object()).and_then(|v| v.get("channelId")));
    if let Some(value) = identity {
        let id = super::channel_identity(value).ok_or_else(|| bad(command, "invalid channelId"))?;
        let mut index = None;
        for (position, channel) in state.doc.channels.iter().enumerate() {
            if channel.id.0 == id && index.replace(position).is_some() {
                return Err(bad(command, "ambiguous channelId"));
            }
        }
        return index.map(ChanRef::Alpha).ok_or_else(|| bad(command, "unknown channelId"));
    }
    let result = match params.get("channel") {
        Some(value) => super::source_reference(value, &state.doc, command),
        None if command == "channel.target" => Ok(ChanRef::Composite),
        None => Err(bad(command, "missing channel")),
    }?;
    if matches!(result, ChanRef::LayerMask | ChanRef::VectorMask) {
        let layer = state.active_layer.and_then(|id| state.doc.layer(id)).ok_or_else(|| bad(command, "no active mask layer"))?;
        if result == ChanRef::LayerMask && layer.mask.is_none() {
            return Err(bad(command, "the active layer has no layer mask"));
        }
        if result == ChanRef::VectorMask && layer.vector_mask.is_none() {
            return Err(bad(command, "the active layer has no vector mask"));
        }
    }
    Ok(result)
}
fn target_reference(reference: ChanRef, colors: usize, channels: usize, command: &str) -> Result<ChannelTarget> {
    match reference {
        ChanRef::Composite => Ok(ChannelTarget::Composite),
        ChanRef::LayerMask => Ok(ChannelTarget::LayerMask),
        ChanRef::VectorMask => Ok(ChannelTarget::VectorMask),
        ChanRef::Color(k) if colors > 1 && k < colors => Ok(ChannelTarget::Color(k)),
        ChanRef::Alpha(k) if k < channels => Ok(ChannelTarget::Alpha(k)),
        _ => Err(bad(command, "channel is not a target")),
    }
}
fn slot_target(number: usize, colors: usize, channels: usize, command: &str) -> Result<ChannelTarget> {
    if number == 2 {
        return Ok(ChannelTarget::Composite);
    }
    let index = number.checked_sub(3).ok_or_else(|| bad(command, "unknown channel shortcut"))?;
    let shown = if colors > 1 { colors } else { 0 };
    if index < shown {
        return Ok(ChannelTarget::Color(index));
    }
    let alpha = index.checked_sub(shown).ok_or_else(|| bad(command, "channel shortcut overflow"))?;
    if alpha < channels { Ok(ChannelTarget::Alpha(alpha)) } else { Err(bad(command, format!("no channel for {}{number}", super::SLOT_KEY))) }
}
fn booleans(source: &[bool], length: usize, fill: bool, command: &str) -> Result<Vec<bool>> {
    if length > MAX_CHANNELS {
        return Err(bad(command, "view vector limit"));
    }
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| bad(command, "view allocation"))?;
    if values.capacity() > MAX_CHANNELS * 2 {
        return Err(bad(command, "view allocation capacity"));
    }
    values.extend(source.iter().take(length).copied());
    values.resize(length, fill);
    Ok(values)
}
fn copy_view(view: &ChannelView, command: &str) -> Result<ChannelView> {
    Ok(ChannelView {
        target: view.target,
        color_hidden: booleans(&view.color_hidden, view.color_hidden.len(), false, command)?,
        alpha_visible: booleans(&view.alpha_visible, view.alpha_visible.len(), false, command)?,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
pub(super) fn dispatch_preflight(session: &Session, command: &str, params: &Value, _background: bool) -> Result<()> {
    admission(session, command, params, &Interrupt::NONE)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let colors = color_count(&state.doc);
    let n = state.doc.channels.len();
    if let Some(slot) = slot(command) {
        slot_target(slot, colors, n, command)?;
    } else {
        let reference = reference(session, command, params)?;
        if command == "channel.target" {
            target_reference(reference, colors, n, command)?;
        } else if !matches!(reference, ChanRef::Composite | ChanRef::QuickMask)
            && !matches!(reference, ChanRef::Color(k) if k < colors)
            && !matches!(reference, ChanRef::Alpha(k) if k < n)
        {
            return Err(bad(command, "channel has no visibility"));
        }
    }
    Ok(())
}
pub(super) fn run(session: &mut Session, command: &str, params: &Value) -> Result<Value> {
    run_with_interrupt(session, command, params, &Interrupt::NONE)
}
fn run_with_interrupt(session: &mut Session, command: &str, params: &Value, ctl: &Interrupt<'_>) -> Result<Value> {
    admission(session, command, params, ctl)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let view_revision = state.view_revision;
    let weak = baseline.downgrade();
    let before = copy_view(&state.channel_view, command)?;
    let mut next = copy_view(&before, command)?;
    let colors = color_count(&baseline);
    let n = baseline.channels.len();
    if command == "channel.setVisible" {
        let explicit = params.get("visible").and_then(Value::as_bool);
        match reference(session, command, params)? {
            ChanRef::Composite => {
                let on = explicit.unwrap_or(next.visible_colors(colors) != colors);
                next.color_hidden = booleans(&[], colors, !on, command)?;
            }
            ChanRef::Color(k) if k < colors => {
                let on = explicit.unwrap_or(!next.color_visible(k));
                next.color_hidden = booleans(&next.color_hidden, next.color_hidden.len().max(k + 1), false, command)?;
                *next.color_hidden.get_mut(k).ok_or_else(|| bad(command, "color view bounds"))? = !on;
            }
            ChanRef::Alpha(k) if k < n => {
                let on = explicit.unwrap_or(!next.alpha_shown(k));
                next.alpha_visible = booleans(&next.alpha_visible, next.alpha_visible.len().max(k + 1), false, command)?;
                *next.alpha_visible.get_mut(k).ok_or_else(|| bad(command, "alpha view bounds"))? = on;
            }
            ChanRef::QuickMask => {
                next.quick_mask_hidden = !explicit.unwrap_or(next.quick_mask_hidden);
            }
            _ => return Err(bad(command, "channel has no visibility")),
        }
    } else {
        let target = if let Some(slot) = slot(command) {
            slot_target(slot, colors, n, command)?
        } else {
            target_reference(reference(session, command, params)?, colors, n, command)?
        };
        next.target = target;
        next.editing_mask = None;
        match target {
            ChannelTarget::Composite => {
                next.color_hidden.clear();
                next.alpha_visible.clear();
            }
            ChannelTarget::LayerMask | ChannelTarget::VectorMask => next.color_hidden.clear(),
            ChannelTarget::Color(k) => {
                next.color_hidden = booleans(&[], colors, true, command)?;
                *next.color_hidden.get_mut(k).ok_or_else(|| bad(command, "color bounds"))? = false;
                next.alpha_visible.clear();
            }
            ChannelTarget::Alpha(k) => {
                next.color_hidden = booleans(&[], colors, true, command)?;
                next.alpha_visible = booleans(&[], n, false, command)?;
                *next.alpha_visible.get_mut(k).ok_or_else(|| bad(command, "alpha bounds"))? = true;
            }
        }
    }
    poll(ctl)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    if current.doc.id != baseline.id
        || current.revision != revision
        || current.view_revision != view_revision
        || !weak.is_current(&current.doc)
        || !current.doc.shares_state(&baseline)
        || current.channel_view != before
    {
        return Err(bad(command, "channel view changed before publication"));
    }
    let next_revision = revision.checked_add(1).ok_or_else(|| bad(command, "revision overflow"))?;
    let next_view_revision = view_revision.checked_add(1).ok_or_else(|| bad(command, "view revision overflow"))?;
    poll(ctl)?;
    let current = session.active_mut().ok_or(EngineError::NoDocument)?;
    let clean = current.saved_revision == revision;
    current.channel_view = next;
    super::fix_view(current);
    current.revision = next_revision;
    current.view_revision = next_view_revision;
    if clean {
        current.saved_revision = next_revision;
    }
    current.last_damage = None;
    session.tools.target_mask(session.active().is_some_and(|state| state.channel_view.target == ChannelTarget::LayerMask));
    Ok(super::list(session))
}
pub(super) fn target_slot(session: &mut Session, number: usize) -> Result<Value> {
    let command = match number {
        2 => "channel.target.composite",
        3 => "channel.target.slot3",
        4 => "channel.target.slot4",
        5 => "channel.target.slot5",
        6 => "channel.target.slot6",
        7 => "channel.target.slot7",
        8 => "channel.target.slot8",
        9 => "channel.target.slot9",
        _ => return Err(bad("channel.target", "shortcut outside 2..9")),
    };
    run(session, command, &Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{AlphaChannel, Document, Size, item_identity::ChannelId};
    use photocraft_raster::Surface;
    use serde_json::json;

    fn session() -> Session {
        let mut doc = Document::new("View", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let surface = Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap();
        let mut channel = AlphaChannel::new("Native", surface);
        channel.id = ChannelId(9_007_199_254_740_995);
        doc.channels.push(channel);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }

    #[test]
    fn exact_id_view_preserves_native_document_history_and_clean_revision() {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let saved = s.active().unwrap().saved_revision;
        let view_revision = s.active().unwrap().view_revision;
        run(&mut s, "channel.target", &json!({"channelId":"9007199254740995"})).unwrap();
        let st = s.active().unwrap();
        assert!(st.doc.shares_state(&before));
        assert_eq!(st.doc.channels[0].surface, before.channels[0].surface);
        assert_eq!(st.channel_view.target, ChannelTarget::Alpha(0));
        assert_eq!(st.revision, revision + 1);
        assert_eq!(st.view_revision, view_revision + 1);
        assert_eq!(st.revision - st.view_revision, revision - view_revision);
        if saved == revision {
            assert_eq!(st.saved_revision, st.revision);
        }
        assert!(!s.undo());
        run(&mut s, "channel.setVisible", &json!({"channelId":"9007199254740995","visible":false})).unwrap();
        assert!(s.active().unwrap().channel_view.color_visible(0)); // existing never-blank view fix
        assert!(s.active().unwrap().doc.shares_state(&before));
    }

    #[test]
    fn cancellation_overflow_and_hostile_selectors_leave_view_unchanged() {
        let mut s = session();
        let view = s.active().unwrap().channel_view.clone();
        let revision = s.active().unwrap().revision;
        assert!(matches!(run_with_interrupt(&mut s, "channel.target", &json!({"channel":0}), &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(s.active().unwrap().channel_view, view);
        for p in [json!({"channelId":"01"}), json!({"channelId":{},"channel":0}), json!({"channel":0,"visible":"yes"}), json!({"documentId":"1"})] {
            assert!(dispatch_preflight(&s, "channel.setVisible", &p, false).is_err());
        }
        assert_eq!(s.active().unwrap().revision, revision);
        s.active_mut().unwrap().view_revision = u64::MAX;
        assert!(target_slot(&mut s, 2).is_err());
        assert_eq!(s.active().unwrap().channel_view, view);
        assert_eq!(s.active().unwrap().revision, revision);
        s.active_mut().unwrap().view_revision = 0;
        s.active_mut().unwrap().revision = u64::MAX;
        assert!(target_slot(&mut s, 2).is_err());
        assert_eq!(s.active().unwrap().channel_view, view);
    }

    #[test]
    fn shortcut_bounds_and_quick_visibility_keep_legacy_behavior() {
        let mut s = session();
        target_slot(&mut s, 6).unwrap(); // RGB then first saved alpha
        assert_eq!(s.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));
        assert!(target_slot(&mut s, 9).is_err());
        run(&mut s, "channel.setVisible", &json!({"channel":"quickMask"})).unwrap();
        assert!(s.active().unwrap().channel_view.quick_mask_hidden);
        run(&mut s, "channel.setVisible", &json!({"channel":"quickMask","visible":true})).unwrap();
        assert!(!s.active().unwrap().channel_view.quick_mask_hidden);
    }
}
