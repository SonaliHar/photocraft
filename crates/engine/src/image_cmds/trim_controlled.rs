//! Controlled Trim: bounded display comparison for ordinary samples, or exact
//! raw alpha occupancy for a plain single raster. No wide color/palette guess.
use crate::{EngineError, Result, Session};
use photocraft_color::{BlendMode, ColorMode, SampleType, try_read_sample_f64};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

const ID: &str = "image.trim";
const WORK_LIMIT: u64 = 8_000_000_000;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Based {
    Transparent,
    TopLeft,
    BottomRight,
}
#[derive(Clone, Copy)]
struct Request {
    based: Based,
    sides: [bool; 4],
}
fn bad(message: &str) -> EngineError {
    super::bad(ID, message)
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { super::bad(ID, error.to_string()) }
}
fn parse(session: &Session, params: &Value) -> Result<Request> {
    let object = params
        .as_object()
        .filter(|object| {
            object.len() <= 7
                && object.keys().all(|key| matches!(key.as_str(), "basedOn" | "top" | "bottom" | "left" | "right" | "documentId" | "expectedRevision"))
        })
        .ok_or_else(|| bad("expected bounded Trim parameters"))?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX {
        return Err(bad("document revision limit"));
    }
    if session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(bad("target document has pending work"));
    }
    match (object.get("documentId"), object.get("expectedRevision")) {
        (None, None) => {}
        (Some(document), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| bad("captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| bad("captured ID exceeds range"))
            };
            let document = exact(document)?;
            if document == 0 || document != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(bad("captured target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || session.has_jobs()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
            {
                return Err(bad("captured Trim target has floating or transient work"));
            }
        }
        _ => return Err(bad("documentId and expectedRevision must be supplied together")),
    }
    let based = match object.get("basedOn") {
        None => Based::Transparent,
        Some(Value::String(name)) => match name.as_str() {
            "transparent" => Based::Transparent,
            "topLeft" => Based::TopLeft,
            "bottomRight" => Based::BottomRight,
            _ => return Err(bad("basedOn must be transparent, topLeft or bottomRight")),
        },
        _ => return Err(bad("basedOn must be a string")),
    };
    let side = |key| -> Result<bool> {
        object.get(key).map(|value| value.as_bool().ok_or_else(|| bad("Trim sides must be boolean"))).transpose().map(|value| value.unwrap_or(true))
    };
    Ok(Request { based, sides: [side("left")?, side("top")?, side("right")?, side("bottom")?] })
}
/// Call before floating normalization/params cloning. No model/pixel scan here;
/// valid legacy no-pair requests retain the existing authorized auto-anchor.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    parse(session, params).map(|_| ())
}

