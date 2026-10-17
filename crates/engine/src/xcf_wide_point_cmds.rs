//! Explicit current native affine edit, separate from serialized evaluator support.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::gimp::wide_point::{self, Affine, IdentityPolicy, WorkingContext};
use photocraft_color::checked_arithmetic::{Domain, Policy, Rounding, UnitOverflow};
use photocraft_doc::{ColorMode, Document, LayerContent, LayerId, SampleEncoding, SampleType, WorkingProfileInterpretation, item_identity::ItemRef};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
use serde_json::{Value, json};

const ID: &str = "xcf.layer.affine.apply";
const WORK: u64 = 1 << 30;
fn bad(code: &str) -> EngineError {
    EngineError::Other(format!("{ID}.{code}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled_error(control: &Interrupt<'_>, code: &str) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(code) }
}
fn scalar(params: &Value, key: &str, zero: bool) -> Result<u64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return Err(bad("parameters"));
    }
    let value = text.parse::<u64>().map_err(|_| bad("parameters"))?;
    if value == 0 && !zero {
        return Err(bad("parameters"));
    }
    Ok(value)
}
fn bits(params: &Value, key: &str) -> Result<f64> {
    let text = params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"))?;
    if text.len() != 16 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(bad("parameters"));
    }
    let value = f64::from_bits(u64::from_str_radix(text, 16).map_err(|_| bad("parameters"))?);
    if !value.is_finite() {
        return Err(bad("nonfinite"));
    }
    Ok(value)
}
struct Request {
    document: u64,
    revision: u64,
    layer: LayerId,
    roi: Rect,
    affine: Affine,
}
fn request(params: &Value) -> Result<Request> {
    let object = params.as_object().ok_or_else(|| bad("parameters"))?;
    const KEYS: [&str; 10] =
        ["documentId", "expectedRevision", "layerId", "scaleF64Bits", "offsetF64Bits", "domain", "overflow", "rounding", "identity", "roi"];
    if object.len() != KEYS.len() || object.keys().any(|k| !KEYS.contains(&k.as_str())) {
        return Err(bad("parameters"));
    }
    let roi = params.get("roi").and_then(Value::as_object).ok_or_else(|| bad("parameters"))?;
    if roi.len() != 4 || roi.keys().any(|k| !["x0", "y0", "x1", "y1"].contains(&k.as_str())) {
        return Err(bad("parameters"));
    }
    let coordinate =
        |key| -> Result<i32> { i32::try_from(roi.get(key).and_then(Value::as_i64).ok_or_else(|| bad("parameters"))?).map_err(|_| bad("parameters")) };
    let roi = Rect { x0: coordinate("x0")?, y0: coordinate("y0")?, x1: coordinate("x1")?, y1: coordinate("y1")? };
    if roi.is_empty() {
        return Err(bad("bounds"));
    }
    let choice = |key| params.get(key).and_then(Value::as_str).ok_or_else(|| bad("parameters"));
    let domain = match choice("domain")? {
        "unit" => Domain::Unit,
        "hdr" => Domain::FloatHdr,
        _ => return Err(bad("parameters")),
    };
    let overflow = match choice("overflow")? {
        "clamp" => UnitOverflow::Clamp,
        "reject" => UnitOverflow::Reject,
        _ => return Err(bad("parameters")),
    };
    let rounding = match choice("rounding")? {
        "exact" => Rounding::Exact,
        "nearest" => Rounding::Nearest,
        _ => return Err(bad("parameters")),
    };
    let identity = match choice("identity")? {
        "finite-only" => IdentityPolicy::FiniteOnly,
        "preserve-raw" => IdentityPolicy::PreserveRaw,
        _ => return Err(bad("parameters")),
    };
    if domain == Domain::FloatHdr && overflow == UnitOverflow::Clamp {
        return Err(bad("parameters"));
    }
    Ok(Request {
        document: scalar(params, "documentId", false)?,
        revision: scalar(params, "expectedRevision", true)?,
        layer: LayerId(scalar(params, "layerId", false)?),
        roi,
        affine: Affine {
            scale: bits(params, "scaleF64Bits")?,
            offset: bits(params, "offsetF64Bits")?,
            policy: Policy { domain, unit_overflow: overflow, rounding },
            identity,
        },
    })
}

