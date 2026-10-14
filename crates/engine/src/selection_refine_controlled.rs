//! Controlled ordinary selection-output Refine Edge. Other output routes remain
//! upstream-owned; no native-wide interpretation or source-render qualification.
use crate::channel_cmds::ChannelView;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::segment::Sampler;
use photocraft_algo::{
    matting::{self, MattingLimits, RefineParams},
    selection::{self as sel, SelectionMode, WandLimits},
};
use photocraft_color::{ColorMode, SampleType, read_sample, try_read_sample_f64};
use photocraft_doc::{Document, Layer, LayerId, MAX_GROUP_DEPTH};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E, Surface};
use serde_json::{Value, json};
const ID: &str = "select.refineEdge";
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
    *stack.first_mut().ok_or_else(|| bad("select.refineEdge", "hierarchy"))? = Some(doc.layers.iter());
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
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad("select.refineEdge", "owner count limit"))?;
        if layer.id == id {
            return Ok(layer.surface());
        }
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad("select.refineEdge", "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad("select.refineEdge", "hierarchy"))? = Some(children.iter());
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

#[derive(Clone, Copy)]
struct Request {
    params: RefineParams,
    all: bool,
    target_mask: bool,
    eligible: bool,
}
fn scalar(p: &Value, k: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
    let n = p.get(k).map_or(Ok(f64::from(default)), |v| v.as_f64().ok_or_else(|| bad(ID, "numeric parameter required")))?;
    if !n.is_finite() || n.abs() > f64::from(f32::MAX) {
        return Err(bad(ID, "finite f32 parameter required"));
    }
    Ok((n as f32).clamp(lo, hi))
}
fn flag(p: &Value, k: &str, d: bool) -> Result<bool> {
    p.get(k).map_or(Ok(d), |v| v.as_bool().ok_or_else(|| bad(ID, "boolean parameter required")))
}
fn text<'a>(p: &'a Value, k: &str, d: &'a str) -> Result<&'a str> {
    p.get(k).map_or(Ok(d), |v| v.as_str().filter(|s| s.len() <= 32 && !s.contains('\0')).ok_or_else(|| bad(ID, "bounded string required")))
}
fn source_mask<'a>(doc: &'a Document, active: Option<LayerId>, target_mask: bool, c: &Interrupt<'_>) -> Result<Option<&'a Surface>> {
    // Mask lookup follows active layer identity, without a recursive doc.layer walk.
    let mut mask = None;
    if let Some(id) = active {
        let mut stack: [Option<std::slice::Iter<'a, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
        *stack.first_mut().ok_or_else(|| bad(ID, "hierarchy"))? = Some(doc.layers.iter());
        let (mut depth, mut visits) = (0usize, 0usize);
        loop {
            poll(c)?;
            let next = stack.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
            let Some(l) = next else {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                continue;
            };
            visits = visits.checked_add(1).filter(|v| *v <= 100000).ok_or_else(|| bad(ID, "owner limit"))?;
            if l.id == id {
                mask = l.mask.as_ref().map(|m| m.surface.as_ref());
                break;
            }
            if let Some(children) = l.children() {
                depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad(ID, "depth limit"))?;
                *stack.get_mut(depth).ok_or_else(|| bad(ID, "hierarchy"))? = Some(children.iter());
            }
        }
    }
    Ok(if target_mask { mask.or(doc.selection.as_ref()) } else { doc.selection.as_ref().or(mask) })
}
fn json_shape(v: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<()> {
    *nodes = nodes.checked_add(1).filter(|v| *v <= 400000).ok_or_else(|| bad(ID, "parameter node limit"))?;
    *bytes = bytes.checked_add(std::mem::size_of::<Value>()).filter(|v| *v <= 16 << 20).ok_or_else(|| bad(ID, "parameter byte limit"))?;
    if depth > 6 {
        return Err(bad(ID, "parameter nesting limit"));
    }
    match v {
        Value::String(t) => {
            *bytes = bytes.checked_add(t.len()).filter(|v| *v <= 16 << 20).ok_or_else(|| bad(ID, "parameter byte limit"))?;
        }
        Value::Array(a) => {
            if a.len() > 100000 {
                return Err(bad(ID, "parameter array limit"));
            }
            for v in a {
                json_shape(v, depth + 1, nodes, bytes)?;
            }
        }
        Value::Object(o) => {
            if o.len() > 32 {
                return Err(bad(ID, "parameter object limit"));
            }
            for (k, v) in o {
                *bytes = bytes.checked_add(k.len()).filter(|v| *v <= 16 << 20).ok_or_else(|| bad(ID, "parameter byte limit"))?;
                json_shape(v, depth + 1, nodes, bytes)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn brush_header(p: &Value) -> Result<(bool, bool)> {
    let Some(v) = p.get("refineStrokes").filter(|v| !v.is_null()) else {
        return Ok((false, false));
    };
    let a = v.as_array().filter(|a| a.len() <= 4096).ok_or_else(|| bad(ID, "bounded stroke array required"))?;
    let (mut total, mut adds) = (0usize, false);
    for v in a {
        let o = v.as_object().ok_or_else(|| bad(ID, "stroke object required"))?;
        let points = o.get("points").and_then(Value::as_array).filter(|p| !p.is_empty()).ok_or_else(|| bad(ID, "stroke points required"))?;
        total = total.checked_add(points.len()).filter(|v| *v <= 100000).ok_or_else(|| bad(ID, "stroke point limit"))?;
        for xy in points {
            let a = xy.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(ID, "stroke point pair required"))?;
            for v in a {
                let n = v.as_f64().filter(|v| v.is_finite() && v.abs() <= 1e7).ok_or_else(|| bad(ID, "finite bounded stroke point required"))?;
                let _ = n;
            }
        }
        if let Some(v) = o.get("size") {
            let _ = v.as_f64().filter(|v| v.is_finite() && (1.0..=5000.0).contains(v)).ok_or_else(|| bad(ID, "stroke size range"))?;
        }
        match o.get("mode") {
            None => adds = true,
            Some(v) if v.as_str() == Some("add") => adds = true,
            Some(v) if v.as_str() == Some("subtract") => {}
            _ => return Err(bad(ID, "stroke mode must be add|subtract")),
        }
    }
    Ok((!a.is_empty(), adds))
}

fn request(s: &Session, p: &Value) -> Result<Request> {
    if !p.is_null()
        && !p.as_object().is_some_and(|o| {
            o.len() <= 15
                && o.keys().all(|k| {
                    matches!(
                        k.as_str(),
                        "radius"
                            | "smartRadius"
                            | "smooth"
                            | "feather"
                            | "contrast"
                            | "shiftEdge"
                            | "decontaminate"
                            | "amount"
                            | "output"
                            | "sampleAllLayers"
                            | "refineStrokes"
                            | "target"
                            | "documentId"
                            | "expectedRevision"
                    )
                })
        })
    {
        return Err(bad(ID, "bounded Refine parameters required"));
    }
    let params = RefineParams {
        radius: scalar(p, "radius", 0., 0., 250.)?,
        smart_radius: flag(p, "smartRadius", false)?,
        smooth: scalar(p, "smooth", 0., 0., 100.)?,
        feather: scalar(p, "feather", 0., 0., 1000.)?,
        contrast: scalar(p, "contrast", 0., 0., 100.)?,
        shift_edge: scalar(p, "shiftEdge", 0., -100., 100.)?,
    };
    json_shape(p, 0, &mut 0, &mut 0)?;
    let (brush, brush_adds) = brush_header(p)?;
    let all = flag(p, "sampleAllLayers", false)?;
    let decontaminate = flag(p, "decontaminate", false)?;
    let _ = scalar(p, "amount", 100., 0., 100.)?;
    let target_mask = text(p, "target", "")? == "mask";
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let prior = source_mask(&st.doc, st.active_layer, target_mask, &Interrupt::NONE)?;
    // Guard precisely the prior coverage owner and requested sampling owner before
    // even the legacy other-output/brush route can interpret a wide raw word.
    if let Some(m) = prior {
        if m.format().mode != ColorMode::Grayscale || m.format().alpha || matches!(m.format().sample, SampleType::U32 | SampleType::F64) {
            return Err(bad(ID, "native prior coverage interpretation unsupported"));
        }
    }
    if prior.is_some() && (params.radius > 0.0 || brush_adds) {
        crate::selection_sample_policy::sampled_pixels(s, ID, all, &Interrupt::NONE)?;
    }
    let default_output =
        if st.doc.selection.is_none() && source_mask(&st.doc, st.active_layer, true, &Interrupt::NONE)?.is_some() { "layerMask" } else { "selection" };
    let output = text(p, "output", default_output)?;
    if !matches!(output, "selection" | "layerMask" | "newLayer" | "newLayerWithMask" | "newDocument" | "newDocumentWithMask") {
        return Err(bad(ID, "unsupported output"));
    }
    // Other outputs use f32-based pixel helpers and may replace native mask storage.
    // Their existing legacy route cannot promise a raw native output operation.
    if output == "layerMask"
        && source_mask(&st.doc, st.active_layer, true, &Interrupt::NONE)?.is_some_and(|v| matches!(v.format().sample, SampleType::U32 | SampleType::F64))
    {
        return Err(bad(ID, "native mask output replacement unsupported"));
    }
    if output != "selection" || decontaminate {
        if matches!(st.doc.depth, SampleType::U32 | SampleType::F64)
            || find_surface(&st.doc, st.active_layer, &Interrupt::NONE)?.is_some_and(|v| matches!(v.format().sample, SampleType::U32 | SampleType::F64))
        {
            return Err(bad(ID, "native Refine output interpretation unsupported"));
        }
    }
    if output == "selection" && !decontaminate {
        crate::selection_sample_policy::existing_selection(s, ID, &Interrupt::NONE)?;
    }
    let eligible = output == "selection" && !decontaminate && !brush;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if exact(id)? == st.doc.id.0 && exact(rev)? == st.revision && eligible => {}
        _ => return Err(bad(ID, "captured target changed or output route is not controlled")),
    }
    if eligible {
        if st.revision == u64::MAX
            || st.floating.is_some()
            || st.coalesce.is_some()
            || st.doc.accounted().is_some()
            || st.doc.xcf_state.is_some()
            || st.doc.source_color_context.is_some()
            || s.coalesce_request.is_some()
            || s.color_restrict.is_some()
            || s.command_has_job_on(st.doc.id)
            || s.command_has_pending_job()
        {
            return Err(bad(ID, "controlled Refine requires idle ordinary resident source"));
        }
        crate::selection_inverse_controlled::selection_write_authority_metadata(&st.doc, &Interrupt::NONE)?;
    }
    Ok(Request { params, all, target_mask, eligible })
}
pub(crate) fn handles(s: &Session, p: &Value) -> Result<bool> {
    Ok(request(s, p)?.eligible)
}
pub(crate) fn dispatch_preflight(s: &Session, p: &Value, _background: bool) -> Result<()> {
    request(s, p).map(|_| ())
}
fn mask_read(source: &Surface, r: Rect, c: &Interrupt<'_>) -> std::result::Result<Vec<f32>, E> {
    c.check().map_err(|_| E::Cancelled)?;
    let fmt = source.format();
    if fmt.mode != ColorMode::Grayscale || fmt.alpha || matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Err(E::UnsupportedConversion);
    }
    let n = (r.width() as usize).checked_mul(r.height() as usize).ok_or(E::LimitExceeded)?;
    if (n as u64).checked_mul(4).is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES) {
        return Err(E::LimitExceeded);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    let mut raw = [0u8; 8];
    let bytes = raw.get_mut(..fmt.bytes_per_pixel()).ok_or(E::LengthMismatch)?;
    for y in r.y0..r.y1 {
        c.check().map_err(|_| E::Cancelled)?;
        for x in r.x0..r.x1 {
            if x.rem_euclid(1024) == 0 {
                c.check().map_err(|_| E::Cancelled)?;
            }
            source.try_read_pixel_bytes(x, y, bytes)?;
            let v = read_sample(bytes, fmt.sample, 0);
            if !v.is_finite() {
                return Err(E::InvalidSample);
            }
            out.push(v);
        }
    }
    c.check().map_err(|_| E::Cancelled)?;
    Ok(out)
}
fn admission(doc: &Document, active: Option<LayerId>, r: Request, c: &Interrupt<'_>) -> Result<matting::RefineRequirements> {
    poll(c)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, c).map_err(|e| error(ID, c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, c).map_err(|e| error(ID, c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let source = source_mask(doc, active, r.target_mask, c)?;
    let content = source.map_or(Ok(Rect::EMPTY), |m| m.try_content_bounds_with_interrupt(c).map_err(|e| error(ID, c, e)))?;
    let req = matting::refine_requirements(content, doc.bounds(), &r.params).map_err(|e| error(ID, c, e))?;
    let direct = if r.all { None } else { find_surface(doc, active, c)? };
    let (mut render_bytes, mut render_work) = (0u64, 0u64);
    if direct.is_none() && !req.bbox.is_empty() && r.params.radius > 0. {
        let area = Rect::new(0, 0, req.window_width as i32, req.window_height as i32);
        let q = photocraft_compose::ordinary_render_requirements(doc, area, c).map_err(|e| error(ID, c, e))?;
        render_bytes = (q.scratch_bytes as u64).checked_add(q.preparation_reserve_bytes as u64).ok_or_else(|| bad(ID, "render resident overflow"))?;
        let tiles =
            u64::from(req.bbox.width()).div_ceil(256).checked_mul(u64::from(req.bbox.height()).div_ceil(256)).ok_or_else(|| bad(ID, "render visits"))?;
        render_work = q.work_units.checked_mul(tiles).ok_or_else(|| bad(ID, "render work overflow"))?;
    }
    let tiles = u64::from(req.bbox.width())
        .div_ceil(TILE_SIZE as u64)
        .checked_add(1)
        .and_then(|w| w.checked_mul(u64::from(req.bbox.height()).div_ceil(TILE_SIZE as u64).checked_add(1)?))
        .ok_or_else(|| bad(ID, "selection map count"))?;
    let output = tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) + 1024).ok_or_else(|| bad(ID, "selection output overflow"))?;
    raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|v| v.checked_add(model.checked_mul(5)?))
        .and_then(|v| v.checked_add(payload.checked_mul(2)?))
        .and_then(|v| v.checked_add(req.peak_resident_bytes))
        .and_then(|v| v.checked_add(render_bytes))
        .and_then(|v| v.checked_add(output))
        .filter(|v| *v <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(ID, "aggregate Refine resident limit"))?;
    req.work_units
        .checked_add(render_work)
        .and_then(|v| v.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|v| v.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(output.checked_mul(16)?))
        .filter(|v| *v <= MAX_WORK)
        .ok_or_else(|| bad(ID, "aggregate Refine work limit"))?;
    poll(c)?;
    Ok(req)
}
struct Prepared {
    selection: Option<Surface>,
    response: Value,
    change: bool,
}
fn prepare(doc: &Document, active: Option<LayerId>, r: Request, c: &Interrupt<'_>) -> Result<Prepared> {
    let req = admission(doc, active, r, c)?;
    crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
    let source = source_mask(doc, active, r.target_mask, c)?;
    let region = if let Some(source) = source {
        let direct = if r.all { None } else { find_surface(doc, active, c)? };
        let sampler = BoundedSampler { doc, direct, control: c };
        matting::refine_mask_with_interrupt(
            &sampler,
            &|area| mask_read(source, area, c),
            source.try_content_bounds_with_interrupt(c).map_err(|e| error(ID, c, e))?,
            doc.bounds(),
            &r.params,
            MattingLimits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units },
            c,
        )
        .map_err(|e| error(ID, c, e))?
    } else {
        None
    };
    let selection = sel::try_combine_region_with_interrupt(
        None,
        region.as_ref(),
        SelectionMode::Replace,
        WandLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK },
        c,
    )
    .map_err(|e| error(ID, c, e))?;
    let change = !same(doc.selection.as_ref(), selection.as_ref(), c)?;
    let response = json!({"output":"selection","selected":selection.is_some()});
    poll(c)?;
    Ok(Prepared { selection, response, change })
}
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let r = request(s, p)?;
    if !r.eligible {
        return Err(bad(ID, "output route is not controlled"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let active = st.active_layer;
    let view = copy_view(&st.channel_view, ID)?;
    let color = photocraft_color::convert::active_cmyk_space();
    crate::jobs::run(
        s,
        "Select and Mask",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Select and Mask", |c| {
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
                || st.floating.is_some()
                || st.coalesce.is_some()
                || s.coalesce_request.is_some()
                || s.color_restrict.is_some()
                || s.command_has_pending_job()
                || s.command_has_job_on(st.doc.id)
            {
                return Err(bad(ID, "Refine target changed during preparation"));
            }
            ctx.stage(0.95, 1., "Select and Mask", |c| {
                poll(c)?;
                crate::selection_inverse_controlled::selection_write_authority(&s.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if !prepared.change {
                    return Ok(prepared.response);
                }
                let response = prepared.response;
                s.edit_with_interrupt("Select and Mask", c, move |doc, _| {
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
    use photocraft_color::PixelFormat;
    fn document() -> Document {
        let mut d = Document::new("Refine draft", photocraft_geom::Size::new(23, 17), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("source", d.pixel_format());
        layer.surface_mut().unwrap().write_region(d.bounds(), &vec![0.3; 23 * 17 * 4]);
        d.layers.push(layer);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        let bits = (0..23 * 17).map(|i| if i % 23 < 11 { 255 } else { 0 }).collect::<Vec<_>>();
        mask.try_write_interleaved(d.bounds(), &bits).unwrap();
        d.selection = Some(mask);
        d
    }
    #[test]
    fn direct_current_refine_matches_original_and_preserves_pixels() {
        let d = document();
        let params = RefineParams { radius: 4., smart_radius: true, smooth: 17., feather: 2., contrast: 22., shift_edge: 12.5 };
        let r = Request { params, all: false, target_mask: false, eligible: true };
        let before = d.clone();
        let actual = prepare(&d, Some(d.layers[0].id), r, &Interrupt::NONE).unwrap();
        let old = d.selection.as_ref().unwrap();
        let sampler = photocraft_algo::segment::SurfaceSampler(d.layers[0].surface().unwrap());
        let region = matting::refine_mask(&sampler, &matting::surface_reader(old), old.content_bounds(), d.bounds(), &params).unwrap();
        let surface = region.as_ref().map(matting::region_surface);
        assert!(same(actual.selection.as_ref(), surface.as_ref(), &Interrupt::NONE).unwrap());
        assert_eq!(d, before);
    }
    #[test]
    fn actual_target_mask_precedence_and_native_read_guard() {
        let mut d = document();
        let mut m = Surface::new(PixelFormat::GRAY8);
        m.try_write_interleaved(Rect::new(4, 3, 7, 8), &[255; 15]).unwrap();
        d.layers[0].mask = Some(photocraft_doc::LayerMask { surface: std::sync::Arc::new(m), ..photocraft_doc::LayerMask::reveal_all() });
        let id = Some(d.layers[0].id);
        assert_eq!(source_mask(&d, id, false, &Interrupt::NONE).unwrap(), d.selection.as_ref());
        assert_eq!(source_mask(&d, id, true, &Interrupt::NONE).unwrap(), d.layers[0].mask.as_ref().map(|m| m.surface.as_ref()));
        let mut s = Session::new();
        s.add_document(d, None).unwrap();
        s.active_mut().unwrap().doc.try_resident_mut().unwrap().layers[0].mask.as_mut().unwrap().surface =
            std::sync::Arc::new(Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false)));
        assert!(dispatch_preflight(&s, &json!({"target":"mask","output":"selection"}), true).is_err());
    }
    #[test]
    fn captured_other_output_refuses_and_cancel_never_mutates_snapshot() {
        let mut s = Session::new();
        s.add_document(document(), None).unwrap();
        let st = s.active().unwrap();
        let old = st.doc.clone();
        let rev = st.revision;
        assert!(handles(&s, &json!({"output":"layerMask","documentId":old.id.0.to_string(),"expectedRevision":rev.to_string()})).is_err());
        let r = Request { params: RefineParams::default(), all: false, target_mask: false, eligible: true };
        assert!(matches!(prepare(&old, st.active_layer, r, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(old.shares_state(&s.active().unwrap().doc));
        assert_eq!(rev, s.active().unwrap().revision);
    }
    #[test]
    fn unchanged_default_refine_keeps_real_redo() {
        let mut s = Session::new();
        s.add_document(document(), None).unwrap();
        s.edit("seed redo", |d, _| {
            d.name = "edited".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        run(&mut s, &Value::Null).unwrap();
        assert!(before.shares_state(&s.active().unwrap().doc));
        assert_eq!(revision, s.active().unwrap().revision);
        assert!(s.redo());
    }
}