fn layer_census(layers: &[Layer], depth: usize, count: &mut u64, tiles: &mut u64, wide: &mut bool, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("Trim hierarchy depth limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|count| *count <= 100_000).ok_or_else(|| bad("Trim layer count limit"))?;
        let mut visit = |surface: &Surface| -> Result<()> {
            *tiles = tiles.checked_add(surface.tile_count() as u64).ok_or_else(|| bad("Trim tile count overflow"))?;
            *wide |= matches!(surface.format().sample, SampleType::U32 | SampleType::F64);
            Ok(())
        };
        if let Some(mask) = &layer.mask {
            visit(&mask.surface)?;
        }
        if let Some(cache) = &layer.fill_cache {
            visit(&cache.surface)?;
        }
        if let Some(surface) = layer.surface() {
            visit(surface)?;
        }
        if let LayerContent::Group(group) = &layer.content {
            layer_census(&group.children, depth + 1, count, tiles, wide, control)?;
        }
    }
    Ok(())
}
fn single_raw(doc: &Document) -> Option<&Surface> {
    let mut visible = doc.layers.iter().filter(|layer| layer.visible);
    let layer = visible.next()?;
    if visible.next().is_some()
        || layer.blend != BlendMode::Normal
        || layer.opacity != 1.
        || layer.fill_opacity != 1.
        || layer.clipped
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || (layer.effects.enabled && !layer.effects.items.is_empty())
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.advanced != Default::default()
        || layer.import_compositing.is_some()
        || layer.video.is_some()
        || layer.fill_cache.is_some()
    {
        return None;
    }
    match &layer.content {
        LayerContent::Raster(surface) => Some(surface),
        _ => None,
    }
}
fn union_pixel(bounds: &mut Rect, x: i32, y: i32) -> Result<()> {
    let pixel = Rect::new(
        x,
        y,
        x.checked_add(1).ok_or_else(|| bad("Trim right coordinate overflow"))?,
        y.checked_add(1).ok_or_else(|| bad("Trim bottom coordinate overflow"))?,
    );
    *bounds = if bounds.is_empty() { pixel } else { bounds.union(&pixel) };
    Ok(())
}
fn alpha_present(pixel: &[u8], surface: &Surface) -> Result<bool> {
    let format = surface.format();
    if pixel.len() != format.bytes_per_pixel() {
        return Err(bad("Trim native pixel length mismatch"));
    }
    if !format.alpha {
        return Ok(true);
    }
    // U32 is read directly to f64, never through f32; color bytes are untouched.
    let alpha = try_read_sample_f64(pixel, format.sample, format.mode.color_channels()).map_err(|_| bad("Trim native alpha is invalid"))?;
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(bad("Trim alpha must be finite in [0,1]"));
    }
    Ok(alpha > 0.)
}
fn raw_bounds(source: &Surface, extent: Rect, canvas: Rect, control: &Interrupt<'_>) -> Result<Rect> {
    let area = canvas.intersect(&extent);
    if area.is_empty() {
        return Ok(Rect::EMPTY);
    }
    source.try_validate_tile_origins_with(control).map_err(|e| error(control, e))?;
    let default_present = alpha_present(source.default_bytes(), source)?;
    let bpp = source.format().bytes_per_pixel();
    let mut bounds = Rect::EMPTY;
    // Absent tiles contribute only their explicit default alpha; no dense
    // canvas allocation or default-image fabrication. Stored tiles scan rows.
    for coordinate in area.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&area);
        let Some(tile) = source.tile(coordinate) else {
            if default_present {
                bounds = if bounds.is_empty() { part } else { bounds.union(&part) };
            }
            continue;
        };
        let row_bytes = (TILE_SIZE as usize).checked_mul(bpp).ok_or_else(|| bad("Trim native row overflow"))?;
        if tile.bytes().len() != row_bytes.checked_mul(TILE_SIZE as usize).ok_or_else(|| bad("Trim native tile overflow"))? {
            return Err(bad("Trim native tile length mismatch"));
        }
        let origin = coordinate.rect();
        for y in part.y0..part.y1 {
            poll(control)?;
            let row = usize::try_from(i64::from(y) - i64::from(origin.y0))
                .ok()
                .and_then(|n| n.checked_mul(row_bytes))
                .ok_or_else(|| bad("Trim row offset overflow"))?;
            for x in part.x0..part.x1 {
                let offset = usize::try_from(i64::from(x) - i64::from(origin.x0))
                    .ok()
                    .and_then(|n| n.checked_mul(bpp))
                    .and_then(|n| n.checked_add(row))
                    .ok_or_else(|| bad("Trim pixel offset overflow"))?;
                let pixel = tile
                    .bytes()
                    .get(offset..offset.checked_add(bpp).ok_or_else(|| bad("Trim pixel range overflow"))?)
                    .ok_or_else(|| bad("Trim pixel range mismatch"))?;
                if alpha_present(pixel, source)? {
                    union_pixel(&mut bounds, x, y)?;
                }
            }
        }
    }
    poll(control)?;
    Ok(bounds)
}
fn rendered_bounds(doc: &Document, canvas: Rect, based: Based, control: &Interrupt<'_>) -> Result<Rect> {
    let reference = match based {
        Based::Transparent => None,
        Based::TopLeft | Based::BottomRight => {
            let (x, y) = if based == Based::TopLeft {
                (canvas.x0, canvas.y0)
            } else {
                (canvas.x1.checked_sub(1).ok_or_else(|| bad("Trim corner bounds"))?, canvas.y1.checked_sub(1).ok_or_else(|| bad("Trim corner bounds"))?)
            };
            let buffer = photocraft_compose::render_with_interrupt(
                doc,
                Rect::new(x, y, x.checked_add(1).ok_or_else(|| bad("Trim corner bounds"))?, y.checked_add(1).ok_or_else(|| bad("Trim corner bounds"))?),
                control,
            )
            .map_err(|e| error(control, e))?;
            Some(*buffer.px.first().ok_or_else(|| bad("Trim reference pixel unavailable"))?)
        }
    };
    let mut bounds = Rect::EMPTY;
    // Preserve the old f32 premultiplied comparison and threshold exactly for
    // eligible ordinary depths. Buffers are one tile, never the whole canvas.
    for coordinate in canvas.tiles() {
        poll(control)?;
        let part = coordinate.rect().intersect(&canvas);
        let buffer = photocraft_compose::render_with_interrupt(doc, part, control).map_err(|e| error(control, e))?;
        let width = usize::try_from(part.width()).map_err(|_| bad("Trim render width"))?;
        if width == 0 || buffer.px.len() != width.checked_mul(part.height() as usize).ok_or_else(|| bad("Trim render length overflow"))? {
            return Err(bad("Trim render length mismatch"));
        }
        for (index, q) in buffer.px.iter().enumerate() {
            if index.is_multiple_of(width) {
                poll(control)?;
            }
            let [_, _, _, alpha] = *q;
            let differs = match reference {
                None => alpha > 0.,
                Some(reference) => {
                    let [_, _, _, reference_alpha] = reference;
                    q.iter().zip(reference).enumerate().any(|(channel, (value, reference))| {
                        (*value * alpha - reference * reference_alpha).abs() > 0.5 / 255. || (channel == 3 && (alpha - reference_alpha).abs() > 0.5 / 255.)
                    })
                }
            };
            if differs {
                let x = part
                    .x0
                    .checked_add(i32::try_from(index % width).map_err(|_| bad("Trim render column"))?)
                    .ok_or_else(|| bad("Trim render column overflow"))?;
                let y =
                    part.y0.checked_add(i32::try_from(index / width).map_err(|_| bad("Trim render row"))?).ok_or_else(|| bad("Trim render row overflow"))?;
                union_pixel(&mut bounds, x, y)?;
            }
        }
    }
    poll(control)?;
    Ok(bounds)
}
fn find_bounds(doc: &Document, request: Request, control: &Interrupt<'_>) -> Result<Rect> {
    poll(control)?;
    let canvas = doc.bounds();
    if canvas.is_empty() || canvas.x0 != 0 || canvas.y0 != 0 || canvas.x1 > 300_000 || canvas.y1 > 300_000 {
        return Err(bad("invalid Trim canvas"));
    }
    // Existing Crop guard refuses unknown source/filter geometry and validates
    // canonical indexed planes before any native alpha/read/display comparison.
    super::crop_prepare(doc, canvas, true, control)?;
    let mut count = 0;
    let mut tiles = 0;
    let mut wide = matches!(doc.depth, SampleType::U32 | SampleType::F64);
    layer_census(&doc.layers, 0, &mut count, &mut tiles, &mut wide, control)?;
    let indexed = doc.mode == ColorMode::Indexed || doc.xcf_state.as_ref().is_some_and(|state| !state.editable_indexed.is_empty());
    if indexed {
        crate::xcf_indexed_geometry::validate_current_with_interrupt(doc, control)?;
        if request.based != Based::Transparent {
            return Err(bad("indexed color-based Trim comparison is not qualified"));
        }
    }
    let raw = if request.based == Based::Transparent { single_raw(doc) } else { None };
    let pixels = u64::from(canvas.width()).checked_mul(u64::from(canvas.height())).ok_or_else(|| bad("Trim canvas area overflow"))?;
    let tile_visits = super::canvas_tiles(canvas)?;
    let sample_work = if let Some(source) = raw {
        (source.tile_count() as u64).checked_mul((TILE_SIZE as u64).pow(2)).and_then(|n| n.checked_mul(source.format().bytes_per_pixel() as u64))
    } else {
        pixels.checked_mul(count.max(1)).and_then(|n| n.checked_mul(64))
    };
    let work = sample_work
        .and_then(|n| n.checked_add(tile_visits.checked_mul(256)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(1536)?))
        .ok_or_else(|| bad("Trim aggregate work overflow"))?;
    if work > WORK_LIMIT {
        return Err(bad("Trim aggregate read/crop work exceeds budget"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    // Known ordinary renderer preparation ceilings (patterns/effects/vector/
    // feather caches) are held alongside this source, not treated as zero.
    // Inner legacy style allocator overhead/polling remains renderer-owned.
    let scratch = if raw.is_some() { 2u64 << 20 } else { (64 + 64 + 128 + 64 + 2) << 20 };
    if model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(scratch))
        .is_none_or(|n| n > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad("Trim retained/render scratch budget exceeded"));
    }
    let content = match raw {
        Some(source) => {
            let extent = if indexed {
                let state = doc.xcf_state.as_ref().ok_or_else(|| bad("indexed Trim requires current plane metadata"))?;
                let owner = doc.layers.iter().find(|layer| layer.visible).ok_or_else(|| bad("Trim raw owner unavailable"))?.id;
                state.editable_indexed.iter().find(|plane| plane.owner == owner).ok_or_else(|| bad("Trim current indexed owner unavailable"))?.raster.bounds()
            } else {
                if doc.xcf_state.is_some() {
                    return Err(bad("raw Trim source geometry is not qualified"));
                }
                canvas
            };
            raw_bounds(source, extent, canvas, control)?
        }
        None => {
            if wide || indexed {
                return Err(bad("composited wide/indexed Trim comparison is not qualified"));
            }
            rendered_bounds(doc, canvas, request.based, control)?
        }
    };
    if content.is_empty() {
        return Err(EngineError::Other("nothing to trim to".into()));
    }
    let [left, top, right, bottom] = request.sides;
    Ok(Rect::new(
        if left { content.x0 } else { canvas.x0 },
        if top { content.y0 } else { canvas.y0 },
        if right { content.x1 } else { canvas.x1 },
        if bottom { content.y1 } else { canvas.y1 },
    ))
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let request = parse(session, params)?;
    let active = session.active().ok_or(EngineError::NoDocument)?;
    active.doc.validate_resident()?;
    let baseline = active.doc.clone();
    let revision = active.revision;
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Trim",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Trim", |control| {
                let rectangle = find_bounds(&work_base, request, control)?;
                super::crop_prepare(&work_base, rectangle, true, control)?;
                let indexed = super::crop_stage_indexed(&work_base, rectangle, true, control)?;
                poll(control)?;
                let mut document = (*work_base).clone();
                super::crop_doc_with_interrupt(&mut document, rectangle, true, indexed, control)?;
                poll(control)?;
                Ok((document, rectangle, ctx.clone()))
            })
        },
        move |session, (document, rectangle, ctx)| {
            let active = session.active().ok_or(EngineError::NoDocument)?;
            if active.doc.id != baseline.id || active.revision != revision || !active.doc.shares_state(&baseline) {
                return Err(bad("target changed while preparing Trim"));
            }
            ctx.stage(0.95, 1., "Trim", |control| {
                poll(control)?;
                session.edit_with_interrupt("Trim", control, move |current, _| {
                    *current = document;
                    Ok(())
                })?;
                Ok(json!({"x":rectangle.x0,"y":rectangle.y0,"width":rectangle.width(),"height":rectangle.height()}))
            })
        },
    )
}

