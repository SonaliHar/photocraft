//! Controlled path.toSelection. Ordinary GRAY8 quantization remains unchanged.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::selection::{self, SelectionMode};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Path, Subpath, VectorMask};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use serde_json::{Value, json};
const ID: &str = "path.toSelection";
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn raster(e: RasterInputError) -> EngineError {
    if e == RasterInputError::Cancelled { EngineError::Cancelled } else { error(e) }
}
fn exact(v: &Value, nonzero: bool) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
    let n = s.parse::<u64>().map_err(|_| error("captured ID exceeds range"))?;
    if nonzero && n == 0 {
        return Err(error("documentId must be nonzero"));
    }
    Ok(n)
}
#[derive(Clone, Copy)]
struct Options {
    feather: f32,
    aa: bool,
    mode: SelectionMode,
}
fn options(p: &Value) -> Result<Options> {
    let feather = match p.get("feather") {
        None => 0.0,
        Some(v) => {
            v.as_f64().filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX)).ok_or_else(|| error("feather must be finite representable pixels"))? as f32
        }
    };
    let aa = match p.get("antiAlias") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| error("antiAlias must be boolean"))?,
    };
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(v) => {
            let s = v.as_str().filter(|s| matches!(*s, "replace" | "add" | "subtract" | "intersect")).ok_or_else(|| error("invalid selection mode"))?;
            SelectionMode::parse(s)
        }
    };
    Ok(Options { feather, aa, mode })
}
fn context(session: &Session, p: &Value, early: bool) -> Result<Options> {
    if !p.is_null()
        && !p.as_object().is_some_and(|p| {
            p.len() <= 7 && p.keys().all(|k| matches!(k.as_str(), "pathId" | "name" | "feather" | "antiAlias" | "mode" | "documentId" | "expectedRevision"))
        })
    {
        return Err(error("expected bounded path selection parameters"));
    }
    if p.get("name").is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 4096 && !s.as_bytes().contains(&0))) {
        return Err(error("name must be bounded UTF8 without NUL"));
    }
    let _ = super::saved_path_id(p)?;
    let opts = options(p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id, true)? == state.doc.id.0 && exact(rev, false)? == state.revision => {}
        _ => return Err(error("captured target changed or incomplete")),
    }
    if (state.floating.is_some() && !(early && p.get("documentId").is_none()))
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
        || state.revision == u64::MAX
        || state.doc.accounted().is_some()
    {
        return Err(error("finish transient or pending work before making a selection"));
    }
    if opts.mode != SelectionMode::Replace && state.doc.selection.as_ref().is_some_and(|s| matches!(s.format().sample, SampleType::U32 | SampleType::F64)) {
        return Err(error("wide native selection arithmetic is not qualified for legacy f32 combination"));
    }
    if state.doc.paths.len() > 10_000 {
        return Err(error("saved path count limit"));
    }
    let path = resolve(&state.doc, state.active_layer, super::saved_path_id(p)?, p.get("name").and_then(Value::as_str), &Interrupt::NONE)?;
    if path.subpaths.len() > photocraft_vector::VectorPreparationLimits::default().max_components {
        return Err(error("path component limit"));
    }
    Ok(opts)
}
pub(super) fn dispatch_preflight(session: &Session, id: &str, p: &Value, _background: bool) -> Result<bool> {
    if id != ID {
        return Ok(false);
    }
    let _ = context(session, p, true)?;
    Ok(true)
}
fn layer_path<'a>(layers: &'a [Layer], id: LayerId, depth: usize, count: &mut usize, found: &mut Option<&'a Path>, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|n| *n <= 100_000).ok_or_else(|| error("layer count limit"))?;
        if layer.id == id {
            let path = match &layer.content {
                LayerContent::Shape(shape) => Some(&shape.path),
                _ => layer.vector_mask.as_ref().map(|mask| &mask.path),
            };
            if found.is_some() {
                return Err(error("ambiguous layer path identity"));
            }
            *found = path;
        }
        if let Some(children) = layer.children() {
            layer_path(children, id, depth + 1, count, found, control)?;
        }
    }
    Ok(())
}
/// Borrowed equivalent of resolve_path_params; no path copy before admission.
fn resolve<'a>(
    doc: &'a Document,
    active: Option<LayerId>,
    id: Option<photocraft_doc::item_identity::PathId>,
    name: Option<&str>,
    control: &Interrupt<'_>,
) -> Result<&'a Path> {
    if let Some(id) = id {
        for path in &doc.paths {
            poll(control)?;
            if path.id == id {
                return Ok(&path.path);
            }
        }
        return Err(error("no saved path with pathId"));
    }
    let layer = || -> Result<&'a Path> {
        let id = active.ok_or_else(|| error("active layer has no shape path or vector mask"))?;
        let mut found = None;
        layer_path(&doc.layers, id, 0, &mut 0, &mut found, control)?;
        found.ok_or_else(|| error("active layer has no shape path or vector mask"))
    };
    if name == Some("layer") {
        return layer();
    }
    if super::is_work(name) {
        return match &doc.work_path {
            Some(path) => Ok(path),
            None => layer().map_err(|e| if matches!(e, EngineError::Cancelled) { EngineError::Cancelled } else { error("no work path") }),
        };
    }
    for path in &doc.paths {
        poll(control)?;
        if path.name == name.unwrap_or_default() {
            return Ok(&path.path);
        }
    }
    Err(error("no saved path with name"))
}
fn copy_path(path: &Path, control: &Interrupt<'_>) -> Result<Path> {
    let mut subpaths = Vec::new();
    subpaths.try_reserve_exact(path.subpaths.len()).map_err(|_| error("component allocation"))?;
    if subpaths.capacity() != path.subpaths.len() {
        return Err(error("component capacity"));
    }
    for component in &path.subpaths {
        poll(control)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(component.knots.len()).map_err(|_| error("knot allocation"))?;
        if knots.capacity() != component.knots.len() {
            return Err(error("knot capacity"));
        }
        for chunk in component.knots.chunks(256) {
            poll(control)?;
            knots.extend_from_slice(chunk);
        }
        subpaths.push(Subpath { closed: component.closed, knots, op: component.op });
    }
    Ok(Path { subpaths, fill_rule: path.fill_rule, inverted: path.inverted })
}
fn prepare(doc: &DocumentSnapshot, path: &Path, options: Options, control: &Interrupt<'_>) -> Result<photocraft_vector::VectorPreparationLimits> {
    poll(control)?;
    doc.validate_resident()?;
    doc.validate_xcf_state_with(control).map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })?;
    let limits = photocraft_vector::VectorPreparationLimits::default();
    if path.subpaths.len() > limits.max_components {
        return Err(error("path component limit"));
    }
    let mut knots = 0usize;
    let mut geometry = path.subpaths.capacity().checked_mul(std::mem::size_of::<Subpath>()).ok_or_else(|| error("geometry overflow"))?;
    for component in &path.subpaths {
        poll(control)?;
        knots = knots.checked_add(component.knots.len()).filter(|n| *n <= limits.max_edges).ok_or_else(|| error("path knot limit"))?;
        geometry = geometry
            .checked_add(component.knots.capacity().checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).ok_or_else(|| error("geometry overflow"))?)
            .ok_or_else(|| error("geometry overflow"))?;
    }
    if geometry > limits.max_bytes / 2 {
        return Err(error("path copy exceeds bounded geometry budget"));
    }
    let width = i32::try_from(doc.size.width).map_err(|_| error("canvas width exceeds coordinates"))?;
    let height = i32::try_from(doc.size.height).map_err(|_| error("canvas height exceeds coordinates"))?;
    if width <= 0 || height <= 0 {
        return Err(error("empty canvas"));
    }
    let area = photocraft_geom::Rect::new(0, 0, width, height);
    let pixels = area.size().area();
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control)
        .map_err(|e| if control.cancelled() { EngineError::Cancelled } else { error(e) })? as u64;
    let raster = doc.resident_raster_footprint_with(control)?;
    let coverage =
        checked_region_byte_len(area, PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F32, false)).map_err(raster_error)? as u64;
    let sigma = (options.feather.max(0.0) * 0.5).min(100_000.0);
    let support = (3.0f64 * f64::from(sigma)).ceil() as u64;
    let kernel =
        support.checked_mul(2).and_then(|n| n.checked_add(1)).and_then(|n| n.checked_mul(64)).ok_or_else(|| error("feather kernel budget overflow"))?;
    let ts = i64::from(photocraft_geom::TILE_SIZE);
    let tx = (i64::from(area.x1) - 1).div_euclid(ts) - i64::from(area.x0).div_euclid(ts) + 1;
    let ty = (i64::from(area.y1) - 1).div_euclid(ts) - i64::from(area.y0).div_euclid(ts) + 1;
    let tiles = u64::try_from(tx).ok().and_then(|x| x.checked_mul(u64::try_from(ty).ok()?)).ok_or_else(|| error("tile count overflow"))?;
    let output = tiles
        .checked_mul((photocraft_geom::TILE_SIZE as u64) * (photocraft_geom::TILE_SIZE as u64) + 512)
        .ok_or_else(|| error("output tile budget overflow"))?;
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.retained_bytes))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(coverage.checked_mul(12)?))
        .and_then(|n| n.checked_add(output.checked_mul(3)?))
        .and_then(|n| n.checked_add(limits.max_bytes as u64))
        .and_then(|n| n.checked_add(kernel))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("model/geometry/coverage/feather staging limit"))?;
    let _ = peak;
    let feather_work = if options.feather > 0.0 {
        pixels
            .checked_mul(support.checked_mul(4).and_then(|n| n.checked_add(64)).ok_or_else(|| error("feather work overflow"))?)
            .ok_or_else(|| error("feather work overflow"))?
    } else {
        0
    };
    let other = pixels
        .checked_mul(64)
        .and_then(|n| n.checked_add(feather_work))
        .and_then(|n| n.checked_add(output.checked_mul(4)?))
        .ok_or_else(|| error("selection work overflow"))?;
    let remaining = (1u64 << 32).checked_sub(other).ok_or_else(|| error("selection work limit"))?;
    poll(control)?;
    Ok(photocraft_vector::VectorPreparationLimits { max_work: limits.max_work.min(remaining / 2), ..limits })
}
fn raster_error(e: RasterInputError) -> EngineError {
    raster(e)
}
fn buffer(length: usize, value: f32, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    let mut values = Vec::new();
    values.try_reserve_exact(length).map_err(|_| error("coverage allocation"))?;
    if values.capacity() != length {
        return Err(error("coverage capacity"));
    }
    while values.len() < length {
        poll(control)?;
        values.resize(values.len().saturating_add(8192).min(length), value);
    }
    Ok(values)
}
fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
fn read_previous(surface: &Surface, area: photocraft_geom::Rect, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(error("wide native selection cannot enter legacy f32 combination"));
    }
    let mut values = buffer(usize::try_from(area.size().area()).map_err(|_| error("canvas size"))?, 0.0, control)?;
    let mut raw = [0u8; 40];
    for y in area.y0..area.y1 {
        poll(control)?;
        for x in area.x0..area.x1 {
            let index =
                (y as usize).checked_mul(area.width() as usize).and_then(|n| n.checked_add(x as usize)).ok_or_else(|| error("selection index overflow"))?;
            if index % 256 == 0 {
                poll(control)?;
            }
            let pixel = raw.get_mut(..surface.format().bytes_per_pixel()).ok_or_else(|| error("unsupported selection pixel size"))?;
            surface.try_read_pixel_bytes(x, y, pixel).map_err(raster)?;
            let value = photocraft_color::read_sample(pixel, surface.format().sample, 0);
            if !value.is_finite() {
                return Err(error("nonfinite old selection coverage"));
            }
            *values.get_mut(index).ok_or_else(|| error("selection length mismatch"))? = value;
        }
    }
    Ok(values)
}
fn combine(old: Option<&Surface>, new: &[f32], area: photocraft_geom::Rect, mode: SelectionMode, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    let previous = if mode == SelectionMode::Replace { None } else { old.map(|surface| read_previous(surface, area, control)).transpose()? };
    let mut out = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[0]).map_err(raster)?;
    let mut any = false;
    for tile in area.tiles() {
        poll(control)?;
        let r = tile.rect().intersect(&area);
        let length = checked_region_byte_len(r, PixelFormat::GRAY8).map_err(raster)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| error("quantized coverage allocation"))?;
        if bytes.capacity() != length {
            return Err(error("quantized coverage capacity"));
        }
        while bytes.len() < length {
            poll(control)?;
            bytes.resize(bytes.len().saturating_add(8192).min(length), 0);
        }
        let mut cursor = 0usize;
        for y in r.y0..r.y1 {
            poll(control)?;
            for x in r.x0..r.x1 {
                let index =
                    (y as usize).checked_mul(area.width() as usize).and_then(|n| n.checked_add(x as usize)).ok_or_else(|| error("coverage index overflow"))?;
                let value = *new.get(index).ok_or_else(|| error("coverage length mismatch"))?;
                if !value.is_finite() {
                    return Err(error("nonfinite rasterized coverage"));
                }
                let old = previous.as_ref().and_then(|v| v.get(index)).copied().unwrap_or(0.0);
                let quantized = f32::from(q(value)) / 255.0;
                let value = match mode {
                    SelectionMode::Replace => value,
                    SelectionMode::Add => old.max(quantized),
                    SelectionMode::Subtract => (old - quantized).max(0.0),
                    SelectionMode::Intersect => old.min(quantized),
                };
                let byte = q(value);
                any |= byte != 0;
                *bytes.get_mut(cursor).ok_or_else(|| error("output coverage length mismatch"))? = byte;
                cursor += 1;
            }
        }
        out.try_write_interleaved_with(r, &bytes, control).map_err(raster)?;
    }
    out.try_prune_with(control).map_err(raster)?;
    Ok(any.then_some(out))
}
fn equal(a: Option<&Surface>, b: Option<&Surface>, control: &Interrupt<'_>) -> Result<bool> {
    match (a, b) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) => {
            if a.format() != b.format() || a.default_bytes() != b.default_bytes() || a.tile_count() != b.tile_count() {
                return Ok(false);
            }
            for ((ac, at), (bc, bt)) in a.tiles().zip(b.tiles()) {
                poll(control)?;
                if ac != bc || at.bytes().len() != bt.bytes().len() {
                    return Ok(false);
                }
                for (a, b) in at.bytes().chunks(8192).zip(bt.bytes().chunks(8192)) {
                    poll(control)?;
                    if a != b {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
pub(super) fn path_to_selection(session: &mut Session, p: &Value) -> Result<Value> {
    let options = context(session, p, false)?;
    let id = super::saved_path_id(p)?;
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(|s| {
            let mut v = String::new();
            v.try_reserve_exact(s.len()).map_err(|_| error("selector allocation"))?;
            v.push_str(s);
            Ok::<_, EngineError>(v)
        })
        .transpose()?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let active = state.active_layer;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    let work = baseline.clone();
    crate::jobs::run(
        session,
        "Make Selection",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Make Selection", |control| {
                let path = resolve(&work, active, id, name.as_deref(), control)?;
                let limits = prepare(&work, path, options, control)?;
                let path = copy_path(path, control)?;
                let area = work.bounds();
                let mut empty = true;
                for component in &path.subpaths {
                    poll(control)?;
                    if !component.knots.is_empty() {
                        empty = false;
                        break;
                    }
                }
                let mut mask = if empty {
                    buffer(usize::try_from(area.size().area()).map_err(|_| error("canvas size"))?, if path.inverted { 1.0 } else { 0.0 }, control)?
                } else {
                    let compiled = photocraft_vector::CompiledVectorMask::try_new_with_interrupt(
                        &VectorMask { path, enabled: true, linked: true, density: 1.0, feather: 0.0 },
                        limits,
                        control,
                    )
                    .map_err(raster)?;
                    compiled.try_render_with_interrupt(area, limits, control).map_err(raster)?
                };
                if !options.aa {
                    for part in mask.chunks_mut(8192) {
                        poll(control)?;
                        for value in part {
                            *value = if *value >= 0.5 { 1.0 } else { 0.0 };
                        }
                    }
                }
                if options.feather > 0.0 {
                    mask = selection::try_feather_with_interrupt(&mask, area.width() as usize, area.height() as usize, options.feather, control)
                        .map_err(raster)?;
                }
                let proposed = combine(work.selection.as_ref(), &mask, area, options.mode, control)?;
                let unchanged = equal(work.selection.as_ref(), proposed.as_ref(), control)?;
                poll(control)?;
                Ok((proposed, unchanged, ctx.clone()))
            })
        },
        move |session, (proposed, unchanged, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.floating.is_some()
                || current.coalesce.is_some()
                || current.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
                || session.command_has_job_on(current.doc.id)
            {
                return Err(error("target changed during path selection"));
            }
            if unchanged {
                return ctx.stage(0.95, 1.0, "Make Selection", |control| {
                    poll(control)?;
                    Ok(json!({"selected":proposed.is_some()}))
                });
            }
            ctx.stage(0.95, 1.0, "Make Selection", |control| {
                poll(control)?;
                let selected = session.edit_with_interrupt("Make Selection", control, move |doc, _| {
                    poll(control)?;
                    doc.selection = proposed;
                    Ok(doc.selection.is_some())
                })?;
                Ok(json!({"selected":selected}))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_gray8_combination_matches_legacy_each_mode_and_default() {
        let area = photocraft_geom::Rect::new(0, 0, 4, 1);
        let mut old = Surface::new(PixelFormat::GRAY8);
        old.try_write_interleaved(area, &[0, 64, 128, 255]).unwrap();
        let new = [0.49 / 255.0, 0.5 / 255.0, 0.501, 1.0];
        for mode in [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect] {
            let result = combine(Some(&old), &new, area, mode, &Interrupt::NONE).unwrap();
            assert_eq!(result, selection::combine(Some(&old), &new, area, mode));
            assert_eq!(combine(None, &new, area, mode, &Interrupt::NONE).unwrap(), selection::combine(None, &new, area, mode));
        }
    }
    #[test]
    fn bounded_rasterizer_matches_fill_and_empty_fill_is_not_empty_mask() {
        let area = photocraft_geom::Rect::new(0, 0, 8, 8);
        let path = Path::new(vec![Subpath::polygon(&[(0.25, 0.5), (6.5, 1.0), (3.5, 7.0)])]);
        let expected = photocraft_vector::fill_rasterizer(&path, photocraft_vector::DEFAULT_TOLERANCE).render(area);
        let limits = photocraft_vector::VectorPreparationLimits::default();
        let compiled = photocraft_vector::CompiledVectorMask::try_new_with_interrupt(
            &VectorMask { path, enabled: true, linked: true, density: 1.0, feather: 0.0 },
            limits,
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(compiled.try_render_with_interrupt(area, limits, &Interrupt::NONE).unwrap(), expected);
        for inverted in [false, true] {
            let path = Path { inverted, ..Default::default() };
            assert_eq!(
                buffer(64, if inverted { 1.0 } else { 0.0 }, &Interrupt::NONE).unwrap(),
                photocraft_vector::fill_rasterizer(&path, photocraft_vector::DEFAULT_TOLERANCE).render(area)
            );
        }
    }
    #[test]
    fn identical_path_selection_keeps_actual_revision_and_real_redo() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2})).unwrap();
        session
            .edit("work path", |d, _| {
                d.work_path = Some(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)])]));
                Ok(())
            })
            .unwrap();
        path_to_selection(&mut session, &Value::Null).unwrap();
        session
            .edit("redo seed", |d, _| {
                d.name = "redo seed".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let revision = session.active().unwrap().revision;
        let original = session.active().unwrap().doc.clone();
        path_to_selection(&mut session, &Value::Null).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(original.shares_state(&session.active().unwrap().doc));
        assert!(session.redo());
    }
    #[test]
    fn wide_combinations_refuse_and_cancellation_keeps_source() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
        session
            .edit("work path", |d, _| {
                d.work_path = Some(Path::default());
                d.selection = Some(Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F64, false)));
                Ok(())
            })
            .unwrap();
        assert!(context(&session, &json!({"mode":"add"}), false).is_err());
        assert!(context(&session, &json!({"mode":"replace"}), false).is_ok());
        let original = session.active().unwrap().doc.clone();
        let cancel = || true;
        assert!(matches!(combine(None, &[1.0], original.bounds(), SelectionMode::Replace, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        assert!(original.shares_state(&session.active().unwrap().doc));
    }
}
