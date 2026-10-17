//! Controlled native stored-color Posterize. Alpha/source metadata are copied raw.
use crate::native_gray_fill_controlled::unit_math::{self, Blend};
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const ID: &str = "image.adjustments.posterize";
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
    levels: u8,
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
    let LayerContent::Raster(source) = &layer.content else { return Err(bad("native Posterize requires a plain resident raster")) };
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
            return Err(bad("source/indexed/filter carrier posterize is not qualified"));
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
        return Err(bad("finish transient work before native raster Posterize"));
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
        return Err(bad("native Posterize requires all color channels"));
    }
    if !p.is_null() {
        let o = p.as_object().filter(|o| o.len() <= 7).ok_or_else(|| bad("bounded native Posterize parameters required"))?;
        if o.keys().any(|k| !matches!(k.as_str(), "layer" | "target" | "documentId" | "expectedRevision" | "opacity" | "__kind" | "levels")) {
            return Err(bad("unknown native raster Posterize parameter"));
        }
    }
    if p.get("__kind").is_some_and(|v| v.as_str() != Some("posterize")) {
        return Err(bad("native raster route requires Posterize"));
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
        return Err(bad("U32 native Posterize requires whole-percent opacity"));
    }
    let parsed = crate::adjust_params::from_params("posterize", p, None, st.doc.mode)?;
    let photocraft_doc::Adjustment::Posterize { levels } = parsed else { return Err(bad("posterize parameter mismatch")) };
    if !(2..=255).contains(&levels) {
        return Err(bad("posterize levels domain"));
    }
    let levels = u8::try_from(levels).map_err(|_| bad("posterize levels domain"))?;
    Ok(Some(Request { layer, opacity: percent / 100.0, percent: (sample == SampleType::U32).then_some(percent as u8), levels }))
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
            _ => return Err(bad("U32 Posterize requires integer selection or Boolean floating coverage")),
        },
    })
}
fn u32_grid(value: u32) -> u64 {
    (u64::from(value) * 255 + u64::from(u32::MAX) / 2) / u64::from(u32::MAX)
}
fn posterize_byte(grid: u64, levels: u8) -> Result<u8> {
    if grid > 255 || !(2..=255).contains(&levels) {
        return Err(bad("posterize grid/levels domain"));
    }
    let n = u64::from(levels);
    let bin = (grid * n / 256).min(n - 1);
    u8::try_from(bin * 255 / (n - 1)).map_err(|_| bad("posterize output domain"))
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
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("Posterize extent overflow"))?;
    let output = pixels.checked_mul(bpp as u64).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("Posterize output overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("model overflow"))?)
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("Posterize staging overflow"))?;
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
        .ok_or_else(|| bad("Posterize work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("Posterize retained/output/work limit"));
    }
    if let Some(selection) = &doc.selection {
        if selection.format().mode != ColorMode::Grayscale || selection.format().alpha {
            return Err(bad("selection requires Gray coverage without alpha"));
        }
        selection.try_validate_tile_origins_with(c).map_err(|e| error(c, e))?;
    }
    if area.is_empty() || r.opacity == 0.0 {
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
        let width = usize::try_from(part.width()).map_err(|_| bad("Posterize row"))?;
        let mut changed = false;
        for (i, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if i % 256 == 0 {
                poll(c)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("Posterize x"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("Posterize y"))?;
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
            // Alpha is only read by the raw nonpositive skip gate; never rewritten.
            for color in 0..colors {
                let start = color.checked_mul(fmt.sample.bytes()).ok_or_else(|| bad("color offset"))?;
                let end = start.checked_add(fmt.sample.bytes()).ok_or_else(|| bad("color offset"))?;
                let sample = pixel.get_mut(start..end).ok_or_else(|| bad("color sample width"))?;
                match fmt.sample {
                    SampleType::U32 => {
                        let before = u32::from_ne_bytes((&*sample).try_into().map_err(|_| bad("U32 width"))?);
                        let (num, den) = fraction.ok_or_else(|| bad("integer selection domain"))?;
                        let after = unit_math::u32_blend(
                            before,
                            u32::from(posterize_byte(u32_grid(before), r.levels)?) * (u32::MAX / 255),
                            Blend::Normal,
                            r.percent.ok_or_else(|| bad("integer opacity domain"))?,
                            num,
                            den,
                        )
                        .map_err(bad)?;
                        if after != before {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    SampleType::F64 => {
                        let before = f64::from_ne_bytes((&*sample).try_into().map_err(|_| bad("F64 width"))?);
                        if !before.is_finite() || !(0.0..=1.0).contains(&before) {
                            return Err(bad("native Posterize requires finite unit color samples"));
                        }
                        let posterized = f64::from(posterize_byte((before * 255.0).round() as u64, r.levels)?) / 255.0;
                        let after = if weight == 1.0 {
                            posterized
                        } else {
                            let original = before * (1.0 - weight);
                            let edited = posterized * weight;
                            original + edited
                        };
                        if !after.is_finite() {
                            return Err(bad("native Posterize arithmetic overflow"));
                        }
                        if after.to_bits() != before.to_bits() {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    _ => return Err(bad("native Posterize depth")),
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
        "Posterize",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Posterize", |c| Ok((prepare(&worker, &r, c)?, ctx.clone()))),
        move |s, (output, ctx)| {
            ctx.stage(0.95, 1.0, "Posterize", |c| {
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
                    return Err(bad("captured raster Posterize changed"));
                }
                target(&st.doc, layer, c)?;
                let Some(output) = output else { return Ok(Value::Null) };
                s.edit_with_interrupt("Posterize", c, move |doc, _| {
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

// Draft regressions only: execution deferred to the coordinated batch.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, Size};
    fn fixture(sample: SampleType, raw: &[u8]) -> (Document, LayerId) {
        let mut doc = Document::new("Native stored color", Size::new(1, 1), ColorMode::Rgb, sample);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        let id = layer.id;
        layer.content = LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(), Rect::new(-1, 0, 0, 1), raw, &Interrupt::NONE).unwrap());
        doc.layers.push(layer);
        (doc, id)
    }

    #[test]
    fn exact_native_grid_and_bins_preserve_alpha_and_offcanvas_default() {
        let raw = [64u32 * (u32::MAX / 255), 128 * (u32::MAX / 255), u32::MAX, 19].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        let (doc, id) = fixture(SampleType::U32, &raw);
        let before = doc.clone();
        let r = Request { layer: id, opacity: 1.0, percent: Some(100), levels: 4 };
        let out = prepare(&doc, &r, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(
            out.to_interleaved(Rect::new(-1, 0, 0, 1)),
            [85u32 * (u32::MAX / 255), 170 * (u32::MAX / 255), u32::MAX, 19].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        );
        assert_eq!(out.default_bytes(), doc.layer(id).unwrap().surface().unwrap().default_bytes());
        assert_eq!(doc, before);
        let cancel = || true;
        assert!(matches!(prepare(&doc, &r, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn all_declared_grid_bins_match_upstream_integer_equation() {
        for levels in 2u8..=255 {
            for grid in 0u64..=255 {
                let n = u64::from(levels);
                let expected = ((grid * n / 256).min(n - 1) * 255 / (n - 1)) as u8;
                assert_eq!(posterize_byte(grid, levels).unwrap(), expected);
            }
        }
        assert!(posterize_byte(256, 4).is_err());
        assert!(posterize_byte(0, 1).is_err());
    }
}
