//! Frozen path stroke using the existing controlled flatten/brush equations.
use crate::{EngineError, Result, Session};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Path, item_identity::ItemRef};
use photocraft_geom::Rect;
use photocraft_paint::{BrushSettings, DualBrush, GrayTile, Pattern, StoredTile, StrokePoint, Texture, TipShape};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const ID: &str = "path.stroke";
fn error(message: impl std::fmt::Display) -> EngineError {
    super::bad(ID, message.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
#[derive(Clone, Copy)]
struct Request {
    target: LayerId,
    size: Option<f32>,
    hardness: Option<f32>,
    opacity: Option<f32>,
    color: Option<[f32; 4]>,
    tool: u8,
}
fn request(session: &Session, params: &Value) -> Result<Request> {
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 10
                && object.keys().all(|key| {
                    matches!(key.as_str(), "name" | "pathId" | "layer" | "tool" | "size" | "hardness" | "opacity" | "color" | "documentId" | "expectedRevision")
                })
        })
    {
        return Err(error("expected bounded path-stroke parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.paths.len() > 10_000 {
        return Err(error("saved path count limit"));
    }
    if params.get("name").is_some_and(|value| !value.as_str().is_some_and(|value| value.len() <= 4096 && !value.contains('\0'))) {
        return Err(error("path name must be bounded UTF8 without NUL"));
    }
    super::saved_path_id(params)?;
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(state.doc.id.0) && state.doc.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error("captured document/revision changed or incomplete")),
    }
    let target = match params.get("layer") {
        None => state.active_layer.ok_or_else(|| error("no active pixel layer"))?,
        Some(value) => LayerId(value.as_u64().or_else(|| decimal(value)).filter(|id| *id != 0).ok_or_else(|| error("invalid target layer identity"))?),
    };
    let number = |key: &str, percent: bool| -> Result<Option<f32>> {
        params
            .get(key)
            .map(|value| {
                let value = value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("stroke numbers must be finite"))?;
                let value = if percent { value / 100. } else { value } as f32;
                if !value.is_finite() {
                    return Err(error("stroke number exceeds native f32 precision"));
                }
                Ok(value)
            })
            .transpose()
    };
    let tool = match params.get("tool").map(|value| value.as_str().ok_or_else(|| error("tool must be a known string"))).transpose()?.unwrap_or("brush") {
        "brush" => 0,
        "pencil" => 1,
        "eraser" => 2,
        _ => return Err(error("tool must be brush, pencil or eraser")),
    };
    let color = params
        .get("color")
        .map(|value| -> Result<[f32; 4]> {
            let valid = match value {
                Value::String(text) => text.len() <= 16,
                Value::Array(values) => {
                    (3..=4).contains(&values.len())
                        && values.iter().all(|value| value.as_f64().is_some_and(|value| value.is_finite() && (value as f32).is_finite()))
                }
                _ => false,
            };
            if !valid {
                return Err(error("color must be bounded finite RGB(A) or hex"));
            }
            let color = super::color(value).ok_or_else(|| error("invalid stroke color"))?;
            let rgb = color.to_rgb();
            Ok([rgb[0], rgb[1], rgb[2], color.alpha])
        })
        .transpose()?;
    Ok(Request { target, size: number("size", false)?, hardness: number("hardness", false)?, opacity: number("opacity", true)?, color, tool })
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(error("idle uncoalesced path-stroke context required"));
    }
    crate::brush_cmds::validate_brush(&session.tools.brush, ID)?;
    brush_bytes(&session.tools.brush)?;
    Ok(())
}
fn tile_bytes(tile: &GrayTile) -> Result<u64> {
    let count = u64::from(tile.width)
        .checked_mul(u64::from(tile.height))
        .filter(|count| *count > 0 && *count == tile.data.len() as u64)
        .ok_or_else(|| error("invalid sampled brush bitmap"))?;
    count
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(32))
        .filter(|bytes| *bytes <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("sampled brush capacity limit"))
}
// Stored payloads stay owned by their upstream cache; only the bounded key is copied.
fn stored_bytes(tile: &StoredTile) -> Result<u64> {
    if tile.key.len() > 4096 || tile.width == 0 || tile.height == 0 {
        return Err(error("invalid stored brush metadata"));
    }
    let full = match &tile.full {
        Some(full) => tile_bytes(full)?,
        None => 0,
    };
    tile_bytes(&tile.preview)?
        .checked_add(full)
        .and_then(|n| n.checked_add(tile.key.capacity() as u64))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("stored brush capacity limit"))
}
fn stored_copy(tile: &StoredTile, control: &Interrupt<'_>) -> Result<StoredTile> {
    poll(control)?;
    stored_bytes(tile)?;
    let mut key = String::new();
    key.try_reserve_exact(tile.key.len()).map_err(|_| error("stored brush key allocation denied"))?;
    key.push_str(&tile.key);
    poll(control)?;
    Ok(StoredTile { key, width: tile.width, height: tile.height, preview: tile.preview.clone(), full: tile.full.clone() })
}
fn tip_bytes(tip: &TipShape) -> Result<u64> {
    match tip {
        TipShape::Round => Ok(0),
        TipShape::Sampled(tile) => tile_bytes(tile),
        TipShape::Stored(tile) => stored_bytes(tile),
    }
}
pub(super) fn brush_bytes(brush: &BrushSettings) -> Result<u64> {
    let pattern = match &brush.texture.pattern {
        Pattern::Tile(tile) => tile_bytes(tile)?,
        Pattern::Stored(tile) => stored_bytes(tile)?,
        Pattern::Procedural { size, .. } => u64::from((*size).clamp(8, 1024)).pow(2) * 4,
    };
    tip_bytes(&brush.tip)?
        .checked_add(tip_bytes(&brush.dual_brush.tip)?)
        .and_then(|bytes| bytes.checked_add(pattern))
        .and_then(|bytes| bytes.checked_mul(8))
        .filter(|bytes| *bytes <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("brush/tip/texture residency limit"))
}
fn tile_copy(tile: &GrayTile, control: &Interrupt<'_>) -> Result<GrayTile> {
    poll(control)?;
    tile_bytes(tile)?;
    // Upstream immutable shared samples are retained, not reconstructed.
    let data = std::sync::Arc::clone(&tile.data);
    poll(control)?;
    Ok(GrayTile { width: tile.width, height: tile.height, data })
}

