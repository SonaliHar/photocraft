//! Controlled upstream Sampled Colors query; no inferred native/source interpretation.
use super::{ColorRangeSamples, b, f, sel};
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_color::{PixelFormat, SampleType, read_sample};
use photocraft_doc::{Document, LayerId};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface};
use serde_json::{Value, json};
const CMD: &str = "select.colorRange";
fn failure(reason: impl std::fmt::Display) -> EngineError {
    super::EngineError::BadParams { cmd: CMD.into(), msg: reason.to_string() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn error(control: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { failure(e) }
}
fn reserve<T>(count: usize) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| failure("query allocation failed"))?;
    if out.capacity() != count {
        return Err(failure("query capacity exceeds reservation"));
    }
    Ok(out)
}
fn collect<T>(iter: impl Iterator<Item = T>, bound: usize) -> Result<Vec<T>> {
    let mut out = reserve(bound)?;
    for v in iter {
        if out.len() >= bound {
            return Err(failure("query count exceeds reservation"));
        }
        out.push(v);
    }
    Ok(out)
}
pub(super) fn with_samples(
    s: &Session,
    p: &Value,
    area: Rect,
    plus: &[sel::RangeSample],
    minus: &[sel::RangeSample],
    control: &Interrupt<'_>,
) -> Result<ColorRangeSamples> {
    envelope(p)?;
    poll(control)?;
    if plus.len().checked_add(minus.len()).is_none_or(|n| n > 4096) {
        return Err(failure("sample count limit"));
    }
    if !matches!(preset(p)?, Preset::Sampled) {
        return Err(failure("Sampled Colors query required"));
    }
    let context = QueryContext::capture(s, control)?;
    with_context(&context, p, area, plus, minus, control)
}
fn with_context(
    context: &QueryContext,
    p: &Value,
    area: Rect,
    plus: &[sel::RangeSample],
    minus: &[sel::RangeSample],
    control: &Interrupt<'_>,
) -> Result<ColorRangeSamples> {
    envelope(p)?;
    poll(control)?;
    if plus.len().checked_add(minus.len()).is_none_or(|n| n > 4096) {
        return Err(failure("sample count limit"));
    }
    for sample in plus.iter().chain(minus) {
        poll(control)?;
        if !sample.color.iter().all(|v| v.is_finite()) || sample.at.is_some_and(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return Err(failure("finite query samples required"));
        }
    }
    let bad = |msg: String| EngineError::BadParams { cmd: "select.colorRange".into(), msg };
    let fuzz = |d: f32| f(p, "fuzziness", d).max(0.0);
    // The order the eyedropper clicks came in ("+" a point, "-" a subtracted point):
    // Photoshop applies them one after another. Without it, all additions come first.
    let order: Vec<bool> = match p.get("order") {
        None => collect(std::iter::repeat_n(true, plus.len()).chain(std::iter::repeat_n(false, minus.len())), plus.len() + minus.len())?,
        Some(o) => {
            let o = o.as_array().ok_or_else(|| bad("`order` must be a list of \"+\" and \"-\"".into()))?;
            let mut order = reserve(o.len())?;
            for value in o {
                poll(control)?;
                order.push(match value.as_str() {
                    Some("+") => true,
                    Some("-") => false,
                    _ => return Err(failure("order entries must be + or -")),
                });
            }
            if order.iter().filter(|x| **x).count() != plus.len() || order.iter().filter(|x| !**x).count() != minus.len() {
                return Err(bad("`order` must list each of `points` (\"+\") and `subtractPoints` (\"-\") once".into()));
            }
            order
        }
    };
    let fixed_count = p.get("colors").and_then(Value::as_array).map_or(0, Vec::len) + usize::from(p.get("color").is_some());
    let mut fixed: Vec<[f32; 3]> = reserve(fixed_count)?;
    if let Some(cs) = p.get("colors") {
        let cs = cs.as_array().ok_or_else(|| bad("`colors` must be a list of colours".into()))?;
        for c in cs {
            poll(control)?;
            fixed.push(color(c)?);
        }
    }
    if p.get("color").is_some() {
        fixed.push(color(p.get("color").ok_or_else(|| failure("missing color"))?)?);
    }
    // Localized Color Clusters: Range is a percentage of the canvas's longer side.
    let plus_at: Vec<(f32, f32)> = collect(plus.iter().filter_map(|x| x.at), plus.len())?;
    let localized = if b(p, "localized", false) {
        if plus_at.is_empty() {
            return Err(bad("localized clusters need eyedropper `points`".into()));
        }
        Some(f(p, "range", 100.0).clamp(0.0, 100.0) / 100.0 * area.width().max(area.height()) as f32)
    } else {
        None
    };
    // Photoshop compares colours in Lab, through the document's profile
    // (sel::lab_range_coverage). The samples span one Lab box; "Add to Sample" grows it,
    // "Subtract from Sample" reshapes it (sel::LabRange::subtract).
    poll(control)?;
    let to_lab = context.to_lab.clone();
    poll(control)?;
    if !context.foreground.iter().all(|v| v.is_finite()) {
        return Err(failure("finite foreground required"));
    }
    // 8-bit documents are compared on the 8-bit Lab grid, samples and pixels alike.
    let grid = context.grid;
    let q = |c: [f32; 3]| if grid { sel::quantize_lab8(c) } else { c };
    let colour_to_lab = |t: &photocraft_cms::Transform, c: [f32; 3]| q(super::colour_to_lab(t, c));
    let foreground = || {
        let [r, g, b, _] = context.foreground;
        sel::LabRange::point(colour_to_lab(&to_lab, [r, g, b]))
    };
    let mut fixed_lab = reserve(fixed.len())?;
    for c in &fixed {
        poll(control)?;
        fixed_lab.push(colour_to_lab(&to_lab, *c));
    }
    let mut range = sel::LabRange::spanning(&fixed_lab);
    let (mut pi, mut mi) = (plus.iter(), minus.iter());
    for add in order {
        poll(control)?;
        if add {
            if let Some(x) = pi.next() {
                let c = colour_to_lab(&to_lab, x.color);
                range = Some(range.map_or(sel::LabRange::point(c), |r| r.grow(&c)));
            }
        } else if let Some(x) = mi.next() {
            range = Some(range.unwrap_or_else(foreground).subtract(&colour_to_lab(&to_lab, x.color), fuzz(40.0)));
        }
    }
    let range = range.unwrap_or_else(foreground);
    poll(control)?;
    Ok(ColorRangeSamples { to_lab, range, grid, fuzziness: fuzz(40.0), localized, plus_at })
}

fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|v| v.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
fn color(value: &Value) -> Result<[f32; 3]> {
    if let Some(a) = value.as_array().filter(|a| (3..=4).contains(&a.len())) {
        if !a.iter().all(|v| v.as_f64().is_some_and(|v| (v as f32).is_finite())) {
            return Err(failure("finite RGB color required"));
        }
        let mut rgb = [0.; 3];
        for (dst, v) in rgb.iter_mut().zip(a) {
            *dst = v.as_f64().ok_or_else(|| failure("color component"))? as f32;
        }
        return Ok(rgb);
    }
    if let Some(text) = value.as_str().filter(|s| s.len() <= 9) {
        let h = text.strip_prefix('#').unwrap_or(text);
        if !matches!(h.len(), 6 | 8) || !h.bytes().all(|v| v.is_ascii_hexdigit()) {
            return Err(failure("six/eight hexadecimal RGB color required"));
        }
        let mut rgb = [0.; 3];
        for (i, dst) in rgb.iter_mut().enumerate() {
            *dst = f32::from(u8::from_str_radix(h.get(i * 2..i * 2 + 2).ok_or_else(|| failure("color width"))?, 16).map_err(|_| failure("color hex"))?) / 255.;
        }
        return Ok(rgb);
    }
    Err(failure("bounded RGB color required"))
}

