//! Controlled Grow/Similar in the existing narrow color-sampling domain.
//! Canonical imported merged sampling is closed: ordinary Ctx rendering is not
//! evidence for an imported source's rendering/profile interpretation.
use crate::channel_cmds::ChannelView;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::selection_grow_controlled as grow;
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample};
use photocraft_doc::{Document, Layer, LayerId, MAX_GROUP_DEPTH};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(cmd: &str, ctl: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(cmd, &error.to_string()) }
}
#[derive(Clone, Copy)]
struct Request {
    tolerance: f32,
    all: bool,
    contiguous: bool,
    label: &'static str,
}
fn exact(cmd: &str, v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(cmd, "canonical captured identity required"))?;
    s.parse().map_err(|_| bad(cmd, "captured identity range"))
}
fn find_surface<'a>(doc: &'a Document, id: Option<LayerId>, ctl: &Interrupt<'_>) -> Result<Option<&'a Surface>> {
    let Some(id) = id else { return Ok(None) };
    let mut stack: [Option<std::slice::Iter<'a, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *stack.first_mut().ok_or_else(|| bad("select.grow", "hierarchy"))? = Some(doc.layers.iter());
    let (mut depth, mut count) = (0usize, 0usize);
    loop {
        poll(ctl)?;
        let next = stack.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("select.grow", "owner count limit"))?;
        if layer.id == id {
            return Ok(layer.surface());
        }
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad("select.grow", "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad("select.grow", "hierarchy"))? = Some(children.iter());
        }
    }
    Ok(None)
}
fn request(session: &Session, cmd: &str, p: &Value) -> Result<Request> {
    let (contiguous, label) = match cmd {
        "select.grow" => (true, "Grow"),
        "select.similar" => (false, "Similar"),
        _ => return Err(bad(cmd, "unknown Grow/Similar route")),
    };
    if !p.is_null()
        && !p
            .as_object()
            .is_some_and(|o| o.len() <= 4 && o.keys().all(|k| matches!(k.as_str(), "tolerance" | "sampleAllLayers" | "documentId" | "expectedRevision")))
    {
        return Err(bad(cmd, "bounded Grow/Similar parameters required"));
    }
    let tolerance = match p.get("tolerance") {
        None => 32.,
        Some(v) => {
            let value = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "finite tolerance required"))? as f32;
            if !value.is_finite() {
                return Err(bad(cmd, "tolerance is outside f32 range"));
            }
            value
        }
    };
    let all = match p.get("sampleAllLayers") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "sampleAllLayers must be boolean"))?,
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(cmd, id)? == state.doc.id.0 && exact(cmd, rev)? == state.revision => {}
        _ => return Err(bad(cmd, "captured selection target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || state.doc.accounted().is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_job_on(state.doc.id)
        || session.command_has_pending_job()
    {
        return Err(bad(cmd, "selection target must be idle resident state"));
    }
    // Header/owner classification must remain metadata-only. Full canonical
    // validation can hash retained payloads or inspect mask samples, so it runs
    // in admission under the job token, and again in the private final edit.
    crate::selection_sample_policy::metadata_preflight(session, cmd, p, &Interrupt::NONE)?;
    if state.doc.xcf_state.is_some() && (all || find_surface(&state.doc, state.active_layer, &Interrupt::NONE)?.is_none()) {
        return Err(bad(cmd, "canonical imported merged color sampling is unsupported"));
    }
    Ok(Request { tolerance, all, contiguous, label })
}
pub(crate) fn dispatch_preflight(session: &Session, cmd: &str, p: &Value, _background: bool) -> Result<()> {
    request(session, cmd, p).map(|_| ())
}
fn copy_flags(values: &[bool], cmd: &str) -> Result<Vec<bool>> {
    if values.len() > 100000 {
        return Err(bad(cmd, "view flag limit"));
    }
    let mut result = Vec::new();
    result.try_reserve_exact(values.len()).map_err(|_| bad(cmd, "view allocation"))?;
    if result.capacity() > values.len().saturating_add(16) {
        return Err(bad(cmd, "view capacity"));
    }
    result.extend_from_slice(values);
    Ok(result)
}
fn copy_view(view: &ChannelView, cmd: &str) -> Result<ChannelView> {
    Ok(ChannelView {
        target: view.target,
        color_hidden: copy_flags(&view.color_hidden, cmd)?,
        alpha_visible: copy_flags(&view.alpha_visible, cmd)?,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
struct Plan {
    area: Rect,
    req: grow::Requirements,
}
fn admission(doc: &Document, active: Option<LayerId>, r: Request, cmd: &str, ctl: &Interrupt<'_>) -> Result<Plan> {
    poll(ctl)?;
    let width = doc.size.width as usize;
    let height = doc.size.height as usize;
    let req = grow::requirements(width, height, r.contiguous).map_err(|e| error(cmd, ctl, e))?;
    let area = doc.bounds();
    crate::selection_inverse_controlled::selection_write_authority_metadata(doc, ctl)?;
    if let Some(selection) = &doc.selection {
        if selection.format().mode != ColorMode::Grayscale || selection.format().alpha || matches!(selection.format().sample, SampleType::U32 | SampleType::F64)
        {
            return Err(bad(cmd, "narrow Gray coverage required"));
        }
    }
    let direct = if r.all { None } else { find_surface(doc, active, ctl)? };
    if direct.is_none() && doc.xcf_state.is_some() {
        return Err(bad(cmd, "canonical imported merged color sampling is unsupported"));
    }
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, ctl).map_err(|e| error(cmd, ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, ctl).map_err(|e| error(cmd, ctl, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, ctl)?;
    let tiles = u64::from(doc.size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(doc.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| bad(cmd, "output tile count"))?;
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64);
    let (render_extra, render_work) = if direct.is_some() {
        (0, 0)
    } else {
        let render = photocraft_compose::ordinary_render_requirements(doc, area, ctl).map_err(|e| error(cmd, ctl, e))?;
        let extra = (render.scratch_bytes as u64).checked_add(render.preparation_reserve_bytes as u64).ok_or_else(|| bad(cmd, "render resident overflow"))?;
        (extra, render.work_units)
    };
    raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(req.peak_resident_bytes))
        .and_then(|n| n.checked_add(render_extra))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(1024)?)?))
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(3)?))
        .and_then(|n| n.checked_add(1 << 20))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(cmd, "aggregate Grow/Similar resident limit"))?;
    req.work_units
        .checked_add(render_work)
        .and_then(|n| n.checked_add(area.size().area().checked_mul(64)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_mul(8)?)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|n| n.checked_add(raster.retained_bytes.checked_mul(2)?))
        .filter(|n| *n <= 1 << 30)
        .ok_or_else(|| bad(cmd, "aggregate Grow/Similar work limit"))?;
    poll(ctl)?;
    Ok(Plan { area, req })
}
fn floats<T: Clone>(count: usize, zero: T, cmd: &str, ctl: &Interrupt<'_>) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| bad(cmd, "input plane allocation"))?;
    if values.capacity() > count.saturating_add(16) {
        return Err(bad(cmd, "input plane capacity"));
    }
    while values.len() < count {
        poll(ctl)?;
        values.resize(values.len().saturating_add(4096).min(count), zero.clone());
    }
    Ok(values)
}
fn sample(doc: &Document, active: Option<LayerId>, r: Request, plan: &Plan, cmd: &str, ctl: &Interrupt<'_>) -> Result<Vec<[f32; 4]>> {
    let n = (doc.size.width as usize).checked_mul(doc.size.height as usize).ok_or_else(|| bad(cmd, "pixel count"))?;
    let direct = if r.all { None } else { find_surface(doc, active, ctl)? };
    let Some(surface) = direct else {
        if doc.xcf_state.is_some() {
            return Err(bad(cmd, "canonical imported merged color sampling is unsupported"));
        }
        poll(ctl)?;
        let buffer = photocraft_compose::render_with_interrupt(doc, plan.area, ctl).map_err(|e| error(cmd, ctl, e))?;
        poll(ctl)?;
        if buffer.rect != plan.area || buffer.px.len() != n {
            return Err(bad(cmd, "composite shape mismatch"));
        }
        return Ok(buffer.px);
    };
    if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(bad(cmd, "native color sampling unsupported"));
    }
    // Admission validated every retained tile/default width and origin. The same
    // upstream model/depth reader runs in <=256-pixel rows, never a full unchecked
    // input-derived region. No new color-space conversion is substituted.
    surface.try_validate_tile_origins_with(ctl).map_err(|e| error(cmd, ctl, e))?;
    let mut pixels = floats(n, [0.; 4], cmd, ctl)?;
    let width = doc.size.width as usize;
    for y in plan.area.y0..plan.area.y1 {
        poll(ctl)?;
        for x in (plan.area.x0..plan.area.x1).step_by(256) {
            poll(ctl)?;
            let end = x.saturating_add(256).min(plan.area.x1);
            let offset = (y as usize).checked_mul(width).and_then(|n| n.checked_add(x as usize)).ok_or_else(|| bad(cmd, "sample offset"))?;
            let row = pixels.get_mut(offset..offset + (end - x) as usize).ok_or_else(|| bad(cmd, "sample row length"))?;
            surface.read_rgba_into(Rect::new(x, y, end, y + 1), row);
            poll(ctl)?;
        }
    }
    Ok(pixels)
}
fn prepare(doc: &Document, active: Option<LayerId>, r: Request, cmd: &str, ctl: &Interrupt<'_>) -> Result<Option<Surface>> {
    let plan = admission(doc, active, r, cmd, ctl)?;
    // Aggregate admission precedes expensive canonical payload/sample validation;
    // both execute inside the same worker cancellation domain.
    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
    let width = doc.size.width as usize;
    let height = doc.size.height as usize;
    let n = width.checked_mul(height).ok_or_else(|| bad(cmd, "pixel count"))?;
    let pixels = sample(doc, active, r, &plan, cmd, ctl)?;
    let mut mask = floats(n, 0f32, cmd, ctl)?;
    let mut raw = [0u8; 8];
    let mut seed = false;
    if let Some(selection) = &doc.selection {
        let sample = selection.format().sample;
        let word = raw.get_mut(..sample.bytes()).ok_or_else(|| bad(cmd, "coverage word width"))?;
        for y in 0..height {
            poll(ctl)?;
            for x in 0..width {
                if x.is_multiple_of(1024) {
                    poll(ctl)?;
                }
                selection.try_read_pixel_bytes(x as i32, y as i32, word).map_err(|e| error(cmd, ctl, e))?;
                let value = read_sample(word, sample, 0);
                if !value.is_finite() {
                    return Err(bad(cmd, "nonfinite prior coverage"));
                }
                *mask.get_mut(y * width + x).ok_or_else(|| bad(cmd, "coverage offset"))? = value;
                seed |= value >= 0.5;
            }
        }
    }
    if !seed {
        return Err(EngineError::Other("selection is empty".into()));
    }
    let bytes = grow::grow_with_interrupt(
        &pixels,
        &mask,
        width,
        height,
        r.tolerance,
        r.contiguous,
        grow::Limits { resident_bytes: plan.req.peak_resident_bytes, work_units: plan.req.work_units },
        ctl,
    )
    .map_err(|e| error(cmd, ctl, e))?;
    let mut output = Surface::new(PixelFormat::GRAY8);
    let mut tile_bytes = Vec::new();
    let tile_cap = (TILE_SIZE as usize) * (TILE_SIZE as usize);
    tile_bytes.try_reserve_exact(tile_cap).map_err(|_| bad(cmd, "output tile buffer allocation"))?;
    if tile_bytes.capacity() > tile_cap.saturating_add(16) {
        return Err(bad(cmd, "output tile capacity"));
    }
    for coordinate in plan.area.tiles() {
        poll(ctl)?;
        let part = coordinate.rect().intersect(&plan.area);
        tile_bytes.clear();
        for y in part.y0..part.y1 {
            poll(ctl)?;
            let start = (y as usize).checked_mul(width).and_then(|n| n.checked_add(part.x0 as usize)).ok_or_else(|| bad(cmd, "output row offset"))?;
            tile_bytes.extend_from_slice(bytes.get(start..start + part.width() as usize).ok_or_else(|| bad(cmd, "output row length"))?);
        }
        let mut replacement = Surface::new(PixelFormat::GRAY8);
        replacement.try_write_interleaved_with(part, &tile_bytes, ctl).map_err(|e| error(cmd, ctl, e))?;
        poll(ctl)?;
        output.put_tiles(replacement.tiles().map(|(position, tile)| (*position, std::sync::Arc::clone(tile))));
    }
    output.try_prune_with(ctl).map_err(|e| error(cmd, ctl, e))?;
    poll(ctl)?;
    Ok((output.tile_count() > 0).then_some(output))
}
pub(crate) fn run(session: &mut Session, cmd: &str, p: &Value) -> Result<Value> {
    let r = request(session, cmd, p)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let view = copy_view(&state.channel_view, cmd)?;
    let worker = baseline.clone();
    let command = if r.contiguous { "select.grow" } else { "select.similar" };
    crate::jobs::run(
        session,
        r.label,
        true,
        move |ctx| ctx.stage(0., 0.95, r.label, |ctl| Ok((prepare(&worker, active, r, command, ctl)?, ctx.clone()))),
        move |session, (selection, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id
                || state.revision != revision
                || !baseline.downgrade().is_current(&state.doc)
                || state.active_layer != active
                || state.channel_view != view
                || state.floating.is_some()
                || state.coalesce.is_some()
                || session.command_has_job_on(state.doc.id)
                || session.command_has_pending_job()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad(command, "Grow/Similar target changed during preparation"));
            }
            ctx.stage(0.95, 1., r.label, |ctl| {
                poll(ctl)?;
                let response = json!({"selected":selection.is_some()});
                session.edit_with_interrupt(r.label, ctl, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    doc.selection = selection;
                    crate::selection_inverse_controlled::selection_write_authority(doc, ctl)?;
                    poll(ctl)
                })?;
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerContent;
    fn doc() -> Document {
        let mut doc = Document::new("grow", photocraft_geom::Size::new(4, 1), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("pixels", doc.pixel_format());
        layer.surface_mut().unwrap().write_region(doc.bounds(), &[0.2, 0.2, 0.2, 1., 0.2, 0.2, 0.2, 1., 0.9, 0.9, 0.9, 1., 0.2, 0.2, 0.2, 1.]);
        doc.layers.push(layer);
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.try_write_interleaved(Rect::new(0, 0, 1, 1), &[128]).unwrap();
        doc.selection = Some(selection);
        doc
    }
    #[test]
    fn grow_and_similar_publish_one_selection_edit_and_undo_preserves_source_tiles() {
        for command in ["select.grow", "select.similar"] {
            let mut session = Session::new();
            session.add_document(doc(), None).unwrap();
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            let response =
                run(&mut session, command, &json!({"tolerance":0,"documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()})).unwrap();
            assert_eq!(response, json!({"selected":true}));
            assert_eq!(session.active().unwrap().revision, revision + 1);
            let selection = session.active().unwrap().doc.selection.as_ref().unwrap();
            let bytes = selection.try_read_interleaved_with(Rect::new(0, 0, 4, 1), &Interrupt::NONE).unwrap();
            assert_eq!(bytes, if command == "select.grow" { [255, 255, 0, 0] } else { [255, 255, 0, 255] });
            assert_eq!(session.active().unwrap().doc.layers, before.layers);
            assert!(session.undo());
            assert_eq!(session.active().unwrap().doc, before);
        }
    }
    #[test]
    fn bounded_raw_sampling_uses_the_same_upstream_model_depth_reader() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut source = Document::new("sample", photocraft_geom::Size::new(300, 2), mode, depth);
                let mut layer = Layer::raster("raw", source.pixel_format());
                let values = vec![0.25; source.pixel_format().channels() * 600];
                layer.surface_mut().unwrap().write_region(source.bounds(), &values);
                let id = layer.id;
                source.layers.push(layer);
                let mut old = vec![[0.; 4]; 600];
                source.layers[0].surface().unwrap().read_rgba_into(source.bounds(), &mut old);
                let request = Request { tolerance: 32., all: false, contiguous: false, label: "Similar" };
                let plan = admission(&source, Some(id), request, "select.similar", &Interrupt::NONE).unwrap();
                let actual = sample(&source, Some(id), request, &plan, "select.similar", &Interrupt::NONE).unwrap();
                assert_eq!(actual.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>(), old.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn hostile_current_tile_shape_stale_target_and_cancel_preserve_original() {
        let mut source = doc();
        let mut gray = Surface::new(PixelFormat::GRAY8);
        gray.try_write_interleaved(Rect::new(0, 0, 1, 1), &[1]).unwrap();
        source.layers[0].surface_mut().unwrap().put_tiles(gray.tiles().map(|(coordinate, tile)| (*coordinate, std::sync::Arc::clone(tile))));
        let before = source.clone();
        let request = Request { tolerance: 32., all: false, contiguous: true, label: "Grow" };
        assert!(prepare(&source, Some(source.layers[0].id), request, "select.grow", &Interrupt::NONE).is_err());
        assert_eq!(source, before);
        assert!(matches!(prepare(&doc(), None, request, "select.grow", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        let mut session = Session::new();
        session.add_document(doc(), None).unwrap();
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        assert!(run(&mut session, "select.grow", &json!({"documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()})).is_err());
        assert!(before.shares_state(&session.active().unwrap().doc));
        assert_eq!(session.active().unwrap().revision, revision);
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].content =
            LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U32, true)));
        assert!(dispatch_preflight(&session, "select.grow", &Value::Null, false).is_err());
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    #[test]
    fn canonical_direct_header_defers_full_validation_to_cancelled_job() {
        let mut source = Document::new("direct", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        source.layers.push(Layer::raster("pixels", source.pixel_format()));
        let mut session = Session::new();
        session.add_document(source, None).unwrap();
        // A default canonical state is not claimed to be a qualified source;
        // execution still validates it before allocation/publication.
        session.active_mut().unwrap().doc.try_resident_mut().unwrap().xcf_state = Some(Default::default());
        assert!(dispatch_preflight(&session, "select.grow", &Value::Null, true).is_ok());
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let r = Request { tolerance: 32., all: false, contiguous: true, label: "Grow" };
        assert!(matches!(prepare(&state.doc, state.active_layer, r, "select.grow", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(before.shares_state(&session.active().unwrap().doc));
    }
}
