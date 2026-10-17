//! Controlled closed-unit native Gray Levels; no display/color conversion.
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, item_identity::ChannelId};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const ID: &str = "image.adjustments.levels";
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
    master: photocraft_doc::adjust::LevelsChannel,
    per_channel: [photocraft_doc::adjust::LevelsChannel; 3],
}
fn request(session: &Session, p: &Value) -> Result<Option<Request>> {
    let Some(target) = resolve(session, p)? else { return Ok(None) };
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let fmt = surface(&st.doc, target)?.format();
    if !matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Ok(None);
    }
    if fmt.mode != ColorMode::Grayscale || fmt.alpha {
        return Err(bad("Levels requires native Gray without alpha"));
    }
    envelope(p)?;
    if p.get("__kind").is_some_and(|v| v.as_str() != Some("levels")) {
        return Err(bad("native route accepts Levels only"));
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
        return Err(bad("native U32 Levels requires whole-percent opacity"));
    }
    let parsed = crate::adjust_params::from_params("levels", p, None, ColorMode::Grayscale)?;
    let photocraft_doc::Adjustment::Levels { master, per_channel, space, .. } = parsed else { return Err(bad("Levels parameter mismatch")) };
    if space != photocraft_doc::adjust::ToneSpace::Rgb || per_channel.iter().any(|record| *record != photocraft_doc::adjust::LevelsChannel::default()) {
        return Err(bad("native Gray Levels requires master-only RGB tone records"));
    }
    let request = Request { target, opacity: percent / 100.0, percent: (fmt.sample == SampleType::U32).then_some(percent as u8), master, per_channel };
    visit_tables(&request, &Interrupt::NONE, |_, _| Ok(()))?;
    Ok(Some(request))
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
            _ => return Err(bad("U32 Levels requires integer selection or Boolean floating coverage")),
        },
    })
}
use crate::native_tone_lut::TABLE_LEN;
fn envelope(params: &Value) -> Result<()> {
    if params.is_null() {
        return Ok(());
    }
    let object = params.as_object().filter(|object| object.len() <= 15).ok_or_else(|| bad("bounded Levels parameters required"))?;
    for (key, value) in object {
        match key.as_str() {
            "layer" | "target" | "documentId" | "expectedRevision" | "opacity" | "__kind" => {}
            "inBlack" | "inWhite" | "gamma" | "outBlack" | "outWhite" => {
                if value.as_f64().is_none_or(|value| !value.is_finite()) {
                    return Err(bad("Levels parameters must be finite numbers"));
                }
            }
            "gray" | "red" | "green" | "blue" => {
                let record = value.as_object().filter(|record| record.len() <= 5).ok_or_else(|| bad("bounded Levels channel record required"))?;
                for (field, value) in record {
                    if !matches!(field.as_str(), "inBlack" | "inWhite" | "gamma" | "outBlack" | "outWhite")
                        || value.as_f64().is_none_or(|value| !value.is_finite())
                    {
                        return Err(bad("invalid Levels channel parameter"));
                    }
                }
            }
            _ => return Err(bad("unsupported native Levels parameter")),
        }
    }
    Ok(())
}
fn visit_tables(request: &Request, control: &Interrupt<'_>, mut accept: impl FnMut(usize, u64) -> Result<()>) -> Result<()> {
    for (channel, record) in request.per_channel.iter().enumerate() {
        for index in 0..TABLE_LEN {
            if index % 128 == 0 {
                poll(control)?;
            }
            // This is the exact existing typed f32 channel-before-master table recipe,
            // not evaluation of native source words through f32.
            let x = index as f32 / (TABLE_LEN - 1) as f32;
            let output = photocraft_compose::adjust::levels_q(&request.master, photocraft_compose::adjust::levels_q(record, x, None), None);
            accept(channel, crate::native_tone_lut::exact_word(output).map_err(bad)?)?;
        }
    }
    poll(control)
}
fn tables(request: &Request, control: &Interrupt<'_>) -> Result<[Vec<u64>; 3]> {
    let mut tables: [Vec<u64>; 3] = std::array::from_fn(|_| Vec::new());
    for table in &mut tables {
        table.try_reserve_exact(TABLE_LEN).map_err(|_| bad("Levels table allocation denied"))?;
    }
    visit_tables(request, control, |channel, value| {
        tables.get_mut(channel).ok_or_else(|| bad("Levels table channel"))?.push(value);
        Ok(())
    })?;
    let allocated = tables
        .iter()
        .try_fold(0usize, |sum, table| sum.checked_add(table.capacity().checked_mul(std::mem::size_of::<u64>())?))
        .ok_or_else(|| bad("Levels table capacity overflow"))?;
    if allocated > 1 << 20 {
        return Err(bad("Levels table allocator capacity exceeded admitted scratch"));
    }
    Ok(tables)
}
fn tone_pixel(raw: &mut [u8], sample: SampleType, r: &Request, table: &[u64], selection: Option<&Surface>, x: i32, y: i32) -> Result<bool> {
    match sample {
        SampleType::U32 => {
            let before = u32::from_ne_bytes((&*raw).try_into().map_err(|_| bad("U32 width"))?);
            let (num, den) = ratio(selection, x, y)?;
            let after = crate::native_tone_lut::interpolate_u32(table, before, r.percent.ok_or_else(|| bad("U32 opacity domain"))?, num, den).map_err(bad)?;
            if after == before {
                return Ok(false);
            }
            raw.copy_from_slice(&after.to_ne_bytes());
        }
        SampleType::F64 => {
            let selected = if let Some(selection) = selection {
                let mut value = [0f64];
                selection.try_read_pixel_f64(x, y, &mut value).map_err(|e| bad(e.to_string()))?;
                value[0]
            } else {
                1.0
            };
            if !selected.is_finite() || !(0.0..=1.0).contains(&selected) {
                return Err(bad("coverage is not finite unit"));
            }
            let weight = r.opacity * selected;
            if weight == 0.0 {
                return Ok(false);
            }
            let before = f64::from_ne_bytes((&*raw).try_into().map_err(|_| bad("F64 width"))?);
            let after = crate::native_tone_lut::interpolate_f64(table, before, weight).map_err(bad)?;
            if after.to_bits() == before.to_bits() {
                return Ok(false);
            }
            raw.copy_from_slice(&after.to_ne_bytes());
        }
        _ => return Err(bad("native Gray tone depth")),
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
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("Levels extent overflow"))?;
    let output = pixels
        .checked_mul(source.format().bytes_per_pixel() as u64)
        .and_then(|n| n.checked_add(count.checked_mul(2048)?))
        .ok_or_else(|| bad("Levels output overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("Levels model overflow"))?)
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(4 << 20))
        .ok_or_else(|| bad("Levels staging overflow"))?;
    let visits = (source.tile_count() as u64)
        .checked_add(doc.selection.as_ref().map_or(0, |s| s.tile_count() as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .and_then(|n| n.checked_mul(TILE_SIZE as u64))
        .ok_or_else(|| bad("Levels work overflow"))?;
    let work = visits
        .checked_add(pixels)
        .and_then(|n| n.checked_mul(128))
        .and_then(|n| n.checked_add(payload.checked_add(model)?.checked_mul(16)?))
        .and_then(|n| n.checked_add(100_000_000))
        .ok_or_else(|| bad("Levels work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("Levels retained/output/work limit"));
    }
    source.try_validate_tile_origins_with(ctl).map_err(|e| error(ctl, e))?;
    if r.opacity == 0.0 {
        return Ok(None);
    }
    if let Some(selection) = &doc.selection {
        unit(selection, ctl)?;
    }
    let tables = tables(r, ctl)?;
    let table = tables.first().ok_or_else(|| bad("missing native Gray master table"))?;
    let mut defaults = [0u8; 8];
    let sample = source.format().sample;
    let bytes = defaults.get_mut(..sample.bytes()).ok_or_else(|| bad("default width"))?;
    bytes.copy_from_slice(source.default_bytes());
    let default_changed = if global { tone_pixel(bytes, sample, r, table, None, 0, 0)? } else { false };
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
        let width = usize::try_from(part.width()).map_err(|_| bad("Levels row"))?;
        let mut changed = global;
        for (i, pixel) in raw.chunks_exact_mut(sample.bytes()).enumerate() {
            if i % 256 == 0 {
                poll(ctl)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("Levels x"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("Levels y"))?;
            let pixel_changed = tone_pixel(pixel, sample, r, table, doc.selection.as_ref(), x, y)?;
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
        "Levels",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Levels", |ctl| Ok((prepare(&worker, &r, ctl)?, ctx.clone()))),
        move |session, (output, ctx)| {
            ctx.stage(0.95, 1.0, "Levels", |ctl| {
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
                    return Err(bad("captured Levels state changed"));
                }
                locks(&st.doc, target)?;
                let Some(output) = output else { return Ok(Value::Null) };
                session.edit_with_interrupt("Levels", ctl, move |doc, _| {
                    poll(ctl)?;
                    install(doc, target, output, ctl)?;
                    Ok(Value::Null)
                })
            })
        },
    )?;
    Ok(Some(result))
}

// Meaningful transaction/target regressions, intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer, LayerMask, Size};
    use serde_json::json;
    fn fixture() -> (Session, LayerId) {
        let mut doc = Document::new("Gray tone", Size::new(2, 1), ColorMode::Rgb, SampleType::F64);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut source = Surface::try_with_default_bytes(fmt, &0.25f64.to_ne_bytes()).unwrap();
        source.try_write_pixel_f64(-1000000, 0, &[0.75]).unwrap();
        layer.mask = Some(LayerMask { surface: Arc::new(source), enabled: false, linked: false, density: 0.7, feather: 3.0 });
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, id)
    }
    #[test]
    fn sparse_global_mask_default_and_offcanvas_change_without_presentation_edits() {
        let (mut session, id) = fixture();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let params = json!({"target":"mask","outWhite":128});
        try_run(&mut session, &params).unwrap();
        let state = session.active().unwrap();
        let mask = state.doc.layer(id).unwrap().mask.as_ref().unwrap();
        assert_ne!(mask.surface.default_bytes(), before.layer(id).unwrap().mask.as_ref().unwrap().surface.default_bytes());
        assert!(mask.surface.tile_count() <= 1);
        assert_eq!((mask.enabled, mask.linked, mask.density, mask.feather), (false, false, 0.7, 3.0));
        assert_eq!(state.history.past_len(), history + 1);
        let after = state.doc.clone();
        session.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(*session.active().unwrap().doc, *before);
        session.execute("edit.redo", Value::Null).unwrap();
        assert_eq!(*session.active().unwrap().doc, *after);
    }
    #[test]
    fn channel_and_quick_mask_defaults_stay_raw_while_current_samples_change() {
        let (mut session, _) = fixture();
        let fmt = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        session
            .edit("Native targets", |doc, _| {
                let mut surface = Surface::try_with_default_bytes(fmt, &0.25f64.to_ne_bytes()).unwrap();
                surface.try_write_pixel_f64(-1, 0, &[0.75]).unwrap();
                doc.channels.push(AlphaChannel::new("Channel", surface.clone()));
                doc.quick_mask = Some(AlphaChannel::new("Quick", surface));
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let channel = request(&session, &json!({"target":{"channel":0},"outWhite":128})).unwrap().unwrap();
        let output = prepare(&before, &channel, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(output.default_bytes(), before.channels[0].surface.default_bytes());
        assert_ne!(output.to_interleaved(Rect::new(-1, 0, 0, 1)), before.channels[0].surface.to_interleaved(Rect::new(-1, 0, 0, 1)));
        let quick = request(&session, &json!({"target":"quickMask","outWhite":128})).unwrap().unwrap();
        let output = prepare(&before, &quick, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(output.default_bytes(), before.quick_mask.as_ref().unwrap().surface.default_bytes());
    }
    #[test]
    fn zero_coverage_nan_and_stale_cancelled_edits_preserve_original_redo() {
        let (mut session, _) = fixture();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let r = request(&session, &json!({"target":"mask","outWhite":128})).unwrap().unwrap();
        let table = tables(&r, &Interrupt::NONE).unwrap();
        let nan = 0x7ff8_0000_0000_0042u64.to_ne_bytes();
        let mut raw = nan;
        let selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false));
        assert!(!tone_pixel(&mut raw, SampleType::F64, &r, &table[0], Some(&selection), 0, 0).unwrap());
        assert_eq!(raw, nan);
        assert!(try_run(&mut session, &json!({"target":"mask","documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        let cancel = || true;
        assert!(matches!(prepare(&before, &r, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        try_run(&mut session, &json!({"target":"mask","opacity":0})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.can_redo());
    }
}
