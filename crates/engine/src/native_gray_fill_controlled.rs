//! Controlled native Gray Fill/Clear for retained masks and channel storage.
//! Ordinary-depth and pixel-layer operations keep their existing command paths.
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, item_identity::ChannelId};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
use std::sync::Arc;
pub(crate) mod unit_math;
use unit_math::Blend;
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::Other(format!("native Gray Fill/Clear: {}", message.into()))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) }
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
    value: f64,
    opacity: f64,
    blend: Blend,
    u32_percent: Option<u8>,
    contents: &'static str,
    clear: bool,
}
fn request(session: &Session, id: &str, p: &Value) -> Result<Option<Request>> {
    if !matches!(id, "edit.fill" | "edit.clear") {
        return Ok(None);
    }
    let Some(target) = resolve(session, p)? else { return Ok(None) };
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let format = surface(&st.doc, target)?.format();
    if !matches!(format.sample, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    if format.mode != ColorMode::Grayscale || format.alpha {
        return Err(bad("native mask/channel editing requires Gray without alpha"));
    }
    if let Target::Mask(id) = target {
        let mut found = 0usize;
        photocraft_doc::resident_structure::visit_layers_with(&st.doc, &Interrupt::NONE, |layer| {
            if layer.id == id {
                found = found.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
            }
            Ok(true)
        })?;
        if id.0 == 0 || found != 1 {
            return Err(bad("ambiguous mask owner"));
        }
    }
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 9).ok_or_else(|| bad("bounded parameters required"))?;
        if o.keys().any(|k| {
            !matches!(k.as_str(), "layer" | "target" | "contents" | "color" | "mode" | "opacity" | "preserveTransparency" | "documentId" | "expectedRevision")
        }) {
            return Err(bad("unsupported native Gray parameter"));
        }
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(doc), Some(rev)) if decimal(doc) == Some(st.doc.id.0) && st.doc.id.0 != 0 && decimal(rev) == Some(st.revision) => {}
        _ => return Err(bad("captured document changed or incomplete")),
    }
    context(session)?;
    locks(&st.doc, target)?;
    let clear = id == "edit.clear";
    let contents = if clear {
        "background"
    } else {
        match p.get("contents") {
            None | Some(Value::Null) => "color",
            Some(Value::String(s)) if s.len() <= 32 => {
                crate::fill_cmds::CONTENTS.iter().copied().find(|v| v.eq_ignore_ascii_case(s)).ok_or_else(|| bad("unknown Fill contents"))?
            }
            _ => return Err(bad("contents must be a bounded string")),
        }
    };
    let blend = if clear {
        Blend::Normal
    } else {
        match p.get("mode") {
            None | Some(Value::Null) => Blend::Normal,
            Some(Value::String(s)) if s.len() <= 32 => match crate::commands::blend_from_str(s) {
                Some(BlendMode::Normal) => Blend::Normal,
                Some(BlendMode::Multiply) => Blend::Multiply,
                Some(BlendMode::Screen) => Blend::Screen,
                _ => return Err(bad("native Gray Fill supports Normal, Multiply or Screen only")),
            },
            _ => return Err(bad("bounded blend mode required")),
        }
    };
    if !clear && p.get("preserveTransparency").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
        return Err(bad("preserveTransparency must be Boolean"));
    }
    let rgba = match contents {
        "foreground" => session.tools.foreground.map(f64::from),
        "background" => session.tools.background.map(f64::from),
        "color" => {
            if let Some(v) = p.get("color") {
                match v {
                    Value::Null => {}
                    Value::String(s) if s.len() <= 9 => {}
                    Value::Array(a) if matches!(a.len(), 3 | 4) => {}
                    _ => return Err(bad("bounded color required")),
                }
            }
            crate::paint_cmds::wide_color(p.get("color"), session.tools.foreground)?
        }
        "black" => [0.0, 0.0, 0.0, 1.0],
        "gray" => [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0],
        "white" => [1.0; 4],
        _ => return Err(bad("native Gray pattern/history/Content-Aware Fill is unsupported")),
    };
    // Reuse the established mask/channel solid-color policy; no stored RGB pixel
    // or document CMS transform participates in native Gray sample writing.
    let gray_params = json!({"target":"mask"});
    let values = crate::brush_cmds::native_wide_color(&st.doc, format, &gray_params, rgba)?;
    let value = values.first().copied().ok_or_else(|| bad("Gray constant width"))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(bad("Gray constant must be finite unit"));
    }
    let opacity = if clear { 1.0 } else { crate::paint_cmds::wide_opacity(p)? * rgba[3] };
    let u32_percent = if format.sample == SampleType::U32 && blend != Blend::Normal {
        let percent = match p.get("opacity") {
            None | Some(Value::Null) => 100.0,
            Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("opacity must be finite"))?.clamp(0.0, 100.0),
        };
        if percent.fract() != 0.0 || rgba[3] != 1.0 {
            return Err(bad("U32 Multiply/Screen require whole-percent opacity and opaque solid color"));
        }
        Some(percent as u8)
    } else {
        None
    };
    Ok(Some(Request { target, value, opacity, blend, u32_percent, contents, clear }))
}
pub(crate) fn dispatch_preflight(session: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    request(session, id, p).map(|r| r.is_some())
}
fn cells(area: Rect) -> Result<u64> {
    if area.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(area.x0, area.y0);
    let last = TileCoord::containing(area.x1.checked_sub(1).ok_or_else(|| bad("extent"))?, area.y1.checked_sub(1).ok_or_else(|| bad("extent"))?);
    u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|w| u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| bad("tile span overflow"))
}
fn unit(surface: &Surface, ctl: &Interrupt<'_>) -> Result<()> {
    if surface.format().mode != ColorMode::Grayscale || surface.format().alpha {
        return Err(bad("coverage requires Gray without alpha"));
    }
    surface.try_validate_tile_origins_with(ctl).map_err(|e| error(ctl, e))?;
    let sample = surface.format().sample;
    let default = photocraft_color::try_read_sample_f64(surface.default_bytes(), sample, 0).map_err(|e| bad(e.to_string()))?;
    if !default.is_finite() || !(0.0..=1.0).contains(&default) {
        return Err(bad("non-unit native default"));
    }
    for (cell, _) in surface.tiles() {
        poll(ctl)?;
        let raw = surface.try_read_interleaved_with(cell.rect(), ctl).map_err(|e| error(ctl, e))?;
        for (i, pixel) in raw.chunks_exact(sample.bytes()).enumerate() {
            if i % 256 == 0 {
                poll(ctl)?;
            }
            let v = photocraft_color::try_read_sample_f64(pixel, sample, 0).map_err(|e| bad(e.to_string()))?;
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(bad("native Gray coverage is not finite unit"));
            }
        }
    }
    poll(ctl)
}
fn prepare(doc: &Document, r: &Request, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    poll(ctl)?;
    locks(doc, r.target)?;
    let source = surface(doc, r.target)?;
    let area = match &doc.selection {
        Some(s) => {
            let b = s.try_content_bounds_with_interrupt(ctl).map_err(|e| error(ctl, e))?;
            if r.clear { b.intersect(&doc.bounds()) } else { b }
        }
        None => doc.bounds(),
    };
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let count = cells(area)?;
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("output extent overflow"))?;
    let output = pixels
        .checked_mul(source.format().bytes_per_pixel() as u64)
        .and_then(|n| n.checked_add(count.checked_mul(2048)?))
        .ok_or_else(|| bad("output accounting overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("model accounting overflow"))?)
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("staging overflow"))?;
    let visits = (source.tile_count() as u64)
        .checked_add(doc.selection.as_ref().map_or(0, |s| s.tile_count() as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .ok_or_else(|| bad("coverage work overflow"))?;
    let work = visits
        .checked_add(pixels)
        .and_then(|n| n.checked_mul(128))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(16)?))
        .ok_or_else(|| bad("work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("retained/output/work limit"));
    }
    unit(source, ctl)?;
    if let Some(selection) = &doc.selection {
        unit(selection, ctl)?;
    }
    if area.is_empty() || r.opacity == 0.0 {
        return Ok(None);
    }
    let mut output = Surface::try_with_default_bytes(source.format(), source.default_bytes()).map_err(|e| error(ctl, e))?;
    for (cell, tile) in source.tiles() {
        poll(ctl)?;
        output.put_tiles(std::iter::once((*cell, Arc::clone(tile))));
    }
    if r.blend == Blend::Normal {
        photocraft_algo::wide_paint::try_paint_solid(
            &mut output,
            area,
            &[r.value],
            r.opacity,
            doc.selection.as_ref(),
            false,
            photocraft_algo::wide_paint::PaintLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 - work.min(8_000_000_000) },
            ctl,
        )
        .map_err(|e| error(ctl, e))?;
    } else {
        blend_units(&mut output, source, area, doc.selection.as_ref(), r, ctl)?;
    }
    poll(ctl)?;
    Ok((output != *source).then_some(output))
}
fn selection_ratio(selection: Option<&Surface>, x: i32, y: i32) -> Result<(u32, u32)> {
    let Some(selection) = selection else { return Ok((1, 1)) };
    let sample = selection.format().sample;
    let mut raw = [0u8; 8];
    let bytes = raw.get_mut(..sample.bytes()).ok_or_else(|| bad("selection sample width"))?;
    selection.try_read_pixel_bytes(x, y, bytes).map_err(|e| bad(e.to_string()))?;
    Ok(match sample {
        SampleType::U8 => (*bytes.first().ok_or_else(|| bad("selection sample width"))? as u32, 255),
        SampleType::U16 => (u32::from(u16::from_ne_bytes((&*bytes).try_into().map_err(|_| bad("selection sample width"))?)), 65_535),
        SampleType::U32 => (u32::from_ne_bytes((&*bytes).try_into().map_err(|_| bad("selection sample width"))?), u32::MAX),
        SampleType::F32 | SampleType::F64 => match photocraft_color::try_read_sample_f64(bytes, sample, 0).map_err(|e| bad(e.to_string()))? {
            0.0 => (0, 1),
            1.0 => (1, 1),
            _ => return Err(bad("U32 Multiply/Screen support integer selections or Boolean floating selection only")),
        },
    })
}
fn blend_units(output: &mut Surface, source: &Surface, area: Rect, selection: Option<&Surface>, r: &Request, ctl: &Interrupt<'_>) -> Result<()> {
    let sample = source.format().sample;
    let mut color = [0u8; 4];
    let code = if sample == SampleType::U32 {
        photocraft_color::try_write_sample_f64(&mut color, sample, 0, r.value).map_err(|e| bad(e.to_string()))?;
        u32::from_ne_bytes(color)
    } else {
        0
    };
    for cell in area.tiles() {
        poll(ctl)?;
        let part = cell.rect().intersect(&area);
        if part.is_empty() {
            continue;
        }
        let mut raw = source.try_read_interleaved_with(part, ctl).map_err(|e| error(ctl, e))?;
        let width = usize::try_from(part.width()).map_err(|_| bad("blend row width"))?;
        let mut changed = false;
        for (i, pixel) in raw.chunks_exact_mut(sample.bytes()).enumerate() {
            if i % 256 == 0 {
                poll(ctl)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("blend coordinate"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("blend coordinate"))?;
            match sample {
                SampleType::U32 => {
                    let original = u32::from_ne_bytes((&*pixel).try_into().map_err(|_| bad("blend sample width"))?);
                    let (num, den) = selection_ratio(selection, x, y)?;
                    let result =
                        unit_math::u32_blend(original, code, r.blend, r.u32_percent.ok_or_else(|| bad("U32 blend opacity domain"))?, num, den).map_err(bad)?;
                    if result != original {
                        pixel.copy_from_slice(&result.to_ne_bytes());
                        changed = true;
                    }
                }
                SampleType::F64 => {
                    let original = f64::from_ne_bytes((&*pixel).try_into().map_err(|_| bad("blend sample width"))?);
                    let weight = if let Some(selection) = selection {
                        let mut value = [0f64];
                        selection.try_read_pixel_f64(x, y, &mut value).map_err(|e| error(ctl, e))?;
                        value[0]
                    } else {
                        1.0
                    };
                    let result = unit_math::f64_blend(original, r.value, r.blend, r.opacity, weight).map_err(bad)?;
                    if result.to_bits() != original.to_bits() {
                        pixel.copy_from_slice(&result.to_ne_bytes());
                        changed = true;
                    }
                }
                _ => return Err(bad("native unit blend depth")),
            }
        }
        if changed {
            output.try_write_interleaved_with(part, &raw, ctl).map_err(|e| error(ctl, e))?;
        }
    }
    poll(ctl)
}
fn install(doc: &mut Document, t: Target, value: Surface) -> Result<()> {
    locks(doc, t)?;
    match t {
        Target::Mask(id) => {
            doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("mask disappeared"))?.surface = Arc::new(value);
        }
        Target::Channel(index, id) => {
            doc.channels.get_mut(index).filter(|c| c.id == id).ok_or_else(|| bad("channel changed"))?.surface = value;
        }
        Target::QuickMask(id) => {
            doc.quick_mask.as_mut().filter(|c| c.id == id).ok_or_else(|| bad("Quick Mask changed"))?.surface = value;
        }
    }
    Ok(())
}
pub(crate) fn try_run(session: &mut Session, id: &str, p: &Value) -> Result<Option<Value>> {
    let Some(r) = request(session, id, p)? else { return Ok(None) };
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let weak = baseline.downgrade();
    let revision = st.revision;
    let active = st.active_layer;
    let view = st.channel_view.target;
    let editing = st.channel_view.editing_mask;
    let shown = st.channel_view.layer_mask;
    let target = r.target;
    let clear = r.clear;
    let contents = r.contents;
    let label = if clear { "Clear" } else { "Fill" };
    let result = crate::jobs::run(
        session,
        label,
        true,
        move |ctx| ctx.stage(0.0, 0.95, label, |ctl| Ok((prepare(&worker, &r, ctl)?, ctx.clone()))),
        move |session, (output, ctx)| {
            ctx.stage(0.95, 1.0, label, |ctl| {
                poll(ctl)?;
                let st = session.active().ok_or(EngineError::NoDocument)?;
                if st.doc.id != baseline.id
                    || st.revision != revision
                    || !st.doc.shares_state(&baseline)
                    || !weak.is_current(&st.doc)
                    || st.active_layer != active
                    || st.channel_view.target != view
                    || st.channel_view.editing_mask != editing
                    || st.channel_view.layer_mask != shown
                    || st.floating.is_some()
                    || st.coalesce.is_some()
                    || session.command_has_job_on(baseline.id)
                    || session.command_has_pending_job()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                {
                    return Err(bad("captured Gray target changed"));
                }
                locks(&st.doc, target)?;
                let Some(output) = output else { return Ok(if clear { Value::Null } else { json!({"contents":contents}) }) };
                session.edit_with_interrupt(label, ctl, move |doc, _| {
                    poll(ctl)?;
                    install(doc, target, output)?;
                    Ok(if clear { Value::Null } else { json!({"contents":contents}) })
                })
            })
        },
    )?;
    Ok(Some(result))
}

