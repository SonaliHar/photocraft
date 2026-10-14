//! Existing rectangle Object Selection, prepared privately with real worker control.
//! Imported merged sampling is closed; ordinary rendering is not source proof.
use crate::channel_cmds::ChannelView;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::segment::{
    Sampler,
    grabcut::{self, ObjectSelectionLimits},
};
use photocraft_algo::selection::{self as sel, SelectionMode, WandLimits};
use photocraft_color::{SampleType, try_read_sample_f64};
use photocraft_doc::{Document, Layer, LayerId, MAX_GROUP_DEPTH};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E, Surface};
use serde_json::{Value, json};
const ID: &str = "select.object";
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

#[derive(Clone, Copy)]
struct Request {
    rect: Rect,
    all: bool,
    mode: SelectionMode,
}
pub(crate) fn handles(p: &Value) -> bool {
    p.get("rect").is_some()
}
fn request(s: &Session, p: &Value) -> Result<Request> {
    let o = p
        .as_object()
        .filter(|o| o.len() <= 6 && o.keys().all(|k| matches!(k.as_str(), "rect" | "point" | "mode" | "sampleAllLayers" | "documentId" | "expectedRevision")))
        .ok_or_else(|| bad(ID, "bounded rectangle parameters required"))?;
    let a = o.get("rect").and_then(Value::as_array).filter(|a| a.len() == 4).ok_or_else(|| bad(ID, "rect must contain four numbers"))?;
    let mut v = [0f64; 4];
    for (out, input) in v.iter_mut().zip(a) {
        *out = input.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(ID, "finite rectangle required"))?;
    }
    let end_x = v[0] + v[2];
    let end_y = v[1] + v[3];
    let bounds = [v[0].min(end_x).floor(), v[1].min(end_y).floor(), v[0].max(end_x).ceil(), v[1].max(end_y).ceil()];
    if bounds.iter().any(|v| !v.is_finite() || *v < f64::from(i32::MIN) || *v > f64::from(i32::MAX)) {
        return Err(bad(ID, "rectangle coordinate range"));
    }
    let rect = Rect::new(bounds[0] as i32, bounds[1] as i32, bounds[2] as i32, bounds[3] as i32);
    // Rectangle wins over point as before, but ignored point data must still
    // fit the envelope before dispatcher cloning.
    if let Some(point) = o.get("point") {
        let pair = point.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(ID, "bounded point pair required"))?;
        for value in pair {
            if value.as_f64().is_none_or(|v| !v.is_finite() || v.abs() > f64::from(f32::MAX)) {
                return Err(bad(ID, "finite point required"));
            }
        }
    }
    let all = o.get("sampleAllLayers").map_or(Ok(false), |v| v.as_bool().ok_or_else(|| bad(ID, "sampleAllLayers must be boolean")))?;
    let mode = o.get("mode").map_or(Ok("replace"), |v| v.as_str().filter(|v| v.len() <= 32).ok_or_else(|| bad(ID, "bounded selection mode required")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    match (o.get("documentId"), o.get("expectedRevision")) {
        (None, None) => (),
        (Some(id), Some(rev)) if exact(id)? == st.doc.id.0 && exact(rev)? == st.revision => (),
        _ => return Err(bad(ID, "captured Object Selection target changed")),
    }
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || st.coalesce.is_some()
        || s.coalesce_request.is_some()
        || s.color_restrict.is_some()
        || s.command_has_pending_job()
        || s.command_has_job_on(st.doc.id)
    {
        return Err(bad(ID, "Object Selection requires idle resident state"));
    }
    if s.documents().len() > 1024 || s.documents().iter().any(|d| d.floating.is_some() || d.doc.xcf_state.as_ref().is_some_and(|x| x.floating.is_some())) {
        return Err(bad(ID, "Object Selection floating context unsupported"));
    }
    if st.doc.xcf_state.is_some() {
        return Err(bad(ID, "canonical imported Object Selection is not qualified"));
    }
    crate::selection_sample_policy::existing_selection(s, ID, &Interrupt::NONE)?;
    crate::selection_sample_policy::sampled_pixels(s, ID, all, &Interrupt::NONE)?;
    crate::selection_inverse_controlled::selection_write_authority_metadata(&st.doc, &Interrupt::NONE)?;
    Ok(Request { rect, all, mode: SelectionMode::parse(if mode == "new" { "replace" } else { mode }) })
}
pub(crate) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    request(s, p).map(|_| ())
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
fn admission(doc: &Document, active: Option<LayerId>, r: Request, c: &Interrupt<'_>) -> Result<(grabcut::ObjectSelectionRequirements, ObjectSelectionLimits)> {
    poll(c)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300_000 || doc.size.height > 300_000 {
        return Err(bad(ID, "Object Selection canvas bounds"));
    }
    let canvas = doc.bounds();
    let req = grabcut::requirements(canvas, r.rect, 160_000).map_err(|e| error(ID, c, e))?;
    let direct = if r.all { None } else { find_surface(doc, active, c)? };
    if doc.xcf_state.is_some() {
        return Err(bad(ID, "canonical imported merged sampling unsupported"));
    }
    crate::selection_inverse_controlled::selection_write_authority_metadata(doc, c)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, c).map_err(|e| error(ID, c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, c).map_err(|e| error(ID, c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    // Both low-resolution strips and refinement halos can invoke merged render.
    // Charge the larger call's scratch/preparation, and all possible calls' work.
    let (mut render_extra, mut render_work) = (0u64, 0u64);
    if direct.is_none() && !req.window.is_empty() {
        let strip_h =
            if req.step == 1 { req.window.height() as usize } else { (64usize.div_ceil(req.step).max(1) * req.step).min(req.window.height() as usize) }.max(1);
        let win_w = 216usize.min(req.window.width() as usize).max(1);
        let win_h = 216usize.min(req.window.height() as usize).max(1);
        let strip = Rect::from_xywh(req.window.x0, req.window.y0, req.window.width().max(1), strip_h as u32);
        let win = Rect::from_xywh(req.window.x0, req.window.y0, win_w as u32, win_h as u32);
        let a = photocraft_compose::ordinary_render_requirements(doc, strip, c).map_err(|e| error(ID, c, e))?;
        let b = photocraft_compose::ordinary_render_requirements(doc, win, c).map_err(|e| error(ID, c, e))?;
        render_extra = (a.scratch_bytes as u64)
            .checked_add(a.preparation_reserve_bytes as u64)
            .and_then(|v| Some(v.max((b.scratch_bytes as u64).checked_add(b.preparation_reserve_bytes as u64)?)))
            .ok_or_else(|| bad(ID, "render resident overflow"))?;
        let strips = (req.window.height() as usize).div_ceil(strip_h) as u64;
        let tiles =
            u64::from(req.window.width()).div_ceil(192).checked_mul(u64::from(req.window.height()).div_ceil(192)).ok_or_else(|| bad(ID, "render visits"))?;
        render_work =
            a.work_units.checked_mul(strips).and_then(|v| v.checked_add(b.work_units.checked_mul(tiles)?)).ok_or_else(|| bad(ID, "render work overflow"))?;
    }
    let prior = doc.selection.as_ref().map_or(Ok(Rect::EMPTY), |s| s.try_content_bounds_with_interrupt(c).map_err(|e| error(ID, c, e)))?;
    let extent = if matches!(r.mode, SelectionMode::Add | SelectionMode::Subtract) { canvas.union(&prior) } else { canvas };
    let pixels = extent.size().area();
    let sample_bytes = doc.selection.as_ref().map_or(1, |s| s.format().bytes_per_pixel()) as u64;
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|v| v.checked_mul(sample_bytes)).ok_or_else(|| bad(ID, "selection tile overflow"))?;
    let first = photocraft_geom::TileCoord::containing(extent.x0, extent.y0);
    let last = photocraft_geom::TileCoord::containing(
        extent.x1.checked_sub(1).ok_or_else(|| bad(ID, "selection bounds"))?,
        extent.y1.checked_sub(1).ok_or_else(|| bad(ID, "selection bounds"))?,
    );
    let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|w| w.checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok()?))
        .ok_or_else(|| bad(ID, "selection tiles"))?;
    let selection_bytes = tiles
        .checked_mul(tile_bytes.checked_add(1024).ok_or_else(|| bad(ID, "selection map"))?)
        .and_then(|v| v.checked_add(4096 * sample_bytes))
        .ok_or_else(|| bad(ID, "selection staging overflow"))?;
    raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|v| v.checked_add(model.checked_mul(5)?))
        .and_then(|v| v.checked_add(payload.checked_mul(2)?))
        .and_then(|v| v.checked_add(req.peak_resident_bytes))
        .and_then(|v| v.checked_add(render_extra))
        .and_then(|v| v.checked_add(selection_bytes))
        .filter(|v| *v <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(ID, "aggregate Object Selection resident limit"))?;
    let overhead = Some(render_work)
        .and_then(|v| v.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|v| v.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(pixels.checked_mul(32)?))
        .and_then(|v| v.checked_add(selection_bytes.checked_mul(16)?))
        .ok_or_else(|| bad(ID, "aggregate Object Selection work overflow"))?;
    let work_units = MAX_WORK.checked_sub(overhead).filter(|v| *v >= req.work_units).ok_or_else(|| bad(ID, "aggregate Object Selection work limit"))?;
    poll(c)?;
    Ok((req, ObjectSelectionLimits { resident_bytes: req.peak_resident_bytes, work_units }))
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
struct Prepared {
    selection: Option<Surface>,
    response: Value,
    change: bool,
}
fn prepare(doc: &Document, active: Option<LayerId>, r: Request, c: &Interrupt<'_>) -> Result<Prepared> {
    let (_req, limits) = admission(doc, active, r, c)?;
    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
    let direct = if r.all { None } else { find_surface(doc, active, c)? };
    let sampler = BoundedSampler { doc, direct, control: c };
    let region = grabcut::object_select_with_interrupt(&sampler, doc.bounds(), r.rect, 160_000, limits, c).map_err(|e| error(ID, c, e))?;
    let Some(region) = region else {
        return Ok(Prepared { selection: None, change: false, response: json!({"selected":doc.selection.is_some(),"changed":false}) });
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
    poll(c)?;
    let response = json!({"selected":selection.is_some(),"changed":true,"bounds":[b.x0,b.y0,b.width()as i32,b.height()as i32]});
    let change = !same(doc.selection.as_ref(), selection.as_ref(), c)?;
    Ok(Prepared { selection, response, change })
}
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let r = request(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let active = st.active_layer;
    let view = copy_view(&st.channel_view, ID)?;
    let color = photocraft_color::convert::active_cmyk_space();
    crate::jobs::run(
        s,
        "Object Selection",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Object Selection", |c| {
                photocraft_color::convert::with_cmyk_space(color.as_ref(), || Ok((prepare(&worker, active, r, c)?, ctx.clone())))
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
                return Err(bad(ID, "Object Selection target changed during preparation"));
            }
            ctx.stage(0.95, 1., "Object Selection", |c| {
                poll(c)?;
                crate::selection_inverse_controlled::selection_write_authority(&s.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if !prepared.change {
                    return Ok(prepared.response);
                }
                let response = prepared.response;
                s.edit_with_interrupt("Object Selection", c, move |doc, _| {
                    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                    doc.selection = prepared.selection;
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
    use photocraft_color::ColorMode;
    use photocraft_geom::Size;
    fn doc() -> Document {
        let mut d = Document::new("Object", Size::new(80, 64), ColorMode::Rgb, SampleType::U16);
        let mut l = Layer::raster("Pixels", d.pixel_format());
        let mut v = Vec::new();
        for y in 0..64 {
            for x in 0..80 {
                let c = if (x as i32 - 40).pow(2) + (y as i32 - 32).pow(2) < 18 * 18 { [0.9, 0.1, 0.1, 1.] } else { [0.1, 0.3, 0.8, 1.] };
                v.extend_from_slice(&c);
            }
        }
        l.surface_mut().unwrap().write_region(d.bounds(), &v);
        d.layers.push(l);
        d
    }
    #[test]
    fn rectangle_private_matches_legacy_and_cancel_preserves() {
        let d = doc();
        let active = Some(d.layers[0].id);
        let r = Request { rect: Rect::from_xywh(18, 10, 44, 44), all: false, mode: SelectionMode::Replace };
        let source = photocraft_algo::segment::SurfaceSampler(d.layers[0].surface().unwrap());
        let region = grabcut::object_select(&source, d.bounds(), r.rect, 160_000).unwrap();
        let expected = sel::combine_region(d.selection.as_ref(), region.as_ref(), r.mode);
        let actual = prepare(&d, active, r, &Interrupt::NONE).unwrap();
        assert_eq!(actual.selection, expected);
        assert!(matches!(prepare(&d, active, r, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
    #[test]
    fn registered_rectangle_one_undo_and_stale_redo_retention() {
        let mut s = Session::new();
        s.add_document(doc(), None).unwrap();
        let before = s.active().unwrap().doc.clone();
        s.execute(ID, json!({"rect":[18,10,44,44]})).unwrap();
        let after = s.active().unwrap().doc.clone();
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, *before);
        assert!(s.execute(ID, json!({"rect":[18,10,44,44],"documentId":before.id.0.to_string(),"expectedRevision":"18446744073709551615"})).is_err());
        assert!(s.redo());
        assert_eq!(*s.active().unwrap().doc, *after);
    }
}
