//! Controlled native stored-color Desaturate. Alpha/source metadata are copied raw.
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const ID: &str = "image.adjustments.desaturate";
fn bad(m: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: m.into() }
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn error(c: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if c.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) }
}
fn decimal(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    if s.is_empty() || s.len() > 20 || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}
struct Request {
    layer: LayerId,
    opacity: f64,
    percent: Option<u8>,
}
fn target<'a>(doc: &'a Document, id: LayerId, c: &Interrupt<'_>) -> Result<(&'a Surface, Rect)> {
    poll(c)?;
    let mut found = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc, c, |layer| {
        if layer.id == id {
            found = found.checked_add(1).ok_or(photocraft_raster::RasterInputError::LimitExceeded)?;
        }
        Ok(true)
    })?;
    if id.0 == 0 || found != 1 {
        return Err(bad("missing or ambiguous raster owner"));
    }
    crate::commands::check_pixels_unlocked(doc, id)?;
    crate::xcf_item_locks::transparency_locked(doc, id)?;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Raster(source) = &layer.content else { return Err(bad("native Desaturate requires a plain resident raster")) };
    let format = source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(format.sample, SampleType::U32 | SampleType::F64)
        || format.mode != doc.mode
        || format.sample != doc.depth
        || doc.color_table.is_some()
        || layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.import_compositing.is_some()
    {
        return Err(bad("native raster color/geometry context unsupported"));
    }
    if source.tile_count() > 100_000 {
        return Err(bad("native raster tile limit"));
    }
    let area = source.try_content_bounds_with_interrupt(c).map_err(|e| error(c, e))?;
    if let Some(state) = &doc.xcf_state {
        if state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.filter_stacks.len() > 10_000 || state.editable_indexed.len() > 10_000 {
            return Err(bad("current state limit"));
        }
        if state.source_items.iter().any(|i| i.owner == ItemRef::Layer(id))
            || state.filter_stacks.iter().any(|i| i.owner == ItemRef::Layer(id))
            || state.editable_indexed.iter().any(|i| i.owner == id)
        {
            return Err(bad("source/indexed/filter carrier desaturate is not qualified"));
        }
        let mut items = state.items.iter().filter(|i| i.item == ItemRef::Layer(id));
        let item = items.next().ok_or_else(|| bad("missing current typed raster"))?;
        if items.next().is_some() || item.composition.is_some() {
            return Err(bad("ambiguous or interpreted current raster"));
        }
        let extent = item.raster_bounds.ok_or_else(|| bad("missing current raster extent"))?;
        if !area.is_empty() && area.intersect(&extent) != area {
            return Err(bad("retained pixels exceed declared current extent"));
        }
    }
    Ok((source, area))
}
fn context(s: &Session) -> Result<()> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || st.floating.is_some()
        || st.coalesce.is_some()
        || s.has_jobs()
        || s.command_has_pending_job()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some())
    {
        return Err(bad("finish transient work before native raster Desaturate"));
    }
    Ok(())
}
fn request(s: &Session, p: &Value) -> Result<Option<Request>> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(t) = p.get("target") {
        if t.as_str() != Some("pixels") {
            return Ok(None);
        }
    } else if matches!(st.channel_view.target, ChannelTarget::Alpha(_) | ChannelTarget::LayerMask | ChannelTarget::VectorMask)
        || st.doc.quick_mask.is_some()
        || crate::mask_view_cmds::current(st).is_some()
    {
        return Ok(None);
    }
    if let Some(v) = p.get("layer")
        && v.as_u64().is_none_or(|v| v == 0)
    {
        return Err(bad("layer must be a nonzero unsigned integer"));
    }
    let layer = crate::commands::layer_param(s, p)?;
    let current = st.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    if current.surface().is_none_or(|v| !matches!(v.format().sample, SampleType::U32 | SampleType::F64)) {
        return Ok(None);
    }
    if matches!(st.channel_view.target, ChannelTarget::Color(_)) {
        return Err(bad("native Desaturate requires all color channels"));
    }
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 6).ok_or_else(|| bad("bounded native Desaturate parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "layer" | "target" | "documentId" | "expectedRevision" | "opacity" | "__kind")) {
            return Err(bad("unknown native raster Desaturate parameter"));
        }
    }
    if p.get("__kind").is_some_and(|v| v.as_str() != Some("desaturate")) {
        return Err(bad("native raster route requires Desaturate"));
    }
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(d), Some(r)) if st.doc.id.0 != 0 && decimal(d) == Some(st.doc.id.0) && decimal(r) == Some(st.revision) => {}
        _ => return Err(bad("captured target changed or incomplete")),
    }
    context(s)?;
    target(&st.doc, layer, &Interrupt::NONE)?;
    let percent = match p.get("opacity") {
        None | Some(Value::Null) => 100.0,
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("opacity must be finite"))?.clamp(0.0, 100.0),
    };
    let sample = current.surface().ok_or_else(|| bad("raster disappeared"))?.format().sample;
    if sample == SampleType::U32 && percent.fract() != 0.0 {
        return Err(bad("U32 native Desaturate requires whole-percent opacity"));
    }
    Ok(Some(Request { layer, opacity: percent / 100.0, percent: (sample == SampleType::U32).then_some(percent as u8) }))
}
pub(crate) fn dispatch_preflight(s: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    request(s, p).map(|r| r.is_some())
}
fn cells(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Ok(0);
    }
    let a = TileCoord::containing(r.x0, r.y0);
    let b = TileCoord::containing(r.x1.checked_sub(1).ok_or_else(|| bad("extent"))?, r.y1.checked_sub(1).ok_or_else(|| bad("extent"))?);
    u64::try_from(i64::from(b.tx) - i64::from(a.tx) + 1)
        .ok()
        .and_then(|w| u64::try_from(i64::from(b.ty) - i64::from(a.ty) + 1).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| bad("tile span overflow"))
}
fn ratio(s: Option<&Surface>, x: i32, y: i32) -> Result<(u32, u32)> {
    let Some(s) = s else { return Ok((1, 1)) };
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
            _ => return Err(bad("U32 Desaturate requires integer selection or Boolean floating coverage")),
        },
    })
}
fn midpoint_u32(pixel: &[u8]) -> Result<u64> {
    let mut lower = u32::MAX;
    let mut upper = 0u32;
    for channel in 0usize..3 {
        let start = channel.checked_mul(4).ok_or_else(|| bad("color offset"))?;
        let raw = pixel.get(start..start.checked_add(4).ok_or_else(|| bad("color offset"))?).ok_or_else(|| bad("U32 color width"))?;
        let value = u32::from_ne_bytes(raw.try_into().map_err(|_| bad("U32 color width"))?);
        lower = lower.min(value);
        upper = upper.max(value);
    }
    Ok(u64::from(lower) + u64::from(upper))
}
fn midpoint_f64(pixel: &[u8]) -> Result<f64> {
    let mut lower = 1.0f64;
    let mut upper = 0.0f64;
    for channel in 0usize..3 {
        let start = channel.checked_mul(8).ok_or_else(|| bad("color offset"))?;
        let raw = pixel.get(start..start.checked_add(8).ok_or_else(|| bad("color offset"))?).ok_or_else(|| bad("F64 color width"))?;
        let value = f64::from_ne_bytes(raw.try_into().map_err(|_| bad("F64 color width"))?);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(bad("native Desaturate requires finite unit RGB"));
        }
        lower = lower.min(value);
        upper = upper.max(value);
    }
    let sum = lower + upper;
    Ok(sum / 2.0)
}
fn blend_midpoint(before: u32, sum: u64, percent: u8, num: u32, den: u32) -> Result<u32> {
    if sum > 2 * u64::from(u32::MAX) || percent > 100 || den == 0 || num > den {
        return Err(bad("native midpoint/coverage domain"));
    }
    let coverage = u128::from(percent) * u128::from(num);
    let total = 100 * u128::from(den);
    // Sum/2 stays rational until AFTER the convex coverage mixture; all
    // products <2^73, including final half-denominator nearest rounding.
    let numerator = 2 * u128::from(before) * (total - coverage) + u128::from(sum) * coverage;
    let denominator = 2 * total;
    u32::try_from((numerator + denominator / 2) / denominator).map_err(|_| bad("native Desaturate result"))
}
fn prepare(doc: &Document, r: &Request, c: &Interrupt<'_>) -> Result<Option<Surface>> {
    let (source, area) = target(doc, r.layer, c)?;
    let fmt = source.format();
    let bpp = fmt.bytes_per_pixel();
    let colors = fmt.mode.color_channels();
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let footprint = crate::DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let count = cells(area)?;
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("Desaturate extent overflow"))?;
    let output = pixels.checked_mul(bpp as u64).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("Desaturate output overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("model overflow"))?)
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("Desaturate staging overflow"))?;
    let work = pixels
        .checked_mul((colors as u64 + 4) * 128)
        .and_then(|n| {
            n.checked_add(
                (source.tile_count() as u64)
                    .checked_add(doc.selection.as_ref().map_or(0, |s| s.tile_count() as u64))?
                    .checked_mul(TILE_SIZE as u64)?
                    .checked_mul(TILE_SIZE as u64)?
                    .checked_mul(64)?,
            )
        })
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(16)?))
        .ok_or_else(|| bad("Desaturate work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("Desaturate retained/output/work limit"));
    }
    if let Some(selection) = &doc.selection {
        if selection.format().mode != ColorMode::Grayscale || selection.format().alpha {
            return Err(bad("selection requires Gray coverage without alpha"));
        }
        selection.try_validate_tile_origins_with(c).map_err(|e| error(c, e))?;
    }
    if fmt.mode == ColorMode::Grayscale || area.is_empty() || r.opacity == 0.0 {
        return Ok(None);
    }
    let mut out = Surface::try_with_default_bytes(fmt, source.default_bytes()).map_err(|e| error(c, e))?;
    for (cell, tile) in source.tiles() {
        poll(c)?;
        out.put_tiles(std::iter::once((*cell, Arc::clone(tile))));
    }
    let mut any = false;
    for cell in area.tiles() {
        poll(c)?;
        let part = cell.rect().intersect(&area);
        if part.is_empty() {
            continue;
        }
        let mut raw = source.try_read_interleaved_with(part, c).map_err(|e| error(c, e))?;
        let width = usize::try_from(part.width()).map_err(|_| bad("Desaturate row"))?;
        let mut changed = false;
        for (i, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if i % 256 == 0 {
                poll(c)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("Desaturate x"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("Desaturate y"))?;
            let selection = doc.selection.as_ref();
            let fraction = if fmt.sample == SampleType::U32 { Some(ratio(selection, x, y)?) } else { None };
            let weight = if fmt.sample == SampleType::F64 {
                let selected = if let Some(s) = selection {
                    let mut value = [0f64];
                    s.try_read_pixel_f64(x, y, &mut value).map_err(|e| error(c, e))?;
                    value[0]
                } else {
                    1.0
                };
                if !selected.is_finite() || !(0.0..=1.0).contains(&selected) {
                    return Err(bad("selection is not finite unit"));
                }
                selected * r.opacity
            } else {
                1.0
            };
            if weight == 0.0 || fraction.is_some_and(|(num, _)| num == 0) {
                continue;
            }
            if crate::native_adjustment_alpha::skips(pixel, fmt).map_err(bad)? {
                continue;
            }
            let midpoint_integer = if fmt.sample == SampleType::U32 { Some(midpoint_u32(pixel)?) } else { None };
            let midpoint_float = if fmt.sample == SampleType::F64 { Some(midpoint_f64(pixel)?) } else { None };
            // Alpha is only read by the raw nonpositive skip gate; never rewritten.
            for color in 0..colors {
                let start = color.checked_mul(fmt.sample.bytes()).ok_or_else(|| bad("color offset"))?;
                let end = start.checked_add(fmt.sample.bytes()).ok_or_else(|| bad("color offset"))?;
                let sample = pixel.get_mut(start..end).ok_or_else(|| bad("color sample width"))?;
                match fmt.sample {
                    SampleType::U32 => {
                        let before = u32::from_ne_bytes((&*sample).try_into().map_err(|_| bad("U32 width"))?);
                        let (num, den) = fraction.ok_or_else(|| bad("integer selection domain"))?;
                        let after = blend_midpoint(
                            before,
                            midpoint_integer.ok_or_else(|| bad("native midpoint domain"))?,
                            r.percent.ok_or_else(|| bad("integer opacity domain"))?,
                            num,
                            den,
                        )?;
                        if after != before {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    SampleType::F64 => {
                        let before = f64::from_ne_bytes((&*sample).try_into().map_err(|_| bad("F64 width"))?);
                        let midpoint = midpoint_float.ok_or_else(|| bad("native midpoint domain"))?;
                        let after = if weight == 1.0 {
                            midpoint
                        } else {
                            let original = before * (1.0 - weight);
                            let edited = midpoint * weight;
                            original + edited
                        };
                        if !after.is_finite() {
                            return Err(bad("native Desaturate arithmetic domain"));
                        }
                        if after.to_bits() != before.to_bits() {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    _ => return Err(bad("native Desaturate depth")),
                }
            }
        }
        if changed {
            out.try_write_interleaved_with(part, &raw, c).map_err(|e| error(c, e))?;
            any = true;
        }
    }
    poll(c)?;
    Ok(any.then_some(out))
}
pub(crate) fn try_run(s: &mut Session, p: &Value) -> Result<Option<Value>> {
    let Some(r) = request(s, p)? else { return Ok(None) };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let before = st.doc.clone();
    let worker = before.clone();
    let weak = before.downgrade();
    let revision = st.revision;
    let active = st.active_layer;
    let view = st.channel_view.target;
    let editing = st.channel_view.editing_mask;
    let shown = st.channel_view.layer_mask;
    let layer = r.layer;
    let value = crate::jobs::run(
        s,
        "Desaturate",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Desaturate", |c| Ok((prepare(&worker, &r, c)?, ctx.clone()))),
        move |s, (output, ctx)| {
            ctx.stage(0.95, 1.0, "Desaturate", |c| {
                poll(c)?;
                let st = s.active().ok_or(EngineError::NoDocument)?;
                if st.doc.id != before.id
                    || st.revision != revision
                    || !st.doc.shares_state(&before)
                    || !weak.is_current(&st.doc)
                    || st.active_layer != active
                    || st.channel_view.target != view
                    || st.channel_view.editing_mask != editing
                    || st.channel_view.layer_mask != shown
                    || st.floating.is_some()
                    || st.coalesce.is_some()
                    || s.command_has_job_on(before.id)
                    || s.command_has_pending_job()
                    || s.coalesce_request.is_some()
                    || s.color_restrict.is_some()
                {
                    return Err(bad("captured raster Desaturate changed"));
                }
                target(&st.doc, layer, c)?;
                let Some(output) = output else { return Ok(Value::Null) };
                s.edit_with_interrupt("Desaturate", c, move |doc, _| {
                    poll(c)?;
                    target(doc, layer, c)?;
                    doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?.content = LayerContent::Raster(output);
                    doc.validate_xcf_state_with(c).map_err(|e| error(c, e))?;
                    poll(c)?;
                    Ok(Value::Null)
                })
            })
        },
    )?;
    Ok(Some(value))
}

// Meaningful draft regressions, intentionally unrun.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, Size};
    use serde_json::json;
    fn fixture(mode: ColorMode, sample: SampleType, raw: &[u8]) -> (Session, LayerId) {
        let mut doc = Document::new("Native desaturate", Size::new(1, 1), mode, sample);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        layer.content = LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(), Rect::new(-1, 0, 0, 1), raw, &Interrupt::NONE).unwrap());
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, id)
    }
    #[test]
    fn midpoint_is_not_luma_and_integer_half_is_not_prequantized() {
        let raw = [0u32, u32::MAX, 0, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        let (session, _) = fixture(ColorMode::Rgb, SampleType::U32, &raw);
        let r = request(&session, &json!({})).unwrap().unwrap();
        let output = prepare(&session.active().unwrap().doc, &r, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(
            output.to_interleaved(Rect::new(-1, 0, 0, 1)),
            [2147483648u32, 2147483648, 2147483648, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        );
        assert_eq!(blend_midpoint(0, 1, 100, 1, 1).unwrap(), 1);
        assert_eq!(blend_midpoint(0, 1, 50, 1, 1).unwrap(), 0, "no midpoint code before coverage");
        assert_eq!(blend_midpoint(17, 1, 100, 0, u32::MAX).unwrap(), 17);
    }
    #[test]
    fn f64_green_becomes_half_gray_with_raw_alpha_and_default() {
        let alpha = 0.75f64.to_bits() + 1;
        let raw = [0.0f64.to_bits(), 1.0f64.to_bits(), 0.0f64.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let (session, id) = fixture(ColorMode::Rgb, SampleType::F64, &raw);
        let before = session.active().unwrap().doc.clone();
        let r = request(&session, &json!({})).unwrap().unwrap();
        let output = prepare(&before, &r, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(
            output.to_interleaved(Rect::new(-1, 0, 0, 1)),
            [0.5f64.to_bits(), 0.5f64.to_bits(), 0.5f64.to_bits(), alpha].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()
        );
        assert_eq!(output.default_bytes(), before.layer(id).unwrap().surface().unwrap().default_bytes());
    }
    #[test]
    fn gray_identity_keeps_raw_nonfinite_words_snapshot_and_real_redo() {
        let raw = [0x7ff8_0000_0000_0073u64, 1.0f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        let (mut session, id) = fixture(ColorMode::Grayscale, SampleType::F64, &raw);
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        try_run(&mut session, &json!({})).unwrap();
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().history.can_redo());
        assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().to_interleaved(Rect::new(-1, 0, 0, 1)), raw);
        assert!(try_run(&mut session, &json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
    }
}
