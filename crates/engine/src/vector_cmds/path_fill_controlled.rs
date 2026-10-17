//! Controlled ordinary path fill, preserving the legacy coverage/blend equation.
use super::{bad, blend_from_str, is_work, layer_id, saved_path_id};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{BlendMode, Color, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, Path, Subpath, VectorMask};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use photocraft_vector::{CompiledVectorMask, VectorPreparationLimits};
use serde_json::Value;
const CMD: &str = "path.fill";
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(e: impl std::fmt::Display, ctl: &Interrupt<'_>) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(CMD, e.to_string()) }
}
fn decimal(v: &Value, nonzero: bool) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(CMD, "canonical captured IDs required"))?;
    let n = s.parse::<u64>().map_err(|_| bad(CMD, "captured ID overflow"))?;
    if nonzero && n == 0 {
        return Err(bad(CMD, "zero captured ID"));
    }
    Ok(n)
}
#[derive(Clone, Copy)]
struct Options {
    layer: LayerId,
    src: [f32; 4],
    opacity: f32,
    mode: BlendMode,
    feather: f32,
    aa: bool,
}
fn number(p: &Value, key: &str, default: f64) -> Result<f64> {
    p.get(key).map(|v| v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(CMD, format!("finite {key} required")))).transpose().map(|v| v.unwrap_or(default))
}
fn request(s: &Session, p: &Value, normalize: bool) -> Result<Options> {
    if !p.is_null() {
        let map = p.as_object().filter(|m| m.len() <= 10).ok_or_else(|| bad(CMD, "bounded fill object required"))?;
        if map.keys().any(|k| {
            !matches!(k.as_str(), "name" | "pathId" | "layer" | "color" | "opacity" | "mode" | "feather" | "antiAlias" | "documentId" | "expectedRevision")
        }) {
            return Err(bad(CMD, "unknown fill parameter"));
        }
    }
    if let Some(v) = p.get("name")
        && !v.as_str().is_some_and(|s| s.len() <= 2048 && !s.contains('\0'))
    {
        return Err(bad(CMD, "bounded path name required"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let captured = match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if decimal(id, true)? == st.doc.id.0 && decimal(rev, false)? == st.revision => true,
        _ => return Err(bad(CMD, "captured fill target changed or incomplete")),
    };
    if st.revision == u64::MAX
        || st.coalesce.is_some()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || s.has_jobs()
        || s.command_has_pending_job()
        || s.command_has_job_on(st.doc.id)
        || st.doc.accounted().is_some()
        || (st.floating.is_some() && (!normalize || captured || st.doc.xcf_state.is_some()))
    {
        return Err(bad(CMD, "finish target preview/gesture/job"));
    }
    let layer = layer_id(s, p)?;
    let color = if let Some(v) = p.get("color") {
        match v {
            Value::String(text) if text.len() <= 9 => super::color(v).ok_or_else(|| bad(CMD, "invalid fill hex"))?,
            Value::Array(a) if (3..=4).contains(&a.len()) => {
                let mut c = [0.0, 0.0, 0.0, 1.0];
                for (dst, v) in c.iter_mut().zip(a) {
                    *dst = v.as_f64().filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX)).ok_or_else(|| bad(CMD, "finite color components required"))?
                        as f32;
                }
                Color::rgba(c[0], c[1], c[2], c[3])
            }
            _ => return Err(bad(CMD, "bounded fill color required")),
        }
    } else {
        let fg = s.tools.foreground;
        Color::rgba(fg[0], fg[1], fg[2], fg[3])
    };
    let rgb = color.to_rgb();
    let src = [rgb[0], rgb[1], rgb[2], color.alpha];
    if src.iter().any(|v| !v.is_finite()) {
        return Err(bad(CMD, "nonfinite current foreground"));
    }
    let opacity = (number(p, "opacity", 100.0)? / 100.0).clamp(0.0, 1.0) as f32;
    let radius = number(p, "feather", 0.0)?;
    if !(0.0..=200_000.0).contains(&radius) {
        return Err(bad(CMD, "feather bound 0..200000"));
    }
    let feather = radius as f32;
    let mode = if let Some(v) = p.get("mode") {
        v.as_str().filter(|v| v.len() <= 64).and_then(blend_from_str).ok_or_else(|| bad(CMD, "unknown fill blend mode"))?
    } else {
        BlendMode::Normal
    };
    let aa = p.get("antiAlias").map(|v| v.as_bool().ok_or_else(|| bad(CMD, "antiAlias must be boolean"))).transpose()?.unwrap_or(true);
    target_policy(&st.doc, layer, &Interrupt::NONE)?;
    path(s, p, &Interrupt::NONE)?;
    Ok(Options { layer, src, opacity, mode, feather, aa })
}
fn target_policy(doc: &Document, id: LayerId, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    photocraft_io::retained_native_model_bytes(doc, photocraft_io::NativeSaveLimits::default().preparation, ctl).map_err(|e| error(e, ctl))?;
    if doc.xcf_state.is_some()
        || matches!(doc.mode, photocraft_color::ColorMode::Indexed | photocraft_color::ColorMode::Bitmap | photocraft_color::ColorMode::Duotone)
        || doc.has_backed_rasters().map_err(|e| error(e, ctl))?
    {
        return Err(bad(CMD, "source/indexed/backed path-fill interpretation is unsupported"));
    }
    fn census(layers: &[photocraft_doc::Layer], id: LayerId, depth: usize, count: &mut usize, matches: &mut usize, ctl: &Interrupt<'_>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(bad(CMD, "fill hierarchy depth"));
        }
        for layer in layers {
            poll(ctl)?;
            *count = count.checked_add(1).filter(|v| *v <= 10000).ok_or_else(|| bad(CMD, "fill layer count"))?;
            if layer.id == id {
                *matches += 1;
            }
            if let LayerContent::Group(group) = &layer.content {
                census(&group.children, id, depth + 1, count, matches, ctl)?;
            }
        }
        Ok(())
    }
    let (mut count, mut matches) = (0, 0);
    census(&doc.layers, id, 0, &mut count, &mut matches, ctl)?;
    if matches != 1 {
        return Err(bad(CMD, "missing or ambiguous fill layer"));
    }
    crate::commands::check_pixels_unlocked(doc, id)?;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if layer.indexed_source.is_some() || layer.video.is_some() {
        return Err(bad(CMD, "source pixel carrier fill is unsupported"));
    }
    let surface = layer.surface().ok_or_else(|| bad(CMD, "Fill Path needs a pixel layer"))?;
    if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(bad(CMD, "authoritative U32/F64 fill blend arithmetic is unsupported"));
    }
    if surface.format().channels() > 5 {
        return Err(bad(CMD, "fill channel count"));
    }
    poll(ctl)
}
fn limit_selection(selection: Option<&Surface>, area: Rect, coverage: &mut [f32], ctl: &Interrupt<'_>) -> Result<()> {
    let Some(selection) = selection else { return Ok(()) };
    let format = selection.format();
    if matches!(format.sample, SampleType::U32 | SampleType::F64) || format.mode != photocraft_color::ColorMode::Grayscale || format.alpha {
        return Err(bad(CMD, "selected fill coverage requires ordinary Gray native context"));
    }
    let width = area.width() as usize;
    let bpp = format.bytes_per_pixel();
    let mut raw = [0u8; 40];
    let raw = raw.get_mut(..bpp).ok_or_else(|| bad(CMD, "selection sample size"))?;
    for y in area.y0..area.y1 {
        poll(ctl)?;
        for x in area.x0..area.x1 {
            selection.try_read_pixel_bytes(x, y, raw).map_err(|e| error(e, ctl))?;
            let value = photocraft_color::read_sample(raw, format.sample, 0);
            if !value.is_finite() {
                return Err(bad(CMD, "nonfinite selected fill coverage"));
            }
            let offset = usize::try_from(i64::from(y) - i64::from(area.y0))
                .ok()
                .and_then(|v| v.checked_mul(width))
                .and_then(|v| usize::try_from(i64::from(x) - i64::from(area.x0)).ok().and_then(|x| v.checked_add(x)))
                .ok_or_else(|| bad(CMD, "selection coverage offset"))?;
            *coverage.get_mut(offset).ok_or_else(|| bad(CMD, "selection coverage shape"))? *= value;
        }
    }
    poll(ctl)
}
pub(super) fn path<'a>(s: &'a Session, p: &Value, ctl: &Interrupt<'_>) -> Result<&'a Path> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.paths.len() > 10000 {
        return Err(bad(CMD, "saved path count"));
    }
    if let Some(id) = saved_path_id(p)? {
        let mut found = None;
        for saved in &st.doc.paths {
            poll(ctl)?;
            if saved.id == id && found.replace(&saved.path).is_some() {
                return Err(bad(CMD, "ambiguous path identity"));
            }
        }
        return found.ok_or_else(|| bad(CMD, "no saved path with pathId"));
    }
    let name = p.get("name").and_then(Value::as_str);
    if name != Some("layer")
        && is_work(name)
        && let Some(path) = &st.doc.work_path
    {
        return Ok(path);
    }
    if name == Some("layer") || is_work(name) {
        let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or_else(|| bad(CMD, "active layer has no path"))?;
        return match &layer.content {
            LayerContent::Shape(sh) => Ok(&sh.path),
            _ => layer.vector_mask.as_ref().map(|v| &v.path).ok_or_else(|| bad(CMD, "active layer has no vector path")),
        };
    }
    let mut found = None;
    for saved in &st.doc.paths {
        poll(ctl)?;
        if Some(saved.name.as_str()) == name && found.replace(&saved.path).is_some() {
            return Err(bad(CMD, "ambiguous saved path name"));
        }
    }
    found.ok_or_else(|| bad(CMD, "no saved path name"))
}
fn path_bytes(path: &Path, ctl: &Interrupt<'_>) -> Result<u64> {
    if path.subpaths.len() > 10000 {
        return Err(bad(CMD, "subpath count"));
    }
    let mut count = 0usize;
    for sub in &path.subpaths {
        poll(ctl)?;
        count = count.checked_add(sub.knots.len()).filter(|n| *n <= 100000).ok_or_else(|| bad(CMD, "knot count"))?;
        for knot in &sub.knots {
            poll(ctl)?;
            for point in [knot.anchor, knot.in_ctrl, knot.out_ctrl] {
                if !point.x.is_finite() || !point.y.is_finite() || point.x.abs() > 1e6 || point.y.abs() > 1e6 {
                    return Err(bad(CMD, "path coordinate bound"));
                }
            }
        }
    }
    (path.subpaths.capacity() as u64)
        .checked_mul(std::mem::size_of::<Subpath>() as u64)
        .and_then(|v| {
            path.subpaths.iter().try_fold(v, |v, s| v.checked_add((s.knots.capacity() as u64).checked_mul(std::mem::size_of::<photocraft_doc::Knot>() as u64)?))
        })
        .filter(|v| *v <= 64 << 20)
        .ok_or_else(|| bad(CMD, "path retained capacity"))
}
fn copy_path(path: &Path, ctl: &Interrupt<'_>) -> Result<Path> {
    let mut subs = Vec::new();
    subs.try_reserve_exact(path.subpaths.len()).map_err(|_| bad(CMD, "subpath allocation"))?;
    for sub in &path.subpaths {
        poll(ctl)?;
        let mut knots = Vec::new();
        knots.try_reserve_exact(sub.knots.len()).map_err(|_| bad(CMD, "knot allocation"))?;
        for batch in sub.knots.chunks(256) {
            poll(ctl)?;
            knots.extend_from_slice(batch);
        }
        subs.push(Subpath { closed: sub.closed, knots, op: sub.op });
    }
    let copy = Path { subpaths: subs, fill_rule: path.fill_rule, inverted: path.inverted };
    path_bytes(&copy, ctl)?;
    Ok(copy)
}
fn admit(doc: &DocumentSnapshot, path: &Path, o: Options, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    doc.validate_resident()?;
    target_policy(doc, o.layer, ctl)?;
    let path_bytes = path_bytes(path, ctl)?;
    let prep = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, prep, ctl).map_err(|e| error(e, ctl))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, prep, ctl).map_err(|e| error(e, ctl))? as u64;
    let raster = doc.resident_raster_footprint_with(ctl)?;
    let area = doc.bounds();
    let pixels = u64::from(area.width()).checked_mul(u64::from(area.height())).ok_or_else(|| bad(CMD, "fill count"))?;
    let tiles = if area.is_empty() {
        0
    } else {
        let span = |a: i32, b: i32| {
            u64::try_from((i64::from(b) - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(a).div_euclid(i64::from(TILE_SIZE)) + 1)
                .map_err(|_| bad(CMD, "tile span"))
        };
        span(area.x0, area.x1)?.checked_mul(span(area.y0, area.y1)?).ok_or_else(|| bad(CMD, "tile count"))?
    };
    let surface = doc.layer(o.layer).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(o.layer))?;
    let tilebytes =
        photocraft_raster::checked_region_byte_len(photocraft_geom::TileCoord::new(0, 0).rect(), surface.format()).map_err(|e| error(e, ctl))? as u64;
    if doc.selection.as_ref().is_some_and(|s| {
        matches!(s.format().sample, SampleType::U32 | SampleType::F64) || s.format().mode != photocraft_color::ColorMode::Grayscale || s.format().alpha
    }) {
        return Err(bad(CMD, "selected fill coverage requires ordinary Gray native context"));
    }
    let scratch = pixels
        .checked_mul(if o.feather > 0.0 { 56 } else { 8 })
        .and_then(|v| v.checked_add((f64::from(o.feather) * 3.0).ceil() as u64 * 256))
        .and_then(|v| v.checked_add(u64::from(area.width().max(area.height())) * 64))
        .and_then(|v| v.checked_add(65536))
        .ok_or_else(|| bad(CMD, "coverage/kernel scratch overflow"))?;
    let total = model
        .checked_mul(4)
        .and_then(|v| v.checked_add(payload))
        .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|v| v.checked_add(path_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(128 << 20))
        .and_then(|v| v.checked_add(scratch))
        .and_then(|v| v.checked_add(tiles.checked_mul(tilebytes.checked_add(1024)?)?))
        .and_then(|v| v.checked_add(tilebytes.checked_mul(2)?));
    let work = pixels.checked_mul(if o.feather > 0.0 { 2048 } else { 128 }).and_then(|v| v.checked_add(model.checked_mul(8)?));
    if total.is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES) || work.is_none_or(|v| v > 4_000_000_000) {
        return Err(bad(CMD, "fill retained/output/kernel/work limit"));
    }
    poll(ctl)
}
fn paint(surface: &mut Surface, area: Rect, cov: &[f32], o: Options, lock: bool, ctl: &Interrupt<'_>) -> Result<()> {
    let fmt = surface.format();
    let ch = fmt.channels();
    let bpp = fmt.bytes_per_pixel();
    let width = area.width() as usize;
    let expected = width.checked_mul(area.height() as usize).ok_or_else(|| bad(CMD, "coverage dimensions"))?;
    if cov.len() != expected {
        return Err(bad(CMD, "coverage length"));
    }
    for tile in area.tiles() {
        poll(ctl)?;
        let part = tile.rect().intersect(&area);
        let length = photocraft_raster::checked_region_byte_len(part, fmt).map_err(|e| error(e, ctl))?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length).map_err(|_| bad(CMD, "fill tile allocation"))?;
        if raw.capacity() > length.saturating_mul(2) {
            return Err(bad(CMD, "fill tile capacity"));
        }
        raw.resize(length, 0);
        surface.try_read_interleaved_into_with(part, &mut raw, ctl).map_err(|e| error(e, ctl))?;
        let mut offset = 0usize;
        for y in part.y0..part.y1 {
            poll(ctl)?;
            for x in part.x0..part.x1 {
                let ci = usize::try_from(i64::from(y) - i64::from(area.y0))
                    .ok()
                    .and_then(|v| v.checked_mul(width))
                    .and_then(|v| usize::try_from(i64::from(x) - i64::from(area.x0)).ok().and_then(|x| v.checked_add(x)))
                    .ok_or_else(|| bad(CMD, "coverage index"))?;
                let coverage = *cov.get(ci).filter(|v| v.is_finite()).ok_or_else(|| bad(CMD, "invalid coverage"))?;
                let end = offset.checked_add(bpp).ok_or_else(|| bad(CMD, "tile offset"))?;
                let bytes = raw.get_mut(offset..end).ok_or_else(|| bad(CMD, "tile shape"))?;
                if coverage > 0.0 {
                    let mut values = [0.0f32; 5];
                    let values = values.get_mut(..ch).ok_or_else(|| bad(CMD, "sample components"))?;
                    for (index, value) in values.iter_mut().enumerate() {
                        *value = photocraft_color::read_sample(bytes, fmt.sample, index);
                        if !value.is_finite() {
                            return Err(bad(CMD, "nonfinite destination sample"));
                        }
                    }
                    let dst = photocraft_raster::to_rgba(&fmt, values);
                    let mut out = photocraft_compose::psblend::composite(o.mode, dst, [o.src[0], o.src[1], o.src[2], o.src[3] * coverage], o.opacity);
                    if lock {
                        out[3] = dst[3];
                    }
                    photocraft_raster::from_rgba_into(&fmt, out, values);
                    for (index, value) in values.iter().copied().enumerate() {
                        if !value.is_finite() {
                            return Err(bad(CMD, "nonfinite fill result"));
                        }
                        photocraft_color::write_sample(bytes, fmt.sample, index, value);
                    }
                }
                offset = end;
            }
        }
        surface.try_write_interleaved_with(part, &raw, ctl).map_err(|e| error(e, ctl))?;
    }
    surface.try_prune_with(ctl).map_err(|e| error(e, ctl))?;
    poll(ctl)
}
fn stage(doc: &DocumentSnapshot, path: &Path, o: Options, ctl: &Interrupt<'_>) -> Result<Option<Document>> {
    admit(doc, path, o, ctl)?;
    // Empty fill paths hide everything; empty vector masks reveal everything.
    if path.is_empty() && !path.inverted {
        return Ok(None);
    }
    // Preparation and coverage each reserve 2e9 work; the command's remaining
    // kernel/write/metadata admission reserves at most 4e9, cumulatively 8e9.
    let limits = VectorPreparationLimits { max_work: 2_000_000_000, ..VectorPreparationLimits::default() };
    let mut vector_mask = VectorMask::new(copy_path(path, ctl)?);
    // An inverted empty fill is full coverage, unlike the inverted-empty
    // vector-mask convention. The disabled mask's declared constant-one
    // geometry implements that fill case without fabricating source pixels.
    if path.is_empty() {
        vector_mask.enabled = false;
    }
    let mask = CompiledVectorMask::try_new_with_interrupt(&vector_mask, limits, ctl).map_err(|e| error(e, ctl))?;
    let pad = (o.feather.ceil() as i32).checked_mul(3).and_then(|v| v.checked_add(1)).ok_or_else(|| bad(CMD, "feather padding"))?;
    let area = mask.pixel_bounds().map_or(doc.bounds(), |b| b.inflate(pad).intersect(&doc.bounds()));
    if area.is_empty() {
        return Ok(None);
    }
    // Admission above retains 128MiB for preparation and rendering, each with
    // the existing 64MiB ceiling. Rendering also owns row/sort scratch, not
    // just the output pixels and retained prepared edges.
    let mut coverage = mask.try_render_with_interrupt(area, limits, ctl).map_err(|e| error(e, ctl))?;
    if !o.aa {
        for batch in coverage.chunks_mut(256) {
            poll(ctl)?;
            for v in batch {
                *v = if *v >= 0.5 { 1.0 } else { 0.0 };
            }
        }
    }
    if o.feather > 0.0 {
        coverage = photocraft_algo::selection::try_feather_with_interrupt(&coverage, area.width() as usize, area.height() as usize, o.feather, ctl)
            .map_err(|e| error(e, ctl))?;
    }
    limit_selection(doc.selection.as_ref(), area, &mut coverage, ctl)?;
    poll(ctl)?;
    let mut result = (**doc).clone();
    let lock = result.effective_locks(o.layer).transparency;
    let surface = result.layer_mut(o.layer).and_then(|l| l.surface_mut()).ok_or(EngineError::NoLayer(o.layer))?;
    paint(surface, area, &coverage, o, lock, ctl)?;
    let before = doc.layer(o.layer).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(o.layer))?;
    if &*surface == before {
        return Ok(None);
    }
    poll(ctl)?;
    Ok(Some(result))
}
pub(super) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    let o = request(s, p, true)?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    admit(&state.doc, path(s, p, &Interrupt::NONE)?, o, &Interrupt::NONE)
}
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let options = request(s, p, false)?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let borrowed = path(s, p, &Interrupt::NONE)?;
    admit(&state.doc, borrowed, options, &Interrupt::NONE)?;
    let selected = copy_path(borrowed, &Interrupt::NONE)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let weak = baseline.downgrade();
    let work = baseline.clone();
    crate::jobs::run(
        s,
        "Fill Path",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Fill Path", |ctl| {
                let doc = stage(&work, &selected, options, ctl)?;
                Ok((doc, ctx.clone()))
            })
        },
        move |s, (document, ctx)| {
            let current = s.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.active_layer != active
                || current.floating.is_some()
                || current.coalesce.is_some()
                || s.coalesce_request.is_some()
                || s.color_restrict.is_some()
                || s.command_has_job_on(current.doc.id)
            {
                return Err(bad(CMD, "fill target/source changed during preparation"));
            }
            target_policy(&current.doc, options.layer, &Interrupt::NONE)?;
            ctx.stage(0.95, 1.0, "Fill Path", |ctl| {
                poll(ctl)?;
                if let Some(document) = document {
                    s.edit_with_interrupt("Fill Path", ctl, move |doc, _| {
                        poll(ctl)?;
                        *doc = document;
                        Ok(())
                    })?;
                }
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    use photocraft_doc::{Layer, Size};
    use serde_json::json;
    fn setup(depth: SampleType) -> Session {
        let mut doc = Document::new("Fill", Size::new(7, 5), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("Pixel", PixelFormat::new(ColorMode::Rgb, depth, true));
        layer.surface_mut().unwrap().write_region(doc.bounds(), &[0.2, 0.3, 0.4, 0.7].repeat(35));
        doc.layers.push(layer);
        doc.work_path = Some(Path { subpaths: vec![Subpath::polygon(&[(1.0, 1.0), (5.0, 1.0), (4.0, 4.0)])], ..Path::default() });
        let mut s = Session::new();
        s.add_document(doc, None).unwrap();
        s
    }
    #[test]
    fn tiled_controlled_fill_matches_old_coverage_and_blend_words() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for feather in [0.0, 1.0] {
                let s = setup(depth);
                let state = s.active().unwrap();
                let id = state.doc.layers[0].id;
                let o = Options { layer: id, src: [0.8, 0.2, 0.6, 0.9], opacity: 0.65, mode: BlendMode::Normal, feather, aa: true };
                let path = state.doc.work_path.as_ref().unwrap();
                let mut expected = state.doc.layers[0].surface().unwrap().clone();
                let r = photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE);
                let area = r.pixel_bounds().map_or(state.doc.bounds(), |b| b.inflate((feather.ceil() as i32) * 3 + 1).intersect(&state.doc.bounds()));
                let mut cov = r.render(area);
                if feather > 0.0 {
                    cov = photocraft_algo::selection::feather(&cov, area.width() as usize, area.height() as usize, feather);
                }
                super::super::paint_coverage(&mut expected, area, &cov, o.src, o.opacity, o.mode, false);
                let actual = stage(&state.doc, path, o, &Interrupt::NONE).unwrap().unwrap();
                assert_eq!(actual.layer(id).unwrap().surface().unwrap(), &expected);
            }
        }
    }
    #[test]
    fn wide_lock_stale_cancel_and_empty_noop_do_not_publish() {
        let mut wide = setup(SampleType::F64);
        let original = wide.active().unwrap().doc.clone();
        assert!(run(&mut wide, &Value::Null).is_err());
        assert!(wide.active().unwrap().doc.shares_state(&original));
        let mut s = setup(SampleType::U16);
        let p = json!({"documentId":"01","expectedRevision":"0"});
        assert!(dispatch_preflight(&s, &p, false).is_err());
        let o = request(&s, &Value::Null, false).unwrap();
        let snapshot = s.active().unwrap().doc.clone();
        assert!(matches!(stage(&snapshot, snapshot.work_path.as_ref().unwrap(), o, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        s.edit("Empty path", |doc, _| {
            doc.work_path = Some(Path::default());
            Ok(())
        })
        .unwrap();
        s.edit("Redo seed", |doc, _| {
            doc.name.push_str(" changed");
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let revision = s.active().unwrap().revision;
        run(&mut s, &Value::Null).unwrap();
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo());
        s.edit("Lock", |doc, _| {
            doc.layers[0].locks.pixels = true;
            Ok(())
        })
        .unwrap();
        assert!(dispatch_preflight(&s, &Value::Null, false).is_err());
    }
    #[test]
    fn upstream_soft_selection_multiplies_fill_coverage_after_feather() {
        let mut s = setup(SampleType::U16);
        s.edit("Owned soft selection", |doc, _| {
            doc.selection = Some(Surface::try_with_default_bytes(photocraft_color::PixelFormat::GRAY8, &[128]).unwrap());
            Ok(())
        })
        .unwrap();
        let state = s.active().unwrap();
        let id = state.doc.layers[0].id;
        let path = state.doc.work_path.as_ref().unwrap();
        let o = Options { layer: id, src: [0.8, 0.2, 0.6, 0.9], opacity: 0.65, mode: BlendMode::Normal, feather: 1.0, aa: true };
        let r = photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE);
        let area = r.pixel_bounds().map_or(state.doc.bounds(), |b| b.inflate(4).intersect(&state.doc.bounds()));
        let cov = r.render(area);
        let mut cov = photocraft_algo::selection::feather(&cov, area.width() as usize, area.height() as usize, 1.0);
        let mask = photocraft_algo::selection::mask_from_surface(state.doc.selection.as_ref(), area);
        for (c, m) in cov.iter_mut().zip(mask) {
            *c *= m;
        }
        let mut expected = state.doc.layers[0].surface().unwrap().clone();
        super::super::paint_coverage(&mut expected, area, &cov, o.src, o.opacity, o.mode, false);
        assert_eq!(stage(&state.doc, path, o, &Interrupt::NONE).unwrap().unwrap().layer(id).unwrap().surface(), Some(&expected));
    }
}
