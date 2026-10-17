//! Current ordinary Quick Selection stroke, prepared with real worker control.
//! Imported merged sampling is closed; ordinary rendering is not source proof.
use crate::channel_cmds::ChannelView;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::segment::{
    Sampler,
    quick::{self, QuickLimits},
};
use photocraft_algo::selection::{self as sel, SelectionMode, WandLimits};
use photocraft_color::{SampleType, try_read_sample_f64};
use photocraft_doc::{Document, Layer, LayerId, MAX_GROUP_DEPTH};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E, Surface};
use serde_json::{Value, json};
const ID: &str = "select.quick";
const MAX_WORK: u64 = 8_000_000_000;
fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn error(cmd: &str, c: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if c.cancelled() { EngineError::Cancelled } else { bad(cmd, &e.to_string()) }
}
fn exact(v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(ID, "canonical captured identity required"))?;
    s.parse().map_err(|_| bad(ID, "identity range"))
}
fn find_surface<'a>(doc: &'a Document, id: Option<LayerId>, ctl: &Interrupt<'_>) -> Result<Option<&'a Surface>> {
    let Some(id) = id else { return Ok(None) };
    let mut stack: [Option<std::slice::Iter<'a, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *stack.first_mut().ok_or_else(|| bad("select.object", "hierarchy"))? = Some(doc.layers.iter());
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
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("select.object", "owner count limit"))?;
        if layer.id == id {
            return Ok(layer.surface());
        }
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad("select.object", "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad("select.object", "hierarchy"))? = Some(children.iter());
        }
    }
    Ok(None)
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

const ENHANCE: photocraft_algo::matting::RefineParams =
    photocraft_algo::matting::RefineParams { radius: 4., smart_radius: true, smooth: 15., feather: 0., contrast: 25., shift_edge: 0. };