#[derive(Clone, Copy)]
enum Preset {
    Sampled,
    Gamut,
    Named(sel::NamedColorRange),
}
fn preset(p: &Value) -> Result<Preset> {
    let name = match p.get("select") {
        None => "sampledColors",
        Some(value) => value.as_str().filter(|s| s.len() <= 32).ok_or_else(|| failure("bounded Color Range preset name required"))?,
    };
    let fuzz = |default: f32| f(p, "fuzziness", default).max(0.);
    Ok(match name {
        "sampledColors" => Preset::Sampled,
        "outOfGamut" => Preset::Gamut,
        "reds" | "yellows" | "greens" | "cyans" | "blues" | "magentas" => Preset::Named(sel::NamedColorRange::Hue {
            center: match name {
                "reds" => 0.,
                "yellows" => 60.,
                "greens" => 120.,
                "cyans" => 180.,
                "blues" => 240.,
                _ => 300.,
            },
        }),
        "skinTones" => Preset::Named(sel::NamedColorRange::SkinTones { fuzziness: fuzz(40.).min(200.) }),
        "highlights" | "midtones" | "shadows" => {
            let level = |v: &Value| v.as_f64().filter(|v| (0. ..=255.).contains(v)).map(|v| v as f32);
            let (low, high) = match (name, p.get("tonalRange")) {
                ("shadows", None) => (0., 65.),
                ("highlights", None) => (190., 255.),
                (_, None) => (105., 150.),
                ("shadows", Some(v)) => (0., level(v).ok_or_else(|| failure("tonalRange must be a level 0..255"))?),
                ("highlights", Some(v)) => (level(v).ok_or_else(|| failure("tonalRange must be a level 0..255"))?, 255.),
                (_, Some(v)) => {
                    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| failure("tonalRange must be [low,high]"))?;
                    let low = a.first().and_then(level).ok_or_else(|| failure("tonal low level"))?;
                    let high = a.get(1).and_then(level).ok_or_else(|| failure("tonal high level"))?;
                    if low > high {
                        return Err(failure("tonalRange low exceeds high"));
                    }
                    (low, high)
                }
            };
            Preset::Named(sel::NamedColorRange::Tone { low, high, falloff: fuzz(20.).min(100.) / 100. * 255. })
        }
        _ => return Err(failure("unknown or uncontrolled Color Range preset")),
    })
}
/// Every recognized preset now has a controlled route, with explicit provider refusals.
pub(crate) fn supports(_p: &Value) -> bool {
    true
}
/// Cheap borrowed envelope; must run before dispatcher cloning/normalization.
pub(crate) fn envelope(p: &Value) -> Result<()> {
    if p.is_null() {
        return Ok(());
    }
    let obj = p.as_object().filter(|o| o.len() <= 17).ok_or_else(|| failure("bounded sampled-colors object required"))?;
    let mut bytes = 0usize;
    let mut count = 0usize;
    for (key, value) in obj {
        bytes = bytes.checked_add(key.len() + 16).ok_or_else(|| failure("query byte overflow"))?;
        match key.as_str() {
            "select" => {
                if supports(p) {
                    let _ = preset(p)?;
                }
            }
            "documentId" | "expectedRevision" => {
                if decimal(value).is_none() {
                    return Err(failure("canonical decimal capture required"));
                }
                bytes += 20;
            }
            "mode" => {
                if !matches!(value.as_str(), Some("replace" | "add" | "subtract" | "intersect")) {
                    return Err(failure("unknown selection mode"));
                }
                bytes += 16;
            }
            "sampleAllLayers" | "localized" | "invert" | "detectFaces" => {
                if !value.is_boolean() {
                    return Err(failure("boolean query switch required"));
                }
            }
            "tonalRange" => {
                let valid = |v: &Value| v.as_f64().is_some_and(|v| (0. ..=255.).contains(&v));
                if !valid(value) && !value.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(valid)) {
                    return Err(failure("tonalRange must contain levels 0..255"));
                }
                bytes += 72;
            }
            "fuzziness" | "range" => {
                if !value.as_f64().is_some_and(|v| (v as f32).is_finite()) {
                    return Err(failure("finite query numeric parameter required"));
                }
                bytes += 32;
            }
            "points" | "subtractPoints" => {
                let a = value.as_array().filter(|a| a.len() <= 4096).ok_or_else(|| failure("sample count limit"))?;
                count = count.checked_add(a.len()).filter(|n| *n <= 4096).ok_or_else(|| failure("aggregate sample count limit"))?;
                for v in a {
                    let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| failure("point must be [x,y]"))?;
                    if !a.iter().all(|v| v.as_f64().is_some_and(|v| v.is_finite() && v >= i32::MIN as f64 && v <= i32::MAX as f64)) {
                        return Err(failure("finite point coordinate required"));
                    }
                }
                bytes = bytes.checked_add(a.len() * 72).ok_or_else(|| failure("point storage"))?;
            }
            "order" => {
                let a = value.as_array().filter(|a| a.len() <= 4096).ok_or_else(|| failure("sample order limit"))?;
                if !a.iter().all(|v| matches!(v.as_str(), Some("+" | "-"))) {
                    return Err(failure("order must contain + or -"));
                }
                bytes = bytes.checked_add(a.len() * 8).ok_or_else(|| failure("order storage"))?;
            }
            "color" => {
                color(value)?;
                bytes += 128;
            }
            "colors" => {
                let a = value.as_array().filter(|a| a.len() <= 4096).ok_or_else(|| failure("color count limit"))?;
                for v in a {
                    color(v)?;
                }
                bytes = bytes.checked_add(a.len() * 128).ok_or_else(|| failure("color storage"))?;
            }
            _ => return Err(failure("unknown sampled-colors field")),
        }
        if bytes > 64 << 10 {
            return Err(failure("sampled-colors storage limit"));
        }
    }
    if supports(p) {
        let _ = preset(p)?;
    }
    Ok(())
}
fn target(session: &Session, p: &Value) -> Result<()> {
    let st = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => Ok(()),
        (Some(id), Some(rev)) if decimal(id) == Some(st.doc.id.0) && st.doc.id.0 != 0 && decimal(rev) == Some(st.revision) => Ok(()),
        _ => Err(failure("captured Color Range target changed")),
    }
}
fn all_layers(p: &Value) -> bool {
    if matches!(preset(p), Ok(Preset::Gamut)) { true } else { b(p, "sampleAllLayers", true) }
}
fn closed_sampling(doc: &Document, active: Option<LayerId>, all: bool, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let direct = if !all { active.and_then(|id| doc.layer(id)).and_then(|l| l.surface()) } else { None };
    if let Some(surface) = direct {
        if matches!(surface.format().sample, SampleType::U32 | SampleType::F64) {
            return Err(failure("native-owner-unsupported"));
        }
        surface.try_validate_tile_origins_with(control).map_err(|e| error(control, e))?;
        return poll(control);
    }
    if doc.xcf_state.is_some() {
        return Err(failure("merged imported Color Range rendering is not qualified"));
    }
    if !doc.patterns.is_empty() {
        return Err(failure("controlled Color Range pattern preparation unsupported"));
    }
    photocraft_doc::resident_structure::visit_layers_with(doc, control, |layer| {
        if layer.mask.is_some()
            || layer.vector_mask.is_some()
            || layer.effects != Default::default()
            || layer.fill_cache.is_some()
            || layer.video.is_some()
            || layer.import_compositing.is_some()
            || layer.indexed_source.is_some()
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        match &layer.content {
            photocraft_doc::LayerContent::Raster(surface) if !matches!(surface.format().sample, SampleType::U32 | SampleType::F64) => {
                surface.try_validate_tile_origins_with(control)?
            }
            photocraft_doc::LayerContent::Group(_) => {}
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
        Ok(true)
    })
    .map_err(|e| error(control, e))?;
    poll(control)
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, _background: bool) -> Result<()> {
    envelope(p)?;
    if !supports(p) {
        return Ok(());
    }
    target(session, p)?;
    let st = session.active().ok_or(EngineError::NoDocument)?;
    if st.source_read_only
        || st.revision >= u64::MAX - 1
        || session.documents().iter().any(|s| s.floating.is_some())
        || st.coalesce.is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|xcf| xcf.floating.is_some())
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || st.doc.accounted().is_some()
        || session.accounted_slots.is_some()
    {
        return Err(failure("idle resident sampled-colors target required"));
    }
    crate::selection_sample_policy::metadata_preflight(session, CMD, p, &Interrupt::NONE)?;
    if b(p, "detectFaces", false) {
        return Err(failure("detectFaces: face detection isn't available yet; select without it"));
    }
    if matches!(preset(p)?, Preset::Gamut) {
        gamut_eligibility(&st.doc, &Interrupt::NONE)?;
    }
    closed_sampling(&st.doc, st.active_layer, all_layers(p), &Interrupt::NONE)
}

