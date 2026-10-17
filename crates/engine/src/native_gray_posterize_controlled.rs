//! Controlled closed-unit native Gray Posterize; no display/color conversion.
use crate::native_gray_fill_controlled::unit_math::{self, Blend};
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, item_identity::ChannelId};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const ID: &str = "image.adjustments.posterize";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: message.into() }
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
    opacity: f64,
    percent: Option<u8>,
    levels: u8,
}
fn request(session: &Session, p: &Value) -> Result<Option<Request>> {
    let Some(target) = resolve(session, p)? else { return Ok(None) };
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let fmt = surface(&st.doc, target)?.format();
    if !matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    if fmt.mode != ColorMode::Grayscale || fmt.alpha {
        return Err(bad("Posterize requires native Gray without alpha"));
    }
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 7).ok_or_else(|| bad("bounded Posterize parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "target" | "layer" | "documentId" | "expectedRevision" | "opacity" | "__kind" | "levels")) {
            return Err(bad("unsupported native Posterize parameter"));
        }
    }
    if p.get("__kind").is_some_and(|v| v.as_str() != Some("posterize")) {
        return Err(bad("native route accepts Posterize only"));
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if st.doc.id.0 != 0 && decimal(id) == Some(st.doc.id.0) && decimal(rev) == Some(st.revision) => {}
        _ => return Err(bad("captured target changed or incomplete")),
    }
    context(session)?;
    locks(&st.doc, target)?;
    if let Target::Mask(id) = target {
        let mut count = 0usize;
        photocraft_doc::resident_structure::visit_layers_with(&st.doc, &Interrupt::NONE, |layer| {
            if layer.id == id {
                count = count.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
            }
            Ok(true)
        })?;
        if id.0 == 0 || count != 1 {
            return Err(bad("ambiguous mask owner"));
        }
    }
    let percent = match p.get("opacity") {
        None | Some(Value::Null) => 100.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("opacity must be finite"))?.clamp(0.0, 100.0),
    };
    if fmt.sample == SampleType::U32 && percent.fract() != 0.0 {
        return Err(bad("native U32 Posterize requires whole-percent opacity"));
    }
    // Preserve the existing bounded levels parser; only this small UI parameter
    // uses its established storage, never the native plane samples.
    let parsed = crate::adjust_params::from_params("posterize", p, None, st.doc.mode)?;
    let photocraft_doc::Adjustment::Posterize { levels } = parsed else { return Err(bad("posterize parameter mismatch")) };
    if !(2..=255).contains(&levels) {
        return Err(bad("posterize levels domain"));
    }
    Ok(Some(Request { target, opacity: percent / 100.0, percent: (fmt.sample == SampleType::U32).then_some(percent as u8), levels: levels as u8 }))
}
pub(crate) fn dispatch_preflight(session: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    request(session, p).map(|r| r.is_some())
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

fn ratio(selection: Option<&Surface>, x: i32, y: i32) -> Result<(u32, u32)> {
    let Some(s) = selection else { return Ok((1, 1)) };
    let sample = s.format().sample;
    let mut raw = [0u8; 8];
    let bytes = raw.get_mut(..sample.bytes()).ok_or_else(|| bad("selection width"))?;
    s.try_read_pixel_bytes(x, y, bytes).map_err(|e| bad(e.to_string()))?;
    Ok(match sample {
        SampleType::U8 => (*bytes.first().ok_or_else(|| bad("selection width"))? as u32, 255),
        SampleType::U16 => (u32::from(u16::from_ne_bytes((&*bytes).try_into().map_err(|_| bad("selection width"))?)), 65535),
        SampleType::U32 => (u32::from_ne_bytes((&*bytes).try_into().map_err(|_| bad("selection width"))?), u32::MAX),
        SampleType::F32 | SampleType::F64 => match photocraft_color::try_read_sample_f64(bytes, sample, 0).map_err(|e| bad(e.to_string()))? {
            0.0 => (0, 1),
            1.0 => (1, 1),
            _ => return Err(bad("U32 Posterize requires integer selection or Boolean floating coverage")),
        },
    })
}
fn posterize_byte(grid: u64, levels: u8) -> Result<u8> {
    if grid > 255 || !(2..=255).contains(&levels) {
        return Err(bad("posterize grid/levels domain"));
    }
    let n = u64::from(levels);
    let bin = grid.checked_mul(n).ok_or_else(|| bad("posterize bin overflow"))?.checked_div(256).ok_or_else(|| bad("posterize divisor"))?.min(n - 1);
    let byte = bin.checked_mul(255).ok_or_else(|| bad("posterize output overflow"))?.checked_div(n - 1).ok_or_else(|| bad("posterize divisor"))?;
    u8::try_from(byte).map_err(|_| bad("posterize output domain"))
}
fn posterize_pixel(raw: &mut [u8], sample: SampleType, r: &Request, selection: Option<&Surface>, x: i32, y: i32) -> Result<bool> {
    match sample {
        SampleType::U32 => {
            let before = u32::from_ne_bytes((&*raw).try_into().map_err(|_| bad("U32 width"))?);
            let (num, den) = ratio(selection, x, y)?;
            let grid = u64::from(before).checked_mul(255).and_then(|n| n.checked_add(u64::from(u32::MAX) / 2)).ok_or_else(|| bad("posterize grid overflow"))?
                / u64::from(u32::MAX);
            let byte = posterize_byte(grid, r.levels)?;
            let posterized = u32::from(byte).checked_mul(u32::MAX / 255).ok_or_else(|| bad("posterize code overflow"))?;
            let after = unit_math::u32_blend(before, posterized, Blend::Normal, r.percent.ok_or_else(|| bad("U32 opacity domain"))?, num, den).map_err(bad)?;
            if after == before {
                return Ok(false);
            }
            raw.copy_from_slice(&after.to_ne_bytes());
        }
        SampleType::F64 => {
            let before = f64::from_ne_bytes((&*raw).try_into().map_err(|_| bad("F64 width"))?);
            if !before.is_finite() || !(0.0..=1.0).contains(&before) {
                return Err(bad("native Posterize requires finite unit values"));
            }
            let weight = if let Some(s) = selection {
                let mut v = [0f64];
                s.try_read_pixel_f64(x, y, &mut v).map_err(|e| bad(e.to_string()))?;
                v[0]
            } else {
                1.0
            };
            // Same upstream eight-bit grid/bin equations, evaluated from the
            // binary64 unit sample without a display-f32 intermediate.
            let grid = (before * 255.0).round();
            if !(0.0..=255.0).contains(&grid) {
                return Err(bad("posterize grid domain"));
            }
            let byte = posterize_byte(grid as u64, r.levels)?;
            let posterized = f64::from(byte) / 255.0f64;
            let after = if r.opacity == 1.0 && weight == 1.0 {
                posterized
            } else {
                unit_math::f64_blend(before, posterized, Blend::Normal, r.opacity, weight).map_err(bad)?
            };
            if before.to_bits() == after.to_bits() {
                return Ok(false);
            }
            raw.copy_from_slice(&after.to_ne_bytes());
        }
        _ => return Err(bad("native Posterize depth")),
    }
    Ok(true)
}
fn prepare(doc: &Document, r: &Request, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    poll(ctl)?;
    locks(doc, r.target)?;
    let source = surface(doc, r.target)?;
    let mask = matches!(r.target, Target::Mask(_));
    let global = mask && doc.selection.is_none();
    let area = if global {
        Rect::EMPTY
    } else if mask {
        doc.selection.as_ref().ok_or_else(|| bad("selection disappeared"))?.try_content_bounds_with_interrupt(ctl).map_err(|e| error(ctl, e))?
    } else {
        source.try_content_bounds_with_interrupt(ctl).map_err(|e| error(ctl, e))?
    };
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| error(ctl, e))? as u64;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let count = if global { source.tile_count() as u64 } else { cells(area)? };
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("Posterize extent overflow"))?;
    let output = pixels
        .checked_mul(source.format().bytes_per_pixel() as u64)
        .and_then(|n| n.checked_add(count.checked_mul(2048)?))
        .ok_or_else(|| bad("Posterize output overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("Posterize model overflow"))?)
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("Posterize staging overflow"))?;
    let visits = (source.tile_count() as u64)
        .checked_add(doc.selection.as_ref().map_or(0, |s| s.tile_count() as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .ok_or_else(|| bad("Posterize work overflow"))?;
    let work = visits
        .checked_add(pixels)
        .and_then(|n| n.checked_mul(128))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(16)?))
        .ok_or_else(|| bad("Posterize work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("Posterize retained/output/work limit"));
    }
    unit(source, ctl)?;
    if let Some(selection) = &doc.selection {
        unit(selection, ctl)?;
    }
    if r.opacity == 0.0 {
        return Ok(None);
    }
    let mut defaults = [0u8; 8];
    let sample = source.format().sample;
    let bytes = defaults.get_mut(..sample.bytes()).ok_or_else(|| bad("default width"))?;
    bytes.copy_from_slice(source.default_bytes());
    let default_changed = if global { posterize_pixel(bytes, sample, r, None, 0, 0)? } else { false };
    let mut output = Surface::try_with_default_bytes(source.format(), bytes).map_err(|e| error(ctl, e))?;
    if !global {
        for (cell, tile) in source.tiles() {
            poll(ctl)?;
            output.put_tiles(std::iter::once((*cell, Arc::clone(tile))));
        }
    }
    let mut any_pixel_changed = false;
    let mut process = |part: Rect| -> Result<()> {
        poll(ctl)?;
        if part.is_empty() {
            return Ok(());
        }
        let mut raw = source.try_read_interleaved_with(part, ctl).map_err(|e| error(ctl, e))?;
        let width = usize::try_from(part.width()).map_err(|_| bad("Posterize row"))?;
        let mut changed = global;
        for (i, pixel) in raw.chunks_exact_mut(sample.bytes()).enumerate() {
            if i % 256 == 0 {
                poll(ctl)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("Posterize x"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("Posterize y"))?;
            let pixel_changed = posterize_pixel(pixel, sample, r, doc.selection.as_ref(), x, y)?;
            changed |= pixel_changed;
            any_pixel_changed |= pixel_changed;
        }
        if changed {
            output.try_write_interleaved_with(part, &raw, ctl).map_err(|e| error(ctl, e))?;
        }
        Ok(())
    };
    if global {
        for (cell, _) in source.tiles() {
            process(cell.rect())?;
        }
    } else {
        for cell in area.tiles() {
            process(cell.rect().intersect(&area))?;
        }
    }
    if !default_changed && !any_pixel_changed {
        return Ok(None);
    }
    output.try_prune_with(ctl).map_err(|e| error(ctl, e))?;
    poll(ctl)?;
    Ok((output != *source).then_some(output))
}
fn install(doc: &mut Document, t: Target, output: Surface, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    locks(doc, t)?;
    match t {
        Target::Mask(id) => {
            let outside = photocraft_color::try_read_sample_f64(output.default_bytes(), output.format().sample, 0).map_err(|e| bad(e.to_string()))?;
            if !outside.is_finite() || !(0.0..=1.0).contains(&outside) {
                return Err(bad("current mask default is not finite unit"));
            }
            doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or_else(|| bad("mask disappeared"))?.surface = Arc::new(output);
            if let Some(state) = &mut doc.xcf_state {
                for item in &mut state.items {
                    poll(ctl)?;
                    if let Some(mask) = &mut item.mask
                        && mask.owner == photocraft_doc::item_identity::ItemRef::Layer(id)
                    {
                        mask.outside_f64_bits = outside.to_bits();
                    }
                }
            }
        }
        Target::Channel(index, id) => doc.channels.get_mut(index).filter(|c| c.id == id).ok_or_else(|| bad("channel changed"))?.surface = output,
        Target::QuickMask(id) => doc.quick_mask.as_mut().filter(|c| c.id == id).ok_or_else(|| bad("Quick Mask changed"))?.surface = output,
    }
    doc.validate_xcf_state_with(ctl).map_err(|e| error(ctl, e))?;
    poll(ctl)
}

pub(crate) fn try_run(session: &mut Session, p: &Value) -> Result<Option<Value>> {
    let Some(r) = request(session, p)? else { return Ok(None) };
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let weak = baseline.downgrade();
    let rev = st.revision;
    let active = st.active_layer;
    let view = st.channel_view.target;
    let editing = st.channel_view.editing_mask;
    let shown = st.channel_view.layer_mask;
    let target = r.target;
    let result = crate::jobs::run(
        session,
        "Posterize",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Posterize", |ctl| Ok((prepare(&worker, &r, ctl)?, ctx.clone()))),
        move |session, (output, ctx)| {
            ctx.stage(0.95, 1.0, "Posterize", |ctl| {
                poll(ctl)?;
                let st = session.active().ok_or(EngineError::NoDocument)?;
                if st.doc.id != baseline.id
                    || st.revision != rev
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
                    return Err(bad("captured Posterize state changed"));
                }
                locks(&st.doc, target)?;
                let Some(output) = output else { return Ok(Value::Null) };
                session.edit_with_interrupt("Posterize", ctl, move |doc, _| {
                    poll(ctl)?;
                    install(doc, target, output, ctl)?;
                    Ok(Value::Null)
                })
            })
        },
    )?;
    Ok(Some(result))
}

// Meaningful native equations/transaction regression drafts, intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, LayerMask, Size};
    use serde_json::json;
    fn fixture() -> (Document, LayerId) {
        let mut doc = Document::new("Posterize mask", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        layer.mask = Some(LayerMask {
            surface: Arc::new(Surface::try_with_default_bytes(format, &0.5f64.to_ne_bytes()).unwrap()),
            enabled: false,
            linked: false,
            density: 0.7,
            feather: 3.0,
        });
        doc.layers.push(layer);
        (doc, id)
    }
    #[test]
    fn every_grid_and_level_matches_existing_upstream_integer_bin_equation() {
        for levels in 2u8..=255 {
            for grid in 0u64..=255 {
                let expected = photocraft_compose::adjust::posterize(grid as f32 / 255.0, u32::from(levels));
                assert_eq!(posterize_byte(grid, levels).unwrap(), (expected * 255.0).round() as u8);
            }
        }
        assert_eq!([63, 64, 127, 128, 191, 192].map(|x| posterize_byte(x, 4).unwrap()), [0, 85, 85, 170, 170, 255]);
    }
    #[test]
    fn native_u32_codes_and_f64_half_neighbors_are_not_presented_as_f32() {
        let r = Request { target: Target::QuickMask(ChannelId(1)), opacity: 1.0, percent: Some(100), levels: 4 };
        for (grid, expected) in [(64u32, 85u32), (128, 170), (192, 255)] {
            let mut raw = (grid * (u32::MAX / 255)).to_ne_bytes();
            posterize_pixel(&mut raw, SampleType::U32, &r, None, 0, 0).unwrap();
            assert_eq!(u32::from_ne_bytes(raw), expected * (u32::MAX / 255));
        }
        let r = Request { target: Target::QuickMask(ChannelId(1)), opacity: 1.0, percent: None, levels: 4 };
        for (v, expected) in [(f64::from_bits(0.5f64.to_bits() - 1), 85.0 / 255.0), (0.5, 170.0 / 255.0)] {
            let mut raw = v.to_ne_bytes();
            posterize_pixel(&mut raw, SampleType::F64, &r, None, 0, 0).unwrap();
            assert_eq!(f64::from_ne_bytes(raw).to_bits(), f64::to_bits(expected));
        }
    }
    #[test]
    fn sparse_default_metadata_and_zero_weight_words_preserve_source_and_noop_redo() {
        let (mut doc, id) = fixture();
        let format = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        Arc::make_mut(&mut doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface).try_write_pixel_f64(-1_000_000, 0, &[0.2]).unwrap();
        let before = doc.clone();
        let r = Request { target: Target::Mask(id), opacity: 1.0, percent: None, levels: 4 };
        let result = prepare(&doc, &r, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(result.default_bytes(), (170.0f64 / 255.0).to_ne_bytes());
        assert!(result.tile_count() <= 1);
        assert_eq!(doc, before);
        doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface = Arc::new(Surface::try_with_default_bytes(format, &0.0f64.to_ne_bytes()).unwrap());
        assert!(prepare(&doc, &r, &Interrupt::NONE).unwrap().is_none());
        let cancel = || true;
        assert!(matches!(prepare(&doc, &r, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let st = session.active().unwrap();
        let before = st.doc.clone();
        let rev = st.revision;
        assert!(
            try_run(&mut session, &json!({"target":"mask","layer":id.0,"documentId":before.id.0.to_string(),"expectedRevision":(rev+1).to_string()})).is_err()
        );
        try_run(&mut session, &json!({"target":"mask","layer":id.0})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.can_redo());
    }
}