/// Bound the tree before existing recursive layer/lock accessors or edit cloning.
fn tree(document: &Document) -> Result<()> {
    let mut frames: [Option<std::slice::Iter<'_, photocraft_doc::Layer>>; photocraft_doc::MAX_GROUP_DEPTH + 1] = std::array::from_fn(|_| None);
    *frames.first_mut().ok_or_else(|| bad("limit"))? = Some(document.layers.iter());
    let (mut depth, mut count) = (0usize, 0usize);
    loop {
        let next = frames.get_mut(depth).and_then(Option::as_mut).and_then(Iterator::next);
        let Some(layer) = next else {
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        };
        count = count.checked_add(1).filter(|n| *n <= 10_000).ok_or_else(|| bad("limit"))?;
        if let LayerContent::Group(group) = &layer.content {
            depth = depth.checked_add(1).filter(|n| *n <= photocraft_doc::MAX_GROUP_DEPTH).ok_or_else(|| bad("limit"))?;
            *frames.get_mut(depth).ok_or_else(|| bad("limit"))? = Some(group.children.iter());
        }
    }
    Ok(())
}
fn context(session: &Session) -> Result<()> {
    if session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_is_background()
        || session.command_has_pending_job()
        || session.has_jobs()
    {
        return Err(bad("context"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.floating.is_some() || state.doc.accounted().is_some() || state.revision == u64::MAX {
        return Err(bad("context"));
    }
    tree(&state.doc)
}
fn target<'a>(document: &'a Document, req: &Request) -> Result<&'a Surface> {
    if document.xcf_state.as_ref().is_some_and(|state| state.items.len() > 100_000 || state.source_items.len() > 10_000) {
        return Err(bad("limit"));
    }
    crate::commands::check_pixels_unlocked(document, req.layer)?;
    // Validate imported/native alpha lock relationships, but alpha is raw copied:
    // a valid alpha lock does not prohibit this color-only operation.
    crate::xcf_item_locks::transparency_locked(document, req.layer)?;
    let layer = document.layer(req.layer).ok_or(EngineError::NoLayer(req.layer))?;
    let LayerContent::Raster(source) = &layer.content else {
        return Err(bad("unsupported"));
    };
    if !matches!(source.format().sample, SampleType::U32 | SampleType::F64)
        || !matches!(source.format().mode, ColorMode::Rgb | ColorMode::Grayscale)
        || source.format() != document.pixel_format()
    {
        return Err(bad("unsupported"));
    }
    if source.tile_count() > 100_000 || wide_point::required_work(source, req.roi).map_err(|_| bad("limit"))? > WORK {
        return Err(bad("limit"));
    }
    let bounds = if let Some(state) = &document.xcf_state {
        if state.items.len() > 100_000 || state.source_items.len() > 10_000 {
            return Err(bad("limit"));
        }
        if state.source_items.iter().any(|item| item.owner == ItemRef::Layer(req.layer)) {
            return Err(bad("source-carrier"));
        }
        state.items.iter().find(|item| item.item == ItemRef::Layer(req.layer)).and_then(|item| item.raster_bounds).ok_or_else(|| bad("bounds"))?
    } else {
        // Ordinary resident layers have no declared drawable extent. Use only
        // their retained storage extent, never allocating/scanning pixel content.
        let mut bounds = Rect::EMPTY;
        if source.tile_count() > 100_000 {
            return Err(bad("limit"));
        }
        for (coordinate, _) in source.tiles() {
            let x0 = coordinate.tx.checked_mul(TILE_SIZE).ok_or_else(|| bad("bounds"))?;
            let y0 = coordinate.ty.checked_mul(TILE_SIZE).ok_or_else(|| bad("bounds"))?;
            let rect =
                Rect { x0, y0, x1: x0.checked_add(TILE_SIZE).ok_or_else(|| bad("bounds"))?, y1: y0.checked_add(TILE_SIZE).ok_or_else(|| bad("bounds"))? };
            bounds = bounds.union(&rect);
        }
        bounds
    };
    if bounds.is_empty() || req.roi.intersect(&bounds) != req.roi {
        return Err(bad("bounds"));
    }
    Ok(source)
}

pub(crate) fn dispatch_preflight(session: &Session, params: &Value, background: bool) -> Result<()> {
    let req = request(params)?;
    if background {
        return Err(bad("context"));
    }
    context(session)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.doc.id.0 != req.document || state.revision != req.revision {
        return Err(bad("stale-target"));
    }
    target(&state.doc, &req)?;
    Ok(())
}

fn working(document: &Document, control: &Interrupt<'_>) -> Result<WorkingContext> {
    let encoding = document.sample_encoding.ok_or_else(|| bad("color-context"))?;
    let bytes = document.icc_profile.as_ref().filter(|b| !b.is_empty() && b.len() <= 16 << 20).ok_or_else(|| bad("color-context"))?;
    document
        .working_profile_interpretation
        .validate_with_interrupt(Some(bytes), document.mode, control)
        .map_err(|_| controlled_error(control, "color-context"))?;
    let space = match document.mode {
        ColorMode::Rgb => photocraft_cms::ColorSpace::Rgb,
        ColorMode::Grayscale => photocraft_cms::ColorSpace::Gray,
        _ => return Err(bad("color-context")),
    };
    let cms_encoding = match encoding {
        SampleEncoding::Linear => photocraft_cms::xcf_color::Encoding::Linear,
        SampleEncoding::NonlinearProfile => photocraft_cms::xcf_color::Encoding::NonlinearProfile,
        SampleEncoding::Perceptual => photocraft_cms::xcf_color::Encoding::Perceptual,
    };
    match document.working_profile_interpretation {
        WorkingProfileInterpretation::Icc => {
            photocraft_cms::xcf_color::effective_profile(Some(bytes), space, cms_encoding).map_err(|_| bad("color-context"))?;
        }
        _ => {
            photocraft_cms::xcf_color::effective_implicit_profile(bytes, space, cms_encoding).map_err(|_| bad("color-context"))?;
        }
    }
    let mut hash = blake3::Hasher::new();
    for chunk in bytes.chunks(4096) {
        poll(control)?;
        hash.update(chunk);
    }
    Ok(WorkingContext { encoding, profile_identity: *hash.finalize().as_bytes() })
}

pub fn apply_with_interrupt(session: &mut Session, params: &Value, control: &Interrupt<'_>) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    poll(control)?;
    let req = request(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let source = target(&state.doc, &req)?;
    let work = wide_point::required_work(source, req.roi).map_err(|_| bad("limit"))?;
    if work > WORK {
        return Err(bad("limit"));
    }
    let snapshot = state.doc.downgrade();
    let footprint = state.doc.resident_raster_footprint_with(control).map_err(|_| controlled_error(control, "limit"))?;
    let retained =
        footprint.retained_bytes.checked_add(footprint.clone_metadata_bytes.checked_mul(2).ok_or_else(|| bad("limit"))?).ok_or_else(|| bad("limit"))?;
    let remaining = MAX_CHECKED_RASTER_BYTES.checked_sub(retained).ok_or_else(|| bad("limit"))?;
    let working = working(&state.doc, control)?;
    let output =
        wide_point::try_apply(source, req.roi, working, working, req.affine, wide_point::Limits { resident_bytes: remaining, work_units: WORK }, control)
            .map_err(|error| if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("{ID}.{error}")) })?;
    let response = json!({"documentId":req.document.to_string(),"layerId":req.layer.0.to_string(),"applied":true,"assessmentCommand":"xcf.assess"});
    poll(control)?;
    context(session)?;
    let current = session.active().ok_or(EngineError::NoDocument)?;
    let original = snapshot.upgrade().ok_or_else(|| bad("stale-target"))?;
    if current.doc.id.0 != req.document || current.revision != req.revision || !original.shares_state(&current.doc) {
        return Err(bad("stale-target"));
    }
    target(&current.doc, &req)?;
    poll(control)?;
    session.edit_with_interrupt("Apply Native Wide Affine", control, move |document, _| {
        if document.id.0 != req.document {
            return Err(bad("stale-target"));
        }
        target(document, &req)?;
        poll(control)?;
        let layer = document.layer_mut(req.layer).ok_or(EngineError::NoLayer(req.layer))?;
        if !matches!(layer.content, LayerContent::Raster(_)) {
            return Err(bad("stale-target"));
        }
        layer.content = LayerContent::Raster(output);
        Ok(())
    })?;
    Ok(response)
}
fn run(session: &mut Session, params: &Value) -> Result<Value> {
    apply_with_interrupt(session, params, &Interrupt::NONE)
}
fn enabled(session: &Session) -> std::result::Result<(), String> {
    context(session).map_err(|e| e.to_string())
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Apply Native Wide Affine",
        menu: &[],
        shortcut: None,
        params: r#"Exact documentId/expectedRevision/layerId decimal strings; scaleF64Bits/offsetF64Bits lowercase16hex; domain unit|hdr; overflow clamp|reject; rounding exact|nearest; identity finite-only|preserve-raw; roi {x0,y0,x1,y1} i32 endpoints. Current explicit ICC/encoding required; no evaluator admission."#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> Value {
        json!({"documentId":"1","expectedRevision":"0","layerId":"2","scaleF64Bits":"3ff0000000000000","offsetF64Bits":"0000000000000000",
            "domain":"unit","overflow":"reject","rounding":"exact","identity":"preserve-raw","roi":{"x0":0,"y0":0,"x1":1,"y1":1}})
    }
    #[test]
    fn exact_envelope_refuses_aliases_nonfinite_and_unknown_fields() {
        assert!(request(&params()).is_ok());
        for (field, value) in [
            ("layerId", json!(2)),
            ("layerId", json!("02")),
            ("scaleF64Bits", json!("3FF0000000000000")),
            ("scaleF64Bits", json!("7ff0000000000000")),
            ("offsetF64Bits", json!("7ff8000000000001")),
            ("domain", json!("auto")),
        ] {
            let mut p = params();
            p[field] = value;
            assert!(request(&p).is_err());
        }
        let mut p = params();
        p["unknown"] = json!(true);
        assert!(request(&p).is_err());
        let mut p = params();
        p["roi"]["x1"] = json!(2147483648i64);
        assert!(request(&p).is_err());
        let mut p = params();
        p["domain"] = json!("hdr");
        p["overflow"] = json!("clamp");
        assert!(request(&p).is_err());
    }
    #[test]
    fn background_empty_dispatch_refuses_without_installing_document() {
        let session = Session::new();
        assert!(dispatch_preflight(&session, &params(), true).is_err());
        assert!(dispatch_preflight(&session, &params(), false).is_err());
        assert!(session.documents().is_empty());
        assert!(session.journal.is_empty());
    }
}