/// Metadata-only source/selection eligibility and retained-storage admission before
/// UI proxy construction. No profile parsing, point sampling, raster decoding, or grant.
pub(crate) fn preview_admission(session: &Session, p: &Value, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    envelope(p)?;
    target(session, p)?;
    crate::selection_sample_policy::existing_selection(session, CMD, control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let all = if supports(p) { all_layers(p) } else { true };
    crate::selection_sample_policy::sampled_pixels(session, CMD, all, control)?;
    if supports(p) {
        closed_sampling(&state.doc, state.active_layer, all, control)?;
    }
    let points = ["points", "subtractPoints"].iter().map(|key| p.get(key).and_then(Value::as_array).map_or(0, Vec::len)).sum::<usize>();
    admission(&state.doc, 0, points, false, control)?;
    if matches!(preset(p)?, Preset::Gamut) {
        gamut_eligibility(&state.doc, control)?;
    }
    poll(control)
}

#[derive(Clone, Copy, Default)]
struct RenderCharge {
    bytes: u64,
    work: u64,
}
fn render_charge(doc: &Document, active: Option<LayerId>, all: bool, pixels: usize, points: usize, control: &Interrupt<'_>) -> Result<RenderCharge> {
    poll(control)?;
    if !all && active.and_then(|id| doc.layer(id)).and_then(|layer| layer.surface()).is_some() {
        return Ok(RenderCharge::default());
    }
    let bounds = doc.bounds();
    let mut result = RenderCharge::default();
    if pixels > 0 {
        let band = Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0.checked_add(1).ok_or_else(|| failure("render row bound"))?);
        let req = photocraft_compose::ordinary_render_requirements(doc, band, control).map_err(|e| error(control, e))?;
        let per_pixel = req.work_units.checked_div(u64::from(band.width())).ok_or_else(|| failure("render work shape"))?;
        result.bytes = (req.scratch_bytes as u64).checked_add(req.preparation_reserve_bytes as u64).ok_or_else(|| failure("render reserve"))?;
        result.work = per_pixel.checked_mul(pixels as u64).ok_or_else(|| failure("render work"))?;
    }
    if points > 0 {
        let req = photocraft_compose::ordinary_render_requirements(doc, Rect::from_xywh(bounds.x0, bounds.y0, 1, 1), control).map_err(|e| error(control, e))?;
        let bytes = (req.scratch_bytes as u64).checked_add(req.preparation_reserve_bytes as u64).ok_or_else(|| failure("point-render reserve"))?;
        result.bytes = result.bytes.max(bytes);
        result.work = result
            .work
            .checked_add(req.work_units.checked_mul(points as u64).ok_or_else(|| failure("repeated point-render work"))?)
            .ok_or_else(|| failure("aggregate render work"))?;
    }
    // RGBA output/band arrays are already charged in the caller's pixel/point storage.
    // This known backend envelope excludes inherited preparation-private work/allocator gaps.
    poll(control)?;
    Ok(result)
}
fn admission(doc: &Document, pixels: usize, points: usize, output: bool, control: &Interrupt<'_>) -> Result<()> {
    admission_extra(doc, pixels, points, output, 0, 0, control)
}
fn admission_extra(doc: &Document, pixels: usize, points: usize, output: bool, extra_bytes: u64, extra_work: u64, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 300_000 || doc.size.height > 300_000 {
        return Err(failure("invalid Color Range canvas dimensions"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(|e| error(control, e))? as u64;
    let held = DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let tiles = u64::from(doc.size.width)
        .div_ceil(TILE_SIZE as u64)
        .checked_mul(u64::from(doc.size.height).div_ceil(TILE_SIZE as u64))
        .ok_or_else(|| failure("tile count"))?;
    let stage = if output { tiles.checked_mul((TILE_SIZE as u64) * (TILE_SIZE as u64) * 2 + 4096).ok_or_else(|| failure("selection stage size"))? } else { 0 };
    model
        .checked_mul(5)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(held.retained_bytes))
        .and_then(|n| n.checked_add(held.clone_metadata_bytes.checked_mul(5)?))
        .and_then(|n| n.checked_add((pixels as u64).checked_mul(56)?))
        .and_then(|n| n.checked_add(stage))
        .and_then(|n| n.checked_add(2 << 20))
        .and_then(|n| n.checked_add(extra_bytes))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| failure("aggregate Color Range residency limit"))?;
    (pixels as u64)
        .checked_mul((points as u64).checked_mul(48).and_then(|n| n.checked_add(1024)).ok_or_else(|| failure("query work"))?)
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(model.checked_add(payload)?.checked_mul((points as u64).checked_mul(16)?.checked_add(8)?)?))
        .and_then(|n| n.checked_add(extra_work))
        .filter(|n| *n <= 8_000_000_000)
        .ok_or_else(|| failure("Color Range work limit"))?;
    poll(control)
}
fn source_pixel(surface: &Surface, x: i32, y: i32, control: &Interrupt<'_>) -> Result<[f32; 4]> {
    poll(control)?;
    let fmt = surface.format();
    if matches!(fmt.sample, SampleType::U32 | SampleType::F64) || fmt.channels() > 5 {
        return Err(failure("unsupported sampled owner format"));
    }
    let mut raw = [0u8; 20];
    let encoded = raw.get_mut(..fmt.bytes_per_pixel()).ok_or_else(|| failure("sample bytes"))?;
    surface.try_read_pixel_bytes(x, y, encoded).map_err(|e| error(control, e))?;
    let mut values = [0.; 5];
    for (i, v) in values.get_mut(..fmt.channels()).ok_or_else(|| failure("sample channels"))?.iter_mut().enumerate() {
        *v = read_sample(encoded, fmt.sample, i);
    }
    Ok(photocraft_raster::to_rgba(&fmt, values.get(..fmt.channels()).ok_or_else(|| failure("sample channels"))?))
}
fn previous(doc: &Document, x: i32, y: i32, control: &Interrupt<'_>) -> Result<f32> {
    let Some(source) = doc.selection.as_ref() else {
        return Ok(0.);
    };
    if matches!(source.format().sample, SampleType::U32 | SampleType::F64) {
        return Err(failure("native-selection-unsupported"));
    }
    // Match mask_from_surface's first native channel; do not convert Gray to RGB.
    let mut raw = [0u8; 20];
    let bytes = raw.get_mut(..source.format().bytes_per_pixel()).ok_or_else(|| failure("selection sample width"))?;
    poll(control)?;
    source.try_read_pixel_bytes(x, y, bytes).map_err(|e| error(control, e))?;
    Ok(read_sample(bytes, source.format().sample, 0))
}
struct QueryContext {
    to_lab: std::sync::Arc<photocraft_cms::Transform>,
    grid: bool,
    foreground: [f32; 4],
}
impl QueryContext {
    fn capture(session: &Session, control: &Interrupt<'_>) -> Result<Self> {
        poll(control)?;
        let st = session.active().ok_or(EngineError::NoDocument)?;
        admission(&st.doc, 0, 0, false, control)?;
        let to_lab = super::lab_transform(session)?;
        poll(control)?;
        Ok(Self { to_lab, grid: st.doc.depth == SampleType::U8, foreground: session.tools.foreground })
    }
}
pub(super) fn new_query(session: &Session, p: &Value, control: &Interrupt<'_>) -> Result<ColorRangeSamples> {
    envelope(p)?;
    target(session, p)?;
    if !matches!(preset(p)?, Preset::Sampled) {
        return Err(failure("Sampled Colors query required"));
    }
    crate::selection_sample_policy::color_range_preview(session, p, control)?;
    let st = session.active().ok_or(EngineError::NoDocument)?;
    closed_sampling(&st.doc, st.active_layer, all_layers(p), control)?;
    let context = QueryContext::capture(session, control)?;
    query_document(&st.doc, st.active_layer, &context, p, control)
}
fn query_document(doc: &Document, active: Option<LayerId>, context: &QueryContext, p: &Value, control: &Interrupt<'_>) -> Result<ColorRangeSamples> {
    let area = doc.bounds();
    closed_sampling(doc, active, all_layers(p), control)?;
    let count = ["points", "subtractPoints"].iter().map(|key| p.get(key).and_then(Value::as_array).map_or(0, Vec::len)).sum::<usize>();
    let render = render_charge(doc, active, all_layers(p), 0, count, control)?;
    admission_extra(doc, count.max(1), count, false, render.bytes, render.work, control)?;
    let sample = |key: &str| -> Result<Vec<sel::RangeSample>> {
        let list = p.get(key).and_then(Value::as_array);
        let mut samples = reserve(list.map_or(0, Vec::len))?;
        if let Some(list) = list {
            for point in list {
                poll(control)?;
                let pair = point.as_array().ok_or_else(|| failure("point shape"))?;
                let x = pair.first().and_then(Value::as_f64).ok_or_else(|| failure("point x"))?.floor() as i32;
                let y = pair.get(1).and_then(Value::as_f64).ok_or_else(|| failure("point y"))?.floor() as i32;
                if !area.contains(x, y) {
                    return Err(failure("sample point outside canvas"));
                }
                if super::mode(p) == sel::SelectionMode::Replace && doc.selection.is_some() && !(previous(doc, x, y, control)? > 0.) {
                    return Err(failure("sample point outside active selection"));
                }
                let direct = if !all_layers(p) { active.and_then(|id| doc.layer(id)).and_then(|l| l.surface()) } else { None };
                let pixel = if let Some(source) = direct {
                    source_pixel(source, x, y, control)?
                } else {
                    photocraft_compose::render_with_interrupt(doc, Rect::from_xywh(x, y, 1, 1), control)
                        .map_err(|e| error(control, e))?
                        .px
                        .first()
                        .copied()
                        .ok_or_else(|| failure("empty point composite"))?
                };
                samples.push(sel::RangeSample { color: [pixel[0], pixel[1], pixel[2]], at: Some(((x - area.x0) as f32, (y - area.y0) as f32)) });
            }
        }
        Ok(samples)
    };
    with_context(context, p, area, &sample("points")?, &sample("subtractPoints")?, control)
}
pub(super) fn coverage(
    query: &ColorRangeSamples,
    px: &[[f32; 4]],
    width: usize,
    scale: f32,
    origin: [f32; 2],
    limits: sel::ColorRangeLimits,
    control: &Interrupt<'_>,
) -> Result<Vec<f32>> {
    poll(control)?;
    let default = sel::ColorRangeLimits::default();
    if limits.resident_bytes > default.resident_bytes
        || limits.work_units > default.work_units
        || width == 0
        || !px.len().is_multiple_of(width)
        || !origin.iter().all(|v| v.is_finite())
    {
        return Err(failure("query shape/domain/limits"));
    }
    let scale = if scale.is_finite() && scale > 0. { scale } else { 1. };
    (px.len() as u64)
        .checked_mul(36)
        .and_then(|n| n.checked_add((query.plus_at.capacity() as u64).checked_mul(16)?))
        .and_then(|n| n.checked_add(2 * 512 * 16))
        .filter(|n| *n <= limits.resident_bytes)
        .ok_or_else(|| failure("query Lab/mask residency limit"))?;
    (px.len() as u64)
        .checked_mul((query.plus_at.len() as u64).checked_mul(48).and_then(|n| n.checked_add(1024)).ok_or_else(|| failure("query work"))?)
        .filter(|n| *n <= limits.work_units)
        .ok_or_else(|| failure("query CMS/sample work limit"))?;
    let mut lab = reserve(px.len())?;
    lab.resize(px.len(), [0.; 4]);
    let mut source = reserve(512)?;
    for (input, output) in px.chunks(512).zip(lab.chunks_mut(512)) {
        poll(control)?;
        source.clear();
        for p in input {
            source.push([p[0].clamp(0., 1.), p[1].clamp(0., 1.), p[2].clamp(0., 1.), p[3]]);
        }
        query.to_lab.convert_f32(source.as_flattened(), 4, output.as_flattened_mut(), 4, true);
        poll(control)?;
        for p in output {
            let [l, a, b] = super::decode_lab([p[0], p[1], p[2]]);
            *p = [l, a, b, p[3]];
            if query.grid {
                let [l, a, b] = sel::quantize_lab8([p[0], p[1], p[2]]);
                *p = [l, a, b, p[3]];
            }
        }
    }
    let mut points = reserve(query.plus_at.len())?;
    for &(x, y) in &query.plus_at {
        poll(control)?;
        points.push(((x - origin[0]) / scale, (y - origin[1]) / scale));
    }
    let held = (px.len() as u64) * 16 + (query.plus_at.capacity() as u64) * 8 + (source.capacity() as u64) * 16;
    let available = limits.resident_bytes.checked_sub(held).ok_or_else(|| failure("retained query buffers"))?;
    let work =
        limits.work_units.checked_sub((px.len() as u64).checked_mul(512).ok_or_else(|| failure("CMS work"))?).ok_or_else(|| failure("CMS work limit"))?;
    sel::color_range_lab_with_interrupt(
        &lab,
        width,
        &query.range,
        query.fuzziness,
        query.grid,
        query.localized.map(|r| (r / scale, points.as_slice())),
        sel::ColorRangeLimits { resident_bytes: available, work_units: work },
        control,
    )
    .map_err(|e| error(control, e))
}

