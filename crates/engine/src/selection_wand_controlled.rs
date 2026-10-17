//! Existing Magic Wand sampled-owner policy, controlled private preparation.
//! Canonical imported merged appearance is not routed through ordinary render.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::selection::{self as sel, SelectionMode, WandLimits};
use photocraft_doc::{Document, LayerId, item_identity::ChannelId};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};
const ID: &str = "select.magicWand";
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
    x: i32,
    y: i32,
    tolerance: f32,
    contiguous: bool,
    aa: bool,
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
                o.len() <= 9
                    && o.keys().all(|k| {
                        matches!(
                            k.as_str(),
                            "x" | "y" | "tolerance" | "contiguous" | "antiAlias" | "sampleAllLayers" | "mode" | "documentId" | "expectedRevision"
                        )
                    })
            })
            .ok_or_else(|| bad("bounded Wand parameters required"))?;
        let _ = o;
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let captured = match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if decimal(id)? == st.doc.id.0 && decimal(rev)? == st.revision => true,
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
    let coord = |k| -> Result<i32> {
        let n = scalar(p, k, 0.0)?.floor();
        if f64::from(n) < f64::from(i32::MIN) || f64::from(n) > f64::from(i32::MAX) {
            return Err(bad("seed coordinate overflow"));
        }
        Ok(n as i32)
    };
    let mode = match p.get("mode") {
        None => SelectionMode::Replace,
        Some(Value::String(n)) if n.len() <= 64 => SelectionMode::parse(n),
        _ => return Err(bad("bounded mode required")),
    };
    Ok(Request {
        x: coord("x")?,
        y: coord("y")?,
        tolerance: scalar(p, "tolerance", 32.0)?,
        contiguous: flag(p, "contiguous", true)?,
        aa: flag(p, "antiAlias", true)?,
        all: flag(p, "sampleAllLayers", false)?,
        mode,
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Layer(LayerId),
    Mask(LayerId),
    Alpha(ChannelId),
    QuickMask,
    Merged,
}
fn route(s: &Session, r: Request, c: &Interrupt<'_>) -> Result<Route> {
    poll(c)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if let Some(shown) = crate::selection_sample_policy::shown_gray_surface(s, ID, c)? {
        for channel in &st.doc.channels {
            poll(c)?;
            if std::ptr::eq(shown, &channel.surface) {
                return Ok(Route::Alpha(channel.id));
            }
        }
        if st.doc.quick_mask.as_ref().is_some_and(|q| std::ptr::eq(shown, &q.surface)) {
            return Ok(Route::QuickMask);
        }
        for id in [st.channel_view.layer_mask.map(|v| v.layer), st.active_layer].into_iter().flatten() {
            poll(c)?;
            if st.doc.layer(id).and_then(|l| l.mask.as_ref()).is_some_and(|m| std::ptr::eq(shown, &*m.surface)) {
                return Ok(Route::Mask(id));
            }
        }
        return Err(bad("shown Gray owner changed"));
    }
    crate::selection_sample_policy::sampled_pixels(s, ID, r.all, c)?;
    if !r.all
        && let Some(id) = st.active_layer
        && st.doc.layer(id).and_then(|l| l.surface()).is_some()
    {
        return Ok(Route::Layer(id));
    }
    if st.doc.xcf_state.is_some() {
        return Err(bad("canonical imported merged sampler is not qualified by ordinary compositor"));
    }
    Ok(Route::Merged)
}
pub(crate) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    let r = request(s, p, true)?;
    crate::selection_sample_policy::metadata_preflight(s, ID, p, &Interrupt::NONE)?;
    route(s, r, &Interrupt::NONE)?;
    Ok(())
}
fn surface(doc: &Document, r: Route) -> Result<Option<&Surface>> {
    Ok(match r {
        Route::Layer(id) => Some(doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad("sample layer lost"))?),
        Route::Mask(id) => Some(&*doc.layer(id).and_then(|l| l.mask.as_ref()).ok_or_else(|| bad("sample mask lost"))?.surface),
        Route::Alpha(id) => {
            let mut matching = doc.channels.iter().filter(|v| v.id == id);
            let v = matching.next().ok_or_else(|| bad("sample channel lost"))?;
            if matching.next().is_some() {
                return Err(bad("ambiguous sample channel"));
            }
            Some(&v.surface)
        }
        Route::QuickMask => Some(&doc.quick_mask.as_ref().ok_or_else(|| bad("quick mask lost"))?.surface),
        Route::Merged => None,
    })
}
fn admission(doc: &Document, r: Request, target: Route, c: &Interrupt<'_>) -> Result<WandLimits> {
    poll(c)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad("canvas limit"));
    }
    let n = u64::from(doc.size.width).checked_mul(u64::from(doc.size.height)).ok_or_else(|| bad("pixel count"))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, c).map_err(|e| error(c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let temporary = n.checked_mul(if r.contiguous { 48 } else { 24 }).and_then(|n| n.checked_add(2 << 20)).ok_or_else(|| bad("Wand working planes"))?;
    let retained = raster
        .retained_bytes
        .checked_mul(3)
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add(model.checked_mul(5)?))
        .and_then(|n| n.checked_add(payload.checked_mul(2)?))
        .ok_or_else(|| bad("retained limit"))?;
    if retained.checked_add(temporary).is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES) || n.checked_mul(128).is_none_or(|n| n > MAX_WORK) {
        return Err(bad("Wand aggregate working limit"));
    }
    crate::selection_inverse_controlled::selection_write_authority_metadata(doc, c)?;
    let mut limits = WandLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES.checked_sub(retained).ok_or_else(|| bad("working limit"))?, work_units: MAX_WORK };
    if target == Route::Merged {
        // Match the one-row render_bands sampling route. The retained byte
        // output/plane charge above already includes its transient RGBA row.
        let bounds = doc.bounds();
        let end = bounds.y0.checked_add(1).ok_or_else(|| bad("render row"))?.min(bounds.y1);
        let req = photocraft_compose::ordinary_render_requirements(doc, photocraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, end), c)
            .map_err(|e| error(c, e))?;
        let reserve = req.scratch_bytes.checked_add(req.preparation_reserve_bytes).ok_or_else(|| bad("render reserve"))? as u64;
        limits.resident_bytes = limits.resident_bytes.checked_sub(reserve).ok_or_else(|| bad("aggregate merged render storage"))?;
        let row_pixels = u64::from(bounds.width());
        if row_pixels == 0 {
            return Err(bad("empty render row"));
        }
        let work = req.work_units.checked_div(row_pixels).and_then(|layers| layers.checked_mul(n)).ok_or_else(|| bad("render work"))?;
        limits.work_units = limits.work_units.checked_sub(work).ok_or_else(|| bad("aggregate merged render work"))?;
        if temporary > limits.resident_bytes || n.checked_mul(128).is_none_or(|v| v > limits.work_units) {
            return Err(bad("aggregate merged Wand limit"));
        }
    }
    Ok(limits)
}
fn prepare(doc: &Document, r: Request, target: Route, c: &Interrupt<'_>) -> Result<Option<Surface>> {
    let l = admission(doc, r, target, c)?;
    // Full retained source validation belongs inside the cancellable worker.
    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
    let area = doc.bounds();
    let pixels = if let Some(s) = surface(doc, target)? {
        sel::try_rgba8_image_with_interrupt(s, area, l, c).map_err(|e| error(c, e))?
    } else {
        if doc.xcf_state.is_some() {
            return Err(bad("unqualified imported merged sampler"));
        }
        let mut out = Vec::new();
        let n = usize::try_from(u64::from(doc.size.width) * u64::from(doc.size.height)).map_err(|_| bad("sample length"))?;
        out.try_reserve_exact(n).map_err(|_| bad("sample allocation"))?;
        if (out.capacity() as u64).checked_mul(4).is_none_or(|v| v > l.resident_bytes) {
            return Err(bad("sample capacity"));
        }
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        photocraft_compose::render_bands_with_interrupt(doc, area, 1, c, |band| {
            for p in band.px {
                c.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
                if out.len() >= n || out.len() >= out.capacity() {
                    return Err(photocraft_raster::RasterInputError::LimitExceeded);
                }
                out.push(p.map(q));
            }
            Ok(())
        })
        .map_err(|e| error(c, e))?;
        if out.len() != n {
            return Err(bad("incomplete composite sampling"));
        }
        out
    };
    let region = sel::try_wand_region_with_interrupt(&pixels, area, (r.x, r.y), r.tolerance, r.contiguous, r.aa, l, c).map_err(|e| error(c, e))?;
    drop(pixels);
    let output = sel::try_combine_region_with_interrupt(doc.selection.as_ref(), region.as_ref(), r.mode, l, c).map_err(|e| error(c, e))?;
    poll(c)?;
    Ok(output)
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
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let r = request(s, p, false)?;
    crate::selection_sample_policy::metadata_preflight(s, ID, p, &Interrupt::NONE)?;
    let target = route(s, r, &Interrupt::NONE)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    admission(&st.doc, r, target, &Interrupt::NONE)?;
    st.doc.validate_resident()?;
    if st.channel_view.color_hidden.len() > 100000 || st.channel_view.alpha_visible.len() > 100000 {
        return Err(bad("runtime view limit"));
    }
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let active_layer = st.active_layer;
    let view = st.channel_view.clone();
    // Preserve an existing caller-selected ordinary CMYK sampling scope across
    // a worker thread; neither a new profile nor filesystem authority is chosen.
    let color_scope = photocraft_color::convert::active_cmyk_space();
    crate::jobs::run(
        s,
        "Magic Wand",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Magic Wand", |c| {
                photocraft_color::convert::with_cmyk_space(color_scope.as_ref(), || Ok((prepare(&worker, r, target, c)?, ctx.clone())))
            })
        },
        move |s, (output, ctx)| {
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
                return Err(bad("Wand target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, "Magic Wand", |c| {
                poll(c)?;
                crate::selection_sample_policy::existing_selection(s, ID, c)?;
                crate::selection_inverse_controlled::selection_write_authority(&s.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if route(s, r, c)? != target {
                    return Err(bad("sampled owner changed"));
                }
                let selected = output.is_some();
                if same(s.active().ok_or(EngineError::NoDocument)?.doc.selection.as_ref(), output.as_ref(), c)? {
                    return Ok(json!({"selected":selected}));
                }
                s.edit_with_interrupt("Magic Wand", c, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                    doc.selection = output;
                    poll(c)
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
    fn only_merged_wand_route_debits_known_render_reserve() {
        let doc = Document::with_background(
            "merged ledger draft",
            photocraft_geom::Size::new(8, 8),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let r = Request { x: 0, y: 0, tolerance: 32.0, contiguous: true, aa: true, all: true, mode: SelectionMode::Replace };
        let direct = admission(&doc, r, Route::Layer(doc.layers[0].id), &Interrupt::NONE).unwrap();
        let merged = admission(&doc, r, Route::Merged, &Interrupt::NONE).unwrap();
        let req = photocraft_compose::ordinary_render_requirements(&doc, photocraft_geom::Rect::new(0, 0, 8, 1), &Interrupt::NONE).unwrap();
        assert_eq!(direct.resident_bytes - merged.resident_bytes, (req.scratch_bytes + req.preparation_reserve_bytes) as u64);
        assert_eq!(direct.work_units - merged.work_units, req.work_units * 8);
    }
    #[test]
    fn captured_pair_refuses_stale_revision_before_sampling() {
        let mut s = Session::new();
        s.add_document(Document::new("Wand draft", photocraft_geom::Size::new(2, 2), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8), None)
            .unwrap();
        let id = s.active().unwrap().doc.id.0;
        let revision = s.active().unwrap().revision;
        assert!(dispatch_preflight(&s, &json!({"documentId":id.to_string(),"expectedRevision":revision.to_string()}), false).is_ok());
        let stale_revision = revision.checked_add(1).unwrap();
        assert!(dispatch_preflight(&s, &json!({"documentId":id.to_string(),"expectedRevision":stale_revision.to_string()}), false).is_err());
    }
}
