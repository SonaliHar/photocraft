//! Controlled existing Feather consumer. Other Modify kernels retain upstream
//! ordinary callers; this module provides their pre-dispatch native-wide fence.
//! No native Gaussian/EDT precision schedule or source admission is inferred.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::selection as sel;
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample};
use photocraft_doc::Document;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use serde_json::{Value, json};
fn bad(command: &str, message: impl std::fmt::Display) -> EngineError {
    EngineError::BadParams { cmd: command.into(), msg: message.to_string() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn failure(command: &str, ctl: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(command, error) }
}
fn decimal(command: &str, value: &Value) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(command, "captured IDs must be canonical decimal strings"))?;
    s.parse().map_err(|_| bad(command, "identity overflow"))
}
#[derive(Clone, Copy)]
struct Request {
    radius: f32,
    at_bounds: bool,
}
fn request(session: &Session, command: &str, p: &Value) -> Result<Request> {
    let (minimum, maximum) = match command {
        "select.modify.border" => (1.0, 200.0),
        "select.modify.feather" => (0.1, 1000.0),
        "select.modify.expand" | "select.modify.contract" | "select.modify.smooth" => (1.0, 500.0),
        _ => return Err(bad(command, "unknown Modify command")),
    };
    if !p.is_null() {
        let object = p
            .as_object()
            .filter(|o| o.len() <= 4 && o.keys().all(|k| matches!(k.as_str(), "radius" | "applyAtCanvasBounds" | "documentId" | "expectedRevision")))
            .ok_or_else(|| bad(command, "bounded Modify parameters required"))?;
        let _ = object;
    }
    let radius = match p.get("radius") {
        None => 1.0,
        Some(value) => value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(command, "radius must be finite numeric"))? as f32,
    };
    if !(minimum..=maximum).contains(&radius) {
        return Err(bad(command, format!("radius must be in {minimum}..{maximum}")));
    }
    let at_bounds = match p.get("applyAtCanvasBounds") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| bad(command, "applyAtCanvasBounds must be boolean"))?,
    } || command == "select.modify.border";
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(command, id)? == state.doc.id.0 && decimal(command, rev)? == state.revision => {}
        _ => return Err(bad(command, "captured target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_job_on(state.doc.id)
        || session.command_has_pending_job()
    {
        return Err(bad(command, "selection target is not idle"));
    }
    let source = state.doc.selection.as_ref().ok_or_else(|| bad(command, "no selection"))?;
    if matches!(source.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(bad(command, "native-wide Modify arithmetic is not qualified; no f32 conversion performed"));
    }
    if source.format().mode != ColorMode::Grayscale || source.format().alpha {
        return Err(bad(command, "selection must be Gray without alpha"));
    }
    Ok(Request { radius, at_bounds })
}
/// Join before generic floating/async normalization for all five Modify IDs.
pub(crate) fn dispatch_preflight(session: &Session, command: &str, p: &Value, _background: bool) -> Result<()> {
    request(session, command, p).map(|_| ())
}
struct Plan {
    width: usize,
    height: usize,
    pad: usize,
    pw: usize,
    ph: usize,
    pixels: usize,
    padded: usize,
    stage_bytes: u64,
    stage_work: u64,
}
fn edge_sample(source: &Surface, x: i32, y: i32, ctl: &Interrupt<'_>) -> Result<f32> {
    poll(ctl)?;
    let mut word = [0u8; 4];
    let sample = source.format().sample;
    let bytes = word.get_mut(..sample.bytes()).ok_or_else(|| bad("select.modify.feather", "source word width"))?;
    source.try_read_pixel_bytes(x, y, bytes).map_err(|e| failure("select.modify.feather", ctl, e))?;
    let value = read_sample(bytes, sample, 0);
    if !value.is_finite() {
        return Err(bad("select.modify.feather", "nonfinite selection coverage"));
    }
    Ok(value)
}
fn touches_edge(doc: &Document, ctl: &Interrupt<'_>) -> Result<bool> {
    let source = doc.selection.as_ref().ok_or_else(|| bad("select.modify.feather", "missing selection"))?;
    let width = doc.size.width as i32;
    let height = doc.size.height as i32;
    for x in 0..width {
        if edge_sample(source, x, 0, ctl)? > 0.0 || edge_sample(source, x, height - 1, ctl)? > 0.0 {
            return Ok(true);
        }
    }
    for y in 0..height {
        if edge_sample(source, 0, y, ctl)? > 0.0 || edge_sample(source, width - 1, y, ctl)? > 0.0 {
            return Ok(true);
        }
    }
    Ok(false)
}
fn admission(doc: &Document, r: Request, ctl: &Interrupt<'_>) -> Result<Plan> {
    const CMD: &str = "select.modify.feather";
    poll(ctl)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad(CMD, "canvas size limit"));
    }
    let width = doc.size.width as usize;
    let height = doc.size.height as usize;
    let pixels = width.checked_mul(height).ok_or_else(|| bad(CMD, "pixel count overflow"))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| failure(CMD, ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| failure(CMD, ctl, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let base = raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(CMD, "retained model/raster limit"))?;
    let source = doc.selection.as_ref().ok_or_else(|| bad(CMD, "no selection"))?;
    if matches!(source.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(bad(CMD, "native-wide Feather math is not qualified"));
    }
    if source.format().mode != ColorMode::Grayscale || source.format().alpha {
        return Err(bad(CMD, "selection must be Gray without alpha"));
    }
    source.try_validate_tile_origins_with(ctl).map_err(|e| failure(CMD, ctl, e))?;
    let radius = (r.radius * 3.0).ceil() as usize;
    let pad = if touches_edge(doc, ctl)? { radius.checked_add(2).ok_or_else(|| bad(CMD, "padding overflow"))? } else { 0 };
    let pw = pad.checked_mul(2).and_then(|n| width.checked_add(n)).ok_or_else(|| bad(CMD, "padded width"))?;
    let ph = pad.checked_mul(2).and_then(|n| height.checked_add(n)).ok_or_else(|| bad(CMD, "padded height"))?;
    let padded = pw.checked_mul(ph).filter(|n| *n <= 1 << 30).ok_or_else(|| bad(CMD, "padded pixel limit"))?;
    let tiles = (width as u64).div_ceil(TILE_SIZE as u64).checked_mul((height as u64).div_ceil(TILE_SIZE as u64)).ok_or_else(|| bad(CMD, "tile count"))?;
    let stage_bytes = tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 4 + 4096).ok_or_else(|| bad(CMD, "output capacity"))?;
    let stage_work = tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 9 + 8192).ok_or_else(|| bad(CMD, "output work"))?;
    // Matches the lower kernel's conservative live-plane envelope, plus input,
    // padding/crop ownership and the final native output simultaneously retained.
    let temporary = (padded as u64)
        .checked_mul(64)
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(8)?))
        .and_then(|n| n.checked_add((radius as u64).checked_mul(256)?))
        .and_then(|n| n.checked_add((pw.max(ph) as u64).checked_mul(64)?))
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| bad(CMD, "feather capacity"))?;
    if base.checked_add(temporary).and_then(|n| n.checked_add(stage_bytes)).and_then(|n| n.checked_add(4 << 20)).is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) {
        return Err(bad(CMD, "aggregate resident limit"));
    }
    let work = (padded as u64)
        .checked_mul((radius as u64).checked_mul(4).and_then(|n| n.checked_add(160)).ok_or_else(|| bad(CMD, "feather work"))?)
        .and_then(|n| n.checked_add((radius as u64).checked_mul(512)?))
        .and_then(|n| n.checked_add(stage_work))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad(CMD, "aggregate work limit"))?;
    let _ = work;
    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
    poll(ctl)?;
    Ok(Plan { width, height, pad, pw, ph, pixels, padded, stage_bytes, stage_work })
}
fn floats(count: usize, ctl: &Interrupt<'_>) -> Result<Vec<f32>> {
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| bad("select.modify.feather", "plane allocation"))?;
    if result.capacity() > count.saturating_add(64) {
        return Err(bad("select.modify.feather", "plane capacity"));
    }
    while result.len() < count {
        poll(ctl)?;
        result.resize(result.len().saturating_add(4096).min(count), 0.0);
    }
    Ok(result)
}
struct Budget {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Budget {
    type Lease = ();
    fn reserve(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, n: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(n).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn prepare(doc: &Document, r: Request, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    const CMD: &str = "select.modify.feather";
    let plan = admission(doc, r, ctl)?;
    let source = doc.selection.as_ref().ok_or_else(|| bad(CMD, "no selection"))?;
    let mut mask = floats(plan.pixels, ctl)?;
    for y in 0..plan.height {
        poll(ctl)?;
        for x in 0..plan.width {
            let value = edge_sample(source, x as i32, y as i32, ctl)?;
            *mask.get_mut(y * plan.width + x).ok_or_else(|| bad(CMD, "input plane offset"))? = value;
        }
    }
    let padded = if plan.pad == 0 {
        mask
    } else {
        let mut padded = floats(plan.padded, ctl)?;
        for py in 0..plan.ph {
            poll(ctl)?;
            if r.at_bounds && !(plan.pad..plan.pad + plan.height).contains(&py) {
                continue;
            }
            let y = py.saturating_sub(plan.pad).min(plan.height - 1);
            for px in 0..plan.pw {
                if px.is_multiple_of(1024) {
                    poll(ctl)?;
                }
                if r.at_bounds && !(plan.pad..plan.pad + plan.width).contains(&px) {
                    continue;
                }
                let x = px.saturating_sub(plan.pad).min(plan.width - 1);
                *padded.get_mut(py * plan.pw + px).ok_or_else(|| bad(CMD, "padded offset"))? =
                    *mask.get(y * plan.width + x).ok_or_else(|| bad(CMD, "input offset"))?;
            }
        }
        padded
    };
    let blurred = sel::try_feather_with_interrupt(&padded, plan.pw, plan.ph, r.radius, ctl).map_err(|e| failure(CMD, ctl, e))?;
    let mut output = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[0]).map_err(|e| failure(CMD, ctl, e))?;
    let mut budget = Budget { bytes: plan.stage_bytes, work: plan.stage_work };
    let mut positive = false;
    let mut raw = Vec::new();
    raw.try_reserve_exact((TILE_SIZE as usize) * (TILE_SIZE as usize)).map_err(|_| bad(CMD, "tile scratch"))?;
    if raw.capacity() > 1 << 20 {
        return Err(bad(CMD, "tile scratch capacity"));
    }
    raw.resize((TILE_SIZE as usize) * (TILE_SIZE as usize), 0);
    let mut cell = Vec::new();
    cell.try_reserve_exact(64 * 64).map_err(|_| bad(CMD, "cell scratch"))?;
    if cell.capacity() > 1 << 20 {
        return Err(bad(CMD, "cell scratch capacity"));
    }
    let mut full: Option<std::sync::Arc<photocraft_raster::Tile>> = None;
    for position in doc.bounds().tiles() {
        poll(ctl)?;
        raw.fill(0);
        let tile = position.rect();
        let area = tile.intersect(&doc.bounds());
        for y in area.y0..area.y1 {
            poll(ctl)?;
            for x in area.x0..area.x1 {
                poll(ctl)?;
                let source =
                    (y as usize + plan.pad).checked_mul(plan.pw).and_then(|n| n.checked_add(x as usize + plan.pad)).ok_or_else(|| bad(CMD, "crop offset"))?;
                let value = *blurred.get(source).ok_or_else(|| bad(CMD, "crop value"))?;
                if !value.is_finite() {
                    return Err(bad(CMD, "nonfinite feather output"));
                }
                let encoded = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                positive |= encoded != 0;
                let destination = ((y - tile.y0) as usize) * (TILE_SIZE as usize) + (x - tile.x0) as usize;
                *raw.get_mut(destination).ok_or_else(|| bad(CMD, "tile sample"))? = encoded;
            }
        }
        let mut builder = StagedSurfaceBuilder::new(PixelFormat::GRAY8, &[0], tile, &mut budget, ctl).map_err(|e| failure(CMD, ctl, e))?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                poll(ctl)?;
                cell.clear();
                for row in cy..cy + 64 {
                    let offset = (row as usize) * (TILE_SIZE as usize) + cx as usize;
                    cell.extend_from_slice(raw.get(offset..offset + 64).ok_or_else(|| bad(CMD, "cell row"))?);
                }
                builder
                    .push_region(Rect::new(tile.x0 + cx, tile.y0 + cy, tile.x0 + cx + 64, tile.y0 + cy + 64), &cell, &mut budget, ctl)
                    .map_err(|e| failure(CMD, ctl, e))?;
            }
        }
        let (replacement, _lease) = builder.finalize(&mut budget, ctl).map_err(|e| failure(CMD, ctl, e))?.into_parts();
        for (position, tile) in replacement.tiles() {
            poll(ctl)?;
            let mut white = true;
            for chunk in tile.bytes().chunks(4096) {
                poll(ctl)?;
                if chunk.iter().any(|b| *b != 255) {
                    white = false;
                    break;
                }
            }
            let shared = if white {
                if let Some(full) = &full {
                    std::sync::Arc::clone(full)
                } else {
                    full = Some(std::sync::Arc::clone(tile));
                    std::sync::Arc::clone(tile)
                }
            } else {
                std::sync::Arc::clone(tile)
            };
            output.put_tiles(std::iter::once((*position, shared)));
        }
    }
    output.try_prune_with(ctl).map_err(|e| failure(CMD, ctl, e))?;
    poll(ctl)?;
    Ok(positive.then_some(output))
}
/// Join only the existing select.modify.feather run handler. The other four use
/// this module's early native-wide guard and retain their ordinary upstream run.
pub(crate) fn run_feather(session: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "select.modify.feather";
    let r = request(session, CMD, p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let worker = baseline.clone();
    crate::jobs::run(
        session,
        "Feather Selection",
        true,
        move |ctx| ctx.stage(0.0, 0.95, "Feather Selection", |ctl| Ok((prepare(&worker, r, ctl)?, ctx.clone()))),
        move |session, (selection, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !baseline.downgrade().is_current(&state.doc)
                || state.floating.is_some()
                || state.coalesce.is_some()
                || session.command_has_job_on(state.doc.id)
                || session.command_has_pending_job()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad(CMD, "selection target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, "Feather Selection", |ctl| {
                poll(ctl)?;
                let selected = selection.is_some();
                session.edit_with_interrupt("Feather Selection", ctl, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    doc.selection = selection;
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    poll(ctl)
                })?;
                Ok(json!({"selected":selected}))
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Document {
        let mut doc = Document::new("Feather consumer draft", photocraft_geom::Size::new(9, 7), ColorMode::Rgb, SampleType::U8);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(0, 0, 5, 4), &[1.0]);
        doc.selection = Some(mask);
        doc
    }
    #[test]
    fn ordinary_padding_and_quantization_match_upstream_equations() {
        let doc = fixture();
        let mask = sel::mask_from_surface(doc.selection.as_ref(), doc.bounds());
        for at_bounds in [false, true] {
            let radius = 1.25f32;
            let pad = (radius * 3.0).ceil() as usize + 2;
            let (w, h) = (9usize, 7usize);
            let (pw, ph) = (w + pad * 2, h + pad * 2);
            let mut padded = vec![0f32; pw * ph];
            for py in 0..ph {
                for px in 0..pw {
                    if at_bounds && (!(pad..pad + h).contains(&py) || !(pad..pad + w).contains(&px)) {
                        continue;
                    }
                    padded[py * pw + px] = mask[py.saturating_sub(pad).min(h - 1) * w + px.saturating_sub(pad).min(w - 1)];
                }
            }
            let blurred = sel::feather(&padded, pw, ph, radius);
            let mut cropped = Vec::new();
            for y in 0..h {
                cropped.extend_from_slice(&blurred[(y + pad) * pw + pad..(y + pad) * pw + pad + w]);
            }
            let expected = sel::combine(None, &cropped, doc.bounds(), sel::SelectionMode::Replace);
            let actual = prepare(&doc, Request { radius, at_bounds }, &Interrupt::NONE).unwrap();
            assert_eq!(actual.map(|s| s.try_read_interleaved(doc.bounds()).unwrap()), expected.map(|s| s.try_read_interleaved(doc.bounds()).unwrap()));
        }
    }
    #[test]
    fn wide_source_is_refused_before_reading_and_cancellation_keeps_original() {
        let mut doc = fixture();
        doc.selection = Some(
            Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &0x7ff8000000000001u64.to_ne_bytes()).unwrap(),
        );
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        assert!(dispatch_preflight(&session, "select.modify.feather", &Value::Null, false).is_err());
        assert!(before.downgrade().is_current(&session.active().unwrap().doc));
        let doc = fixture();
        let before = doc.clone();
        assert!(matches!(prepare(&doc, Request { radius: 1.0, at_bounds: false }, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(doc, before);
    }
}