fn cms_error(e: photocraft_cms::control::Error) -> EngineError {
    if matches!(e, photocraft_cms::control::Error::Cancelled) { EngineError::Cancelled } else { failure(e) }
}
enum CompositeProfile {
    Shared(std::sync::Arc<photocraft_cms::Profile>),
    Owned { profile: photocraft_cms::Profile, source_bytes: usize, source_work: u64 },
    Builtin(&'static photocraft_cms::Profile),
    BuiltinPrepared { profile: &'static photocraft_cms::Profile, source_work: u64 },
}
impl CompositeProfile {
    fn provider_cost(&self) -> (usize, u64) {
        match self {
            Self::Owned { source_bytes, source_work, .. } => (*source_bytes, *source_work),
            Self::BuiltinPrepared { source_work, .. } => (0, *source_work),
            _ => (0, 0),
        }
    }
    fn get(&self) -> &photocraft_cms::Profile {
        match self {
            Self::Shared(p) => p,
            Self::Owned { profile, .. } => profile,
            Self::Builtin(p) => p,
            Self::BuiltinPrepared { profile, .. } => profile,
        }
    }
}

fn gamut_eligibility(document: &Document, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    let mode = crate::color_cmds::mode_space(document.mode);
    if matches!(mode, photocraft_cms::ColorSpace::Rgb | photocraft_cms::ColorSpace::Gray) {
        if !matches!(document.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc)
            || document.sample_encoding.is_some_and(|encoding| encoding != photocraft_doc::SampleEncoding::NonlinearProfile)
        {
            return Err(failure("controlled gamut effective/implicit profile preparation is not available"));
        }
    }
    poll(control)
}

fn cold_profile(document: &Document, bytes: &[u8], control: &Interrupt<'_>) -> Result<(Option<photocraft_cms::Profile>, usize, u64)> {
    poll(control)?;
    let limits = photocraft_cms::control::Limits::default();
    let cancel = || control.cancelled();
    // Malformed input has no returned census; reserve/charge the declared existing
    // parser work ceiling, never an invented byte multiplier. No model is allocated.
    let required = match photocraft_cms::Profile::parse_requirements_with_control(bytes, limits, &cancel) {
        Ok(required) => required,
        Err(photocraft_cms::control::Error::Cms(_)) => {
            admission_extra(document, 0, 0, false, 0, limits.work_units, control)?;
            return Ok((None, 0, limits.work_units));
        }
        Err(error) => return Err(cms_error(error)),
    };
    admission_extra(document, 0, 0, false, required.peak_resident_bytes as u64, required.work_units, control)?;
    match photocraft_cms::Profile::parse_with_control(bytes, limits, &cancel) {
        Ok(profile) => {
            poll(control)?;
            Ok((Some(profile), required.input_bytes, required.work_units))
        }
        Err(photocraft_cms::control::Error::Cms(_)) => {
            poll(control)?;
            Ok((None, 0, required.work_units))
        }
        Err(error) => Err(cms_error(error)),
    }
}
fn gamut_source(document: &Document, control: &Interrupt<'_>) -> Result<CompositeProfile> {
    gamut_eligibility(document, control)?;
    let mode = crate::color_cmds::mode_space(document.mode);
    if !matches!(mode, photocraft_cms::ColorSpace::Rgb | photocraft_cms::ColorSpace::Gray) {
        let profile = photocraft_cms::Builtin::Srgb.profile();
        poll(control)?;
        return Ok(CompositeProfile::Builtin(profile));
    }
    if !matches!(document.working_profile_interpretation, photocraft_doc::WorkingProfileInterpretation::Icc)
        || document.sample_encoding.is_some_and(|encoding| encoding != photocraft_doc::SampleEncoding::NonlinearProfile)
    {
        return Err(failure("controlled gamut effective/implicit profile preparation is not available"));
    }

    let working = || photocraft_cms::builtin::default_for(mode).ok_or_else(|| failure("working gamut profile is unavailable"));
    let base = if let Some(bytes) = &document.icc_profile {
        if let Some(profile) = crate::color_cmds::cached_profile_from_bytes(bytes) {
            if profile.color_space == mode { CompositeProfile::Shared(profile) } else { CompositeProfile::Builtin(working()?) }
        } else {
            let (profile, input, work) = cold_profile(document, bytes, control)?;
            match profile {
                Some(profile) if profile.color_space == mode => CompositeProfile::Owned { profile, source_bytes: input, source_work: work },
                _ => CompositeProfile::BuiltinPrepared { profile: working()?, source_work: work },
            }
        }
    } else {
        CompositeProfile::Builtin(working()?)
    };
    poll(control)?;
    if mode == photocraft_cms::ColorSpace::Gray {
        let defaults = photocraft_cms::control::Limits::default();
        let (prior_bytes, prior_work) = base.provider_cost();
        let limits = photocraft_cms::control::Limits {
            resident_bytes: defaults.resident_bytes.checked_sub(prior_bytes).ok_or_else(|| failure("parsed/Gray-view resident limit"))?,
            work_units: defaults.work_units,
        };
        let cancel = || control.cancelled();
        if let Some(required) = base.get().gray_as_rgb_requirements_with_control(limits, &cancel).map_err(cms_error)? {
            admission_extra(
                document,
                0,
                0,
                false,
                (required.retained_bytes as u64)
                    .checked_add(required.staged_bytes as u64)
                    .and_then(|n| n.checked_add(prior_bytes as u64))
                    .ok_or_else(|| failure("Gray RGB-view residency"))?,
                required.work_units.checked_add(prior_work).ok_or_else(|| failure("parse/Gray-view work"))?,
                control,
            )?;
            let profile =
                base.get().gray_as_rgb_with_control(limits, &cancel).map_err(cms_error)?.ok_or_else(|| failure("prepared Gray RGB view disappeared"))?;
            return Ok(CompositeProfile::Owned {
                profile,
                source_bytes: required.retained_bytes.checked_add(prior_bytes).ok_or_else(|| failure("parsed/Gray-view retention"))?,
                source_work: required.work_units.checked_add(prior_work).ok_or_else(|| failure("parsed/Gray-view work"))?,
            });
        }
        // Release private parsed models before preparing the unsupported-Gray fallback.
        drop(base);
        // Same upstream unsupported-Gray fallback, never a substitution for a valid view.
        let fallback = photocraft_cms::Builtin::SGray.profile();
        poll(control)?;
        if let Some(required) = fallback.gray_as_rgb_requirements_with_control(limits, &cancel).map_err(cms_error)? {
            admission_extra(
                document,
                0,
                0,
                false,
                (required.retained_bytes as u64)
                    .checked_add(required.staged_bytes as u64)
                    .and_then(|n| n.checked_add(prior_bytes as u64))
                    .ok_or_else(|| failure("fallback Gray RGB-view residency"))?,
                required.work_units.checked_add(prior_work).ok_or_else(|| failure("parse/fallback Gray work"))?,
                control,
            )?;
            let profile =
                fallback.gray_as_rgb_with_control(limits, &cancel).map_err(cms_error)?.ok_or_else(|| failure("fallback Gray RGB view disappeared"))?;
            return Ok(CompositeProfile::Owned {
                profile,
                source_bytes: required.retained_bytes.checked_add(prior_bytes).ok_or_else(|| failure("parsed/Gray-view retention"))?,
                source_work: required.work_units.checked_add(prior_work).ok_or_else(|| failure("parsed/Gray-view work"))?,
            });
        }
        return Ok(CompositeProfile::BuiltinPrepared { profile: photocraft_cms::Builtin::Srgb.profile(), source_work: prior_work });
    }
    Ok(base)
}
/// Owns only the already-authorized proof Arc and bounded setting labels. No locator reads.
struct ProofSnapshot {
    configured: bool,
    profile: Option<std::sync::Arc<photocraft_cms::Profile>>,
    name: String,
    intent: photocraft_cms::Intent,
    bpc: bool,
    simulate_paper: bool,
    kind: crate::proof_sim::ProofKind,
    enabled: bool,
    warning: bool,
    threshold: f32,
}
impl ProofSnapshot {
    fn capture(session: &Session, id: photocraft_doc::DocId) -> Result<Self> {
        if let Some(view) = session.color.proof_ref(id) {
            if view.setup.name.len() > 4096 || !view.gamut_threshold.is_finite() || view.gamut_threshold < 0. {
                return Err(failure("bounded proof name and finite nonnegative gamut threshold required"));
            }
            let mut name = String::new();
            name.try_reserve_exact(view.setup.name.len()).map_err(|_| failure("proof-setting capture allocation failed"))?;
            if name.capacity() > 4096 {
                return Err(failure("proof-setting capacity limit"));
            }
            name.push_str(&view.setup.name);
            Ok(Self {
                configured: true,
                profile: Some(view.setup.profile.clone()),
                name,
                intent: view.setup.intent,
                bpc: view.setup.bpc,
                simulate_paper: view.setup.simulate_paper,
                kind: view.setup.kind,
                enabled: view.enabled,
                warning: view.gamut_warning,
                threshold: view.gamut_threshold,
            })
        } else {
            Ok(Self {
                configured: false,
                profile: None,
                name: String::new(),
                intent: photocraft_cms::Intent::RelativeColorimetric,
                bpc: true,
                simulate_paper: false,
                kind: crate::proof_sim::ProofKind::Profile,
                enabled: false,
                warning: false,
                threshold: photocraft_cms::gamut::DEFAULT_THRESHOLD,
            })
        }
    }
    fn profile(&self, control: &Interrupt<'_>) -> Result<&photocraft_cms::Profile> {
        poll(control)?;
        let profile = if let Some(profile) = &self.profile { profile.as_ref() } else { photocraft_cms::Builtin::CoatedCmyk.profile() };
        // All builtin OnceLock initialization remains an explicit provider boundary.
        poll(control)?;
        Ok(profile)
    }
    fn matches(&self, session: &Session, id: photocraft_doc::DocId) -> bool {
        match (session.color.proof_ref(id), self.configured) {
            (None, false) => true,
            (Some(view), true) => {
                self.profile.as_ref().is_some_and(|profile| std::sync::Arc::ptr_eq(profile, &view.setup.profile))
                    && self.name == view.setup.name
                    && self.intent == view.setup.intent
                    && self.bpc == view.setup.bpc
                    && self.simulate_paper == view.setup.simulate_paper
                    && self.kind == view.setup.kind
                    && self.enabled == view.enabled
                    && self.warning == view.gamut_warning
                    && self.threshold.to_bits() == view.gamut_threshold.to_bits()
            }
            _ => false,
        }
    }
}
fn prepare_gamut(document: &Document, count: usize, proof: &ProofSnapshot, control: &Interrupt<'_>) -> Result<photocraft_cms::gamut::GamutCheck> {
    poll(control)?;
    let limits = photocraft_cms::control::Limits::default();
    // Reserve the lower ceiling before source/default proof provider access. Parsed cache
    // Cache Arcs are shared, cold assigned profiles parsed privately, effective/implicit routes refuse.
    let render = render_charge(document, None, true, count, 0, control)?;
    admission_extra(
        document,
        count,
        0,
        true,
        render.bytes.checked_add(limits.resident_bytes as u64).ok_or_else(|| failure("render/gamut reserve"))?,
        render.work.checked_add(limits.work_units).ok_or_else(|| failure("render/gamut work"))?,
        control,
    )?;
    let source = gamut_source(document, control)?;
    let (provider_bytes, provider_work) = source.provider_cost();
    let limits = photocraft_cms::control::Limits {
        resident_bytes: limits.resident_bytes.checked_sub(provider_bytes).ok_or_else(|| failure("Gray provider and gamut resident limit"))?,
        work_units: limits.work_units,
    };
    let profile = proof.profile(control)?;
    let cancel = || control.cancelled();
    let required = photocraft_cms::gamut::GamutCheck::requirements_with_control(source.get(), profile, proof.threshold, limits, &cancel).map_err(cms_error)?;
    let total = required
        .work_units
        .checked_add(provider_work)
        .ok_or_else(|| failure("gamut source preparation work"))?
        .checked_add((count as u64).checked_mul(required.evaluation_work_units).ok_or_else(|| failure("gamut pixel work"))?)
        .ok_or_else(|| failure("gamut aggregate work"))?;
    admission_extra(
        document,
        count,
        0,
        true,
        (required.peak_resident_bytes as u64)
            .checked_add(provider_bytes as u64)
            .and_then(|n| n.checked_add(render.bytes))
            .ok_or_else(|| failure("gamut source/render preparation residency"))?,
        total.checked_add(render.work).ok_or_else(|| failure("gamut source/render work"))?,
        control,
    )?;
    let check = photocraft_cms::gamut::GamutCheck::new_with_control(source.get(), profile, proof.threshold, limits, &cancel).map_err(cms_error)?;
    if check.inputs() != 3 {
        return Err(failure("gamut compositor profile must consume RGB"));
    }
    poll(control)?;
    Ok(check)
}
fn gamut_coverage(check: &photocraft_cms::gamut::GamutCheck, px: &[[f32; 4]], control: &Interrupt<'_>) -> Result<Vec<f32>> {
    let cancel = || control.cancelled();
    let mut mask = reserve(px.len())?;
    for (index, pixel) in px.iter().enumerate() {
        if index % 64 == 0 {
            poll(control)?;
        }
        let selected = if pixel[3] > 0. {
            check.out_of_gamut_with_control(pixel.get(..3).ok_or_else(|| failure("gamut RGB width"))?, &cancel).map_err(cms_error)?
        } else {
            false
        };
        mask.push(if selected { 1. } else { 0. });
    }
    poll(control)?;
    Ok(mask)
}

/// Existing proxy gamut math with the current document's captured proof setup.
/// No scene mutation or authority is granted; proxy/source IDs must agree.
pub(crate) fn gamut_preview(session: &Session, proxy: &Document, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    preview_admission(session, &json!({"select":"outOfGamut"}), control)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if proxy.id != state.doc.id {
        return Err(failure("gamut preview source changed"));
    }
    closed_sampling(proxy, None, true, control)?;
    let count = photocraft_raster::checked_region_byte_len(proxy.bounds(), PixelFormat::RGBA32F).map_err(|e| error(control, e))? / 16;
    admission_extra(
        proxy,
        count,
        0,
        false,
        photocraft_cms::control::Limits::default().resident_bytes as u64,
        photocraft_cms::control::Limits::default().work_units,
        control,
    )?;
    let proof = ProofSnapshot::capture(session, state.doc.id)?;
    let check = prepare_gamut(proxy, count, &proof, control)?;
    let px = pixels(proxy, None, true, control)?;
    let mask = gamut_coverage(&check, &px, control)?;
    if !proof.matches(session, state.doc.id) {
        return Err(failure("gamut preview proof changed"));
    }
    poll(control)?;
    Ok(mask)
}

/// Named preset proxy coverage without a scratch Session, history or model clone.
pub(crate) fn named_preview(session: &Session, proxy: &Document, p: &Value, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    preview_admission(session, p, control)?;
    let Preset::Named(named) = preset(p)? else {
        return Err(failure("named Color Range preview required"));
    };
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if proxy.id != state.doc.id {
        return Err(failure("named preview source changed"));
    }
    closed_sampling(proxy, state.active_layer, all_layers(p), control)?;
    // Old scratch add_document chose top_layer when no imported current target exists;
    // the attempted original active-layer selection then replaced it only when present.
    let active = state.active_layer.filter(|id| proxy.layer(*id).is_some()).or_else(|| proxy.top_layer());
    closed_sampling(proxy, active, all_layers(p), control)?;
    let count = photocraft_raster::checked_region_byte_len(proxy.bounds(), PixelFormat::RGBA32F).map_err(|e| error(control, e))? / 16;
    let render = render_charge(proxy, active, all_layers(p), count, 0, control)?;
    admission_extra(proxy, count, 0, true, render.bytes, render.work, control)?;
    let px = pixels(proxy, active, all_layers(p), control)?;
    let mask = sel::named_color_range_with_interrupt(&px, proxy.size.width as usize, named, sel::ColorRangeLimits::default(), control)
        .map_err(|e| error(control, e))?;
    let result = prepared_selection(proxy, &mask, super::mode(p), b(p, "invert", false), control)?;
    poll(control)?;
    Ok(result)
}
fn pixels(doc: &Document, active: Option<LayerId>, all: bool, control: &Interrupt<'_>) -> Result<Vec<[f32; 4]>> {
    let area = doc.bounds();
    let count = photocraft_raster::checked_region_byte_len(area, PixelFormat::RGBA32F).map_err(|e| error(control, e))? / 16;
    let mut output = reserve(count)?;
    let direct = if !all { active.and_then(|id| doc.layer(id)).and_then(|layer| layer.surface()) } else { None };
    if let Some(surface) = direct {
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                output.push(source_pixel(surface, x, y, control)?);
            }
        }
    } else {
        photocraft_compose::render_bands_with_interrupt(doc, area, 1, control, |band| {
            if output.len().checked_add(band.px.len()).is_none_or(|n| n > count) {
                return Err(RasterInputError::LengthMismatch);
            }
            output.extend_from_slice(&band.px);
            Ok(())
        })
        .map_err(|e| error(control, e))?;
    }
    if output.len() != count {
        return Err(failure("sampled buffer shape"));
    }
    poll(control)?;
    Ok(output)
}
fn prepared_selection(doc: &Document, mask: &[f32], mode: sel::SelectionMode, invert: bool, control: &Interrupt<'_>) -> Result<Option<Surface>> {
    let area = doc.bounds();
    let width = usize::try_from(area.width()).map_err(|_| failure("selection width"))?;
    let count = photocraft_raster::checked_region_byte_len(area, PixelFormat::GRAY8).map_err(|e| error(control, e))?;
    if mask.len() != count {
        return Err(failure("coverage mask shape"));
    }
    let mut output = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[0]).map_err(|e| error(control, e))?;
    let mut bytes = reserve((TILE_SIZE as usize) * (TILE_SIZE as usize))?;
    bytes.resize((TILE_SIZE as usize) * (TILE_SIZE as usize), 0);
    // Same quantization and combine order as selection::combine, without its unchecked buffers.
    let q = |v: f32| f32::from((v.clamp(0., 1.) * 255. + 0.5) as u8) / 255.;
    for tile in area.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&area);
        let len = (part.width() as usize).checked_mul(part.height() as usize).ok_or_else(|| failure("tile pixels"))?;
        let encoded = bytes.get_mut(..len).ok_or_else(|| failure("tile scratch"))?;
        let mut local = 0usize;
        for y in part.y0..part.y1 {
            poll(control)?;
            for x in part.x0..part.x1 {
                let index = usize::try_from(y - area.y0)
                    .ok()
                    .and_then(|row| row.checked_mul(width))
                    .and_then(|row| usize::try_from(x - area.x0).ok().and_then(|x| row.checked_add(x)))
                    .ok_or_else(|| failure("selection pixel offset"))?;
                let mut value = *mask.get(index).ok_or_else(|| failure("coverage pixel"))?;
                if invert {
                    value = 1. - value;
                }
                let prior = if doc.selection.is_some() { previous(doc, x, y, control)? } else { 0. };
                if mode == sel::SelectionMode::Replace && doc.selection.is_some() {
                    value = value.min(prior);
                }
                value = match mode {
                    sel::SelectionMode::Replace => value,
                    sel::SelectionMode::Add => prior.max(q(value)),
                    sel::SelectionMode::Subtract => (prior - q(value)).max(0.),
                    sel::SelectionMode::Intersect => prior.min(q(value)),
                };
                *encoded.get_mut(local).ok_or_else(|| failure("tile pixel offset"))? = (value.clamp(0., 1.) * 255. + 0.5) as u8;
                local += 1;
            }
        }
        output.try_write_interleaved_with(part, encoded, control).map_err(|e| error(control, e))?;
    }
    output.try_prune_with(control).map_err(|e| error(control, e))?;
    poll(control)?;
    Ok((output.tile_count() > 0).then_some(output))
}
pub(crate) fn run(session: &mut Session, p: &Value) -> Result<Value> {
    dispatch_preflight(session, p, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let before = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let weak = before.downgrade();
    let count = photocraft_raster::checked_region_byte_len(before.bounds(), PixelFormat::RGBA32F).map_err(|e| error(&Interrupt::NONE, e))? / 16;
    let point_count = ["points", "subtractPoints"].iter().map(|key| p.get(key).and_then(Value::as_array).map_or(0, Vec::len)).sum::<usize>();
    let render = render_charge(&before, active, all_layers(p), count, point_count, &Interrupt::NONE)?;
    admission_extra(&before, count, point_count, true, render.bytes, render.work, &Interrupt::NONE)?;
    let kind = preset(p)?;
    let proof = if matches!(kind, Preset::Gamut) {
        admission_extra(
            &before,
            count,
            point_count,
            true,
            render.bytes.checked_add(photocraft_cms::control::Limits::default().resident_bytes as u64).ok_or_else(|| failure("render/gamut reserve"))?,
            render.work.checked_add(photocraft_cms::control::Limits::default().work_units).ok_or_else(|| failure("render/gamut work"))?,
            &Interrupt::NONE,
        )?;
        Some(std::sync::Arc::new(ProofSnapshot::capture(session, before.id)?))
    } else {
        None
    };
    let context = if matches!(kind, Preset::Sampled) { Some(QueryContext::capture(session, &Interrupt::NONE)?) } else { None };
    // The borrowed envelope has bounded all Value nesting, strings, arrays and number storage.
    let proof_for_work = proof.as_ref().map(std::sync::Arc::clone);
    let parameters = p.clone();
    let baseline = before.clone();
    let foreground = session.tools.foreground;
    crate::jobs::run(
        session,
        "Color Range",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Color Range", |control| {
                let render = render_charge(&before, active, all_layers(&parameters), count, point_count, control)?;
                admission_extra(&before, count, point_count, true, render.bytes, render.work, control)?;
                crate::selection_inverse_controlled::selection_write_authority(&before, control)?;
                closed_sampling(&before, active, all_layers(&parameters), control)?;
                let query = match &context {
                    Some(context) => Some(query_document(&before, active, context, &parameters, control)?),
                    None => None,
                };
                let gamut = if matches!(kind, Preset::Gamut) {
                    Some(prepare_gamut(&before, count, proof_for_work.as_ref().ok_or_else(|| failure("missing proof capture"))?, control)?)
                } else {
                    None
                };
                let px = pixels(&before, active, all_layers(&parameters), control)?;
                let mask = match kind {
                    Preset::Sampled => coverage(
                        query.as_ref().ok_or_else(|| failure("missing prepared sampled query"))?,
                        &px,
                        before.size.width as usize,
                        1.,
                        [0., 0.],
                        sel::ColorRangeLimits::default(),
                        control,
                    )?,
                    Preset::Gamut => gamut_coverage(gamut.as_ref().ok_or_else(|| failure("missing prepared gamut checker"))?, &px, control)?,
                    Preset::Named(preset) => {
                        sel::named_color_range_with_interrupt(&px, before.size.width as usize, preset, sel::ColorRangeLimits::default(), control)
                            .map_err(|e| error(control, e))?
                    }
                };
                let selection = prepared_selection(&before, &mask, super::mode(&parameters), b(&parameters, "invert", false), control)?;
                let response = json!({"selected":selection.is_some()});
                poll(control)?;
                Ok((selection, response, ctx.clone()))
            })
        },
        move |session, (selection, response, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.active_layer != active
                || current.source_read_only
                || current.coalesce.is_some()
                || session.documents().iter().any(|s| s.floating.is_some())
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || session.command_has_pending_job()
                || proof.as_ref().is_some_and(|proof| !proof.matches(session, baseline.id))
                || (matches!(kind, Preset::Sampled) && session.tools.foreground.map(f32::to_bits) != foreground.map(f32::to_bits))
            {
                return Err(failure("Color Range source/context changed while preparing"));
            }
            ctx.stage(0.95, 1., "Color Range", |control| {
                poll(control)?;
                crate::selection_inverse_controlled::selection_write_authority(&baseline, control)?;
                session.edit_with_interrupt("Color Range", control, move |doc, _| {
                    doc.selection = selection;
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
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Layer, Size};
    #[test]
    fn checked_query_matches_legacy_grid_and_localization() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("Range", Size::new(2, 2), ColorMode::Rgb, depth);
            let mut layer = Layer::raster("Pixels", doc.pixel_format());
            layer.surface_mut().unwrap().write_region(doc.bounds(), &[0.8, 0.2, 0.1, 1., 0.2, 0.5, 0.9, 1., 0.8, 0.2, 0.1, 0.5, 0., 0., 0., 0.]);
            doc.layers.push(layer);
            let mut session = Session::new();
            session.add_document(doc, None).unwrap();
            let p = json!({"points":[[0,0]],"subtractPoints":[[1,0]],"order":["+","-"],"localized":true,"range":70.,"fuzziness":40.});
            let query = new_query(&session, &p, &Interrupt::NONE).unwrap();
            let px = super::super::sample_pixels(&session, true).unwrap().1;
            assert_eq!(
                coverage(&query, &px, 2, 1., [0., 0.], sel::ColorRangeLimits::default(), &Interrupt::NONE).unwrap(),
                query.coverage(&px, 2, 1., [0., 0.])
            );
            assert!(matches!(
                coverage(&query, &px, 2, 1., [0., 0.], sel::ColorRangeLimits::default(), &Interrupt::cancel_only(&|| true)),
                Err(EngineError::Cancelled)
            ));
        }
    }
    #[test]
    fn controlled_combine_matches_legacy_scope_invert_and_all_modes() {
        let mut doc = Document::new("Scope", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.write_region(doc.bounds(), &[0.4, 1.]);
        doc.selection = Some(selection);
        for mode in [sel::SelectionMode::Replace, sel::SelectionMode::Add, sel::SelectionMode::Subtract, sel::SelectionMode::Intersect] {
            for invert in [false, true] {
                let mut mask = vec![0.2, 0.8];
                if invert {
                    for v in &mut mask {
                        *v = 1. - *v;
                    }
                }
                if mode == sel::SelectionMode::Replace {
                    for (v, p) in mask.iter_mut().zip(sel::mask_from_surface(doc.selection.as_ref(), doc.bounds())) {
                        *v = v.min(p);
                    }
                }
                let old = sel::combine(doc.selection.as_ref(), &mask, doc.bounds(), mode);
                let actual = prepared_selection(&doc, &[0.2, 0.8], mode, invert, &Interrupt::NONE).unwrap();
                assert_eq!(
                    actual.as_ref().map(|s| s.try_read_interleaved(doc.bounds()).unwrap()),
                    old.as_ref().map(|s| s.try_read_interleaved(doc.bounds()).unwrap())
                );
            }
        }
    }
    #[test]
    fn envelope_refuses_oversize_and_unknown_before_sampling() {
        assert!(envelope(&json!({"points":vec![vec![0,0];4097]})).is_err());
        assert!(envelope(&json!({"fuzziness":1e100})).is_err());
        assert!(envelope(&json!({"unknown":"value"})).is_err());
    }
    #[test]
    fn registered_sampled_selection_undo_and_stale_refusal_retain_real_redo() {
        let mut doc = Document::new("History", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("Pixels", doc.pixel_format());
        layer.surface_mut().unwrap().write_region(doc.bounds(), &[1., 0., 0., 1., 0., 0., 1., 1.]);
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        let before = session.active().unwrap().doc.clone();
        session.execute(CMD, json!({"points":[[0,0]],"fuzziness":0.})).unwrap();
        let after = session.active().unwrap().doc.clone();
        assert!(after.selection.is_some());
        assert!(session.undo());
        assert_eq!(*session.active().unwrap().doc, *before);
        let rejected = session.execute(CMD, json!({"points":[[0,0]],"documentId":before.id.0.to_string(),"expectedRevision":"18446744073709551615"}));
        assert!(rejected.is_err());
        assert_eq!(*session.active().unwrap().doc, *before);
        assert!(session.redo());
        assert_eq!(*session.active().unwrap().doc, *after);
    }
    #[test]
    fn named_defaults_and_tonal_ranges_keep_upstream_thresholds() {
        for (name, expected) in [("shadows", (0., 65., 51.)), ("highlights", (190., 255., 51.)), ("midtones", (105., 150., 51.))] {
            let Preset::Named(sel::NamedColorRange::Tone { low, high, falloff }) = preset(&json!({"select":name})).unwrap() else {
                panic!("wrong typed preset")
            };
            assert_eq!((low, high, falloff), expected);
        }
        let Preset::Named(sel::NamedColorRange::Tone { low, high, falloff }) =
            preset(&json!({"select":"midtones","tonalRange":[80,160],"fuzziness":100})).unwrap()
        else {
            panic!("wrong typed preset")
        };
        assert_eq!((low, high, falloff), (80., 160., 255.));
        assert!(envelope(&json!({"select":"midtones","tonalRange":[160,80]})).is_err());
        assert!(envelope(&json!({"select":"skinTones","detectFaces":"true"})).is_err());
        assert!(matches!(preset(&json!({"select":"outOfGamut"})).unwrap(), Preset::Gamut));
    }
    #[test]
    fn registered_named_outputs_keep_old_native_masks_and_one_undo() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for name in ["reds", "yellows", "greens", "cyans", "blues", "magentas", "skinTones", "highlights", "midtones", "shadows"] {
                let mut doc = Document::new("Named", Size::new(2, 1), ColorMode::Rgb, depth);
                let mut layer = Layer::raster("Pixels", doc.pixel_format());
                layer.surface_mut().unwrap().write_region(doc.bounds(), &[0.8, 0.3, 0.2, 1., 0.1, 0.2, 0.9, 0.5]);
                doc.layers.push(layer);
                let mut session = Session::new();
                session.add_document(doc, None).unwrap();
                let before = session.active().unwrap().doc.clone();
                let params = json!({"select":name,"fuzziness":40.,"invert":true});
                let px = super::super::sample_pixels(&session, true).unwrap().1;
                let mut old = match name {
                    "reds" => sel::hue_range(&px, 0.),
                    "yellows" => sel::hue_range(&px, 60.),
                    "greens" => sel::hue_range(&px, 120.),
                    "cyans" => sel::hue_range(&px, 180.),
                    "blues" => sel::hue_range(&px, 240.),
                    "magentas" => sel::hue_range(&px, 300.),
                    "skinTones" => sel::skin_tones(&px, 40.),
                    "highlights" => sel::tone_range(&px, 190., 255., 40. / 100. * 255.),
                    "shadows" => sel::tone_range(&px, 0., 65., 40. / 100. * 255.),
                    _ => sel::tone_range(&px, 105., 150., 40. / 100. * 255.),
                };
                for v in &mut old {
                    *v = 1. - *v;
                }
                let expected = sel::combine(None, &old, before.bounds(), sel::SelectionMode::Replace);
                session.execute(CMD, params).unwrap();
                assert_eq!(
                    session.active().unwrap().doc.selection.as_ref().map(|s| s.try_read_interleaved(before.bounds()).unwrap()),
                    expected.as_ref().map(|s| s.try_read_interleaved(before.bounds()).unwrap())
                );
                assert!(session.undo());
                assert_eq!(*session.active().unwrap().doc, *before);
            }
        }
    }
    #[test]
    fn controlled_gamut_and_gray_match_existing_proof_mask() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            let mut doc = Document::new("Gamut", Size::new(2, 1), mode, SampleType::U16);
            let mut layer = Layer::raster("Pixels", doc.pixel_format());
            let pixels = if mode == ColorMode::Rgb { vec![1., 0., 0., 1., 0.2, 0.5, 0.8, 0.5] } else { vec![0.3, 1., 0.8, 0.5] };
            layer.surface_mut().unwrap().write_region(doc.bounds(), &pixels);
            doc.layers.push(layer);
            let mut session = Session::new();
            session.add_document(doc, None).unwrap();
            let id = session.active().unwrap().doc.id;
            let proof = session.color.proof_mut(id);
            proof.setup.profile = std::sync::Arc::new(photocraft_cms::Builtin::DisplayP3.profile().clone());
            proof.setup.intent = photocraft_cms::Intent::Perceptual;
            proof.setup.bpc = true;
            let before = session.active().unwrap().doc.clone();
            let proof = ProofSnapshot::capture(&session, id).unwrap();
            let check = prepare_gamut(&before, 2, &proof, &Interrupt::NONE).unwrap();
            let px = super::super::sample_pixels(&session, true).unwrap().1;
            let actual = gamut_coverage(&check, &px, &Interrupt::NONE).unwrap();
            let view = session.color.proof(id);
            let old = crate::color_cmds::gamut_mask(&before, &view.setup, view.gamut_threshold).unwrap().0;
            assert_eq!(actual, old.iter().map(|&v| f32::from(v) / 255.).collect::<Vec<_>>());
            assert!(matches!(prepare_gamut(&before, 2, &proof, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
            assert!(proof.matches(&session, id));
            session.color.proof_mut(id).gamut_threshold += 1.;
            assert!(!proof.matches(&session, id));
            assert_eq!(*session.active().unwrap().doc, *before);
        }
    }
    #[test]
    fn malformed_cold_uses_legacy_builtin_but_effective_provider_refuses() {
        let mut doc = Document::new("Cold", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        doc.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4]));
        assert!(gamut_eligibility(&doc, &Interrupt::NONE).is_ok());
        assert_eq!(gamut_source(&doc, &Interrupt::NONE).unwrap().get(), photocraft_cms::Builtin::Srgb.profile());
        doc.icc_profile = None;
        doc.sample_encoding = Some(photocraft_doc::SampleEncoding::Linear);
        assert!(gamut_eligibility(&doc, &Interrupt::NONE).is_err());
    }
}
