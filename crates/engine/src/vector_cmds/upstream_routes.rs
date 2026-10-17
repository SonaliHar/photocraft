//! Admission for additive upstream mapping and multi-tool/pressure stroke routes.
//! Their legacy inner render/tool boundaries are not deep-cancellation guarantees.
use super::{bad, layer_id};
use crate::{EngineError, Result, Session};
use photocraft_doc::LayerContent;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
pub(super) fn mapping(p: &Value) -> bool {
    ["quad", "warp", "style", "mesh"].iter().any(|k| p.get(*k).is_some())
}
pub(super) fn stroke(p: &Value) -> bool {
    p.get("simulatePressure").is_some() || p.get("tool").and_then(Value::as_str).is_some_and(|v| !matches!(v, "brush" | "pencil" | "eraser"))
}
fn decimal(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    if s.is_empty() || s.len() > 20 || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|v| v.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}
fn envelope(v: &Value, depth: usize, n: &mut usize, bytes: &mut usize) -> bool {
    if depth > 12 {
        return false;
    }
    let Some(next) = n.checked_add(1).filter(|v| *v <= 16384) else {
        return false;
    };
    *n = next;
    match v {
        Value::String(s) => {
            if s.len() > 4096 {
                return false;
            }
            let Some(next) = bytes.checked_add(s.len()).filter(|v| *v <= 65536) else {
                return false;
            };
            *bytes = next;
            true
        }
        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
        Value::Array(a) => a.len() <= 4096 && a.iter().all(|v| envelope(v, depth + 1, n, bytes)),
        Value::Object(m) => m.len() <= 32 && m.iter().all(|(k, v)| k.len() <= 128 && envelope(v, depth + 1, n, bytes)),
        Value::Null | Value::Bool(_) => true,
    }
}
pub(super) fn preflight(s: &Session, p: &Value, command: &str, normalize: bool) -> Result<()> {
    if !p.is_object() || !envelope(p, 0, &mut 0, &mut 0) {
        return Err(bad(command, "bounded upstream vector parameters required"));
    }
    if p.get("name").is_some_and(|v| !v.as_str().is_some_and(|v| v.len() <= 2048 && !v.contains('\0'))) {
        return Err(bad(command, "bounded selector name required"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let captured = match (p.get("documentId"), p.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if decimal(id) == Some(st.doc.id.0) && st.doc.id.0 != 0 && decimal(rev) == Some(st.revision) => true,
        _ => return Err(bad(command, "captured vector target changed or incomplete")),
    };
    if st.revision == u64::MAX
        || st.doc.accounted().is_some()
        || s.has_jobs()
        || s.command_has_pending_job()
        || s.command_has_job_on(st.doc.id)
        || s.color_restrict.is_some()
        || (st.floating.is_some() && (!normalize || captured || st.doc.xcf_state.is_some()))
        || st.doc.xcf_state.is_some()
    {
        return Err(bad(command, "source/transient upstream vector route is unsupported"));
    }
    let ctl = &Interrupt::NONE;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(&st.doc, preparation, ctl).map_err(|e| bad(command, e.to_string()))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(&st.doc, preparation, ctl).map_err(|e| bad(command, e.to_string()))? as u64;
    let raster = st.doc.resident_raster_footprint_with(ctl)?;
    let id = if command == "path.stroke" || p.get("name").and_then(Value::as_str) == Some("layer") { Some(layer_id(s, p)?) } else { None };
    let layer = id.map(|id| st.doc.layer(id).ok_or(EngineError::NoLayer(id))).transpose()?;
    let selected = if p.get("name").and_then(Value::as_str) == Some("layer") {
        let layer = layer.ok_or_else(|| bad(command, "no path owner"))?;
        match &layer.content {
            LayerContent::Shape(sh) => &sh.path,
            _ => &layer.vector_mask.as_ref().ok_or_else(|| bad(command, "no vector mask path"))?.path,
        }
    } else {
        super::path_fill_controlled::path(s, p, ctl)?
    };
    photocraft_vector::flatten::validate_shape(selected, None).map_err(|e| bad(command, e))?;
    if command == "path.stroke" {
        let id = id.ok_or_else(|| bad(command, "no stroke target"))?;
        let layer = layer.ok_or(EngineError::NoLayer(id))?;
        crate::commands::check_pixels_unlocked(&st.doc, id)?;
        let surface = layer.surface().ok_or_else(|| bad(command, "Stroke Path needs a pixel layer"))?;
        if matches!(surface.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64)
            || st.doc.mode == photocraft_color::ColorMode::Indexed
            || layer.indexed_source.is_some()
            || layer.video.is_some()
            || layer.import_compositing.is_some()
            || !matches!(layer.content, LayerContent::Raster(_))
        {
            return Err(bad(command, "source/wide stroke interpretation is unsupported"));
        }
        if st.doc.selection.as_ref().is_some_and(|v| matches!(v.format().sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64)) {
            return Err(bad(command, "wide selected stroke is unsupported"));
        }
        if let Some(value) = p.get("simulatePressure")
            && !value.is_null()
            && !value.is_boolean()
        {
            return Err(bad(command, "simulatePressure must be boolean"));
        }
        if super::path_stroke_controlled::brush_bytes(&s.tools.brush)? > 64 << 20 {
            return Err(bad(command, "upstream tool brush staging bound"));
        }
    } else if let (Some(id), Some(layer)) = (id, layer)
        && matches!(layer.content, LayerContent::Shape(_))
    {
        let locks = st.doc.effective_locks(id);
        if locks.all || locks.pixels || locks.position {
            return Err(bad(command, "shape owner is locked"));
        }
        if matches!(st.doc.depth, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) {
            return Err(bad(command, "wide shape regeneration is unsupported"));
        }
    }
    if command == "path.transform" {
        if let Some(matrix) = p.get("matrix") {
            let values = matrix.as_array().filter(|v| v.len() == 6).ok_or_else(|| bad(command, "matrix must have six values"))?;
            if values.iter().any(|v| !v.as_f64().is_some_and(|v| v.is_finite() && v.abs() <= 1e6)) {
                return Err(bad(command, "invalid matrix values"));
            }
        } else {
            let frame = if let Some(v) = super::coords(p, "rect", 4)? {
                [v[0], v[1], v[2], v[3]]
            } else {
                let (x0, y0, x1, y1) = selected.control_bounds().ok_or_else(|| bad(command, "path has no points"))?;
                [x0, y0, x1, y1]
            };
            if frame[2] - frame[0] <= 0.0 || frame[3] - frame[1] <= 0.0 {
                return Err(bad(command, "mapping frame must have positive sides"));
            }
            if let Some(q) = super::coords(p, "quad", 8)? {
                let quad = [[q[0], q[1]], [q[2], q[3]], [q[4], q[5]], [q[6], q[7]]];
                photocraft_algo::transform::Homography::rect_to_quad(frame, quad)
                    .filter(|h| h.0.iter().all(|v| v.is_finite()) && h.inverse().is_some())
                    .ok_or_else(|| bad(command, "degenerate quad"))?;
            } else {
                let w = crate::warp_cmds::warp_from_params(command, p, frame)?;
                if [w.bend, w.h_distort, w.v_distort].iter().chain(w.bounds.iter()).any(|v| !v.is_finite() || v.abs() > 1e6) {
                    return Err(bad(command, "invalid warp range"));
                }
            }
        }
    }
    let renders = command == "path.stroke" || layer.is_some_and(|l| matches!(l.content, LayerContent::Shape(_)));
    let pixels =
        if renders { u64::from(st.doc.size.width).checked_mul(u64::from(st.doc.size.height)).ok_or_else(|| bad(command, "vector output count"))? } else { 0 };
    if pixels.checked_mul(2048).is_none_or(|v| v > 4_000_000_000)
        || model
            .checked_mul(4)
            .and_then(|v| v.checked_add(payload))
            .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|v| v.checked_add(pixels.checked_mul(128)?))
            .and_then(|v| v.checked_add(256 << 20))
            .is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad(command, "upstream vector preparation/output limit"));
    }
    Ok(())
}