// Draft regressions for the native storage seam, intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, LayerMask, Size};
    fn fixture() -> (Document, LayerId) {
        let mut doc = Document::new("Gray mask", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let raw = [(-0.0f64).to_bits(), 0.5f64.to_bits() + 1].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let mask = Surface::try_from_interleaved_with(format, doc.bounds(), &raw, &Interrupt::NONE).unwrap();
        layer.mask = Some(LayerMask { surface: Arc::new(mask), enabled: false, linked: false, density: 0.7, feather: 2.0 });
        doc.layers.push(layer);
        (doc, id)
    }
    #[test]
    fn full_native_gray_fill_keeps_zero_weight_lowbits_and_mask_metadata() {
        let (mut doc, id) = fixture();
        let mask = surface(&doc, Target::Mask(id)).unwrap().clone();
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        doc.selection = Some(
            Surface::try_from_interleaved_with(
                format,
                doc.bounds(),
                &[u32::MAX, 0].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
                &Interrupt::NONE,
            )
            .unwrap(),
        );
        let before = doc.clone();
        let request = Request { target: Target::Mask(id), value: 0.75, opacity: 1.0, blend: Blend::Normal, u32_percent: None, contents: "color", clear: false };
        let result = prepare(&doc, &request, &Interrupt::NONE).unwrap().unwrap();
        let expected = [0.75f64.to_bits(), 0.5f64.to_bits() + 1].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        assert_eq!(result.to_interleaved(doc.bounds()), expected);
        assert_eq!(result.default_bytes(), mask.default_bytes());
        assert_eq!(doc, before);
        install(&mut doc, request.target, result).unwrap();
        let current = doc.layer(id).unwrap().mask.as_ref().unwrap();
        let original = before.layer(id).unwrap().mask.as_ref().unwrap();
        assert_eq!(current.enabled, original.enabled);
        assert_eq!(current.linked, original.linked);
        assert_eq!(current.density, original.density);
        assert_eq!(current.feather, original.feather);
        assert_eq!(doc.layer(id).unwrap().content, before.layer(id).unwrap().content);
    }
    #[test]
    fn cancellation_stale_and_unsupported_blend_leave_native_mask_and_real_redo() {
        let (doc, id) = fixture();
        let request = Request { target: Target::Mask(id), value: 1.0, opacity: 1.0, blend: Blend::Normal, u32_percent: None, contents: "white", clear: false };
        let before = doc.clone();
        let cancelled = || true;
        assert!(matches!(prepare(&doc, &request, &Interrupt::cancel_only(&cancelled)), Err(EngineError::Cancelled)));
        assert_eq!(doc, before);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
            .edit("Seed", |d, _| {
                d.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let st = session.active().unwrap();
        let before = st.doc.clone();
        let revision = st.revision;
        for params in [
            json!({"layer":id.0,"target":"mask","mode":"overlay"}),
            json!({"layer":id.0,"target":"mask","documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
        ] {
            assert!(try_run(&mut session, "edit.fill", &params).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert!(session.active().unwrap().history.can_redo());
        }
        let params = json!({"layer":id.0,"target":"mask","opacity":0});
        try_run(&mut session, "edit.fill", &params).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.can_redo());
    }
}
