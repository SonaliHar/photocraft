//! Existing Focus Area sampled-owner policy, controlled private preparation.
//! Canonical imported merged appearance is not routed through ordinary render.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::{
    segment::focus::{self, FocusLimits},
    selection::{self as sel, SelectionMode, WandLimits},
};
use photocraft_doc::{Document, LayerId};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const ID: &str = "select.focusArea";
const MAX_WORK: u64 = 8_000_000_000;
fn bad(s: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("{ID}: {s}"))
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn error(c: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if c.cancelled() { EngineError::Cancelled } else { bad(e) }
}
fn decimal(v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("canonical captured target required"))?;
    s.parse().map_err(|_| bad("identity overflow"))
}
#[derive(Clone, Copy)]
struct Request {
    range: f32,
    noise: f32,
    all: bool,
    mode: SelectionMode,
}
fn flag(p: &Value, k: &str, d: bool) -> Result<bool> {
    p.get(k).map_or(Ok(d), |v| v.as_bool().ok_or_else(|| bad("boolean flag required")))
}
fn scalar(p: &Value, k: &str, d: f32) -> Result<f32> {
    let n = p.get(k).map_or(Ok(f64::from(d)), |v| v.as_f64().ok_or_else(|| bad("numeric parameter required")))?;
    if !n.is_finite() || n.abs() > f64::from(f32::MAX) {
        return Err(bad("finite ordinary parameter required"));
    }
    Ok(n as f32)
}
fn request(s: &Session, p: &Value, early: bool) -> Result<Request> {
    if !p.is_null() {
        let o = p
            .as_object()
            .filter(|o| {
                o.len() <= 6 && o.keys().all(|k| matches!(k.as_str(), "range" | "noise" | "sampleAllLayers" | "mode" | "documentId" | "expectedRevision"))
            })
            .ok_or_else(|| bad("bounded Focus Area parameters required"))?;
        let _ = o;
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let captured = match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if st.doc.id.0 != 0 && decimal(id)? == st.doc.id.0 && decimal(rev)? == st.revision => true,
        _ => return Err(bad("captured target changed")),
    };
    if st.revision == u64::MAX
        || st.coalesce.is_some()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || s.command_has_pending_job()
        || s.command_has_job_on(st.doc.id)
        || st.doc.accounted().is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some())
        || st.floating.is_some() && (!early || captured)
    {
        return Err(bad("selection target not idle"));
    }
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(Value::String(n)) if n.len() <= 64 => SelectionMode::parse(if n == "new" { "replace" } else { n }),
        _ => return Err(bad("bounded mode required")),
    };
    Ok(Request {
        range: scalar(p, "range", 0.5)?.clamp(0.0, 1.0),
        noise: scalar(p, "noise", 0.0)?.clamp(0.0, 1.0),
        all: flag(p, "sampleAllLayers", true)?,
        mode,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Layer(LayerId),
    Merged,
}
fn route(s: &Session, r: Request, c: &Interrupt<'_>) -> Result<Route> {
    poll(c)?;
    crate::selection_sample_policy::sampled_pixels(s, ID, r.all, c)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if !r.all {
        if let Some(id) = st.active_layer {
            if let Some(layer) = st.doc.layer(id) {
                if matches!(layer.content, photocraft_doc::LayerContent::BackedRaster(_)) {
                    return Err(bad("active backed layer needs an accounted sampler"));
                }
                if layer.surface().is_some() {
                    return Ok(Route::Layer(id));
                }
            }
        }
    }
    if st.doc.xcf_state.is_some() {
        return Err(bad("canonical imported merged sampler is not qualified by ordinary compositor"));
    }
    Ok(Route::Merged)
}
fn metadata(s: &Session, c: &Interrupt<'_>) -> Result<()> {
    crate::selection_sample_policy::existing_selection(s, ID, c)?;
    crate::selection_inverse_controlled::selection_write_authority_metadata(&s.active().ok_or(EngineError::NoDocument)?.doc, c)
}
pub(crate) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    let r = request(s, p, true)?;
    metadata(s, &Interrupt::NONE)?;
    route(s, r, &Interrupt::NONE)?;
    Ok(())
}
fn admission(doc: &Document, target: Route, c: &Interrupt<'_>) -> Result<FocusLimits> {
    poll(c)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad("canvas limit"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let retained = raster
        .retained_bytes
        .checked_mul(3)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .and_then(|n| n.checked_add(2 << 20))
        .ok_or_else(|| bad("retained working limit"))?;
    let mut l = FocusLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES.checked_sub(retained).ok_or_else(|| bad("working limit"))?, work_units: MAX_WORK };
    let (width, height, step) = focus::requirements(doc.bounds(), l).map_err(|e| error(c, e))?;
    if target == Route::Merged {
        // Match Focus's largest Sampler::rgb_scaled strip, not a full-canvas
        // render allocation. Output pixels are already in Focus's strip charge.
        let rows = 64usize.div_ceil(step).max(1).checked_mul(step).ok_or_else(|| bad("render strip height"))?.min(height);
        let bounds = doc.bounds();
        let end =
            i32::try_from(i64::from(bounds.y0) + i64::try_from(rows).map_err(|_| bad("render strip height"))?).map_err(|_| bad("render strip coordinates"))?;
        let render = photocraft_compose::ordinary_render_requirements(doc, photocraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, end), c)
            .map_err(|e| error(c, e))?;
        let reserve = render.scratch_bytes.checked_add(render.preparation_reserve_bytes).ok_or_else(|| bad("render preparation envelope"))? as u64;
        l.resident_bytes = l.resident_bytes.checked_sub(reserve).ok_or_else(|| bad("aggregate render preparation limit"))?;
        let strip_pixels = (width as u64).checked_mul(rows as u64).ok_or_else(|| bad("render work"))?;
        if strip_pixels == 0 {
            return Err(bad("render strip is empty"));
        }
        // work_units is exactly strip_pixels * max(layer_count,1). Each strip
        // visits its real pixels once, so charge all canvas pixels, not just one.
        let layer_units = render.work_units.checked_div(strip_pixels).ok_or_else(|| bad("render work"))?;
        let all_work = (width as u64).checked_mul(height as u64).and_then(|n| n.checked_mul(layer_units)).ok_or_else(|| bad("render work"))?;
        l.work_units = l.work_units.checked_sub(all_work).ok_or_else(|| bad("aggregate render work limit"))?;
        focus::requirements(bounds, l).map_err(|e| error(c, e))?;
    }
    crate::selection_inverse_controlled::selection_write_authority_metadata(doc, c)?;
    Ok(l)
}
fn read(
    doc: &Document,
    target: Route,
    area: photocraft_geom::Rect,
    c: &Interrupt<'_>,
) -> std::result::Result<Vec<[f32; 4]>, photocraft_raster::RasterInputError> {
    use photocraft_raster::RasterInputError as E;
    c.check().map_err(|_| E::Cancelled)?;
    let Route::Layer(id) = target else { return Ok(photocraft_compose::render_with_interrupt(doc, area, c)?.px) };
    let surface = doc.layer(id).and_then(|l| l.surface()).ok_or(E::UnsupportedConversion)?;
    let fmt = surface.format();
    let channels = fmt.channels();
    let required = match fmt.mode {
        photocraft_doc::ColorMode::Grayscale | photocraft_doc::ColorMode::Bitmap | photocraft_doc::ColorMode::Duotone => 1,
        photocraft_doc::ColorMode::Cmyk => 4,
        _ => 3,
    };
    if channels < required + usize::from(fmt.alpha) || channels > 16 {
        return Err(E::UnsupportedConversion);
    }
    let raw = surface.try_read_region_f64_with(area, c)?;
    let n = usize::try_from(u64::from(area.width()) * u64::from(area.height())).map_err(|_| E::LimitExceeded)?;
    if raw.len() != n.checked_mul(channels).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n {
        return Err(E::LimitExceeded);
    }
    for values in raw.chunks_exact(channels) {
        c.check().map_err(|_| E::Cancelled)?;
        let mut pixel = [0.0f32; 16];
        let pixel = pixel.get_mut(..channels).ok_or(E::LengthMismatch)?;
        for (v, input) in pixel.iter_mut().zip(values) {
            *v = *input as f32;
            if !v.is_finite() {
                return Err(E::InvalidSample);
            }
        }
        let rgba = photocraft_raster::to_rgba(&fmt, pixel);
        if rgba.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        out.push(rgba);
    }
    c.check().map_err(|_| E::Cancelled)?;
    Ok(out)
}
fn prepare(doc: &Document, r: Request, target: Route, c: &Interrupt<'_>) -> Result<(Option<Surface>, Option<photocraft_geom::Rect>)> {
    let l = admission(doc, target, c)?;
    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
    let region = focus::focus_area_with_interrupt(doc.bounds(), r.range, r.noise, l, c, |area, c| read(doc, target, area, c)).map_err(|e| error(c, e))?;
    let Some(region) = region else {
        return Ok((None, None));
    };
    let bounds = Some(region.bbox);
    let out = sel::try_combine_region_with_interrupt(
        doc.selection.as_ref(),
        Some(&region),
        r.mode,
        WandLimits { resident_bytes: l.resident_bytes, work_units: l.work_units },
        c,
    )
    .map_err(|e| error(c, e))?;
    poll(c)?;
    Ok((out, bounds))
}
fn same(a: Option<&Surface>, b: Option<&Surface>, c: &Interrupt<'_>) -> Result<bool> {
    poll(c)?;
    let (a, b) = match (a, b) {
        (None, None) => return Ok(true),
        (Some(a), Some(b)) => (a, b),
        _ => return Ok(false),
    };
    if a.format() != b.format() || a.default_bytes() != b.default_bytes() || a.tile_count() != b.tile_count() {
        return Ok(false);
    }
    for ((ac, at), (bc, bt)) in a.tiles().zip(b.tiles()) {
        poll(c)?;
        if ac != bc || at.bytes().len() != bt.bytes().len() {
            return Ok(false);
        }
        for (x, y) in at.bytes().chunks(65536).zip(bt.bytes().chunks(65536)) {
            poll(c)?;
            if x != y {
                return Ok(false);
            }
        }
    }
    poll(c)?;
    Ok(true)
}
fn copy_view(view: &crate::channel_cmds::ChannelView) -> Result<crate::channel_cmds::ChannelView> {
    let flags = |v: &[bool]| -> Result<Vec<bool>> {
        if v.len() > 100000 {
            return Err(bad("runtime view limit"));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(v.len()).map_err(|_| bad("runtime view allocation"))?;
        if out.capacity() > v.len().saturating_add(16) {
            return Err(bad("runtime view capacity"));
        }
        out.extend_from_slice(v);
        Ok(out)
    };
    Ok(crate::channel_cmds::ChannelView {
        target: view.target,
        color_hidden: flags(&view.color_hidden)?,
        alpha_visible: flags(&view.alpha_visible)?,
        quick_mask_hidden: view.quick_mask_hidden,
        layer_mask: view.layer_mask,
        editing_mask: view.editing_mask,
    })
}
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let r = request(s, p, false)?;
    metadata(s, &Interrupt::NONE)?;
    let target = route(s, r, &Interrupt::NONE)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    admission(&st.doc, target, &Interrupt::NONE)?;
    st.doc.validate_resident()?;
    if st.channel_view.color_hidden.len() > 100000 || st.channel_view.alpha_visible.len() > 100000 {
        return Err(bad("runtime view limit"));
    }
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let active_layer = st.active_layer;
    let view = copy_view(&st.channel_view)?;
    // Preserve an existing caller-selected ordinary CMYK sampling scope across
    // a worker thread; neither a new profile nor filesystem authority is chosen.
    let color_scope = photocraft_color::convert::active_cmyk_space();
    crate::jobs::run(
        s,
        "Focus Area",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Focus Area", |c| {
                photocraft_color::convert::with_cmyk_space(color_scope.as_ref(), || Ok((prepare(&worker, r, target, c)?, ctx.clone())))
            })
        },
        move |s, ((output, bounds), ctx)| {
            let st = s.active().ok_or(EngineError::NoDocument)?;
            if st.doc.id != baseline.id
                || st.revision != revision
                || !baseline.downgrade().is_current(&st.doc)
                || st.active_layer != active_layer
                || st.channel_view != view
                || st.floating.is_some()
                || st.coalesce.is_some()
                || s.coalesce_request.is_some()
                || s.color_restrict.is_some()
                || s.command_has_pending_job()
                || s.command_has_job_on(st.doc.id)
            {
                return Err(bad("Focus Area target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, "Focus Area", |c| {
                poll(c)?;
                crate::selection_sample_policy::existing_selection(s, ID, c)?;
                crate::selection_inverse_controlled::selection_write_authority(&s.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if route(s, r, c)? != target {
                    return Err(bad("sampled owner changed"));
                }
                // No detected region preserves prior selection and redo, in every mode.
                if bounds.is_none(){let selected=s.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some();return Ok(json!({"selected":selected,"changed":false}));}
                let selected = output.is_some();
                let response=json!({"selected":selected,"changed":true,"bounds":bounds.map(|r|[i64::from(r.x0),i64::from(r.y0),i64::from(r.width()),i64::from(r.height())])});
                if same(s.active().ok_or(EngineError::NoDocument)?.doc.selection.as_ref(), output.as_ref(), c)? {
                    return Ok(json!({"selected":selected,"changed":false}));
                }
                s.edit_with_interrupt("Focus Area", c, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                    doc.selection = output;
                    poll(c)
                })?;
                Ok(response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        let mut s = Session::new();
        s.add_document(Document::new("Focus draft", photocraft_geom::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), None)
            .unwrap();
        s
    }
    #[test]
    fn merged_admission_charges_known_render_preparation_separately() {
        let fixture = Document::with_background(
            "Focus render ledger",
            photocraft_geom::Size::new(8, 8),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let doc = &fixture;
        let direct = admission(doc, Route::Layer(doc.layers[0].id), &Interrupt::NONE).unwrap();
        let merged = admission(doc, Route::Merged, &Interrupt::NONE).unwrap();
        let req = photocraft_compose::ordinary_render_requirements(doc, doc.bounds(), &Interrupt::NONE).unwrap();
        assert_eq!(direct.resident_bytes - merged.resident_bytes, (req.scratch_bytes + req.preparation_reserve_bytes) as u64);
        assert_eq!(direct.work_units - merged.work_units, req.work_units);
    }
    #[test]
    fn captured_stale_and_wide_coverage_refuse_without_edit() {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let id = before.id.0;
        assert!(dispatch_preflight(&s, &json!({"documentId":id.to_string(),"expectedRevision":"1"}), false).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().selection =
            Some(Surface::new(photocraft_color::PixelFormat::new(photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::F64, false)));
        assert!(dispatch_preflight(&s, &Value::Null, false).is_err());
    }
    #[test]
    fn empty_focus_keeps_selection_and_redo() {
        let mut s = session();
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().selection = Some(Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]));
        s.edit("redo seed", |d, _| {
            d.name = "later".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let rev = s.active().unwrap().revision;
        let response = run(&mut s, &json!({"sampleAllLayers":true})).unwrap();
        assert_eq!(response["changed"], false);
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, rev);
        assert!(s.redo());
    }
    #[test]
    fn cancelled_private_preparation_leaves_original() {
        let s = session();
        let st = s.active().unwrap();
        let before = st.doc.clone();
        let req = request(&s, &Value::Null, false).unwrap();
        assert!(matches!(prepare(&st.doc, req, Route::Merged, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(before.shares_state(&st.doc));
    }
}