// Unrun controlled Trim history/raw/default/early-dispatch drafts.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{Layer, PhysicalResolution, PhysicalUnit, Size};
    fn wide() -> Session {
        let mut document = Document::new("Own raw Trim", Size::new(3, 3), ColorMode::Grayscale, SampleType::F64);
        let mut surface = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true), &[0; 16]).unwrap();
        let pixel = [0x7ff8000000000019u64, 1u64].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        surface.try_write_interleaved(Rect::new(1, 1, 2, 2), &pixel).unwrap();
        document.layers.push(Layer::new("Raw", LayerContent::Raster(surface)));
        document.set_physical_resolution(PhysicalResolution { x_dpi: 144.25, y_dpi: 287.75, unit: PhysicalUnit::Millimeter }).unwrap();
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn registered_raw_f64_alpha_keeps_nan_color_and_subnormal_without_f32_narrowing() {
        let mut session = wide();
        let before = session.active().unwrap().doc.clone();
        let result = session.execute(ID, json!({"basedOn":"transparent"})).unwrap();
        assert_eq!(result, json!({"x":1,"y":1,"width":1,"height":1}));
        let document = &session.active().unwrap().doc;
        assert_eq!(document.size, Size::new(1, 1));
        assert_eq!(document.effective_resolution(), before.effective_resolution());
        assert_eq!(
            document.layers.first().unwrap().surface().unwrap().try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(),
            [0x7ff8000000000019u64, 1u64].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()
        );
        let native = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&native).unwrap(), **document);
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
        assert!(session.redo());
    }
    #[test]
    fn explicit_nonzero_default_and_side_flags_do_not_expand_a_dense_canvas_buffer() {
        let source = Surface::try_with_default_bytes(
            PixelFormat::new(ColorMode::Grayscale, SampleType::U32, true),
            &[17u32, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>(),
        )
        .unwrap();
        let canvas = Rect::new(0, 0, 300_000, 1);
        assert_eq!(raw_bounds(&source, canvas, canvas, &Interrupt::NONE).unwrap(), canvas);
        let mut session = wide();
        assert_eq!(session.execute(ID, json!({"left":false,"bottom":false})).unwrap(), json!({"x":0,"y":1,"width":2,"height":2}));
    }
    #[test]
    fn interpreted_wide_color_cancel_and_captured_stale_requests_keep_real_redo() {
        let mut session = wide();
        session
            .edit("Real redo", |document, _| {
                document.name = "redo retained".into();
                Ok(())
            })
            .unwrap();
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        assert!(session.execute(ID, json!({"basedOn":"topLeft"})).is_err());
        assert!(session.execute(ID, json!({"basedOn":"bottomRight"})).is_err());
        assert!(session.execute(ID, json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        assert!(matches!(
            find_bounds(&before, Request { based: Based::Transparent, sides: [true; 4] }, &Interrupt::cancel_only(&|| true)),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc.name, "redo retained");
    }
    #[test]
    fn ordinary_top_left_comparison_retains_premultiplied_tolerance_and_one_undo() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":4,"height":3,"background":"#ffffff"})).unwrap();
        session
            .edit("Own center pixel", |document, active| {
                document.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().try_write_pixel_f64(2, 1, &[1., 0., 0., 1.]).unwrap();
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        assert_eq!(session.execute(ID, json!({"basedOn":"topLeft"})).unwrap(), json!({"x":2,"y":1,"width":1,"height":1}));
        assert!(session.undo());
        assert_eq!(session.active().unwrap().doc, before);
    }
}
