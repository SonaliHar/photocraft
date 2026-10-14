//! Controlled ordinary Refine outputs including color decontamination.
//! Unported brush routes retain upstream behavior; no native-wide
//! interpretation or source-render qualification is added.
use crate::channel_cmds::ChannelView;
use crate::{DocState, DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::segment::Sampler;
use photocraft_algo::{
    matting::{self, MattingLimits, RefineParams},
    selection::{self as sel, SelectionMode, WandLimits},
};
use photocraft_color::{ColorMode, SampleType, read_sample, try_read_sample_f64};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask, MAX_GROUP_DEPTH};
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
fn find_owner<'a>(doc: &'a Document, id: Option<LayerId>, ctl: &Interrupt<'_>) -> Result<Option<&'a Layer>> {
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
            return Ok(Some(layer));
        }
        if let Some(children) = layer.children() {
            depth = depth.checked_add(1).filter(|d| *d <= MAX_GROUP_DEPTH).ok_or_else(|| bad("select.refineEdge", "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad("select.refineEdge", "hierarchy"))? = Some(children.iter());
        }
    }
    Ok(None)
}
fn find_surface<'a>(doc: &'a Document, id: Option<LayerId>, c: &Interrupt<'_>) -> Result<Option<&'a Surface>> {
    Ok(find_owner(doc, id, c)?.and_then(Layer::surface))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Selection,
    LayerMask,
    NewLayer,
    NewLayerWithMask,
    NewDocument,
    NewDocumentWithMask,
}
impl Output {
    fn text(self) -> &'static str {
        match self {
            Self::Selection => "selection",
            Self::LayerMask => "layerMask",
            Self::NewLayer => "newLayer",
            Self::NewLayerWithMask => "newLayerWithMask",
            Self::NewDocument => "newDocument",
            Self::NewDocumentWithMask => "newDocumentWithMask",
        }
    }
    fn document(self) -> bool {
        matches!(self, Self::NewDocument | Self::NewDocumentWithMask)
    }
    fn with_mask(self) -> bool {
        matches!(self, Self::NewLayerWithMask | Self::NewDocumentWithMask)
    }
}
#[derive(Clone, Copy)]
struct Request {
    params: RefineParams,
    all: bool,
    target_mask: bool,
    eligible: bool,
    output: Output,
    decontaminate: bool,
    amount: f32,
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
    let amount = scalar(p, "amount", 100., 0., 100.)?;
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
        && !decontaminate
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
    let eligible = !brush;
    let output_kind = match if decontaminate && matches!(output, "selection" | "layerMask") { "newLayerWithMask" } else { output } {
        "selection" => Output::Selection,
        "layerMask" => Output::LayerMask,
        "newLayer" => Output::NewLayer,
        "newLayerWithMask" => Output::NewLayerWithMask,
        "newDocument" => Output::NewDocument,
        "newDocumentWithMask" => Output::NewDocumentWithMask,
        _ => return Err(bad(ID, "unsupported output")),
    };
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
    if eligible && output_kind != Output::Selection {
        let id = st.active_layer.ok_or_else(|| bad(ID, "no active layer"))?;
        let owner = find_owner(&st.doc, Some(id), &Interrupt::NONE)?.ok_or(EngineError::NoLayer(id))?;
        if output_kind != Output::LayerMask && owner.surface().is_none() {
            return Err(bad(ID, "active layer has no pixels"));
        }
        photocraft_doc::resident_structure::visit_layers_with(&st.doc, &Interrupt::NONE, |_| Ok(true))?;
        if output_kind == Output::LayerMask {
            crate::xcf_item_locks::require_layer_mask_content(&st.doc, id)?;
        }
    }
    Ok(Request { params, all, target_mask, eligible, output: output_kind, decontaminate, amount })
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
    if r.output != Output::Selection {
        let _ = identities(doc, c)?;
    }
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
    let (copy_bytes, copy_work) = if matches!(r.output, Output::NewLayer | Output::NewDocument) {
        let pixels = find_surface(doc, active, c)?.ok_or_else(|| bad(ID, "active layer has no pixels"))?;
        matting::masked_copy_requirements(pixels, req.bbox).map_err(|e| error(ID, c, e))?
    } else {
        (0, 0)
    };
    let (decontaminate_bytes, decontaminate_work) = if r.decontaminate {
        let pixels = find_surface(doc, active, c)?.ok_or_else(|| bad(ID, "active layer has no pixels"))?;
        let d = matting::decontaminate_requirements(pixels, req.bbox, r.params.radius, c).map_err(|e| error(ID, c, e))?;
        (d.peak_resident_bytes, d.work_units)
    } else {
        (0, 0)
    };
    raster
        .retained_bytes
        .checked_mul(2)
        .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|v| v.checked_add(model.checked_mul(5)?))
        .and_then(|v| v.checked_add(payload.checked_mul(2)?))
        .and_then(|v| v.checked_add(req.peak_resident_bytes))
        .and_then(|v| v.checked_add(render_bytes))
        .and_then(|v| v.checked_add(output))
        .and_then(|v| v.checked_add((std::mem::size_of::<DocState>() as u64).checked_mul(8192 * 2)?))
        .and_then(|v| v.checked_add(copy_bytes))
        .and_then(|v| v.checked_add(decontaminate_bytes))
        .filter(|v| *v <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad(ID, "aggregate Refine resident limit"))?;
    req.work_units
        .checked_add(render_work)
        .and_then(|v| v.checked_add(copy_work))
        .and_then(|v| v.checked_add(decontaminate_work))
        .and_then(|v| v.checked_add(model.checked_add(payload)?.checked_mul(8)?))
        .and_then(|v| v.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|v| v.checked_add(output.checked_mul(16)?))
        .filter(|v| *v <= MAX_WORK)
        .ok_or_else(|| bad(ID, "aggregate Refine work limit"))?;
    poll(c)?;
    Ok(req)
}
fn owned(s: &str, cap: usize, c: &Interrupt<'_>) -> Result<String> {
    poll(c)?;
    if s.len() > cap || s.contains('\0') {
        return Err(bad(ID, "bounded metadata text required"));
    }
    let mut out = String::new();
    out.try_reserve_exact(s.len()).map_err(|_| bad(ID, "metadata text allocation"))?;
    if out.capacity() > s.len().saturating_add(16) {
        return Err(bad(ID, "metadata text capacity"));
    }
    let mut start = 0usize;
    while start < s.len() {
        poll(c)?;
        let mut end = start.saturating_add(4096).min(s.len());
        while !s.is_char_boundary(end) {
            end = end.checked_sub(1).ok_or_else(|| bad(ID, "metadata boundary"))?;
        }
        out.push_str(s.get(start..end).ok_or_else(|| bad(ID, "metadata boundary"))?);
        start = end;
    }
    poll(c)?;
    Ok(out)
}
fn copied_vec<T: Clone>(v: &[T], cap: usize, c: &Interrupt<'_>) -> Result<Vec<T>> {
    poll(c)?;
    if v.len() > cap {
        return Err(bad(ID, "metadata vector limit"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(v.len()).map_err(|_| bad(ID, "metadata vector allocation"))?;
    if out.capacity() > v.len().saturating_add(16) {
        return Err(bad(ID, "metadata vector capacity"));
    }
    for chunk in v.chunks(1024) {
        poll(c)?;
        out.extend_from_slice(chunk);
    }
    poll(c)?;
    Ok(out)
}
fn copy_palette(p: Option<&photocraft_doc::ColorTable>, c: &Interrupt<'_>) -> Result<Option<photocraft_doc::ColorTable>> {
    p.map(|p| Ok(photocraft_doc::ColorTable { colors: copied_vec(&p.colors, 256, c)?, transparent: p.transparent })).transpose()
}
fn copy_duotone(d: Option<&photocraft_doc::Duotone>, c: &Interrupt<'_>) -> Result<Option<photocraft_doc::Duotone>> {
    let Some(d) = d else { return Ok(None) };
    if d.inks.len() > 4 {
        return Err(bad(ID, "duotone ink limit"));
    }
    let mut inks = Vec::new();
    inks.try_reserve_exact(d.inks.len()).map_err(|_| bad(ID, "duotone allocation"))?;
    for i in &d.inks {
        poll(c)?;
        inks.push(photocraft_doc::DuotoneInk { name: owned(&i.name, 4096, c)?, color: i.color, curve: copied_vec(&i.curve, 100000, c)? });
    }
    let psd_raw = d.psd_raw.as_ref().map(|v| copied_vec(v, 64 << 20, c)).transpose()?;
    poll(c)?;
    Ok(Some(photocraft_doc::Duotone { inks, psd_raw }))
}
fn copy_resolution(r: Option<&photocraft_doc::PhysicalResolution>, c: &Interrupt<'_>) -> Result<Option<photocraft_doc::PhysicalResolution>> {
    use photocraft_doc::{CustomUnit, PhysicalResolution, PhysicalUnit};
    let Some(r) = r else { return Ok(None) };
    r.validate().map_err(|e| error(ID, c, e))?;
    poll(c)?;
    let unit = match &r.unit {
        PhysicalUnit::Pixel => PhysicalUnit::Pixel,
        PhysicalUnit::Inch => PhysicalUnit::Inch,
        PhysicalUnit::Millimeter => PhysicalUnit::Millimeter,
        PhysicalUnit::Point => PhysicalUnit::Point,
        PhysicalUnit::Pica => PhysicalUnit::Pica,
        PhysicalUnit::Custom(u) => PhysicalUnit::Custom(CustomUnit {
            name: owned(&u.name, 1 << 20, c)?,
            factor: u.factor,
            digits: u.digits,
            symbol: owned(&u.symbol, 1 << 20, c)?,
            abbreviation: owned(&u.abbreviation, 1 << 20, c)?,
        }),
    };
    poll(c)?;
    Ok(Some(PhysicalResolution { x_dpi: r.x_dpi, y_dpi: r.y_dpi, unit }))
}
fn copy_working_profile(p: &photocraft_doc::WorkingProfileInterpretation, c: &Interrupt<'_>) -> Result<photocraft_doc::WorkingProfileInterpretation> {
    use photocraft_doc::WorkingProfileInterpretation as P;
    poll(c)?;
    Ok(match p {
        P::Icc => P::Icc,
        P::ImplicitSrgb { icc_blake3 } => P::ImplicitSrgb { icc_blake3: owned(icc_blake3, 64, c)? },
        P::ImplicitSGray { icc_blake3 } => P::ImplicitSGray { icc_blake3: owned(icc_blake3, 64, c)? },
    })
}
fn identities(doc: &Document, c: &Interrupt<'_>) -> Result<std::collections::HashSet<LayerId>> {
    let mut count = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc, c, |_| {
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or(E::LimitExceeded)?;
        Ok(true)
    })?;
    let mut ids = std::collections::HashSet::new();
    ids.try_reserve(count).map_err(|_| bad(ID, "identity census allocation"))?;
    if ids.capacity() > count.saturating_mul(4).saturating_add(64) {
        return Err(bad(ID, "identity census capacity"));
    }
    photocraft_doc::resident_structure::visit_layers_with(doc, c, |layer| {
        if layer.id.0 == 0 || !ids.insert(layer.id) {
            return Err(E::InvalidBounds);
        }
        Ok(true)
    })?;
    poll(c)?;
    Ok(ids)
}
fn copy_name(doc: &Document, name: &str, c: &Interrupt<'_>) -> Result<String> {
    use std::fmt::Write;
    if name.len() > 4096 || name.contains('\0') {
        return Err(bad(ID, "layer name bounds"));
    }
    let root = match name.rsplit_once(" copy") {
        Some((root, rest)) if rest.is_empty() || rest.strip_prefix(' ').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) => root,
        _ => name,
    };
    let mut count = 0usize;
    photocraft_doc::resident_structure::visit_layers_with(doc, c, |_| {
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or(E::LimitExceeded)?;
        Ok(true)
    })?;
    let mut names = std::collections::HashSet::new();
    names.try_reserve(count).map_err(|_| bad(ID, "name census allocation"))?;
    if names.capacity() > count.saturating_mul(4).saturating_add(64) {
        return Err(bad(ID, "name census capacity"));
    }
    // Retain borrowed names through a checked traversal with lifetime fixed here.
    let mut stack: [Option<std::slice::Iter<'_, Layer>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *stack.first_mut().ok_or_else(|| bad(ID, "hierarchy"))? = Some(doc.layers.iter());
    let mut depth = 0usize;
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
        if l.name.len() > 4096 || l.name.contains('\0') {
            return Err(bad(ID, "layer name bounds"));
        }
        names.insert(l.name.as_str());
        if let Some(ch) = l.children() {
            depth = depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH).ok_or_else(|| bad(ID, "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad(ID, "hierarchy"))? = Some(ch.iter());
        }
    }
    let mut out = String::new();
    out.try_reserve_exact(root.len().checked_add(32).ok_or_else(|| bad(ID, "name length"))?).map_err(|_| bad(ID, "copy name allocation"))?;
    for n in 1..=names.len().saturating_add(2) {
        poll(c)?;
        out.clear();
        out.push_str(root);
        out.push_str(" copy");
        if n > 1 {
            write!(&mut out, " {n}").map_err(|_| bad(ID, "copy name formatting"))?;
        }
        if !names.contains(out.as_str()) {
            return Ok(out);
        }
    }
    Err(bad(ID, "copy name exhausted"))
}
fn path(doc: &Document, id: LayerId, c: &Interrupt<'_>) -> Result<Vec<usize>> {
    let mut stack: [Option<std::iter::Enumerate<std::slice::Iter<'_, Layer>>>; MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *stack.first_mut().ok_or_else(|| bad(ID, "hierarchy"))? = Some(doc.layers.iter().enumerate());
    let mut positions = [0usize; MAX_GROUP_DEPTH + 1];
    let (mut depth, mut count) = (0usize, 0usize);
    loop {
        poll(c)?;
        let next = stack.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some((index, l)) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= 100000).ok_or_else(|| bad(ID, "owner limit"))?;
        *positions.get_mut(depth).ok_or_else(|| bad(ID, "hierarchy"))? = index;
        if l.id == id {
            let mut out = Vec::new();
            out.try_reserve_exact(depth + 1).map_err(|_| bad(ID, "owner path allocation"))?;
            out.extend_from_slice(positions.get(..depth + 1).ok_or_else(|| bad(ID, "hierarchy"))?);
            return Ok(out);
        }
        if let Some(ch) = l.children() {
            depth = depth.checked_add(1).filter(|n| *n <= MAX_GROUP_DEPTH).ok_or_else(|| bad(ID, "hierarchy limit"))?;
            *stack.get_mut(depth).ok_or_else(|| bad(ID, "hierarchy"))? = Some(ch.iter().enumerate());
        }
    }
    Err(EngineError::NoLayer(id))
}
fn insert_above(doc: &mut Document, id: LayerId, layer: Layer, c: &Interrupt<'_>) -> Result<()> {
    let location = path(doc, id, c)?;
    let (at, parent) = location.split_last().ok_or_else(|| bad(ID, "owner path"))?;
    let siblings =
        if parent.is_empty() { &mut doc.layers } else { doc.layer_at_mut(parent).and_then(Layer::children_mut).ok_or_else(|| bad(ID, "owner parent"))? };
    let index = at.checked_add(1).filter(|v| *v <= siblings.len()).ok_or_else(|| bad(ID, "insertion index"))?;
    siblings.try_reserve_exact(1).map_err(|_| bad(ID, "layer insertion allocation"))?;
    if siblings.capacity() > 100001 {
        return Err(bad(ID, "layer sibling capacity"));
    }
    poll(c)?;
    siblings.insert(index, layer);
    poll(c)
}
enum Publication {
    Existing(Document, Option<LayerId>),
    NewDocument(DocState, LayerId),
}
struct Prepared {
    selection: Option<Surface>,
    response: Value,
    change: bool,
    publication: Option<Publication>,
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
    if r.output == Output::Selection {
        let change = !same(doc.selection.as_ref(), selection.as_ref(), c)?;
        let response = json!({"output":"selection","selected":selection.is_some()});
        poll(c)?;
        return Ok(Prepared { selection, response, change, publication: None });
    }
    let id = active.ok_or_else(|| bad(ID, "no active layer"))?;
    let source_layer = find_owner(doc, Some(id), c)?.ok_or(EngineError::NoLayer(id))?;
    let mask_surface = selection.unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
    if r.output == Output::LayerMask {
        crate::xcf_item_locks::require_layer_mask_content(doc, id)?;
        let existing = source_layer.mask.clone().unwrap_or_else(LayerMask::reveal_all);
        let unchanged =
            source_layer.mask.is_some() && doc.selection.is_none() && same(source_layer.mask.as_ref().map(|m| m.surface.as_ref()), Some(&mask_surface), c)?;
        let mut next = doc.clone();
        poll(c)?;
        next.layer_at_mut(&path(doc, id, c)?).ok_or(EngineError::NoLayer(id))?.mask =
            Some(LayerMask { surface: std::sync::Arc::new(mask_surface), ..existing });
        next.selection = None;
        let response = json!({"output":"layerMask","layer":id.0,"selected":false});
        poll(c)?;
        return Ok(Prepared { selection: None, response, change: !unchanged, publication: Some(Publication::Existing(next, active)) });
    }
    let pixels = source_layer.surface().ok_or_else(|| bad(ID, "active layer has no pixels"))?;
    let empty = photocraft_algo::segment::Region { bbox: Rect::EMPTY, mask: Vec::new() };
    let region = region.as_ref().unwrap_or(&empty);
    let decontaminated = if r.decontaminate {
        Some(matting::decontaminate_with_interrupt(pixels, region, r.params.radius, r.amount, MattingLimits::default(), c).map_err(|e| error(ID, c, e))?)
    } else {
        None
    };
    let pixels = decontaminated.as_ref().unwrap_or(pixels);
    let content = if r.output.with_mask() {
        poll(c)?;
        pixels.clone()
    } else {
        matting::masked_copy_with_interrupt(pixels, region, MattingLimits::default(), c).map_err(|e| error(ID, c, e))?
    };
    let mask = if r.output.with_mask() { Some(LayerMask { surface: std::sync::Arc::new(mask_surface), ..LayerMask::reveal_all() }) } else { None };
    if r.output.document() {
        doc.working_profile_interpretation.validate_with_interrupt(doc.icc_profile.as_deref().map(Vec::as_slice), doc.mode, c).map_err(|e| error(ID, c, e))?;
        let mut output = Document::new("Untitled", doc.size, doc.mode, doc.depth);
        output.resolution_dpi = doc.resolution_dpi;
        output.icc_profile = doc.icc_profile.clone();
        output.color_table = copy_palette(doc.color_table.as_ref(), c)?;
        output.duotone = copy_duotone(doc.duotone.as_ref(), c)?;
        output.physical_resolution = copy_resolution(doc.physical_resolution.as_ref(), c)?;
        output.working_profile_interpretation = copy_working_profile(&doc.working_profile_interpretation, c)?;
        output.sample_encoding = doc.sample_encoding;
        let mut layer = Layer::new(owned(&source_layer.name, 4096, c)?, LayerContent::Raster(content));
        layer.mask = mask;
        let layer_id = layer.id;
        output.layers.try_reserve_exact(1).map_err(|_| bad(ID, "output layer allocation"))?;
        output.layers.push(layer);
        let state = DocState::new_with_interrupt(output, None, c)?;
        poll(c)?;
        return Ok(Prepared { selection: None, response: Value::Null, change: true, publication: Some(Publication::NewDocument(state, layer_id)) });
    }
    let name = copy_name(doc, &source_layer.name, c)?;
    let mut layer = Layer::new(name, LayerContent::Raster(content));
    layer.mask = mask;
    let inventory = identities(doc, c)?;
    let mut attempts = 0usize;
    while layer.id.0 == 0 || inventory.contains(&layer.id) {
        poll(c)?;
        attempts = attempts.checked_add(1).filter(|v| *v <= inventory.len().saturating_add(1)).ok_or_else(|| bad(ID, "layer identity exhausted"))?;
        layer.id = LayerId::fresh();
    }
    let new_id = layer.id;
    let mut next = doc.clone();
    poll(c)?;
    insert_above(&mut next, id, layer, c)?;
    let original_path = path(&next, id, c)?;
    next.layer_at_mut(&original_path).ok_or(EngineError::NoLayer(id))?.visible = false;
    next.selection = None;
    let response = json!({"output":r.output.text(),"layer":new_id.0,"selected":false});
    poll(c)?;
    Ok(Prepared { selection: None, response, change: true, publication: Some(Publication::Existing(next, Some(new_id))) })
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
                if let Some(publication) = prepared.publication {
                    match publication {
                        Publication::Existing(next, new_active) => {
                            let response = prepared.response;
                            if r.output == Output::LayerMask {
                                crate::xcf_item_locks::require_layer_mask_content(
                                    &s.active().ok_or(EngineError::NoDocument)?.doc,
                                    active.ok_or_else(|| bad(ID, "no active layer"))?,
                                )?;
                            }
                            s.edit_with_interrupt("Select and Mask", c, move |doc, act| {
                                crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                                *doc = next;
                                *act = new_active;
                                poll(c)
                            })?;
                            Ok(response)
                        }
                        Publication::NewDocument(mut output, layer) => {
                            if s.docs.len() >= 4096 || s.docs.capacity() > 8192 || s.accounted_slots.is_some() {
                                return Err(bad(ID, "resident output slot limit"));
                            }
                            let mut ids = std::collections::HashSet::new();
                            ids.try_reserve(s.docs.len() + 1).map_err(|_| bad(ID, "document identity allocation"))?;
                            if ids.capacity() > s.docs.len().saturating_add(1).saturating_mul(4).saturating_add(64) {
                                return Err(bad(ID, "document identity capacity"));
                            }
                            for state in &s.docs {
                                poll(c)?;
                                if state.doc.id.0 == 0 || !ids.insert(state.doc.id) {
                                    return Err(bad(ID, "duplicate open document identity"));
                                }
                            }
                            if output.doc.id.0 == 0 || ids.contains(&output.doc.id) {
                                return Err(bad(ID, "new document identity collision"));
                            }
                            let index = s.docs.len();
                            s.docs.try_reserve_exact(1).map_err(|_| bad(ID, "output slot allocation"))?;
                            if s.docs.capacity() > 8192 {
                                return Err(bad(ID, "output slot capacity"));
                            }
                            output.history.max_states = s.prefs.get().performance.history_states.max(1) as usize;
                            output.history.max_bytes = s.prefs.get().performance.history_budget_bytes();
                            let response = json!({"output":r.output.text(),"document":index,"layer":layer.0,"selected":false});
                            poll(c)?;
                            // publish_document_state has no error/allocation after its final
                            // slot fence. Returning the prepared response cannot then fail.
                            let _ = s.publish_document_state(output, &photocraft_raster::backing::ReadControl { interrupt: c, work: &|_| Ok(()) })?;
                            Ok(response)
                        }
                    }
                } else {
                    let response = prepared.response;
                    s.edit_with_interrupt("Select and Mask", c, move |doc, _| {
                        crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                        doc.selection = prepared.selection;
                        poll(c)
                    })?;
                    Ok(response)
                }
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    pub(super) fn document() -> Document {
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
        let r = Request { params, all: false, target_mask: false, eligible: true, output: Output::Selection, decontaminate: false, amount: 100. };
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
    fn captured_unported_brush_refuses_and_cancel_never_mutates_snapshot() {
        let mut s = Session::new();
        s.add_document(document(), None).unwrap();
        let st = s.active().unwrap();
        let old = st.doc.clone();
        let rev = st.revision;
        assert!(
            handles(
                &s,
                &json!({"output":"layerMask","refineStrokes":[{"points":[[3,4]]}],"documentId":old.id.0.to_string(),"expectedRevision":rev.to_string()})
            )
            .is_err()
        );
        let r = Request {
            params: RefineParams::default(),
            all: false,
            target_mask: false,
            eligible: true,
            output: Output::Selection,
            decontaminate: false,
            amount: 100.,
        };
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

#[cfg(test)]
mod output_tests {
    use super::*;
    fn session() -> Session {
        let mut s = Session::new();
        s.add_document(super::tests::document(), None).unwrap();
        s
    }
    #[test]
    fn all_ordinary_outputs_keep_masks_pixels_and_source_history() {
        for output in ["layerMask", "newLayer", "newLayerWithMask", "newDocument", "newDocumentWithMask"] {
            let mut s = session();
            let old = s.active().unwrap().doc.clone();
            let rev = s.active().unwrap().revision;
            let source_id = s.active().unwrap().active_layer.unwrap();
            let response = run(&mut s, &json!({"output":output,"documentId":old.id.0.to_string(),"expectedRevision":rev.to_string()})).unwrap();
            assert_eq!(response["output"], output);
            assert_eq!(response["selected"], false);
            if output.starts_with("newDocument") {
                assert_eq!(s.documents().len(), 2);
                assert!(old.shares_state(&s.documents()[0].doc));
                assert_eq!(s.documents()[0].revision, rev);
                assert_eq!(s.active().unwrap().doc.layers[0].name, old.layers[0].name);
            } else {
                assert_eq!(s.active().unwrap().revision, rev + 1);
                assert!(s.active().unwrap().doc.selection.is_none());
                if output == "layerMask" {
                    assert!(s.active().unwrap().doc.layer(source_id).unwrap().mask.is_some());
                } else {
                    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
                    assert_eq!(s.active().unwrap().doc.layers[1].name, "source copy");
                    assert!(!s.active().unwrap().doc.layer(source_id).unwrap().visible);
                }
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, old);
            }
        }
    }
    #[test]
    fn with_mask_shares_native_words_and_new_document_keeps_axes_and_encoding() {
        let mut s = session();
        {
            let d = s.active_mut().unwrap().doc.try_resident_mut().unwrap();
            d.physical_resolution = Some(photocraft_doc::PhysicalResolution { x_dpi: 120., y_dpi: 240., unit: photocraft_doc::PhysicalUnit::Millimeter });
            d.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
        }
        let source = s.active().unwrap().doc.clone();
        let input = source.layers[0].surface().unwrap();
        run(&mut s, &json!({"output":"newDocumentWithMask"})).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.physical_resolution, source.physical_resolution);
        assert_eq!(d.sample_encoding, source.sample_encoding);
        assert_eq!(d.working_profile_interpretation, source.working_profile_interpretation);
        let output = d.layers[0].surface().unwrap();
        assert_eq!(output.default_bytes(), input.default_bytes());
        for ((a, at), (b, bt)) in input.tiles().zip(output.tiles()) {
            assert_eq!(a, b);
            assert!(std::sync::Arc::ptr_eq(at, bt));
        }
        assert!(source.shares_state(&s.documents()[0].doc));
    }
    #[test]
    fn existing_mask_flags_and_cancelled_preparation_survive() {
        let mut s = session();
        {
            let d = s.active_mut().unwrap().doc.try_resident_mut().unwrap();
            d.layers[0].mask = Some(LayerMask { enabled: false, linked: false, density: 0.37, feather: 2.25, ..LayerMask::hide_all() });
        }
        let old = s.active().unwrap().doc.clone();
        let active = s.active().unwrap().active_layer;
        let request = Request {
            params: RefineParams::default(),
            all: false,
            target_mask: false,
            eligible: true,
            output: Output::NewLayerWithMask,
            decontaminate: false,
            amount: 100.,
        };
        assert!(matches!(prepare(&old, active, request, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(old.shares_state(&s.active().unwrap().doc));
        run(&mut s, &json!({"output":"layerMask"})).unwrap();
        let m = s.active().unwrap().doc.layers[0].mask.as_ref().unwrap();
        assert!(!m.enabled && !m.linked);
        assert_eq!((m.density, m.feather), (0.37, 2.25));
    }
}

#[cfg(test)]
mod decontamination_tests {
    use super::*;
    #[test]
    fn original_output_promotion_and_all_destination_policies_remain() {
        for requested in ["selection", "layerMask", "newLayer", "newLayerWithMask", "newDocument", "newDocumentWithMask"] {
            let mut s = Session::new();
            s.add_document(super::tests::document(), None).unwrap();
            let source = s.active().unwrap().doc.clone();
            let response = run(&mut s, &json!({"output":requested,"decontaminate":true,"amount":50})).unwrap();
            let expected = if matches!(requested, "selection" | "layerMask") { "newLayerWithMask" } else { requested };
            assert_eq!(response["output"], expected);
            if requested.starts_with("newDocument") {
                assert!(source.shares_state(&s.documents()[0].doc));
            } else {
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc, source);
            }
        }
    }
}
