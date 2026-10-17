//! Controlled native stored-color Curves. Alpha/source metadata are copied raw.
use crate::{EngineError, Result, Session, channel_cmds::ChannelTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::Value;
use std::sync::Arc;
const ID: &str = "image.adjustments.curves";
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
    adjustment: photocraft_doc::Adjustment,
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
    let LayerContent::Raster(source) = &layer.content else { return Err(bad("native Curves requires a plain resident raster")) };
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
            return Err(bad("source/indexed/filter carrier curves is not qualified"));
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
        return Err(bad("finish transient work before native raster Curves"));
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
        return Err(bad("native Curves requires all color channels"));
    }
    envelope(p)?;
    if p.get("__kind").is_some_and(|v| v.as_str() != Some("curves")) {
        return Err(bad("native raster route requires Curves"));
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
        return Err(bad("U32 native Curves requires whole-percent opacity"));
    }
    // All upstream forms are bounded by envelope before this fixed-size,
    // library-managed parser construction (including pencil/smooth conversions).
    let adjustment = crate::adjust_params::from_params("curves", p, None, st.doc.mode)?;
    let photocraft_doc::Adjustment::Curves { per_channel, space, pencil, .. } = &adjustment else { return Err(bad("Curves parameter mismatch")) };
    if *space != photocraft_doc::adjust::ToneSpace::Rgb {
        return Err(bad("native Curves supports RGB tone space only"));
    }
    if st.doc.mode == ColorMode::Grayscale && per_channel.iter().any(|channel| !identity(channel, *pencil)) {
        return Err(bad("native Gray Curves supports the master record only; RGB channel remapping is unsupported"));
    }
    Ok(Some(Request { layer, opacity: percent / 100.0, percent: (sample == SampleType::U32).then_some(percent as u8), adjustment }))
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
            _ => return Err(bad("U32 Curves requires integer selection or Boolean floating coverage")),
        },
    })
}
use crate::native_tone_lut::TABLE_LEN;
#[cfg(test)]
use crate::native_tone_lut::TABLE_SCALE;
fn identity(points: &[photocraft_doc::adjust::CurvePoint], pencil: bool) -> bool {
    if pencil {
        points.len() == 256 && points.iter().enumerate().all(|(index, point)| point.output == index as f32 / 255.0)
    } else {
        photocraft_doc::adjust::is_identity_curve(points)
    }
}
fn envelope(params: &Value) -> Result<()> {
    if params.is_null() {
        return Ok(());
    }
    let object = params.as_object().filter(|object| object.len() <= 13).ok_or_else(|| bad("bounded Curves parameters required"))?;
    for (key, value) in object {
        match key.as_str() {
            "layer" | "target" | "documentId" | "expectedRevision" | "opacity" | "__kind" => {}
            "pencil" => {
                if !value.is_boolean() {
                    return Err(bad("pencil must be boolean"));
                }
            }
            "smooth" => {
                if value.as_f64().is_none_or(|value| !value.is_finite()) {
                    return Err(bad("smooth must be finite"));
                }
            }
            "points" | "gray" | "red" | "green" | "blue" => {
                let entries = value.as_array().ok_or_else(|| bad("Curves requires bounded points or a256-output map"))?;
                if entries.first().is_some_and(Value::is_number) {
                    if entries.len() != 256 || entries.iter().any(|value| value.as_f64().is_none_or(|value| !value.is_finite())) {
                        return Err(bad("Curves pencil map must have exactly256 finite values"));
                    }
                } else {
                    // Upstream permits up to64 input pairs before duplicate collapse
                    // to its19 distinct smooth points; retain that accepted form.
                    if !(2..=64).contains(&entries.len()) {
                        return Err(bad("Curves point envelope limit"));
                    }
                    for entry in entries {
                        let pair = entry.as_array().filter(|pair| pair.len() == 2).ok_or_else(|| bad("Curves point must have exactly2 coordinates"))?;
                        if pair.iter().any(|value| value.as_f64().is_none_or(|value| !value.is_finite() || !(-1e-6..=255.0 + 1e-6).contains(&value))) {
                            return Err(bad("Curves point coordinate domain"));
                        }
                    }
                }
            }
            _ => return Err(bad("unsupported native Curves parameter")),
        }
    }
    Ok(())
}
fn tables(request: &Request, control: &Interrupt<'_>) -> Result<[Vec<u64>; 3]> {
    poll(control)?;
    // Exact upstream natural-spline / pencil recipe, channel then master. Its
    // five bounded input curves and four4096-word outputs use library-managed
    // allocation; cancellation is polled at construction boundaries, not inside.
    let original = photocraft_compose::adjust::tone_luts(&request.adjustment);
    poll(control)?;
    let original_capacity = original
        .iter()
        .try_fold(0usize, |sum, table| sum.checked_add(table.capacity().checked_mul(std::mem::size_of::<f32>())?))
        .ok_or_else(|| bad("Curves table capacity overflow"))?;
    if original_capacity > 1 << 20 || original.iter().any(|table| table.len() != TABLE_LEN) {
        return Err(bad("Curves table construction capacity exceeded fixed envelope"));
    }
    // Check the entire rational word domain before allocating converted tables.
    for table in original.iter().take(3) {
        for (index, output) in table.iter().enumerate() {
            if index % 128 == 0 {
                poll(control)?;
            }
            crate::native_tone_lut::exact_word(*output).map_err(bad)?;
        }
    }
    let mut tables: [Vec<u64>; 3] = std::array::from_fn(|_| Vec::new());
    for table in &mut tables {
        table.try_reserve_exact(TABLE_LEN).map_err(|_| bad("Curves table allocation denied"))?;
    }
    for (channel, table) in original.iter().take(3).enumerate() {
        let destination = tables.get_mut(channel).ok_or_else(|| bad("Curves table channel"))?;
        for (index, value) in table.iter().enumerate() {
            if index % 128 == 0 {
                poll(control)?;
            }
            destination.push(crate::native_tone_lut::exact_word(*value).map_err(bad)?);
        }
    }
    let allocated = tables
        .iter()
        .try_fold(0usize, |sum, table| sum.checked_add(table.capacity().checked_mul(std::mem::size_of::<u64>())?))
        .ok_or_else(|| bad("Curves table capacity overflow"))?;
    if allocated > 1 << 20 {
        return Err(bad("Curves table allocator capacity exceeded admitted scratch"));
    }
    poll(control)?;
    Ok(tables)
}
fn interpolate_u32(table: &[u64], before: u32, percent: u8, selection: u32, selection_max: u32) -> Result<u32> {
    crate::native_tone_lut::interpolate_u32(table, before, percent, selection, selection_max).map_err(bad)
}
fn interpolate_f64(table: &[u64], before: f64, weight: f64) -> Result<f64> {
    crate::native_tone_lut::interpolate_f64(table, before, weight).map_err(bad)
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
    let pixels = count.checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(TILE_SIZE as u64)).ok_or_else(|| bad("Curves extent overflow"))?;
    let output = pixels.checked_mul(bpp as u64).and_then(|n| n.checked_add(count.checked_mul(2048)?)).ok_or_else(|| bad("Curves output overflow"))?;
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| bad("model overflow"))?)
        .and_then(|n| n.checked_add(footprint.retained_bytes.checked_mul(2)?))
        .and_then(|n| n.checked_add(footprint.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(4 << 20))
        .ok_or_else(|| bad("Curves staging overflow"))?;
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
        .and_then(|n| n.checked_add(100_000_000))
        .ok_or_else(|| bad("Curves work overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > 8_000_000_000 {
        return Err(bad("Curves retained/output/work limit"));
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
    let tables = tables(r, c)?;
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
        let width = usize::try_from(part.width()).map_err(|_| bad("Curves row"))?;
        let mut changed = false;
        for (i, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if i % 256 == 0 {
                poll(c)?;
            }
            let x = i32::try_from(i64::from(part.x0) + (i % width) as i64).map_err(|_| bad("Curves x"))?;
            let y = i32::try_from(i64::from(part.y0) + (i / width) as i64).map_err(|_| bad("Curves y"))?;
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
                let table = tables.get(color).ok_or_else(|| bad("Curves table channel"))?;
                match fmt.sample {
                    SampleType::U32 => {
                        let before = u32::from_ne_bytes((&*sample).try_into().map_err(|_| bad("U32 width"))?);
                        let (num, den) = fraction.ok_or_else(|| bad("integer selection domain"))?;
                        let after = interpolate_u32(table, before, r.percent.ok_or_else(|| bad("integer opacity domain"))?, num, den)?;
                        if after != before {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    SampleType::F64 => {
                        let before = f64::from_ne_bytes((&*sample).try_into().map_err(|_| bad("F64 width"))?);
                        let after = interpolate_f64(table, before, weight)?;
                        if after.to_bits() != before.to_bits() {
                            sample.copy_from_slice(&after.to_ne_bytes());
                            changed = true;
                        }
                    }
                    _ => return Err(bad("native Curves depth")),
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
        "Curves",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Curves", |c| Ok((prepare(&worker, &r, c)?, ctx.clone()))),
        move |s, (output, ctx)| {
            ctx.stage(0.95, 1.0, "Curves", |c| {
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
                    return Err(bad("captured raster Curves changed"));
                }
                target(&st.doc, layer, c)?;
                let Some(output) = output else { return Ok(Value::Null) };
                s.edit_with_interrupt("Curves", c, move |doc, _| {
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

// Draft regressions only; no tests executed during implementation.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Layer, Size};
    use serde_json::json;
    fn fixture() -> (Session, LayerId, Vec<u8>) {
        let mut doc = Document::new("Native Curves", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
        let mut layer = Layer::raster("Original", doc.pixel_format());
        let id = layer.id;
        let raw = [0.25f64.to_bits(), 0.5f64.to_bits(), 0.75f64.to_bits(), 0x7ff8_0000_0000_0042u64].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        layer.content = LayerContent::Raster(Surface::try_from_interleaved_with(doc.pixel_format(), Rect::new(-1, 0, 0, 1), &raw, &Interrupt::NONE).unwrap());
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        (session, id, raw)
    }
    #[test]
    fn smooth_and_pencil_tables_preserve_original_channel_master_order() {
        let (session, _, _) = fixture();
        for pencil in [false, true] {
            let params = json!({"pencil":pencil,"points":[[0,0],[128,192],[255,255]],"red":[[0,0],[128,64],[255,255]]});
            let r = request(&session, &params).unwrap().unwrap();
            let exact = tables(&r, &Interrupt::NONE).unwrap();
            let expected = photocraft_compose::adjust::tone_luts(&r.adjustment);
            for (channel, row) in exact.iter().enumerate() {
                for (index, word) in row.iter().enumerate() {
                    assert_eq!(*word as f64 / TABLE_SCALE as f64, f64::from(expected[channel][index]));
                }
            }
        }
    }
    #[test]
    fn hostile_nested_or_giant_forms_refuse_before_parser_and_pencil_map_is_admitted() {
        assert!(envelope(&json!({"points":vec![json!([0,0]);65]})).is_err());
        assert!(envelope(&json!({"points":[[0,0,0],[255,255]]})).is_err());
        assert!(envelope(&json!({"red":vec![0;257]})).is_err());
        assert!(envelope(&json!({"points":[[0,0],[255,{"nested":1}]]})).is_err());
        let (session, _, _) = fixture();
        let map = (0..256).collect::<Vec<_>>();
        let r = request(&session, &json!({"pencil":true,"points":map,"smooth":1})).unwrap().unwrap();
        assert!(matches!(r.adjustment, photocraft_doc::Adjustment::Curves { pencil: true, .. }));
    }
    #[test]
    fn current_native_edit_changes_colors_only_and_cancel_or_stale_keeps_real_redo() {
        let (mut session, id, raw) = fixture();
        session
            .edit("Seed", |doc, _| {
                doc.name = "Seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let params = json!({"points":[[0,255],[255,0]]});
        let r = request(&session, &params).unwrap().unwrap();
        let output = prepare(&before, &r, &Interrupt::NONE).unwrap().unwrap();
        let edited = output.to_interleaved(Rect::new(-1, 0, 0, 1));
        assert_ne!(edited, raw);
        assert_eq!(edited.get(24..), raw.get(24..));
        assert_eq!(output.default_bytes(), before.layer(id).unwrap().surface().unwrap().default_bytes());
        let cancel = || true;
        assert!(matches!(prepare(&before, &r, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(try_run(&mut session, &json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert!(session.active().unwrap().history.can_redo());
    }
}
