//! Metadata-only refusal policy for remaining legacy f32/byte color-selection samplers.
//! The actual sampled owner and previous coverage are checked separately. This does
//! not authorize source rendering, invent a conversion space, or inspect pixel values.
#[cfg(test)]
#[path = "backed_sampling_guard_tests.rs"]
mod backed_sampling_guard_tests;
use crate::{EngineError, Result, Session};
use photocraft_color::SampleType;
use photocraft_doc::{Document, Layer, LayerId, MAX_GROUP_DEPTH};
use photocraft_raster::{Interrupt, Surface};
use serde_json::Value;
fn bad(cmd: &str, reason: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: format!("selection.sample.{reason}") }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn captured_target(session: &Session, cmd: &str, p: &Value, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let decimal = |value: &Value| {
        value
            .as_str()
            .filter(|text| {
                !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
            })
            .and_then(|text| text.parse::<u64>().ok())
    };
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => Ok(()),
        (Some(id), Some(revision)) => {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id.0 != 0 && decimal(id) == Some(current.doc.id.0) && decimal(revision) == Some(current.revision) {
                Ok(())
            } else {
                Err(bad(cmd, "captured-target-changed"))
            }
        }
        _ => Err(bad(cmd, "captured-target-incomplete")),
    }
}
fn wide(sample: SampleType) -> bool {
    matches!(sample, SampleType::U32 | SampleType::F64)
}
fn surface(surface: &Surface, cmd: &str) -> Result<()> {
    if wide(surface.format().sample) {
        return Err(bad(cmd, "native-owner-unsupported"));
    }
    Ok(())
}
fn layers<'a>(doc: &'a Document, visible_only: bool, ctl: &Interrupt<'_>, mut visit: impl FnMut(&'a Layer) -> Result<()>) -> Result<()> {
    let mut stack: [Option<std::slice::Iter<'a, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *stack.first_mut().ok_or_else(|| bad("selection.sample", "hierarchy"))? = Some(doc.layers.iter());
    let mut depth = 0usize;
    let mut count = 0usize;
    loop {
        poll(ctl)?;
        let next = stack.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("selection.sample", "owner-count-limit"))?;
        if visible_only && !layer.visible {
            continue;
        }
        visit(layer)?;
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad("selection.sample", "hierarchy-limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad("selection.sample", "hierarchy"))? = Some(children.iter());
        }
    }
    poll(ctl)
}
fn find_layer<'a>(doc: &'a Document, id: Option<LayerId>, ctl: &Interrupt<'_>) -> Result<Option<&'a Layer>> {
    let Some(id) = id else { return Ok(None) };
    let mut found = None;
    layers(doc, false, ctl, |layer| {
        if found.is_none() && layer.id == id {
            found = Some(layer);
        }
        Ok(())
    })?;
    Ok(found)
}
fn composite(doc: &Document, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    // Composite interpretation is tied to the document's declared current depth;
    // unlike a direct active-layer or shown-Gray read, this route invokes rendering.
    if wide(doc.depth) {
        return Err(bad(cmd, "native-composite-unsupported"));
    }
    layers(doc, true, ctl, |layer| {
        if let Some(pixels) = layer.surface() {
            surface(pixels, cmd)?;
        }
        if let Some(mask) = layer.mask.as_ref().filter(|m| m.enabled) {
            surface(&mask.surface, cmd)?;
        }
        if let Some(fill) = &layer.fill_cache {
            surface(&fill.surface, cmd)?;
        }
        Ok(())
    })?;
    // Existing filter/auxiliary source qualification remains the compositor's
    // authority. This metadata guard does not turn stored mask formats into
    // presumed sampled inputs or inspect disabled/inert source payloads.
    poll(ctl)
}
/// Covers previous selection reads and the current declared canonical destination
/// that these legacy commands would otherwise replace with an implicit GRAY8 mask.
pub(crate) fn existing_selection(session: &Session, cmd: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    if doc.selection.as_ref().is_some_and(|s| wide(s.format().sample)) || doc.selection_channel_id.is_some() && wide(doc.depth) {
        return Err(bad(cmd, "native-selection-unsupported"));
    }
    poll(ctl)
}
/// Direct layer sampling preserves the active pixel owner before any nonpixel fallback.
pub(crate) fn sampled_pixels(session: &Session, cmd: &str, all_layers: bool, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !all_layers {
        if let Some(layer) = find_layer(&state.doc, state.active_layer, ctl)? {
            if matches!(layer.content, photocraft_doc::LayerContent::BackedRaster(_)) {
                return Err(bad(cmd, "active-backed-pixel-sampling-unsupported"));
            }
            if let Some(pixels) = layer.surface() {
                surface(pixels, cmd)?;
                return poll(ctl);
            }
        }
    }
    composite(&state.doc, cmd, ctl)
}
/// Exact shown_gray precedence: shown Gray mask, then first visible saved alpha,
/// Quick Mask, shown overlay mask, active layer mask, then ordinary color sampling.
/// Merely returning a borrowed owner confers no write or source-render authority.
pub(crate) fn shown_gray_surface<'a>(session: &'a Session, cmd: &str, ctl: &Interrupt<'_>) -> Result<Option<&'a Surface>> {
    poll(ctl)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    let view = &state.channel_view;
    let shown = if let Some(v) = view.layer_mask {
        find_layer(doc, Some(v.layer), ctl)?.and_then(|layer| layer.mask.as_ref()).map(|mask| (&*mask.surface, v.mode))
    } else {
        None
    };
    let selected = if let Some((mask, crate::mask_view_cmds::MaskViewMode::Gray)) = shown {
        Some(mask)
    } else if view.visible_colors(crate::channel_cmds::color_count(doc)) == 0 {
        if doc.channels.len() > 100000 {
            return Err(bad(cmd, "channel-count-limit"));
        }
        let mut alpha = None;
        for (index, channel) in doc.channels.iter().enumerate() {
            poll(ctl)?;
            if view.alpha_shown(index) {
                alpha = Some(&channel.surface);
                break;
            }
        }
        if alpha.is_some() {
            alpha
        } else if let Some(quick) = doc.quick_mask.as_ref().filter(|_| !view.quick_mask_hidden) {
            Some(&quick.surface)
        } else if let Some((mask, _)) = shown {
            Some(mask)
        } else {
            find_layer(doc, state.active_layer, ctl)?.and_then(|layer| layer.mask.as_ref()).map(|mask| &*mask.surface)
        }
    } else {
        None
    };
    if let Some(pixels) = selected {
        surface(pixels, cmd)?;
    }
    poll(ctl)?;
    Ok(selected)
}
/// Preview uses the same actual sampling/old-coverage guards, but does not request
/// write authority or falsely refuse an ordinary read solely due to content locks.
pub(crate) fn color_range_preview(session: &Session, p: &Value, ctl: &Interrupt<'_>) -> Result<()> {
    captured_target(session, "select.colorRange", p, ctl)?;
    existing_selection(session, "select.colorRange", ctl)?;
    sampled_pixels(session, "select.colorRange", p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true), ctl)
}
pub(crate) fn dispatch_preflight(session: &Session, cmd: &str, p: &Value, _background: bool) -> Result<()> {
    admission(session, cmd, p, &Interrupt::NONE, true)
}
/// Metadata admission for commands whose worker performs the full tokened
/// source validation before pixels and again before publishing an edit.
pub(crate) fn metadata_preflight(session: &Session, cmd: &str, p: &Value, ctl: &Interrupt<'_>) -> Result<()> {
    admission(session, cmd, p, ctl, false)
}
fn admission(session: &Session, cmd: &str, p: &Value, ctl: &Interrupt<'_>, full: bool) -> Result<()> {
    captured_target(session, cmd, p, ctl)?;
    existing_selection(session, cmd, ctl)?;
    match cmd {
        "select.magicWand" => {
            if shown_gray_surface(session, cmd, ctl)?.is_none() {
                sampled_pixels(session, cmd, p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(false), ctl)?;
            }
        }
        "select.grow" | "select.similar" => sampled_pixels(session, cmd, p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(false), ctl)?,
        "select.colorRange" => sampled_pixels(
            session,
            cmd,
            if p.get("select").and_then(Value::as_str) == Some("outOfGamut") {
                true
            } else {
                p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true)
            },
            ctl,
        )?,
        _ => return Err(bad(cmd, "unknown-route")),
    }
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    if full {
        crate::selection_inverse_controlled::selection_write_authority(doc, ctl)
    } else {
        crate::selection_inverse_controlled::selection_write_authority_metadata(doc, ctl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    use photocraft_doc::{AlphaChannel, LayerContent, LayerMask};
    fn session(depth: SampleType) -> Session {
        let mut doc = Document::new("owner", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, depth);
        doc.layers.push(Layer::raster("pixels", doc.pixel_format()));
        let mut s = Session::new();
        s.add_document(doc, None).unwrap();
        s
    }
    fn hide_colors(s: &mut Session) {
        s.active_mut().unwrap().channel_view.color_hidden = vec![true; 3];
    }
    #[test]
    fn shown_narrow_alpha_precedes_wide_rgb_but_wide_previous_selection_is_independent() {
        let mut s = session(SampleType::U32);
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().channels.push(AlphaChannel::new("shown", Surface::new(PixelFormat::GRAY8)));
        hide_colors(&mut s);
        s.active_mut().unwrap().channel_view.alpha_visible = vec![true];
        assert!(dispatch_preflight(&s, "select.magicWand", &Value::Null, false).is_ok());
        assert!(dispatch_preflight(&s, "select.grow", &Value::Null, false).is_err());
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().selection = Some(Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false)));
        assert!(dispatch_preflight(&s, "select.magicWand", &Value::Null, false).is_err());
    }
    #[test]
    fn direct_narrow_active_cache_is_not_rejected_by_unrelated_declared_wide_doc() {
        let mut s = session(SampleType::F64);
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].content = LayerContent::Raster(Surface::new(PixelFormat::RGBA8));
        assert!(sampled_pixels(&s, "select.grow", false, &Interrupt::NONE).is_ok());
        assert!(sampled_pixels(&s, "select.grow", true, &Interrupt::NONE).is_err());
    }
    #[test]
    fn shown_mask_precedence_and_public_preview_refusal_do_not_read_native_words() {
        let mut s = session(SampleType::U8);
        let id = s.active().unwrap().doc.layers[0].id;
        let wide = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &f64::NAN.to_ne_bytes()).unwrap();
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].mask =
            Some(LayerMask { surface: std::sync::Arc::new(wide), enabled: false, linked: true, density: 1., feather: 0. });
        s.active_mut().unwrap().channel_view.layer_mask =
            Some(crate::mask_view_cmds::LayerMaskView { layer: id, mode: crate::mask_view_cmds::MaskViewMode::Gray });
        assert!(shown_gray_surface(&s, "select.magicWand", &Interrupt::NONE).is_err());
        s.active_mut().unwrap().channel_view.layer_mask = None;
        assert!(crate::selection_cmds::ColorRangeSamples::new(&s, &Value::Null).is_ok());
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().selection = Some(Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false)));
        let before = s.active().unwrap().doc.clone();
        assert!(crate::selection_cmds::ColorRangeSamples::new(&s, &Value::Null).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert!(matches!(shown_gray_surface(&s, "select.magicWand", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
}