#[derive(Clone, Copy)]
struct Request {
    size: f32,
    all: bool,
    enhance: bool,
    mode: SelectionMode,
}
fn request(s: &Session, p: &Value) -> Result<Request> {
    let o = p
        .as_object()
        .filter(|o| {
            o.len() <= 7
                && o.keys().all(|k| matches!(k.as_str(), "points" | "size" | "mode" | "enhanceEdge" | "sampleAllLayers" | "documentId" | "expectedRevision"))
        })
        .ok_or_else(|| bad(ID, "bounded Quick Selection parameters required"))?;
    let points =
        o.get("points").and_then(Value::as_array).filter(|v| !v.is_empty() && v.len() <= 100000).ok_or_else(|| bad(ID, "needs bounded finite points"))?;
    for point in points {
        let pair = point.as_array().filter(|p| p.len() == 2).ok_or_else(|| bad(ID, "point must contain two numbers"))?;
        for v in pair {
            if v.as_f64().is_none_or(|v| !v.is_finite() || v.abs() > 1e7) {
                return Err(bad(ID, "point coordinate range"));
            }
        }
    }
    let size = o.get("size").map_or(Ok(30.), |v| {
        v.as_f64()
            .filter(|v| v.is_finite() && v.abs() <= f64::from(f32::MAX))
            .map(|v| (v as f32).clamp(1., 5000.))
            .ok_or_else(|| bad(ID, "finite brush size required"))
    })?;
    let flag = |key| o.get(key).map_or(Ok(false), |v| v.as_bool().ok_or_else(|| bad(ID, "boolean brush flag required")));
    let all = flag("sampleAllLayers")?;
    let enhance = flag("enhanceEdge")?;
    let mode = o.get("mode").map_or(Ok("add"), |v| v.as_str().filter(|v| v.len() <= 32).ok_or_else(|| bad(ID, "bounded selection mode required")))?;
    let mode = match mode {
        "subtract" => SelectionMode::Subtract,
        "replace" | "new" => SelectionMode::Replace,
        _ => SelectionMode::Add,
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    match (o.get("documentId"), o.get("expectedRevision")) {
        (None, None) => (),
        (Some(id), Some(rev)) if exact(id)? == st.doc.id.0 && exact(rev)? == st.revision => (),
        _ => return Err(bad(ID, "captured Quick Selection target changed")),
    }
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || st.coalesce.is_some()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || s.command_has_pending_job()
        || s.command_has_job_on(st.doc.id)
    {
        return Err(bad(ID, "Quick Selection requires idle resident state"));
    }
    if s.documents().len() > 1024 || s.documents().iter().any(|d| d.floating.is_some() || d.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some())) {
        return Err(bad(ID, "Quick Selection floating context unsupported"));
    }
    // Ordinary rendering is not imported source-space/render qualification.
    if st.doc.xcf_state.is_some() {
        return Err(bad(ID, "canonical imported Quick Selection is unsupported"));
    }
    crate::selection_sample_policy::existing_selection(s, ID, &Interrupt::NONE)?;
    crate::selection_sample_policy::sampled_pixels(s, ID, all, &Interrupt::NONE)?;
    crate::selection_inverse_controlled::selection_write_authority_metadata(&st.doc, &Interrupt::NONE)?;
    Ok(Request { size, all, enhance, mode })
}
pub(crate) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    request(s, p).map(|_| ())
}
fn points(p: &Value) -> Result<Vec<(f32, f32)>> {
    let input = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(ID, "points required"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(input.len()).map_err(|_| bad(ID, "point allocation"))?;
    if out.capacity() > input.len().saturating_add(16) {
        return Err(bad(ID, "point capacity"));
    }
    for point in input {
        out.push((
            point.get(0).and_then(Value::as_f64).ok_or_else(|| bad(ID, "point x"))? as f32,
            point.get(1).and_then(Value::as_f64).ok_or_else(|| bad(ID, "point y"))? as f32,
        ));
    }
    Ok(out)
}
fn read_region(region: &sel::Region, area: Rect, c: &Interrupt<'_>) -> std::result::Result<Vec<f32>, E> {
    c.check().map_err(|_| E::Cancelled)?;
    let n = (area.width() as usize).checked_mul(area.height() as usize).ok_or(E::LimitExceeded)?;
    if (n as u64).checked_mul(4).is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES) {
        return Err(E::LimitExceeded);
    }
    let expected = (region.bbox.width() as usize).checked_mul(region.bbox.height() as usize).ok_or(E::LimitExceeded)?;
    if region.mask.len() != expected {
        return Err(E::LengthMismatch);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            c.check().map_err(|_| E::Cancelled)?;
            let value = if x >= region.bbox.x0 && x < region.bbox.x1 && y >= region.bbox.y0 && y < region.bbox.y1 {
                let i = (y - region.bbox.y0) as usize * region.bbox.width() as usize + (x - region.bbox.x0) as usize;
                f32::from(*region.mask.get(i).ok_or(E::LengthMismatch)?) / 255.
            } else {
                0.
            };
            out.push(value);
        }
    }
    Ok(out)
}
struct Prepared {
    selection: Option<Surface>,
    response: Value,
    change: bool,
}
fn prepare(doc: &Document, active: Option<LayerId>, r: Request, points: &Vec<(f32, f32)>, c: &Interrupt<'_>) -> Result<Prepared> {
    poll(c)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300000 || doc.size.height > 300000 {
        return Err(bad(ID, "canvas bounds"));
    }
    let canvas = doc.bounds();
    let q = quick::requirements(canvas, points, r.size, c).map_err(|e| error(ID, c, e))?;
    let direct = if r.all { None } else { find_surface(doc, active, c)? };
    let lim = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, lim, c).map_err(|e| error(ID, c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, lim, c).map_err(|e| error(ID, c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let prior = doc.selection.as_ref().map_or(Ok(Rect::EMPTY), |s| s.try_content_bounds_with_interrupt(c).map_err(|e| error(ID, c, e)))?;
    let extent = if matches!(r.mode, SelectionMode::Add | SelectionMode::Subtract) { canvas.union(&prior) } else { canvas };
    let tiles = u64::from(extent.width())
        .div_ceil(TILE_SIZE as u64)
        .checked_add(1)
        .and_then(|v| v.checked_mul(u64::from(extent.height()).div_ceil(TILE_SIZE as u64).checked_add(1)?))
        .ok_or_else(|| bad(ID, "selection tiles"))?;
    let output = tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 8 + 1024).ok_or_else(|| bad(ID, "selection output"))?;
    let params = (points.capacity() as u64).checked_mul(8).ok_or_else(|| bad(ID, "points capacity"))?;
    let known = model
        .checked_mul(5)
        .and_then(|v| v.checked_add(payload.checked_mul(2)?))
        .and_then(|v| v.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|v| v.checked_add(output))
        .and_then(|v| v.checked_add(params))
        .ok_or_else(|| bad(ID, "known resident overflow"))?;
    let available = MAX_CHECKED_RASTER_BYTES
        .checked_sub(known)
        .filter(|v| *v >= q.peak_resident_bytes)
        .ok_or_else(|| bad(ID, "aggregate Quick Selection resident limit"))?;
    let fixed_work = model
        .checked_add(payload)
        .and_then(|v| v.checked_mul(8))
        .and_then(|v| v.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(output.checked_mul(16)?))
        .ok_or_else(|| bad(ID, "known work overflow"))?;
    let work_cap = MAX_WORK.checked_sub(fixed_work).filter(|v| *v >= q.work_units).ok_or_else(|| bad(ID, "aggregate Quick Selection work limit"))?;
    let remaining = std::cell::Cell::new(work_cap);
    let before_window = |window: Rect, peak: u64, kernel_work: u64| -> std::result::Result<(), E> {
        c.check().map_err(|_| E::Cancelled)?;
        let (extra, render_work) = if direct.is_none() {
            let step = q.step;
            let h = if step == 1 { window.height() } else { window.height().min(64) };
            let strip = Rect::new(window.x0, window.y0, window.x1, window.y0.checked_add(h as i32).ok_or(E::InvalidBounds)?);
            let rr = photocraft_compose::ordinary_render_requirements(doc, strip, c)?;
            let visits = if step == 1 { 1 } else { u64::from(window.height()).div_ceil(64) };
            (
                (rr.scratch_bytes as u64).checked_add(rr.preparation_reserve_bytes as u64).ok_or(E::LimitExceeded)?,
                rr.work_units.checked_mul(visits).ok_or(E::LimitExceeded)?,
            )
        } else {
            (0, 0)
        };
        if peak.checked_add(extra).is_none_or(|v| v > available) || kernel_work.checked_add(render_work).is_none_or(|v| v > remaining.get()) {
            return Err(E::LimitExceeded);
        }
        remaining.set(remaining.get().checked_sub(render_work).ok_or(E::LimitExceeded)?);
        Ok(())
    };
    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
    let sampler = BoundedSampler { doc, direct, control: c };
    let mut region = quick::quick_select_accounted_with_interrupt(
        &sampler,
        canvas,
        points,
        r.size,
        QuickLimits { resident_bytes: available, work_units: work_cap },
        c,
        &remaining,
        &before_window,
    )
    .map_err(|e| error(ID, c, e))?;
    if r.enhance
        && let Some(found) = region.as_ref()
    {
        let rr = photocraft_algo::matting::refine_requirements(found.bbox, canvas, &ENHANCE).map_err(|e| error(ID, c, e))?;
        let (extra, render_work) = if direct.is_none() {
            let area = Rect::new(0, 0, rr.window_width as i32, rr.window_height as i32);
            let b = photocraft_compose::ordinary_render_requirements(doc, area, c).map_err(|e| error(ID, c, e))?;
            (
                (b.scratch_bytes as u64).checked_add(b.preparation_reserve_bytes as u64).ok_or_else(|| bad(ID, "enhance render resident"))?,
                b.work_units
                    .checked_mul(u64::from(rr.bbox.width()).div_ceil(256) * u64::from(rr.bbox.height()).div_ceil(256))
                    .ok_or_else(|| bad(ID, "enhance render work"))?,
            )
        } else {
            (0, 0)
        };
        let bytes = rr
            .peak_resident_bytes
            .checked_add(found.mask.capacity() as u64)
            .and_then(|v| v.checked_add(extra))
            .filter(|v| *v <= available)
            .ok_or_else(|| bad(ID, "enhance aggregate resident limit"))?;
        let _ = bytes;
        let work = rr.work_units.checked_add(render_work).filter(|v| *v <= remaining.get()).ok_or_else(|| bad(ID, "enhance aggregate work limit"))?;
        remaining.set(remaining.get() - work);
        region = photocraft_algo::matting::refine_mask_with_interrupt(
            &sampler,
            &|area| {
                c.check().map_err(|_| E::Cancelled)?;
                read_region(found, area, c)
            },
            found.bbox,
            canvas,
            &ENHANCE,
            photocraft_algo::matting::MattingLimits::default(),
            c,
        )
        .map_err(|e| error(ID, c, e))?;
    }
    let Some(region) = region else {
        return Ok(Prepared { selection: None, response: json!({"selected":doc.selection.is_some(),"changed":false}), change: false });
    };
    let b = region.bbox;
    let selection = sel::try_combine_region_with_interrupt(
        doc.selection.as_ref(),
        Some(&region),
        r.mode,
        WandLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK },
        c,
    )
    .map_err(|e| error(ID, c, e))?;
    let change = !same(doc.selection.as_ref(), selection.as_ref(), c)?;
    let response = json!({"selected":selection.is_some(),"changed":true,"bounds":[b.x0,b.y0,b.width()as i32,b.height()as i32]});
    poll(c)?;
    Ok(Prepared { selection, response, change })
}
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let r = request(s, p)?;
    let points = points(p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let active = st.active_layer;
    let view = copy_view(&st.channel_view, ID)?;
    let color = photocraft_color::convert::active_cmyk_space();
    crate::jobs::run(
        s,
        "Quick Selection",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Quick Selection", |c| {
                photocraft_color::convert::with_cmyk_space(color.as_ref(), || Ok((prepare(&worker, active, r, &points, c)?, ctx.clone())))
            })
        },
        move |s, (prepared, ctx)| {
            let st = s.active().ok_or(EngineError::NoDocument)?;
            if st.doc.id != baseline.id
                || st.revision != revision
                || !baseline.downgrade().is_current(&st.doc)
                || st.active_layer != active
                || st.channel_view != view
                || s.documents().iter().any(|d| d.floating.is_some() || d.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some()))
                || st.coalesce.is_some()
                || s.coalesce_request.is_some()
                || s.color_restrict.is_some()
                || s.command_has_pending_job()
                || s.command_has_job_on(st.doc.id)
            {
                return Err(bad(ID, "Quick Selection target changed during preparation"));
            }
            ctx.stage(0.95, 1., "Quick Selection", |c| {
                poll(c)?;
                crate::selection_inverse_controlled::selection_write_authority(&s.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if !prepared.change {
                    return Ok(prepared.response);
                }
                let response = prepared.response;
                s.edit_with_interrupt("Quick Selection", c, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                    doc.selection = prepared.selection;
                    poll(c)
                })?;
                Ok(response)
            })
        },
    )
}
struct BoundedSampler<'a, 'c> {
    doc: &'a Document,
    direct: Option<&'a Surface>,
    control: &'a Interrupt<'c>,
}
impl Sampler for BoundedSampler<'_, '_> {
    fn rgba(&self, r: Rect) -> std::result::Result<Vec<[f32; 4]>, E> {
        self.control.check().map_err(|_| E::Cancelled)?;
        let n = (r.width() as usize).checked_mul(r.height() as usize).ok_or(E::LimitExceeded)?;
        let Some(surface) = self.direct else {
            if self.doc.xcf_state.is_some() {
                return Err(E::UnsupportedConversion);
            }
            let b = photocraft_compose::render_with_interrupt(self.doc, r, self.control)?;
            if b.rect != r || b.px.len() != n {
                return Err(E::LengthMismatch);
            }
            return Ok(b.px);
        };
        if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
            return Err(E::UnsupportedConversion);
        }
        let fmt = surface.format();
        let channels = surface.channels();
        let required = match fmt.mode {
            photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Bitmap | photocraft_color::ColorMode::Duotone => 1,
            photocraft_color::ColorMode::Cmyk => 4,
            _ => 3,
        };
        if channels < required + usize::from(fmt.alpha) {
            return Err(E::UnsupportedConversion);
        }
        let width = fmt.bytes_per_pixel();
        if channels == 0 || channels > 5 || width > 40 {
            return Err(E::UnsupportedConversion);
        }
        if (n as u64).checked_mul(16).is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES) {
            return Err(E::LimitExceeded);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
        if out.capacity() > n.saturating_add(16) {
            return Err(E::AllocationFailed);
        }
        let mut raw = [0u8; 40];
        let mut native = [0f32; 5];
        for y in r.y0..r.y1 {
            self.control.check().map_err(|_| E::Cancelled)?;
            for x in r.x0..r.x1 {
                if x.rem_euclid(1024) == 0 {
                    self.control.check().map_err(|_| E::Cancelled)?;
                }
                let bytes = raw.get_mut(..width).ok_or(E::LengthMismatch)?;
                surface.try_read_pixel_bytes(x, y, bytes)?;
                let values = native.get_mut(..channels).ok_or(E::LengthMismatch)?;
                for (i, v) in values.iter_mut().enumerate() {
                    *v = try_read_sample_f64(bytes, fmt.sample, i).map_err(|_| E::InvalidSample)? as f32;
                    if !v.is_finite() {
                        return Err(E::InvalidSample);
                    }
                }
                let value = photocraft_raster::to_rgba(&fmt, values);
                if value.iter().any(|v| !v.is_finite()) {
                    return Err(E::InvalidSample);
                }
                out.push(value);
            }
        }
        self.control.check().map_err(|_| E::Cancelled)?;
        Ok(out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    use photocraft_geom::Size;
    fn doc() -> Document {
        let mut d = Document::new("Quick", Size::new(64, 48), ColorMode::Rgb, SampleType::U16);
        let mut l = Layer::raster("Pixels", d.pixel_format());
        let mut values = Vec::new();
        for y in 0..48 {
            for x in 0..64 {
                values.extend_from_slice(if (12..45).contains(&x) && (8..39).contains(&y) { &[0.9, 0.1, 0.2, 1.] } else { &[0.1, 0.2, 0.8, 1.] });
            }
        }
        l.surface_mut().unwrap().write_region(d.bounds(), &values);
        d.layers.push(l);
        let mut prior = Surface::new(PixelFormat::GRAY8);
        prior.fill_rect(Rect::new(-2, -2, 10, 10), &[0.5]);
        d.selection = Some(prior);
        d
    }
    #[test]
    fn private_stroke_and_enhancement_match_upstream_and_preserve_raw_source() {
        let d = doc();
        let active = Some(d.layers[0].id);
        let before = d.clone();
        let pts = vec![(24., 20.), (26., 22.), (29., 23.)];
        for enhance in [false, true] {
            for mode in [SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Replace] {
                let sampler = photocraft_algo::segment::SurfaceSampler(d.layers[0].surface().unwrap());
                let mut region = quick::quick_select(&sampler, d.bounds(), &pts, 12., quick::WORK_PX).unwrap();
                if enhance && let Some(r) = &region {
                    let refined =
                        photocraft_algo::matting::refine_mask(&sampler, &photocraft_algo::matting::region_reader(r), r.bbox, d.bounds(), &ENHANCE).unwrap();
                    region = refined;
                }
                let expected = region.as_ref().map(|r| sel::combine_region(d.selection.as_ref(), Some(r), mode));
                let got = prepare(&d, active, Request { size: 12., all: false, enhance, mode }, &pts, &Interrupt::NONE).unwrap();
                if let Some(expected) = expected {
                    assert_eq!(got.selection, expected);
                } else {
                    assert!(!got.change);
                }
                assert_eq!(d, before);
            }
        }
    }
    #[test]
    fn late_worker_cancel_and_stale_pair_leave_document_and_history_unchanged() {
        let d = doc();
        let pts = vec![(24., 20.)];
        let calls = std::sync::atomic::AtomicUsize::new(0usize);
        let cancelled = || {
            calls.store(calls.load(std::sync::atomic::Ordering::Relaxed) + 1, std::sync::atomic::Ordering::Relaxed);
            calls.load(std::sync::atomic::Ordering::Relaxed) > 100
        };
        assert!(matches!(
            prepare(
                &d,
                Some(d.layers[0].id),
                Request { size: 12., all: false, enhance: true, mode: SelectionMode::Add },
                &pts,
                &Interrupt::cancel_only(&cancelled)
            ),
            Err(EngineError::Cancelled)
        ));
        let mut s = Session::new();
        s.add_document(d, None).unwrap();
        let st = s.active().unwrap();
        let before = st.doc.clone();
        let rev = st.revision;
        assert!(run(&mut s, &json!({"points":[[24,20]],"documentId":before.id.0.to_string(),"expectedRevision":(rev+1).to_string()})).is_err());
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, rev);
    }
}
