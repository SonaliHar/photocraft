//! Captured controlled Transform Selection; preserves upstream outline warp equations.
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_algo::{
    transform::{Homography, Interp},
    wide_geometry::GeometryLimits,
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::Rect;
use photocraft_geom::warp::Warp;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
const ID: &str = "select.transformSelection";
fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| EngineError::Cancelled)
}
fn failure(c: &Interrupt<'_>, e: impl std::fmt::Display) -> EngineError {
    if c.cancelled() { EngineError::Cancelled } else { bad(e.to_string()) }
}
fn decimal(v: &Value) -> Result<u64> {
    let s = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad("captured IDs require canonical decimal strings"))?;
    s.parse().map_err(|_| bad("identity overflow"))
}
fn number(v: &Value) -> Result<f64> {
    v.as_f64().filter(|v| v.is_finite() && v.abs() <= 1_000_000.0).ok_or_else(|| bad("finite bounded coordinate required"))
}
fn array<const N: usize>(v: &Value) -> Result<[f64; N]> {
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(|| bad("incorrect coordinate array shape"))?;
    let mut out = [0.0; N];
    for (dst, v) in out.iter_mut().zip(a) {
        *dst = number(v)?;
    }
    Ok(out)
}
fn census(v: &Value, depth: usize, count: &mut usize) -> Result<()> {
    *count = count.checked_add(1).filter(|n| *n <= 120000).ok_or_else(|| bad("warp node count limit"))?;
    if depth > 8 {
        return Err(bad("warp nesting limit"));
    }
    match v {
        Value::Array(a) => {
            if a.len() > 40000 {
                return Err(bad("warp array limit"));
            }
            for v in a {
                census(v, depth + 1, count)?;
            }
        }
        Value::Object(o) => {
            if o.len() > 12 || o.keys().any(|k| k.len() > 32) {
                return Err(bad("warp object limit"));
            }
            for v in o.values() {
                census(v, depth + 1, count)?;
            }
        }
        Value::String(s) => {
            if s.len() > 64 {
                return Err(bad("warp string limit"));
            }
        }
        Value::Number(_) => {
            number(v)?;
        }
        Value::Bool(_) | Value::Null => {}
    }
    Ok(())
}
fn envelope(p: &Value) -> Result<()> {
    if p.is_null() {
        return Ok(());
    }
    let o = p
        .as_object()
        .filter(|o| {
            o.len() <= 25
                && o.keys().all(|k| {
                    matches!(
                        k.as_str(),
                        "rect"
                            | "reference"
                            | "scaleX"
                            | "scaleY"
                            | "rotate"
                            | "skewX"
                            | "skewY"
                            | "dx"
                            | "dy"
                            | "x"
                            | "y"
                            | "quad"
                            | "corners"
                            | "matrix"
                            | "interpolation"
                            | "warp"
                            | "style"
                            | "mesh"
                            | "grid"
                            | "bend"
                            | "hDistort"
                            | "vDistort"
                            | "vertical"
                            | "documentId"
                            | "expectedRevision"
                    )
                })
        })
        .ok_or_else(|| bad("bounded Transform Selection parameters required"))?;
    for (k, v) in o {
        match k.as_str() {
            "documentId" | "expectedRevision" => {
                decimal(v)?;
            }
            "rect" => {
                array::<4>(v)?;
            }
            "matrix" => {
                array::<6>(v)?;
            }
            "quad" | "corners" => {
                let a = v.as_array().filter(|a| a.len() == 4).ok_or_else(|| bad("quad requires four points"))?;
                for p in a {
                    array::<2>(p)?;
                }
            }
            "reference" => match v {
                Value::Array(_) => {
                    array::<2>(v)?;
                }
                Value::String(s)
                    if matches!(s.as_str(), "center" | "topLeft" | "top" | "topRight" | "left" | "right" | "bottomLeft" | "bottom" | "bottomRight") => {}
                _ => return Err(bad("invalid reference")),
            },
            "interpolation" => {
                if !matches!(v.as_str(), Some("nearest" | "nearestNeighbor" | "bilinear" | "bicubic")) {
                    return Err(bad("unsupported interpolation"));
                }
            }
            "vertical" => {
                if !v.is_boolean() {
                    return Err(bad("vertical must be boolean"));
                }
            }
            "grid" => {
                let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("grid requires two integers"))?;
                for v in a {
                    if v.as_u64().is_none_or(|n| !(1..=64).contains(&n)) {
                        return Err(bad("grid must be 1..64"));
                    }
                }
            }
            "warp" | "mesh" => census(v, 0, &mut 0)?,
            "style" => {
                if v.as_str().is_none_or(|s| s.len() > 32) {
                    return Err(bad("bounded warp style required"));
                }
            }
            _ => {
                let n = number(v)?;
                if matches!(k.as_str(), "skewX" | "skewY") && n.abs() >= 89.0 {
                    return Err(bad("skew must be less than 89 degrees"));
                }
                if matches!(k.as_str(), "scaleX" | "scaleY") && (n == 0.0 || n.abs() > 10000.0) {
                    return Err(bad("scale must be nonzero and bounded"));
                }
            }
        }
    }
    Ok(())
}
enum Map {
    Homography(Homography),
    Mesh(Warp),
}
impl Map {
    fn identity(&self) -> bool {
        match self {
            Self::Homography(h) => h.0.iter().zip(Homography::IDENTITY.0).all(|(a, b)| (a - b).abs() < 1e-12),
            Self::Mesh(w) => w.is_identity(),
        }
    }
}
fn native(sample: SampleType) -> bool {
    matches!(sample, SampleType::U32 | SampleType::F64)
}
fn native_policy(map: &Map, p: &Value) -> Result<bool> {
    let Map::Homography(h) = map else {
        crate::selection_mesh_nearest_controlled::policy(p)?;
        return Ok(false);
    };
    let identity = h.0.iter().zip(Homography::IDENTITY.0).all(|(a, b)| *a == b);
    // Preserve genuine no-arithmetic identity regardless of requested sampler.
    // Every nonidentity native operation needs explicit Nearest.
    if !identity && !matches!(p.get("interpolation").and_then(Value::as_str), Some("nearest" | "nearestNeighbor")) {
        return Err(bad("native selection requires explicit nearest homography"));
    }
    Ok(identity)
}
fn native_requested_policy(source: &photocraft_raster::Surface, p: &Value) -> Result<()> {
    if native(source.format().sample) && ["warp", "style", "mesh", "grid"].iter().any(|key| p.get(*key).is_some()) {
        crate::selection_mesh_nearest_controlled::policy(p)?;
    }
    Ok(())
}
fn selection_bounds(source: &photocraft_raster::Surface, c: &Interrupt<'_>) -> Result<Rect> {
    if native(source.format().sample) {
        photocraft_algo::selection_nearest_controlled::source_bounds_with_limits(source, GeometryLimits::default(), c).map_err(|e| failure(c, e))
    } else {
        source.try_content_bounds_with_interrupt(c).map_err(|e| failure(c, e))
    }
}
fn mapping(p: &Value, b: Rect) -> Result<Map> {
    let rect = match p.get("rect") {
        Some(v) => array::<4>(v)?,
        None => [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)],
    };
    if rect[2] < rect[0] || rect[3] < rect[1] {
        return Err(bad("reversed transform rectangle"));
    }
    if ["warp", "style", "mesh", "grid"].iter().any(|k| p.get(*k).is_some()) {
        let r = [rect[0].floor(), rect[1].floor(), rect[2].ceil(), rect[3].ceil()];
        let w = crate::warp_cmds::warp_from_params(ID, p, r)?;
        if w.bounds.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
            || [w.bend, w.h_distort, w.v_distort].iter().any(|v| !v.is_finite() || v.abs() > 100.0)
        {
            return Err(bad("invalid bounded warp"));
        }
        return Ok(Map::Mesh(w));
    }
    let h = if let Some(v) = p.get("quad").or_else(|| p.get("corners")) {
        let a = v.as_array().ok_or_else(|| bad("invalid quad"))?;
        let mut q = [[0.0; 2]; 4];
        for (dst, v) in q.iter_mut().zip(a) {
            *dst = array::<2>(v)?;
        }
        Homography::rect_to_quad(rect, q).ok_or_else(|| bad("degenerate quad"))?
    } else if let Some(v) = p.get("matrix") {
        let [a, b, c, d, e, f] = array::<6>(v)?;
        Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])
    } else {
        let [x0, y0, x1, y1] = rect;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let [px, py] = match p.get("reference") {
            Some(Value::Array(_)) => array::<2>(p.get("reference").ok_or_else(|| bad("reference"))?)?,
            Some(Value::String(k)) => match k.as_str() {
                "topLeft" => [x0, y0],
                "top" => [cx, y0],
                "topRight" => [x1, y0],
                "left" => [x0, cy],
                "right" => [x1, cy],
                "bottomLeft" => [x0, y1],
                "bottom" => [cx, y1],
                "bottomRight" => [x1, y1],
                _ => [cx, cy],
            },
            _ => [cx, cy],
        };
        let get = |key: &str, default: f64| p.get(key).and_then(Value::as_f64).unwrap_or(default);
        let (sx, sy) = (get("scaleX", 100.0) / 100.0, get("scaleY", 100.0) / 100.0);
        let a = get("rotate", 0.0).to_radians();
        let (sa, ca) = a.sin_cos();
        let (kx, ky) = (get("skewX", 0.0).to_radians().tan(), get("skewY", 0.0).to_radians().tan());
        let (dx, dy) = (get("dx", get("x", 0.0)), get("dy", get("y", 0.0)));
        let t = |x, y| Homography([1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0, 1.0]);
        t(px + dx, py + dy)
            .mul(&Homography([ca, -sa, 0.0, sa, ca, 0.0, 0.0, 0.0, 1.0]))
            .mul(&Homography([1.0, kx, 0.0, ky, 1.0, 0.0, 0.0, 0.0, 1.0]))
            .mul(&Homography([sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0]))
            .mul(&t(-px, -py))
    };
    if h.0.iter().any(|v| !v.is_finite()) || h.inverse().is_none() {
        return Err(bad("the transform collapses the selection"));
    }
    Ok(Map::Homography(h))
}
fn context(session: &Session, p: &Value, c: &Interrupt<'_>) -> Result<()> {
    envelope(p)?;
    poll(c)?;
    let st = session.active().ok_or(EngineError::NoDocument)?;
    match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(rev)) if decimal(id)? == st.doc.id.0 && decimal(rev)? == st.revision => {}
        _ => return Err(bad("captured selection target changed")),
    }
    if st.revision == u64::MAX
        || st.floating.is_some()
        || st.coalesce.is_some()
        || st.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_pending_job()
        || session.command_has_job_on(st.doc.id)
    {
        return Err(bad("selection target is not idle"));
    }
    if st.channel_view.color_hidden.len() > 16
        || st.channel_view.color_hidden.capacity() > 16
        || st.channel_view.alpha_visible.len() > 10000
        || st.channel_view.alpha_visible.capacity() > 10000
    {
        return Err(bad("channel view capacity limit"));
    }
    let source = st.doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    if source.format().mode != ColorMode::Grayscale || source.format().alpha || source.tile_count() > 100000 {
        return Err(bad("bounded Gray selection required"));
    }
    crate::selection_inverse_controlled::selection_write_authority(&st.doc, c)
}
fn early_native_identity(p: &Value) -> Result<bool> {
    if ["warp", "style", "mesh", "grid"].iter().any(|key| p.get(*key).is_some()) {
        return Ok(false);
    }
    // Only proofs independent of source occupancy. No near-identity tolerance.
    if p.get("quad").or_else(|| p.get("corners")).is_some() {
        if p.get("rect").is_none() {
            return Ok(false);
        }
        return native_policy(&mapping(p, Rect::EMPTY)?, p);
    }
    if let Some(v) = p.get("matrix") {
        let [a, b, c, d, e, f] = array::<6>(v)?;
        return Ok(a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && e == 0.0 && f == 0.0);
    }
    let get = |key: &str, default: f64| p.get(key).and_then(Value::as_f64).unwrap_or(default);
    Ok(get("scaleX", 100.0) == 100.0
        && get("scaleY", 100.0) == 100.0
        && get("rotate", 0.0) == 0.0
        && get("skewX", 0.0) == 0.0
        && get("skewY", 0.0) == 0.0
        && get("dx", get("x", 0.0)) == 0.0
        && get("dy", get("y", 0.0)) == 0.0)
}
fn early_policy(source: &photocraft_raster::Surface, p: &Value) -> Result<bool> {
    if !native(source.format().sample) {
        return Ok(false);
    }
    native_requested_policy(source, p)?;
    let identity = early_native_identity(p)?;
    if !identity && !matches!(p.get("interpolation").and_then(Value::as_str), Some("nearest" | "nearestNeighbor")) {
        return Err(bad("native selection requires explicit nearest unless identity is provable from parameters"));
    }
    Ok(identity)
}
pub(crate) fn dispatch_preflight(session: &Session, p: &Value, _background: bool) -> Result<()> {
    context(session, p, &Interrupt::NONE)?;
    let source = session.active().and_then(|s| s.doc.selection.as_ref()).ok_or_else(|| bad("no selection"))?;
    early_policy(source, p)?;
    Ok(())
}
/// Fixed envelope is charged before the standard-library JSON capture clone.
/// This is bounded metadata allocation, not a fallible-all-allocator guarantee.
fn parameter_copy_bytes(value: &Value, depth: usize) -> Result<u64> {
    if depth > 10 {
        return Err(bad("parameter capture depth"));
    }
    let mut bytes = 256u64;
    match value {
        Value::Array(a) => {
            for v in a {
                bytes = bytes.checked_add(parameter_copy_bytes(v, depth + 1)?).ok_or_else(|| bad("parameter capture bytes"))?;
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                bytes = bytes
                    .checked_add(k.len() as u64)
                    .and_then(|n| n.checked_add(parameter_copy_bytes(v, depth + 1).ok()?))
                    .ok_or_else(|| bad("parameter capture bytes"))?;
            }
        }
        Value::String(v) => bytes = bytes.checked_add(v.len() as u64).ok_or_else(|| bad("parameter capture bytes"))?,
        _ => {}
    }
    if bytes > 32 << 20 {
        return Err(bad("parameter capture allocation limit"));
    }
    Ok(bytes)
}

