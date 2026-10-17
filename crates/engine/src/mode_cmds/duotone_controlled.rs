//! Controlled metadata-only Duotone transition. Stored gray samples, alpha,
//! resolution, ICC and current encoding metadata are never converted here.
use crate::{EngineError, Result, Session};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Duotone, DuotoneInk, Layer, LayerContent, adjust::CurvePoint};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;

const ID: &str = "image.mode.duotone";
const MAX_POINTS: usize = 4096;
const MAX_NAME: usize = 4096;
fn error(message: impl std::fmt::Display) -> EngineError {
    super::bad(ID, message.to_string())
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn count(params: &Value) -> Result<usize> {
    match params.get("type").map(|value| value.as_str().ok_or_else(|| error("type must be a bounded string"))).transpose()?.unwrap_or("duotone") {
        "monotone" => Ok(1),
        "duotone" => Ok(2),
        "tritone" => Ok(3),
        "quadtone" => Ok(4),
        _ => Err(error("type must be monotone, duotone, tritone or quadtone")),
    }
}
fn validate_wire(params: &Value) -> Result<usize> {
    if !params.is_null()
        && !params
            .as_object()
            .is_some_and(|object| object.len() <= 4 && object.keys().all(|key| matches!(key.as_str(), "type" | "inks" | "documentId" | "expectedRevision")))
    {
        return Err(error("expected bounded Duotone parameters"));
    }
    let n = count(params)?;
    if let Some(inks) = params.get("inks") {
        let inks = inks.as_array().filter(|inks| inks.len() <= 4).ok_or_else(|| error("inks must contain at most four ink objects"))?;
        for ink in inks {
            let object = ink
                .as_object()
                .filter(|object| object.len() <= 3 && object.keys().all(|key| matches!(key.as_str(), "name" | "color" | "curve")))
                .ok_or_else(|| error("ink must be a bounded name/color/curve object"))?;
            if let Some(name) = object.get("name")
                && !name.as_str().is_some_and(|name| name.len() <= MAX_NAME && !name.contains('\0'))
            {
                return Err(error("ink name exceeds the supported string domain"));
            }
            if let Some(color) = object.get("color") {
                let bounded = match color {
                    Value::String(text) => text.len() <= 64 && !text.contains('\0'),
                    Value::Array(values) => (3..=4).contains(&values.len()) && values.iter().all(|value| value.as_f64().is_some_and(f64::is_finite)),
                    _ => false,
                };
                if !bounded || super::parse_color(color).is_none() {
                    return Err(error("ink color must be a bounded hex or finite RGB color"));
                }
            }
            if let Some(curve) = object.get("curve") {
                let points =
                    curve.as_array().filter(|points| points.len() <= MAX_POINTS).ok_or_else(|| error("ink curve exceeds the supported point count"))?;
                for point in points {
                    let values = point.as_array().filter(|values| values.len() == 2).ok_or_else(|| error("ink curve points must be input/output pairs"))?;
                    for value in values {
                        let value = value
                            .as_f64()
                            .filter(|value| value.is_finite() && (*value as f32).is_finite())
                            .ok_or_else(|| error("ink curve values must fit finite legacy f32 precision"))?;
                        if !(value as f32 / 100.).is_finite() {
                            return Err(error("ink curve percentage exceeds supported precision"));
                        }
                    }
                }
            }
        }
    }
    Ok(n)
}
/// Cheap borrowed checks before dispatcher float normalization and Value cloning.
/// Valid no-pair legacy requests keep existing ordinary auto-anchor behavior.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    validate_wire(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if !matches!(state.doc.mode, ColorMode::Grayscale | ColorMode::Duotone) {
        return Err(error("Duotone requires Grayscale or Duotone"));
    }
    if !matches!(state.doc.depth, SampleType::U8 | SampleType::U16 | SampleType::F32) || state.doc.xcf_state.is_some() || state.doc.color_table.is_some() {
        return Err(error("wide/current indexed/imported-source Duotone rendering is not qualified"));
    }
    if state.revision == u64::MAX || session.command_has_pending_job() || session.command_has_job_on(state.doc.id) {
        return Err(error("target revision/pending work refused"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) => {
            let exact = |value: &Value| -> Result<u64> {
                let text = value
                    .as_str()
                    .filter(|text| {
                        !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0'))
                    })
                    .ok_or_else(|| error("captured IDs must be canonical decimal strings"))?;
                text.parse().map_err(|_| error("captured ID exceeds range"))
            };
            let id = exact(id)?;
            if id == 0 || id != state.doc.id.0 || exact(revision)? != state.revision {
                return Err(error("captured Duotone target changed"));
            }
            if state.floating.is_some()
                || state.coalesce.is_some()
                || session.has_jobs()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(error("captured Duotone target has floating or transient work"));
            }
        }
        _ => return Err(error("documentId and expectedRevision must be supplied together")),
    }
    Ok(())
}
fn text(value: &str) -> Result<String> {
    let mut out = String::new();
    out.try_reserve_exact(value.len()).map_err(|_| error("ink name allocation failed"))?;
    if out.capacity() > value.len().saturating_add(1024) {
        return Err(error("ink name capacity exceeded"));
    }
    out.push_str(value);
    Ok(out)
}
fn default_ink(index: usize) -> Result<DuotoneInk> {
    const DEFAULTS: [(&str, [f32; 3]); 4] =
        [("Black", [0., 0., 0.]), ("Warm Brown", [0.62, 0.38, 0.18]), ("Gold", [0.86, 0.68, 0.18]), ("Slate Blue", [0.27, 0.38, 0.58])];
    let (name, color) = DEFAULTS.get(index).ok_or_else(|| error("ink index exceeds default table"))?;
    let mut curve = Vec::new();
    curve.try_reserve_exact(2).map_err(|_| error("default curve allocation failed"))?;
    curve.push(CurvePoint { input: 0., output: 0. });
    curve.push(CurvePoint { input: 1., output: 1. });
    Ok(DuotoneInk { name: text(name)?, color: *color, curve })
}
fn decode(params: &Value, control: &Interrupt<'_>) -> Result<Vec<DuotoneInk>> {
    let n = validate_wire(params)?;
    let mut inks = Vec::new();
    inks.try_reserve_exact(n).map_err(|_| error("ink allocation failed"))?;
    for index in 0..n {
        poll(control)?;
        let mut ink = default_ink(index)?;
        if let Some(value) = params.get("inks").and_then(Value::as_array).and_then(|inks| inks.get(index)) {
            if let Some(name) = value.get("name").and_then(Value::as_str) {
                ink.name = text(name)?;
            }
            if let Some(color) = value.get("color") {
                ink.color = super::parse_color(color).ok_or_else(|| error("invalid ink color"))?.map(|value| f32::from(value) / 255.);
            }
            if let Some(points) = value.get("curve").and_then(Value::as_array).filter(|points| points.len() >= 2) {
                let mut curve = Vec::new();
                curve.try_reserve_exact(points.len()).map_err(|_| error("curve allocation failed"))?;
                if curve.capacity() > points.len().saturating_add(128) {
                    return Err(error("curve capacity exceeded"));
                }
                for point in points {
                    poll(control)?;
                    let input = point.get(0).and_then(Value::as_f64).ok_or_else(|| error("missing curve input"))? as f32 / 100.;
                    let output = point.get(1).and_then(Value::as_f64).ok_or_else(|| error("missing curve output"))? as f32 / 100.;
                    curve.push(CurvePoint { input, output });
                }
                // Stable insertion sort avoids allocating a hidden merge buffer.
                // Ties retain the old stable sort order and legacy f32 inputs.
                for i in 1..curve.len() {
                    poll(control)?;
                    let mut j = i;
                    while j > 0 {
                        poll(control)?;
                        let a = curve.get(j - 1).ok_or_else(|| error("curve sort index"))?;
                        let b = curve.get(j).ok_or_else(|| error("curve sort index"))?;
                        if !a.input.total_cmp(&b.input).is_gt() {
                            break;
                        }
                        curve.swap(j - 1, j);
                        j -= 1;
                    }
                }
                ink.curve = curve;
            }
        }
        inks.push(ink);
    }
    poll(control)?;
    Ok(inks)
}
fn layer_census(layers: &[Layer], depth: usize, count: &mut u64, control: &Interrupt<'_>) -> Result<()> {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(error("layer hierarchy exceeds limit"));
    }
    for layer in layers {
        poll(control)?;
        *count = count.checked_add(1).filter(|value| *value <= 100_000).ok_or_else(|| error("layer census limit"))?;
        if layer.import_compositing.is_some()
            || layer.indexed_source.is_some()
            || layer.video.is_some()
            || matches!(&layer.content, LayerContent::BackedRaster(_))
        {
            return Err(error("imported/indexed/backed Duotone interpretation is not qualified"));
        }
        if layer.surface().is_some_and(|surface| !matches!(surface.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)) {
            return Err(error("wide Duotone rendering is not qualified"));
        }
        if layer.mask.as_ref().is_some_and(|mask| wide(&mask.surface))
            || layer.fill_cache.as_ref().is_some_and(|cache| wide(&cache.surface))
            || matches!(&layer.content, LayerContent::Smart(smart) if smart.filter_mask.as_ref().is_some_and(|mask| wide(&mask.surface)))
        {
            return Err(error("wide retained Duotone mask/cache rendering is not qualified"));
        }
        if let LayerContent::Group(group) = &layer.content {
            layer_census(&group.children, depth + 1, count, control)?;
        }
    }
    Ok(())
}
fn wide(surface: &photocraft_raster::Surface) -> bool {
    !matches!(surface.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32)
}
fn prepare(snapshot: &crate::DocumentSnapshot, params: &Value, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    snapshot.validate_resident()?;
    let mut layers = 0;
    layer_census(&snapshot.layers, 0, &mut layers, control)?;
    for channel in &snapshot.channels {
        poll(control)?;
        if wide(&channel.surface) {
            return Err(error("wide retained Duotone channel rendering is not qualified"));
        }
    }
    if snapshot.selection.as_ref().is_some_and(wide) || snapshot.quick_mask.as_ref().is_some_and(|mask| wide(&mask.surface)) {
        return Err(error("wide retained Duotone coverage rendering is not qualified"));
    }
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    // Known metadata/payload/map slice, not an RSS or complete legacy allocator
    // overhead guarantee. No samples are read or rewritten for this transition.
    let peak = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        // Also cover the bounded cloned JSON request's Value nodes/numbers,
        // transient default curves, response names and sort bookkeeping.
        .and_then(|n| n.checked_add(4 * (MAX_NAME as u64 + MAX_POINTS as u64 * 8) + (2 << 20)))
        .ok_or_else(|| error("Duotone residency overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("Duotone metadata/source residency limit"));
    }
    let points = params
        .get("inks")
        .and_then(Value::as_array)
        .map_or(0, |inks| inks.iter().filter_map(|ink| ink.get("curve").and_then(Value::as_array)).map(Vec::len).sum::<usize>()) as u64;
    let work = points.checked_mul(MAX_POINTS as u64).and_then(|n| n.checked_add(layers.checked_mul(64)?)).ok_or_else(|| error("Duotone work overflow"))?;
    if work > 100_000_000 {
        return Err(error("Duotone metadata work limit"));
    }
    poll(control)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work_base = baseline.clone();
    // Dispatcher already bounded this envelope; clone only the bounded request.
    let params = params.clone();
    crate::jobs::run(
        session,
        "Duotone",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Duotone", |control| {
                prepare(&work_base, &params, control)?;
                let inks = decode(&params, control)?;
                let mut names = Vec::new();
                names.try_reserve_exact(inks.len()).map_err(|_| error("response name allocation failed"))?;
                for ink in &inks {
                    poll(control)?;
                    names.push(Value::String(text(&ink.name)?));
                }
                let mut response = serde_json::Map::new();
                response.insert("inks".into(), Value::Array(names));
                poll(control)?;
                let mut document = (*work_base).clone();
                document.mode = ColorMode::Duotone;
                document.duotone = Some(Duotone { inks, psd_raw: None });
                document.color_table = None;
                poll(control)?;
                Ok((document, Value::Object(response), ctx.clone()))
            })
        },
        move |session, (document, response, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id || current.revision != revision || !current.doc.shares_state(&baseline) {
                return Err(error("target changed while preparing Duotone"));
            }
            ctx.stage(0.95, 1., "Duotone", |control| {
                poll(control)?;
                session.edit_with_interrupt("Duotone", control, move |current, _| {
                    *current = document;
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
    use serde_json::json;
    fn fixture() -> Session {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":1,"mode":"grayscale","depth":16})).unwrap();
        session
            .edit("fixture", |document, active| {
                document.physical_resolution = Some(photocraft_doc::PhysicalResolution { y_dpi: 288., ..photocraft_doc::PhysicalResolution::square(144.) });
                document.resolution_dpi = 72.;
                let bounds = document.bounds();
                let surface = document.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                let format = surface.format();
                let pixels: Vec<f32> = [[0.12345, 0.12345, 0.12345, 0.375], [0.875, 0.875, 0.875, 1.]]
                    .into_iter()
                    .flat_map(|pixel| photocraft_raster::from_rgba(&format, pixel))
                    .collect();
                surface.write_region(bounds, &pixels);
                Ok(())
            })
            .unwrap();
        session
    }
    #[test]
    fn bounded_parser_preserves_defaults_fill_truncate_and_stable_curve_math() {
        let params = json!({"type":"tritone","inks":[{"name":"Test ink","color":[120.5,45.2,255],"curve":[[100,80],[20,30],[20,10],[0,0]]}]});
        let inks = decode(&params, &Interrupt::NONE).unwrap();
        assert_eq!(inks.len(), 3);
        assert_eq!(inks[0].name, "Test ink");
        assert_eq!(inks[0].color, [121. / 255., 45. / 255., 1.]);
        assert_eq!(
            inks[0].curve,
            vec![
                CurvePoint { input: 0., output: 0. },
                CurvePoint { input: 0.2, output: 0.3 },
                CurvePoint { input: 0.2, output: 0.1 },
                CurvePoint { input: 1., output: 0.8 }
            ]
        );
        assert_eq!(inks[1], default_ink(1).unwrap());
        assert_eq!(inks[2], default_ink(2).unwrap());
        let monotone = decode(&json!({"type":"monotone","inks":[{"name":"First","curve":[[20,50]]},{"name":"Unused"}]}), &Interrupt::NONE).unwrap();
        assert_eq!(monotone.len(), 1);
        assert_eq!(monotone[0].name, "First");
        assert_eq!(monotone[0].curve, default_ink(0).unwrap().curve);
    }
    #[test]
    fn malformed_and_oversized_requests_refuse_before_clone_or_history() {
        let session = fixture();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        for params in [
            json!({"inks":[null]}),
            json!({"inks":[{"curve":[[0]]}]}),
            json!({"inks":[{"name":"x".repeat(MAX_NAME+1)}]}),
            json!({"inks":[{}, {}, {}, {}, {}]}),
            json!({"inks":[{"curve":vec![[0,0];MAX_POINTS+1]}]}),
            json!({"type":"other"}),
            json!({"documentId":before.id.0.to_string()}),
        ] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
            assert!(session.active().unwrap().doc.shares_state(&before));
            assert_eq!(session.active().unwrap().history.past_len(), history);
        }
    }
    #[test]
    fn metadata_transition_preserves_native_gray_alpha_and_dual_dpi_one_undo() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let native = before.layers[0].surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap();
        let history = session.active().unwrap().history.past_len();
        let result = run(&mut session, &json!({"type":"duotone","inks":[{"name":"Blue","color":"#123456"}]})).unwrap();
        assert_eq!(result, json!({"inks":["Blue","Warm Brown"]}));
        let state = session.active().unwrap();
        assert_eq!(state.history.past_len(), history + 1);
        assert_eq!(state.doc.mode, ColorMode::Duotone);
        assert_eq!(state.doc.depth, before.depth);
        assert_eq!(state.doc.physical_resolution, before.physical_resolution);
        assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
        assert_eq!(state.doc.sample_encoding, before.sample_encoding);
        assert_eq!(state.doc.working_profile_interpretation, before.working_profile_interpretation);
        assert_eq!(state.doc.icc_profile, before.icc_profile);
        assert_eq!(state.doc.layers[0].surface().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), native);
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.mode, ColorMode::Grayscale);
        assert_eq!(session.active().unwrap().doc.duotone, before.duotone);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.duotone.as_ref().unwrap().inks[0].name, "Blue");
    }
    #[test]
    fn cancelled_metadata_preparation_preserves_source_snapshot() {
        let session = fixture();
        let snapshot = session.active().unwrap().doc.clone();
        let cancel = || true;
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(prepare(&snapshot, &json!({}), &control), Err(EngineError::Cancelled)));
        assert!(matches!(decode(&json!({}), &control), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&snapshot));
    }
}