fn tip_copy(tip: &TipShape, control: &Interrupt<'_>) -> Result<TipShape> {
    match tip {
        TipShape::Round => Ok(TipShape::Round),
        TipShape::Sampled(tile) => Ok(TipShape::Sampled(tile_copy(tile, control)?)),
        TipShape::Stored(tile) => Ok(TipShape::Stored(stored_copy(tile, control)?)),
    }
}
fn brush_copy(b: &BrushSettings, control: &Interrupt<'_>) -> Result<BrushSettings> {
    poll(control)?;
    brush_bytes(b)?;
    let pattern = match &b.texture.pattern {
        Pattern::Procedural { style, size, seed } => Pattern::Procedural { style: *style, size: *size, seed: *seed },
        Pattern::Tile(tile) => Pattern::Tile(tile_copy(tile, control)?),
        Pattern::Stored(tile) => Pattern::Stored(stored_copy(tile, control)?),
    };
    // Only bitmap payloads allocate; scalar/dynamic sections have no containers.
    Ok(BrushSettings {
        size: b.size,
        hardness: b.hardness,
        spacing: b.spacing,
        spacing_enabled: b.spacing_enabled,
        opacity: b.opacity,
        flow: b.flow,
        pressure_size: b.pressure_size,
        pressure_opacity: b.pressure_opacity,
        color: b.color,
        color_cmyk: b.color_cmyk,
        square: b.square,
        background: b.background,
        erase: b.erase,
        mode: b.mode,
        tip: tip_copy(&b.tip, control)?,
        angle: b.angle,
        roundness: b.roundness,
        flip_x: b.flip_x,
        flip_y: b.flip_y,
        aliased: b.aliased,
        shape_dynamics: b.shape_dynamics.clone(),
        scattering: b.scattering.clone(),
        texture: Texture {
            enabled: b.texture.enabled,
            pattern,
            invert: b.texture.invert,
            scale: b.texture.scale,
            brightness: b.texture.brightness,
            contrast: b.texture.contrast,
            each_tip: b.texture.each_tip,
            mode: b.texture.mode,
            depth: b.texture.depth,
            depth_jitter: b.texture.depth_jitter,
        },
        dual_brush: DualBrush {
            enabled: b.dual_brush.enabled,
            mode: b.dual_brush.mode,
            tip: tip_copy(&b.dual_brush.tip, control)?,
            size: b.dual_brush.size,
            hardness: b.dual_brush.hardness,
            roundness: b.dual_brush.roundness,
            angle: b.dual_brush.angle,
            spacing: b.dual_brush.spacing,
            scatter: b.dual_brush.scatter,
            both_axes: b.dual_brush.both_axes,
            count: b.dual_brush.count,
            flip: b.dual_brush.flip,
        },
        color_dynamics: b.color_dynamics.clone(),
        transfer: b.transfer.clone(),
        pose: b.pose.clone(),
        noise: b.noise,
        wet_edges: b.wet_edges,
        build_up: b.build_up,
        build_up_rate: b.build_up_rate,
        smoothing: b.smoothing.clone(),
        protect_texture: b.protect_texture,
        locks: b.locks.clone(),
        mixer: b.mixer.clone(),
        seed: b.seed,
    })
}
/// A bounded owned dialog/job witness, not a brush preset/resource permission.
/// The UI may lower the cap; sampled tip/texture copies are fallible and polled.
pub fn brush_snapshot(brush: &BrushSettings, max_bytes: u64, control: &Interrupt<'_>) -> Result<BrushSettings> {
    poll(control)?;
    crate::brush_cmds::validate_brush(brush, ID)?;
    if max_bytes > MAX_CHECKED_RASTER_BYTES || brush_bytes(brush)? > max_bytes {
        return Err(error("captured brush residency limit"));
    }
    brush_copy(brush, control)
}
fn layer_path(document: &Document, active: Option<LayerId>) -> Result<&Path> {
    let layer = document.layer(active.ok_or_else(|| error("no active path layer"))?).ok_or_else(|| error("missing active path layer"))?;
    match &layer.content {
        LayerContent::Shape(shape) => Ok(&shape.path),
        _ => layer.vector_mask.as_ref().map(|mask| &mask.path).ok_or_else(|| error("active layer has no shape path or vector mask")),
    }
}
fn path<'a>(document: &'a Document, active: Option<LayerId>, params: &Value) -> Result<&'a Path> {
    if let Some(id) = super::saved_path_id(params)? {
        return document.paths.iter().find(|path| path.id == id).map(|path| &path.path).ok_or_else(|| error("no saved path with pathId"));
    }
    let name = params.get("name").and_then(Value::as_str);
    if name == Some("layer") {
        return layer_path(document, active);
    }
    if super::is_work(name) {
        return document.work_path.as_ref().map(Ok).unwrap_or_else(|| layer_path(document, active));
    }
    document.paths.iter().find(|path| Some(path.name.as_str()) == name).map(|path| &path.path).ok_or_else(|| error("no saved path with name"))
}
fn source_policy(document: &Document, active: Option<LayerId>, target: LayerId, params: &Value, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    crate::commands::check_pixels_unlocked(document, target)?;
    let layer = document.layer(target).ok_or(EngineError::NoLayer(target))?;
    let surface = layer.surface().ok_or_else(|| error("Stroke Path needs a pixel layer"))?;
    if !matches!(&layer.content, LayerContent::Raster(_))
        || matches!(surface.format().sample, SampleType::U32 | SampleType::F64)
        || surface.format().mode == ColorMode::Indexed
        || document.mode == ColorMode::Indexed
        || layer.indexed_source.is_some()
        || layer.import_compositing.is_some()
        || layer.video.is_some()
    {
        return Err(error("wide/indexed/backed/source-semantic target stroke arithmetic is not qualified"));
    }
    if let Some(selection) = &document.selection
        && matches!(selection.format().sample, SampleType::U32 | SampleType::F64)
    {
        return Err(error("wide selection stroke coverage is not qualified"));
    }
    if let Some(state) = &document.xcf_state {
        if state.items.len() > 10_000 || state.source_items.len() > 10_000 || state.filter_stacks.len() > 10_000 {
            return Err(error("source relation count limit"));
        }
        let saved = super::saved_path_id(params)?.or_else(|| {
            let name = params.get("name").and_then(Value::as_str);
            if !super::is_work(name) && name != Some("layer") {
                document.paths.iter().find(|path| Some(path.name.as_str()) == name).map(|path| path.id)
            } else {
                None
            }
        });
        let from_layer = params.get("name").and_then(Value::as_str) == Some("layer")
            || (saved.is_none() && super::is_work(params.get("name").and_then(Value::as_str)) && document.work_path.is_none());
        for source in &state.source_items {
            poll(control)?;
            if source.owner == ItemRef::Layer(target)
                || (from_layer && matches!(source.owner,ItemRef::Layer(id)if Some(id)==active))
                || matches!(source.owner,ItemRef::Path(id)if Some(id)==saved)
            {
                return Err(error("source-owned path/target editing is not qualified"));
            }
        }
        for stack in &state.filter_stacks {
            poll(control)?;
            if stack.owner == ItemRef::Layer(target) {
                return Err(error("filtered target stroke is not qualified"));
            }
        }
    }
    poll(control)
}
fn retained(snapshot: &crate::DocumentSnapshot, brush: &BrushSettings, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    snapshot.validate_resident()?;
    let native = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, native, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, native, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    model
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(payload))
        .and_then(|bytes| bytes.checked_add(raster.retained_bytes))
        .and_then(|bytes| bytes.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|bytes| bytes.checked_add(brush_bytes(brush).ok()?))
        .filter(|bytes| *bytes < MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| error("retained source/brush residency limit"))
}
fn same_pixels(first: &Surface, second: &Surface, control: &Interrupt<'_>) -> Result<bool> {
    poll(control)?;
    if first.format() != second.format() || first.default_pixel() != second.default_pixel() || first.tile_count() != second.tile_count() {
        return Ok(false);
    }
    for (coordinate, tile) in first.tiles() {
        poll(control)?;
        let Some(other) = second.tile(*coordinate) else {
            return Ok(false);
        };
        if tile.bytes() != other.bytes() {
            return Ok(false);
        }
    }
    poll(control)?;
    Ok(true)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let request = request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    // Params contain only bounded selectors/scalars; the path stays borrowed from
    // the frozen snapshot until controlled flattening preadmits its own output.
    let params = params.clone();
    let foreground = session.tools.foreground;
    let background = session.tools.background;
    // Bounded frontend snapshot copying has no worker token yet. Admit known
    // source/brush capacities before fallibly copying its sampled bitmaps.
    retained(&baseline, &session.tools.brush, &Interrupt::NONE)?;
    let original_brush = brush_copy(&session.tools.brush, &Interrupt::NONE)?;
    let mut brush = brush_copy(&original_brush, &Interrupt::NONE)?;
    if let Some(size) = request.size {
        brush.size = size;
    }
    if let Some(hardness) = request.hardness {
        brush.hardness = hardness;
    }
    if let Some(opacity) = request.opacity {
        brush.opacity = opacity;
    }
    brush.color = request.color.unwrap_or(foreground);
    brush.erase = request.tool == 2;
    brush.smoothing.amount = 0.;
    if request.tool == 1 {
        brush.hardness = 1.;
    }
    crate::brush_cmds::validate_brush(&brush, ID)?;
    let work = baseline.clone();
    crate::jobs::run(
        session,
        "Stroke Path",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Stroke Path", |control| {
                poll(control)?;
                work.validate_resident()?;
                source_policy(&work, active, request.target, &params, control)?;
                let retained = retained(&work, &brush, control)?;
                let remaining = MAX_CHECKED_RASTER_BYTES.checked_sub(retained).ok_or_else(|| error("source/brush/stroke residency limit"))?;
                let mut vector_limits = photocraft_vector::VectorPreparationLimits::default();
                vector_limits.max_bytes = vector_limits.max_bytes.min(usize::try_from(remaining).unwrap_or(usize::MAX));
                let lines = photocraft_vector::flatten_path_with_interrupt(path(&work, active, &params)?, 0.1, vector_limits, control)
                    .map_err(|e| controlled(control, e))?;
                let engine_work = (vector_limits.max_edges as u64)
                    .checked_mul(256)
                    .and_then(|work| work.checked_add(retained.checked_mul(2)?))
                    .ok_or_else(|| error("stroke preparation work reservation"))?;
                let stroke_work = 8_000_000_000u64
                    .checked_sub(vector_limits.max_work)
                    .and_then(|work| work.checked_sub(engine_work))
                    .ok_or_else(|| error("flatten/engine work reservation"))?;
                let mut nonempty = 0u64;
                for line in &lines {
                    poll(control)?;
                    if !line.pts.is_empty() {
                        nonempty = nonempty.checked_add(1).ok_or_else(|| error("subpath count overflow"))?;
                    }
                }
                let per_line_work = stroke_work / nonempty.max(1);
                let lock = crate::xcf_item_locks::transparency_locked(&work, request.target)?;
                if brush.erase && lock {
                    brush.erase = false;
                    brush.color = background;
                }
                let mut points_bytes = 0u64;
                for line in &lines {
                    poll(control)?;
                    points_bytes = points_bytes
                        .checked_add((line.pts.capacity() as u64).checked_mul(17).ok_or_else(|| error("polyline byte count"))?)
                        .and_then(|bytes| bytes.checked_add((line.pts.len() as u64 + 1).checked_mul(std::mem::size_of::<StrokePoint>() as u64)?))
                        .ok_or_else(|| error("point/line allocation overflow"))?;
                }
                let resident_bytes = remaining.checked_sub(points_bytes).ok_or_else(|| error("point/brush/output residency limit"))?;
                let mut surface = work.layer(request.target).and_then(Layer::surface).ok_or_else(|| error("missing target pixels"))?.clone();
                let mut damage = Rect::EMPTY;
                for line in &lines {
                    poll(control)?;
                    let count =
                        line.pts.len().checked_add(usize::from(line.closed && !line.pts.is_empty())).ok_or_else(|| error("closed point count overflow"))?;
                    let mut points = Vec::new();
                    points.try_reserve_exact(count).map_err(|_| error("stroke point allocation denied"))?;
                    if points.capacity() != count {
                        return Err(error("stroke point capacity differs from admitted count"));
                    }
                    for &(x, y) in &line.pts {
                        poll(control)?;
                        points.push(StrokePoint::new(x, y, 1.));
                    }
                    if line.closed
                        && let Some(first) = points.first().copied()
                    {
                        points.push(first);
                    }
                    if points.is_empty() {
                        continue;
                    }
                    let region = photocraft_paint::render_stroke_with_interrupt(
                        &mut surface,
                        &brush,
                        &points,
                        work.selection.as_ref(),
                        lock,
                        1.,
                        photocraft_paint::StrokeLimits { resident_bytes, work_units: per_line_work },
                        control,
                    )
                    .map_err(|e| controlled(control, e))?;
                    damage = if damage.is_empty() { region } else { damage.union(&region) };
                }
                poll(control)?;
                let width = i64::from(damage.x1) - i64::from(damage.x0);
                let height = i64::from(damage.y1) - i64::from(damage.y0);
                if width < 0 || height < 0 || width > i64::from(i32::MAX) || height > i64::from(i32::MAX) {
                    return Err(error("damage rectangle exceeds native bounds"));
                }
                let response = json!({"damage":[damage.x0,damage.y0,width,height]});
                Ok((surface, damage, response, ctx.clone()))
            })
        },
        move |session, (surface, damage, response, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !state.doc.shares_state(&baseline)
                || state.active_layer != active
                || session.tools.brush != original_brush
                || session.tools.foreground != foreground
                || session.tools.background != background
            {
                return Err(error("target/path/brush changed during stroke preparation"));
            }
            ctx.stage(0.95, 1., "Stroke Path", |control| {
                poll(control)?;
                if damage.is_empty()
                    || same_pixels(baseline.layer(request.target).and_then(Layer::surface).ok_or_else(|| error("missing original target"))?, &surface, control)?
                {
                    return Ok(response);
                }
                session.edit_with_interrupt("Stroke Path", control, move |document, _| {
                    *document.layer_mut(request.target).and_then(Layer::surface_mut).ok_or_else(|| error("stale pixel target"))? = surface;
                    Ok(())
                })?;
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::{Size, Subpath};
    fn fixture(sample: SampleType) -> Session {
        let mut document = Document::with_background("path stroke", Size::new(32, 32), ColorMode::Rgb, sample, Color::WHITE);
        document.work_path = Some(Path::new(vec![Subpath::polygon(&[(5., 5.), (20., 5.), (20., 20.), (5., 20.)])]));
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session.tools.brush.size = 3.;
        session.tools.brush.spacing = 0.25;
        session.tools.brush.color = [0.2, 0.4, 0.7, 1.];
        session.tools.foreground = [0.2, 0.4, 0.7, 1.];
        session.tools.background = [0.8, 0.3, 0.1, 1.];
        session
    }
    #[test]
    fn controlled_stroke_matches_actual_old_sampling_and_paint_across_depths() {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for tool in ["brush", "pencil", "eraser"] {
                let mut session = fixture(sample);
                let before = session.active().unwrap().doc.clone();
                let target = session.active().unwrap().active_layer.unwrap();
                let history = session.active().unwrap().history.past_len();
                let mut expected = before.layer(target).unwrap().surface().unwrap().clone();
                let mut brush = session.tools.brush.clone();
                brush.color = session.tools.foreground;
                brush.erase = tool == "eraser";
                brush.smoothing.amount = 0.;
                if tool == "pencil" {
                    brush.hardness = 1.;
                }
                let lock = crate::xcf_item_locks::transparency_locked(&before, target).unwrap();
                if brush.erase && lock {
                    brush.erase = false;
                    brush.color = session.tools.background;
                }
                let mut damage = Rect::EMPTY;
                for line in photocraft_vector::flatten_path(before.work_path.as_ref().unwrap(), 0.1) {
                    let mut points: Vec<StrokePoint> = line.pts.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.)).collect();
                    if line.closed
                        && let Some(first) = points.first().copied()
                    {
                        points.push(first);
                    }
                    if points.is_empty() {
                        continue;
                    }
                    let current = photocraft_paint::apply_stroke(
                        &mut expected,
                        &photocraft_paint::Stroke { brush: brush.clone(), points },
                        before.selection.as_ref(),
                        lock,
                    );
                    damage = if damage.is_empty() { current } else { damage.union(&current) };
                }
                let response = run(&mut session, &json!({"tool":tool})).unwrap();
                assert_eq!(response, json!({"damage":[damage.x0,damage.y0,damage.width(),damage.height()]}));
                let actual = session.active().unwrap().doc.layer(target).unwrap().surface().unwrap();
                assert_eq!(
                    actual.try_read_interleaved_with(damage, &Interrupt::NONE).unwrap(),
                    expected.try_read_interleaved_with(damage, &Interrupt::NONE).unwrap()
                );
                assert_eq!(session.active().unwrap().history.past_len(), history + 1);
                assert_eq!(session.active().unwrap().doc.work_path, before.work_path);
                session.execute("edit.undo", json!({})).unwrap();
                assert_eq!(
                    session
                        .active()
                        .unwrap()
                        .doc
                        .layer(target)
                        .unwrap()
                        .surface()
                        .unwrap()
                        .try_read_interleaved_with(before.bounds(), &Interrupt::NONE)
                        .unwrap(),
                    before.layer(target).unwrap().surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap()
                );
                session.execute("edit.redo", json!({})).unwrap();
                assert_eq!(
                    session.active().unwrap().doc.layer(target).unwrap().surface().unwrap().try_read_interleaved_with(damage, &Interrupt::NONE).unwrap(),
                    expected.try_read_interleaved_with(damage, &Interrupt::NONE).unwrap()
                );
            }
        }
    }
    #[test]
    fn sampled_brush_snapshot_is_fallible_owned_and_bounded() {
        let mut brush = BrushSettings { tip: TipShape::Sampled(GrayTile { width: 2, height: 2, data: vec![0, 1, 32768, 65535].into() }), ..Default::default() };
        brush.texture.pattern = Pattern::Tile(GrayTile { width: 2, height: 2, data: vec![65535, 32768, 1, 0].into() });
        let snapshot = brush_snapshot(&brush, MAX_CHECKED_RASTER_BYTES, &Interrupt::NONE).unwrap();
        assert_eq!(snapshot, brush);
        let TipShape::Sampled(original) = &brush.tip else { panic!("sampled") };
        let TipShape::Sampled(copied) = &snapshot.tip else { panic!("sampled") };
        assert!(std::sync::Arc::ptr_eq(&original.data, &copied.data));
        assert!(brush_snapshot(&brush, 1, &Interrupt::NONE).is_err());
        let cancel = || true;
        assert!(matches!(brush_snapshot(&brush, MAX_CHECKED_RASTER_BYTES, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn hostile_brush_stale_capture_locks_and_wide_target_refuse_without_changes() {
        let mut session = fixture(SampleType::F64);
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let target = state.active_layer.unwrap();
        assert!(source_policy(&before, state.active_layer, target, &Value::Null, &Interrupt::NONE).is_err());
        assert!(state.doc.shares_state(&before));
        session.tools.brush.size = f32::INFINITY;
        assert!(dispatch_preflight(&session, &Value::Null, false).is_err());
        session.tools.brush = BrushSettings::default();
        assert!(
            dispatch_preflight(
                &session,
                &json!({"documentId":before.id.0.to_string(),"expectedRevision":session.active().unwrap().revision.saturating_add(1).to_string()}),
                false
            )
            .is_err()
        );
        let mut session = fixture(SampleType::U16);
        session
            .edit("lock fixture", |document, active| {
                document.layer_mut(active.unwrap()).unwrap().locks.all = true;
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        assert!(run(&mut session, &Value::Null).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
    #[test]
    fn empty_path_noop_preserves_revision_and_real_redo() {
        let mut session = fixture(SampleType::U8);
        session
            .edit("empty", |document, _| {
                document.work_path = Some(Path::default());
                Ok(())
            })
            .unwrap();
        session
            .edit("redo fixture", |document, _| {
                document.name = "changed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_label().unwrap().to_string();
        assert_eq!(run(&mut session, &Value::Null).unwrap(), json!({"damage":[0,0,0,0]}));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
    }
}
