//! Existing marquee geometry with controlled publication. Wide old coverage is
//! never narrowed: generated GRAY8 shape weights are explicitly lifted to native
//! unit coverage, then combined with native exact integer/f64 arithmetic.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::gimp::multi_input_coverage::{self as coverage, Operation, Params};
use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::checked_arithmetic::{Domain, Policy, Rounding, UnitOverflow};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, try_read_sample_f64};
use photocraft_doc::Document;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
use serde_json::Value;
const ID: &str = "select.rect";
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {message}"))
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn failure(ctl: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(error) }
}
#[derive(Clone, Copy)]
struct Request {
    shape: Rect,
    mode: SelectionMode,
    ellipse: bool,
    aa: bool,
    feather: f32,
    label: &'static str,
}
fn exact(value: &Value) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured IDs must be canonical decimal strings"))?;
    s.parse().map_err(|_| bad("identity overflow"))
}
fn boolean(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None => Ok(default),
        Some(value) => value.as_bool().ok_or_else(|| bad("shape flag must be boolean")),
    }
}
fn request(session: &Session, p: &Value) -> Result<Request> {
    let object = p
        .as_object()
        .filter(|o| {
            o.len() <= 12
                && o.keys().all(|k| {
                    matches!(
                        k.as_str(),
                        "x" | "y" | "width" | "height" | "mode" | "ellipse" | "line" | "antiAlias" | "feather" | "documentId" | "expectedRevision"
                    )
                })
        })
        .ok_or_else(|| bad("bounded marquee parameters required"))?;
    let _ = object;
    let get = |key| crate::commands::int_i32(ID, p, key)?.ok_or_else(|| bad(format!("missing numeric {key}")));
    let x = get("x")?;
    let y = get("y")?;
    let width = get("width")?.max(0);
    let height = get("height")?.max(0);
    let shape = Rect::new(
        x,
        y,
        x.checked_add(width).ok_or_else(|| bad("horizontal edge overflow"))?,
        y.checked_add(height).ok_or_else(|| bad("vertical edge overflow"))?,
    );
    if [shape.x0, shape.y0, shape.x1, shape.y1].iter().any(|n| !(-(1 << 29)..=1 << 29).contains(n)) {
        return Err(bad("shape coordinate limit"));
    }
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(Value::String(value)) if value.len() <= 64 => SelectionMode::parse(value),
        _ => return Err(bad("bounded mode name required")),
    };
    let ellipse = boolean(p, "ellipse", false)?;
    let aa = boolean(p, "antiAlias", true)?;
    let label = match p.get("line") {
        Some(Value::String(value)) if value == "row" => "Single Row Marquee",
        Some(Value::String(value)) if value == "column" => "Single Column Marquee",
        None if ellipse => "Elliptical Marquee",
        None => "Rectangular Marquee",
        _ => return Err(bad("line must be row or column")),
    };
    let feather = match p.get("feather") {
        None => 0.0,
        Some(value) => value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("finite feather required"))?.clamp(0.0, 1000.0) as f32,
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id)? == state.doc.id.0 && exact(rev)? == state.revision => {}
        _ => return Err(bad("captured target changed")),
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
        return Err(bad("selection target is not idle"));
    }
    Ok(Request { shape, mode, ellipse, aa, feather, label })
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, _background: bool) -> Result<()> {
    request(session, p).map(|_| ())
}
struct Plan {
    format: PixelFormat,
    tiles: u64,
    stage_bytes: u64,
    stage_work: u64,
}
fn admission(doc: &Document, r: Request, ctl: &Interrupt<'_>) -> Result<Plan> {
    poll(ctl)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad("canvas size limit"));
    }
    let format = if let Some(old) = &doc.selection {
        if old.format().mode != ColorMode::Grayscale || old.format().alpha {
            return Err(bad("selection must be Gray without alpha"));
        }
        if matches!(old.format().sample, SampleType::U32 | SampleType::F64) { old.format() } else { PixelFormat::GRAY8 }
    } else if doc.selection_channel_id.is_some() && matches!(doc.depth, SampleType::U32 | SampleType::F64) {
        PixelFormat::new(ColorMode::Grayscale, doc.depth, false)
    } else {
        PixelFormat::GRAY8
    };
    let width = u64::from(doc.size.width);
    let height = u64::from(doc.size.height);
    let pixels = width.checked_mul(height).ok_or_else(|| bad("pixel count"))?;
    let tiles = width.div_ceil(TILE_SIZE as u64).checked_mul(height.div_ceil(TILE_SIZE as u64)).ok_or_else(|| bad("tile count"))?;
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * format.bytes_per_pixel() as u64;
    let stage_bytes = tiles
        .checked_mul(tile_bytes.checked_mul(4).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("output capacity"))?)
        .ok_or_else(|| bad("output capacity"))?;
    let stage_work = tiles
        .checked_mul(tile_bytes.checked_mul(9).and_then(|n| n.checked_add(8192)).ok_or_else(|| bad("staging work"))?)
        .ok_or_else(|| bad("staging work"))?;
    let radius = (r.feather * 3.0).ceil() as u64;
    let feather_bytes = if r.feather > 0.0 {
        pixels
            .checked_mul(56)
            .and_then(|n| n.checked_add(radius.checked_mul(256)?))
            .and_then(|n| n.checked_add(width.max(height).checked_mul(64)?))
            .and_then(|n| n.checked_add(65536))
            .ok_or_else(|| bad("feather capacity"))?
    } else {
        0
    };
    let feather_work = if r.feather > 0.0 {
        pixels
            .checked_mul(radius.checked_mul(4).and_then(|n| n.checked_add(128)).ok_or_else(|| bad("feather work"))?)
            .and_then(|n| n.checked_add(radius.checked_mul(512)?))
            .ok_or_else(|| bad("feather work"))?
    } else {
        0
    };
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, ctl).map_err(|e| failure(ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, ctl).map_err(|e| failure(ctl, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let peak = raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(stage_bytes))
        .and_then(|n| n.checked_add(feather_bytes))
        .and_then(|n| n.checked_add(16 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("aggregate resident limit"))?;
    let _ = peak;
    let work = pixels
        .checked_mul(if r.ellipse && r.aa { 128 } else { 32 })
        .and_then(|n| n.checked_add(stage_work))
        .and_then(|n| n.checked_add(feather_work))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad("aggregate work limit"))?;
    let _ = work;
    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
    poll(ctl)?;
    Ok(Plan { format, tiles, stage_bytes, stage_work })
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
fn shape_tile(r: Request, part: Rect, ctl: &Interrupt<'_>) -> Result<Surface> {
    poll(ctl)?;
    let shape = if r.ellipse {
        sel::ellipse_surface(r.shape, part, r.aa)
    } else {
        let mut shape = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[0]).map_err(|e| failure(ctl, e))?;
        let area = r.shape.intersect(&part);
        if !area.is_empty() {
            let len = photocraft_raster::checked_region_byte_len(area, PixelFormat::GRAY8).map_err(|e| failure(ctl, e))?;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(len).map_err(|_| bad("shape scratch allocation"))?;
            if bytes.capacity() > len.saturating_add(64) {
                return Err(bad("shape scratch capacity"));
            }
            bytes.resize(len, 255);
            shape.try_write_interleaved_with(area, &bytes, ctl).map_err(|e| failure(ctl, e))?;
        }
        shape
    };
    poll(ctl)?;
    Ok(shape)
}
fn coverage8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
fn feather(doc: &Document, r: Request, ctl: &Interrupt<'_>) -> Result<Option<Vec<f32>>> {
    if r.feather == 0.0 {
        return Ok(None);
    }
    let bounds = doc.bounds();
    let width = doc.size.width as usize;
    let height = doc.size.height as usize;
    let n = width.checked_mul(height).ok_or_else(|| bad("feather dimensions"))?;
    let mut mask = Vec::new();
    mask.try_reserve_exact(n).map_err(|_| bad("feather input allocation"))?;
    if mask.capacity() > n.saturating_add(64) {
        return Err(bad("feather input capacity"));
    }
    while mask.len() < n {
        poll(ctl)?;
        mask.resize(mask.len().saturating_add(4096).min(n), 0.0);
    }
    for tile in bounds.tiles() {
        poll(ctl)?;
        let part = tile.rect().intersect(&bounds);
        let shape = shape_tile(r, part, ctl)?;
        let mut word = [0u8; 1];
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                shape.try_read_pixel_bytes(x, y, &mut word).map_err(|e| failure(ctl, e))?;
                let offset = (y as usize).checked_mul(width).and_then(|n| n.checked_add(x as usize)).ok_or_else(|| bad("feather offset"))?;
                *mask.get_mut(offset).ok_or_else(|| bad("feather offset"))? = read_sample(&word, SampleType::U8, 0);
            }
        }
    }
    let output = sel::try_feather_with_interrupt(&mask, width, height, r.feather, ctl).map_err(|e| failure(ctl, e))?;
    poll(ctl)?;
    Ok(Some(output))
}
fn native_combine(old: &[u8], shape: u8, format: PixelFormat, r: Request, output: &mut [u8]) -> Result<()> {
    if old.len() != format.bytes_per_pixel() || output.len() != format.bytes_per_pixel() {
        return Err(bad("native combination sample length"));
    }
    let mut new = [0u8; 8];
    let new = new.get_mut(..format.bytes_per_pixel()).ok_or_else(|| bad("shape native width"))?;
    match format.sample {
        SampleType::U32 => new.copy_from_slice(&(u32::from(shape) * 0x01010101).to_ne_bytes()),
        SampleType::F64 => new.copy_from_slice(&(f64::from(shape) / 255.0).to_ne_bytes()),
        _ => return Err(bad("not native wide coverage")),
    }
    if r.mode == SelectionMode::Replace {
        output.copy_from_slice(new);
        return Ok(());
    }
    let operation = match r.mode {
        SelectionMode::Add => Operation::UnionMax,
        SelectionMode::Intersect => Operation::IntersectionMin,
        SelectionMode::Subtract if r.feather > 0.0 => Operation::SubtractSaturating,
        SelectionMode::Subtract => {
            if format.sample == SampleType::U32 {
                let code = u32::from_ne_bytes((&*new).try_into().map_err(|_| bad("U32 shape width"))?);
                new.copy_from_slice(&(u32::MAX - code).to_ne_bytes());
            } else {
                let value = try_read_sample_f64(new, SampleType::F64, 0).map_err(bad)?;
                new.copy_from_slice(&(1.0 - value).to_ne_bytes());
            }
            Operation::Multiply
        }
        _ => return Err(bad("unsupported combination")),
    };
    coverage::evaluate_sample_into(
        format,
        old,
        new,
        output,
        Params { operation, policy: Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest } },
    )
    .map_err(bad)
}
fn prepare(doc: &Document, r: Request, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    let plan = admission(doc, r, ctl)?;
    let bounds = doc.bounds();
    let blurred = feather(doc, r, ctl)?;
    let bpp = plan.format.bytes_per_pixel();
    let zero = [0u8; 8];
    let mut output = Surface::try_with_default_bytes(plan.format, zero.get(..bpp).ok_or_else(|| bad("default width"))?).map_err(|e| failure(ctl, e))?;
    let mut budget = Budget { bytes: plan.stage_bytes, work: plan.stage_work };
    let mut visited = 0u64;
    let mut positive = false;
    // Retain upstream COW sharing of completely selected tiles. This cache owns
    // one already-admitted native tile; equality is encoded, not profile/sample equivalence.
    let mut full_tile: Option<std::sync::Arc<photocraft_raster::Tile>> = None;
    let mut one = [0u8; 8];
    let one = one.get_mut(..bpp).ok_or_else(|| bad("one sample width"))?;
    match plan.format.sample {
        SampleType::U8 => one.copy_from_slice(&[255]),
        SampleType::U32 => one.copy_from_slice(&u32::MAX.to_ne_bytes()),
        SampleType::F64 => one.copy_from_slice(&1f64.to_ne_bytes()),
        _ => return Err(bad("shape output storage")),
    }

    let tile_len = (TILE_SIZE as usize) * (TILE_SIZE as usize) * bpp;
    let mut raw = Vec::new();
    raw.try_reserve_exact(tile_len).map_err(|_| bad("tile scratch allocation"))?;
    if raw.capacity() > 2 << 20 {
        return Err(bad("tile scratch capacity"));
    }
    raw.resize(tile_len, 0);
    let mut cell = Vec::new();
    cell.try_reserve_exact(64 * 64 * bpp).map_err(|_| bad("cell scratch allocation"))?;
    if cell.capacity() > 1 << 20 {
        return Err(bad("cell scratch capacity"));
    }
    for coordinate in bounds.tiles() {
        poll(ctl)?;
        visited = visited.checked_add(1).filter(|n| *n <= plan.tiles).ok_or_else(|| bad("tile census changed"))?;
        raw.fill(0);
        let tile = coordinate.rect();
        let part = tile.intersect(&bounds);
        let shape = if blurred.is_none() { Some(shape_tile(r, part, ctl)?) } else { None };
        let narrow = if plan.format == PixelFormat::GRAY8 {
            if let Some(blurred) = &blurred {
                let mut weights = Vec::new();
                let n = (part.width() as usize) * (part.height() as usize);
                weights.try_reserve_exact(n).map_err(|_| bad("tile feather allocation"))?;
                for y in part.y0..part.y1 {
                    poll(ctl)?;
                    let start = (y as usize) * (doc.size.width as usize) + part.x0 as usize;
                    weights.extend_from_slice(blurred.get(start..start + part.width() as usize).ok_or_else(|| bad("tile feather row"))?);
                }
                sel::combine(doc.selection.as_ref(), &weights, part, r.mode)
            } else {
                let shape = shape.as_ref().ok_or_else(|| bad("missing shape"))?;
                Some(match (r.mode, doc.selection.as_ref()) {
                    (SelectionMode::Add, Some(old)) => sel::combine_surfaces(old, shape, part, |a, b| a.max(b)),
                    (SelectionMode::Subtract, Some(old)) => sel::combine_surfaces(old, shape, part, |a, b| a * (1.0 - b)),
                    (SelectionMode::Intersect, Some(old)) => sel::combine_surfaces(old, shape, part, |a, b| a.min(b)),
                    (SelectionMode::Subtract | SelectionMode::Intersect, None) => Surface::new(PixelFormat::GRAY8),
                    _ => shape.clone(),
                })
            }
        } else {
            None
        };
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                poll(ctl)?;
                let offset = (((y - tile.y0) as usize) * (TILE_SIZE as usize) + (x - tile.x0) as usize) * bpp;
                let destination = raw.get_mut(offset..offset + bpp).ok_or_else(|| bad("tile sample"))?;
                if plan.format == PixelFormat::GRAY8 {
                    if let Some(narrow) = &narrow {
                        narrow.try_read_pixel_bytes(x, y, destination).map_err(|e| failure(ctl, e))?;
                    }
                } else {
                    let weight = if let Some(blurred) = &blurred {
                        coverage8(*blurred.get((y as usize) * (doc.size.width as usize) + x as usize).ok_or_else(|| bad("feather sample"))?)
                    } else {
                        let mut byte = [0u8; 1];
                        shape.as_ref().ok_or_else(|| bad("missing shape"))?.try_read_pixel_bytes(x, y, &mut byte).map_err(|e| failure(ctl, e))?;
                        byte[0]
                    };
                    let mut old = [0u8; 8];
                    let old = old.get_mut(..bpp).ok_or_else(|| bad("old sample width"))?;
                    if let Some(source) = &doc.selection {
                        source.try_read_pixel_bytes(x, y, old).map_err(|e| failure(ctl, e))?;
                    }
                    native_combine(old, weight, plan.format, r, destination)?;
                }
                positive |= try_read_sample_f64(destination, plan.format.sample, 0).map_err(bad)? > 0.0;
            }
        }
        let mut builder = StagedSurfaceBuilder::new(plan.format, zero.get(..bpp).ok_or_else(|| bad("default width"))?, tile, &mut budget, ctl)
            .map_err(|e| failure(ctl, e))?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                poll(ctl)?;
                cell.clear();
                for row in cy..cy + 64 {
                    let start = ((row as usize) * (TILE_SIZE as usize) + cx as usize) * bpp;
                    cell.extend_from_slice(raw.get(start..start + 64 * bpp).ok_or_else(|| bad("cell row"))?);
                }
                builder
                    .push_region(Rect::new(tile.x0 + cx, tile.y0 + cy, tile.x0 + cx + 64, tile.y0 + cy + 64), &cell, &mut budget, ctl)
                    .map_err(|e| failure(ctl, e))?;
            }
        }
        let (replacement, _lease) = builder.finalize(&mut budget, ctl).map_err(|e| failure(ctl, e))?.into_parts();
        poll(ctl)?;
        for (position, tile) in replacement.tiles() {
            poll(ctl)?;
            let mut full = tile.bytes().len() == tile_len;
            if full {
                for (index, pixel) in tile.bytes().chunks_exact(bpp).enumerate() {
                    if index.is_multiple_of(1024) {
                        poll(ctl)?;
                    }
                    if pixel != &*one {
                        full = false;
                        break;
                    }
                }
            }
            let shared = if full {
                if let Some(cached) = &full_tile {
                    std::sync::Arc::clone(cached)
                } else {
                    full_tile = Some(std::sync::Arc::clone(tile));
                    std::sync::Arc::clone(tile)
                }
            } else {
                std::sync::Arc::clone(tile)
            };
            output.put_tiles(std::iter::once((*position, shared)));
        }
    }
    output.try_prune_with(ctl).map_err(|e| failure(ctl, e))?;
    poll(ctl)?;
    Ok(positive.then_some(output))
}
pub(crate) fn run(session: &mut Session, p: &Value) -> Result<Value> {
    let r = request(session, p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let worker = baseline.clone();
    crate::jobs::run(
        session,
        r.label,
        true,
        move |ctx| ctx.stage(0.0, 0.95, r.label, |ctl| Ok((prepare(&worker, r, ctl)?, ctx.clone()))),
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
                return Err(bad("selection target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, r.label, |ctl| {
                poll(ctl)?;
                session.edit_with_interrupt(r.label, ctl, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    doc.selection = selection;
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    poll(ctl)
                })?;
                Ok(Value::Null)
            })
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_low_bits_and_raw_zero_survive_max_min_and_subtraction() {
        let r = Request { shape: Rect::new(0, 0, 1, 1), mode: SelectionMode::Add, ellipse: false, aa: false, feather: 0.0, label: "draft" };
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let old = 0x80000001u32.to_ne_bytes();
        let mut out = [0; 4];
        native_combine(&old, 0, format, r, &mut out).unwrap();
        assert_eq!(out, old);
        native_combine(&old, 255, format, Request { mode: SelectionMode::Subtract, ..r }, &mut out).unwrap();
        assert_eq!(out, [0; 4]);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let old = (-0.0f64).to_ne_bytes();
        let mut out = [0; 8];
        native_combine(&old, 0, format, r, &mut out).unwrap();
        assert_eq!(out, old);
    }
    #[test]
    fn ordinary_ellipse_matches_upstream_geometry_bytes_and_empty_outside_deselects() {
        let doc = Document::new("Ellipse draft", photocraft_geom::Size::new(19, 13), ColorMode::Rgb, SampleType::U8);
        let r = Request { shape: Rect::new(-3, 1, 17, 15), mode: SelectionMode::Replace, ellipse: true, aa: true, feather: 0.0, label: "draft" };
        let expected = sel::ellipse_surface(r.shape, doc.bounds(), true);
        let result = prepare(&doc, r, &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(result.try_read_interleaved(doc.bounds()).unwrap(), expected.try_read_interleaved(doc.bounds()).unwrap());
        assert!(prepare(&doc, Request { shape: Rect::new(50, 50, 52, 52), ..r }, &Interrupt::NONE).unwrap().is_none());
    }
    #[test]
    fn feathered_and_unfeathered_subtract_keep_the_two_upstream_equations() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let old = 0xc0000001u32.to_ne_bytes();
        let mut output = [0; 4];
        let r = Request { shape: Rect::new(0, 0, 1, 1), mode: SelectionMode::Subtract, ellipse: false, aa: false, feather: 1.0, label: "draft" };
        native_combine(&old, 128, format, r, &mut output).unwrap();
        assert_eq!(u32::from_ne_bytes(output), 0x3f7f7f81);
        native_combine(&old, 128, format, Request { feather: 0.0, ..r }, &mut output).unwrap();
        assert!(u32::from_ne_bytes(output) > 0x3f7f7f81);
    }
    #[test]
    fn native_add_keeps_f64_subnormal_outside_new_shape_without_f32() {
        let mut doc = Document::new("Native Add draft", photocraft_geom::Size::new(4, 4), ColorMode::Rgb, SampleType::F64);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut old = Surface::new(format);
        let bits = 1u64.to_ne_bytes();
        old.try_write_interleaved(Rect::new(3, 3, 4, 4), &bits).unwrap();
        doc.selection = Some(old);
        let before = doc.clone();
        let r = Request { shape: Rect::new(0, 0, 2, 2), mode: SelectionMode::Add, ellipse: false, aa: false, feather: 0.0, label: "draft" };
        let output = prepare(&doc, r, &Interrupt::NONE).unwrap().unwrap();
        let mut raw = [0; 8];
        output.try_read_pixel_bytes(3, 3, &mut raw).unwrap();
        assert_eq!(raw, bits);
        assert_eq!(doc, before);
    }
    #[test]
    fn ordinary_feather_matches_the_existing_shape_only_blur_and_combine() {
        let mut doc = Document::new("Feather draft", photocraft_geom::Size::new(7, 5), ColorMode::Rgb, SampleType::U8);
        let mut old = Surface::new(PixelFormat::GRAY8);
        old.fill_rect(Rect::new(1, 1, 6, 4), &[0.75]);
        doc.selection = Some(old);
        let r = Request { shape: Rect::new(0, 0, 4, 4), mode: SelectionMode::Subtract, ellipse: true, aa: true, feather: 1.25, label: "draft" };
        let shape = sel::ellipse_surface(r.shape, doc.bounds(), r.aa);
        let pixels = sel::feather(&sel::mask_from_surface(Some(&shape), doc.bounds()), 7, 5, r.feather);
        let expected = sel::combine(doc.selection.as_ref(), &pixels, doc.bounds(), r.mode);
        let actual = prepare(&doc, r, &Interrupt::NONE).unwrap();
        assert_eq!(actual.map(|s| s.try_read_interleaved(doc.bounds()).unwrap()), expected.map(|s| s.try_read_interleaved(doc.bounds()).unwrap()));
    }
    #[test]
    fn cancellation_and_overflow_leave_real_source_untouched() {
        let doc = Document::new("Rect draft", photocraft_geom::Size::new(4, 4), ColorMode::Rgb, SampleType::F64);
        let before = doc.clone();
        let r = Request { shape: Rect::new(0, 0, 3, 3), mode: SelectionMode::Replace, ellipse: false, aa: false, feather: 0.0, label: "draft" };
        assert!(matches!(prepare(&doc, r, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert_eq!(doc, before);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        assert!(request(&session, &serde_json::json!({"x":i32::MAX,"y":0,"width":10,"height":1})).is_err());
    }
}