fn limits(doc: &photocraft_doc::Document, c: &Interrupt<'_>) -> Result<GeometryLimits> {
    let l = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(doc, l, c).map_err(|e| failure(c, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(doc, l, c).map_err(|e| failure(c, e))? as u64;
    let raster = DocumentSnapshot::resident_raster_footprint_of(doc, c)?;
    let base = raster
        .retained_bytes
        .checked_add(raster.clone_metadata_bytes.checked_mul(4).ok_or_else(|| bad("clone headroom"))?)
        .and_then(|n| n.checked_add(model.checked_mul(4)?))
        .and_then(|n| n.checked_add(payload))
        .ok_or_else(|| bad("retained headroom"))?;
    Ok(GeometryLimits {
        resident_bytes: MAX_CHECKED_RASTER_BYTES.checked_sub(base).ok_or_else(|| bad("retained document limit"))?,
        work_units: GeometryLimits::default().work_units,
    })
}
struct Prepared {
    selection: Option<Option<photocraft_raster::Surface>>,
    response: Value,
}
pub(crate) fn run(session: &mut Session, p: &Value) -> Result<Value> {
    context(session, p, &Interrupt::NONE)?;
    let st = session.active().ok_or(EngineError::NoDocument)?;
    let source = st.doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    let native_identity_hint = early_policy(source, p)?;
    // Metadata/headroom census precedes owned params/view capture. Occupancy,
    // mapping/output checks and every sample loop remain worker-token work.
    let mut admitted = limits(&st.doc, &Interrupt::NONE)?;
    let parameters = parameter_copy_bytes(p, 0)?.checked_mul(4).ok_or_else(|| bad("parameter capture headroom"))?;
    // ChannelView belongs to session state, not Document model accounting.
    // Charge its owned witness vectors before clone, plus bounded response and
    // snapshot/job scalar owners; no response allocation follows publication.
    let views = (st.channel_view.color_hidden.capacity() as u64)
        .checked_add(st.channel_view.alpha_visible.capacity() as u64)
        .and_then(|n| n.checked_add(std::mem::size_of::<crate::channel_cmds::ChannelView>() as u64))
        .and_then(|n| n.checked_mul(2))
        .ok_or_else(|| bad("view capture headroom"))?;
    let capture = parameters.checked_add(views).and_then(|n| n.checked_add(4096)).ok_or_else(|| bad("capture headroom"))?;
    admitted.resident_bytes = admitted.resident_bytes.checked_sub(capture).ok_or_else(|| bad("parameter capture headroom"))?;
    let baseline = st.doc.clone();
    let worker = baseline.clone();
    let revision = st.revision;
    let view = st.channel_view.clone();
    let params = p.clone();
    crate::jobs::run(
        session,
        "Transform Selection",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Transform Selection", |c| {
                poll(c)?;
                worker.validate_resident()?;
                poll(c)?;
                let source = worker.selection.as_ref().ok_or_else(|| bad("selection disappeared"))?;
                if native_identity_hint {
                    // Validate resident native metadata under the real token; identity
                    // needs neither sample occupancy nor a bbox-dependent mapping.
                    photocraft_algo::selection_nearest_controlled::requirements(source, Rect::EMPTY, &Homography::IDENTITY, admitted, c)
                        .map_err(|e| failure(c, e))?;
                    poll(c)?;
                    return Ok((Prepared { selection: None, response: json!({"changed":false,"selected":true}) }, ctx.clone()));
                }
                let b = if native(source.format().sample) {
                    photocraft_algo::selection_nearest_controlled::source_bounds_with_limits(source, admitted, c).map_err(|e| failure(c, e))?
                } else {
                    selection_bounds(source, c)?
                };
                let map = mapping(&params, b)?;
                let raw_native = native(source.format().sample);
                let identity = if raw_native { native_policy(&map, &params)? } else { map.identity() };
                if identity {
                    poll(c)?;
                    return Ok((Prepared { selection: None, response: json!({"changed":false,"selected":true}) }, ctx.clone()));
                }
                let interp = Interp::parse(params.get("interpolation").and_then(Value::as_str).unwrap_or("bilinear"));
                let transformed = match map {
                    Map::Homography(h) if raw_native => {
                        photocraft_algo::selection_nearest_controlled::try_warp_gray_nearest_raw_with_limits(source, b, &h, admitted, c)
                    }
                    Map::Homography(h) => {
                        photocraft_algo::selection_transform_controlled::try_warp_gray_homography_with_limits(source, &h, interp, admitted, c)
                    }
                    Map::Mesh(w) if raw_native => {
                        let transformed = crate::selection_mesh_nearest_controlled::prepare(source, b, &w, admitted, c)?;
                        Ok(transformed)
                    }
                    Map::Mesh(w) => {
                        photocraft_algo::selection_transform_controlled::try_warp_gray_mesh_with_limits(source, &|x, y| w.map(x, y), interp, admitted, c)
                    }
                }
                .map_err(|e| failure(c, e))?;
                let bounds = transformed.try_content_bounds_with_interrupt(c).map_err(|e| failure(c, e))?;
                let unchanged = if raw_native {
                    photocraft_algo::selection_nearest_controlled::raw_equal_with_limits(source, &transformed, admitted, c).map_err(|e| failure(c, e))?
                } else {
                    !bounds.is_empty() && source == &transformed
                };
                let selected = raw_native || !bounds.is_empty();
                let response = if unchanged {
                    json!({"changed":false,"selected":selected})
                } else {
                    json!({"changed":true,"selected":selected,"bounds":selected.then_some([bounds.x0,bounds.y0,bounds.width() as i32,bounds.height() as i32])})
                };
                let selection = if unchanged { None } else { Some(selected.then_some(transformed)) };
                poll(c)?;
                Ok((Prepared { selection, response }, ctx.clone()))
            })
        },
        move |session, (prepared, ctx)| {
            let st = session.active().ok_or(EngineError::NoDocument)?;
            if st.doc.id != baseline.id
                || st.revision != revision
                || !baseline.downgrade().is_current(&st.doc)
                || !baseline.shares_state(&st.doc)
                || st.channel_view != view
                || st.floating.is_some()
                || st.coalesce.is_some()
                || session.command_has_job_on(st.doc.id)
                || session.command_has_pending_job()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad("selection target changed during preparation"));
            }
            ctx.stage(0.95, 1.0, "Transform Selection", |c| {
                poll(c)?;
                crate::selection_inverse_controlled::selection_write_authority(&session.active().ok_or(EngineError::NoDocument)?.doc, c)?;
                if let Some(selection) = prepared.selection {
                    poll(c)?;
                    session.edit_with_interrupt("Transform Selection", c, move |doc, _| {
                        crate::selection_inverse_controlled::selection_write_authority(doc, c)?;
                        doc.selection = selection;
                        poll(c)
                    })?;
                }
                Ok(prepared.response)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_raster::Surface;
    fn session(sample: SampleType) -> Session {
        let mut s = Session::new();
        let mut doc = photocraft_doc::Document::new("selection transform draft", photocraft_geom::Size::new(12, 9), ColorMode::Rgb, sample);
        let mut selection = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, sample, false));
        selection.fill_rect(Rect::new(2, 3, 6, 7), &[1.0]);
        doc.selection = Some(selection);
        s.add_document(doc, None).unwrap();
        s
    }
    #[test]
    fn native_identity_is_exact_noop_and_non_nearest_stays_refused() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut s = session(sample);
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            assert_eq!(run(&mut s, &json!({"matrix":[1,0,0,1,0,0]})).unwrap()["changed"], false);
            assert!(run(&mut s, &json!({"dx":1})).is_err());
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(s.active().unwrap().revision, revision);
        }
    }
    #[test]
    fn registered_outline_edit_moves_selection_only_with_one_undo_and_capture_refusal() {
        let mut s = session(SampleType::U16);
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let p = json!({"dx":3,"dy":-2,"documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()});
        s.execute(ID, p.clone()).unwrap();
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), Rect::new(5, 1, 9, 5));
        assert_eq!(s.active().unwrap().doc.layers, before.layers);
        assert_eq!(s.active().unwrap().doc.physical_resolution, before.physical_resolution);
        assert!(dispatch_preflight(&s, &p, false).is_err());
        s.execute("edit.undo", Value::Null).unwrap();
        assert_eq!(s.active().unwrap().doc.selection, before.selection);
    }
    #[test]
    fn hostile_payloads_never_publish_selection_changes() {
        let s = session(SampleType::U8);
        let before = s.active().unwrap().doc.clone();
        for p in [
            json!({"matrix":[1,2]}),
            json!({"corners":[[0,0],[1,0],[1,1],[0,"bad"]]}),
            json!({"grid":[u64::MAX,1]}),
            json!({"scaleX":0}),
            json!({"rect":[0,0,1e20,3]}),
            json!({"documentId":"01","expectedRevision":"0"}),
        ] {
            assert!(dispatch_preflight(&s, &p, false).is_err());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
    #[test]
    fn registered_native_nearest_moves_offcanvas_words_and_one_undo() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut s = session(sample);
            let source = s.active().unwrap().doc.selection.as_ref().unwrap();
            let raw = source.try_read_interleaved_with(Rect::new(2, 3, 6, 7), &Interrupt::NONE).unwrap();
            let default = source.default_bytes().to_vec();
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            s.execute(ID, json!({"dx":-20,"dy":2,"interpolation":"nearest"})).unwrap();
            let state = s.active().unwrap();
            assert_eq!(state.revision, revision + 1);
            let actual = state.doc.selection.as_ref().unwrap();
            assert_eq!(actual.try_read_interleaved_with(Rect::new(-18, 5, -14, 9), &Interrupt::NONE).unwrap(), raw);
            assert_eq!(actual.default_bytes(), default);
            assert_eq!(state.doc.layers, before.layers);
            assert!(s.undo());
            assert_eq!(&*s.active().unwrap().doc, &*before);
            assert!(s.redo());
        }
    }
    #[test]
    fn native_empty_nan_default_keeps_some_and_real_redo() {
        let mut s = session(SampleType::F64);
        let raw = 0x7ff8_0000_0000_0042u64.to_ne_bytes();
        s.edit("Native default", |doc, _| {
            doc.selection = Some(Surface::try_with_default_bytes(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &raw)?);
            Ok(())
        })
        .unwrap();
        s.edit("Redo", |doc, _| {
            doc.name = "Redo".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let result = s.execute(ID, json!({"dx":3,"interpolation":"nearest"})).unwrap();
        assert_eq!(result["changed"], false);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().default_bytes(), raw);
        assert!(s.active().unwrap().history.can_redo());
    }
    #[test]
    fn native_mesh_and_cancel_do_not_publish() {
        let mut s = session(SampleType::U32);
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(ID, json!({"warp":{"style":"arc"},"interpolation":"bilinear"})).is_err());
        assert!(s.active().unwrap().doc.shares_state(&before));
        let ctx = crate::jobs::JobCtx::default();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute(ID, json!({"dx":1,"interpolation":"nearest"})), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        let h = Map::Homography(Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 9e-13, 0.0, 1.0]));
        assert!(!native_policy(&h, &json!({"interpolation":"nearest"})).unwrap(), "near-identity projective map is not an exact native no-op");
    }

    #[test]
    fn native_exact_identity_uses_inline_job_cancel_token() {
        let mut s = session(SampleType::F64);
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let ctx = crate::jobs::JobCtx::default();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute(ID, json!({"matrix":[1,0,0,1,0,0]})), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
    }
    #[test]
    fn ambiguous_native_quad_identity_requires_explicit_nearest_hint() {
        let s = session(SampleType::U32);
        let p = json!({"corners":[[2,3],[6,3],[6,7],[2,7]]});
        assert!(dispatch_preflight(&s, &p, false).is_err());
        let mut nearest = p;
        nearest["interpolation"] = json!("nearest");
        assert!(dispatch_preflight(&s, &nearest, false).is_ok());
        let declared = json!({"rect":[2,3,6,7],"corners":[[2,3],[6,3],[6,7],[2,7]]});
        assert!(dispatch_preflight(&s, &declared, false).is_ok());
    }

    #[test]
    fn registered_native_mesh_preserves_raw_storage_and_history() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut s = session(sample);
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let mesh = photocraft_geom::warp::BezierMesh::fit(&|u, v| [2.0 + 4.0 * u + 3.0, 3.0 + 4.0 * v - 2.0], vec![0.0, 1.0], vec![0.0, 1.0]);
            let warp = Warp::custom(mesh, [2.0, 3.0, 6.0, 7.0]);
            let p = json!({"warp":warp,"interpolation":"nearest"});
            s.execute(ID, p).unwrap();
            let edited = s.active().unwrap().doc.clone();
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(edited.layers, before.layers);
            assert_eq!(edited.physical_resolution, before.physical_resolution);
            assert_eq!(edited.selection.as_ref().unwrap().format(), before.selection.as_ref().unwrap().format());
            assert_eq!(edited.selection.as_ref().unwrap().default_bytes(), before.selection.as_ref().unwrap().default_bytes());
            assert!(s.undo());
            assert_eq!(&*s.active().unwrap().doc, &*before);
            assert!(s.redo());
            assert_eq!(&*s.active().unwrap().doc, &*edited);
        }
    }
    #[test]
    fn native_identity_mesh_is_raw_noop_and_cancellation_keeps_redo() {
        let mut s = session(SampleType::F64);
        s.edit("Redo", |d, _| {
            d.name = "Redo".into();
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let p = json!({"grid":[1,1],"interpolation":"nearest"});
        let result = s.execute(ID, p.clone()).unwrap();
        assert_eq!(result["changed"], false);
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.active().unwrap().history.can_redo());
        let ctx = crate::jobs::JobCtx::default();
        ctx.cancel();
        s.set_inline_job_ctx(Some(ctx));
        assert!(matches!(s.execute(ID, p), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&before));
        assert!(s.active().unwrap().history.can_redo());
    }
}
