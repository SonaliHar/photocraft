//! Controlled current guide metadata. Original imported provenance is untouched.
use crate::{EngineError, Result, Session};
use photocraft_doc::{
    Document, Guides, PhysicalResolution, PhysicalUnit,
    physical_size::{self, Axis},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::Value;
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("guide: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
#[derive(Clone, Copy)]
struct Plan {
    vertical: bool,
    position: Option<f32>,
    index: Option<usize>,
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
fn plan(session: &Session, params: &Value, op: &str) -> Result<Plan> {
    if !matches!(op, "new" | "move" | "delete" | "clear") {
        return Err(error("unknown guide operation"));
    }
    if !params.is_null()
        && !params.as_object().is_some_and(|object| {
            object.len() <= 6
                && object.keys().all(|key| matches!(key.as_str(), "orientation" | "position" | "unit" | "index" | "documentId" | "expectedRevision"))
        })
    {
        return Err(error("expected bounded guide parameters"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let document = &state.doc;
    if document.guides.horizontal.len() > 100_000 || document.guides.vertical.len() > 100_000 {
        return Err(error("guide count limit"));
    }
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(document.id.0) && document.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error("captured target changed or incomplete; reopen guide dialog")),
    }
    let vertical = match params.get("orientation") {
        None => false,
        Some(Value::String(value)) if value == "horizontal" => false,
        Some(Value::String(value)) if value == "vertical" => true,
        _ => return Err(error("orientation must be horizontal or vertical")),
    };
    let legacy = PhysicalResolution::square(f64::from(document.resolution_dpi));
    let resolution = document.physical_resolution.as_ref().unwrap_or(&legacy);
    // Borrow custom unit fields; never clone an unbounded definition to parse a
    // signed position. Shared converter remains the only coordinate arithmetic.
    if let PhysicalUnit::Custom(unit) = &resolution.unit {
        if [&unit.name, &unit.symbol, &unit.abbreviation].iter().any(|value| value.len() > 4096) {
            return Err(error("custom guide unit metadata limit"));
        }
    }
    let fixed = match params.get("unit") {
        None | Some(Value::String(_)) if params.get("unit").is_none_or(|value| value.as_str() == Some("px")) => Some(PhysicalUnit::Pixel),
        Some(Value::String(value)) => match value.as_str() {
            "in" => Some(PhysicalUnit::Inch),
            "mm" => Some(PhysicalUnit::Millimeter),
            "pt" => Some(PhysicalUnit::Point),
            "pc" => Some(PhysicalUnit::Pica),
            "document" => None,
            _ => return Err(error("unit must be px, in, mm, pt, pc or document")),
        },
        _ => return Err(error("unit must be a bounded known string")),
    };
    let unit = fixed.as_ref().unwrap_or(&resolution.unit);
    let position = params
        .get("position")
        .map(|value| -> Result<f32> {
            let coordinate = value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("position must be finite"))?;
            let axis = if vertical { Axis::X } else { Axis::Y };
            let pixels = physical_size::units_to_pixels(coordinate.abs(), resolution, unit, axis).map_err(error)?.copysign(coordinate);
            let value = pixels as f32;
            if !value.is_finite() {
                return Err(error("position exceeds finite native f32 storage"));
            }
            Ok(value)
        })
        .transpose()?;
    let index = params
        .get("index")
        .map(|value| value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or_else(|| error("index must be a representable unsigned integer")))
        .transpose()?;
    let guides = if vertical { &document.guides.vertical } else { &document.guides.horizontal };
    if matches!(op, "new" | "move") && position.is_none() {
        return Err(error("missing position"));
    }
    if matches!(op, "move" | "delete") && index.is_none_or(|index| index >= guides.len()) {
        return Err(error("no such guide"));
    }
    if op == "new" && guides.len() >= 100_000 {
        return Err(error("guide count limit"));
    }
    Ok(Plan { vertical, position, index })
}
/// Root calls before float normalization or cloning the foreign parameter AST.
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, op: &str, _background: bool) -> Result<()> {
    plan(session, params, op)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
    {
        return Err(error("idle uncoalesced guide context required"));
    }
    Ok(())
}
fn prepare(document: &crate::DocumentSnapshot, control: &Interrupt<'_>) -> Result<()> {
    poll(control)?;
    document.validate_resident()?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(document, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(document, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = document.resident_raster_footprint_with(control)?;
    let peak = model
        .checked_mul(4)
        .and_then(|value| value.checked_add(payload))
        .and_then(|value| value.checked_add(raster.retained_bytes))
        .and_then(|value| value.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|value| value.checked_add(2_000_000))
        .ok_or_else(|| error("metadata residency overflow"))?;
    // Known structural/payload/raster-map slice, not whole-process RSS.
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("guide metadata/source residency limit"));
    }
    poll(control)
}
fn copied(values: &[f32], op: &str, selected: bool, plan: Plan, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    if op == "clear" {
        poll(control)?;
        return Ok(Vec::new());
    }
    let requested = values.len().checked_add(usize::from(selected && op == "new")).ok_or_else(|| error("guide allocation overflow"))?;
    let mut output = Vec::new();
    output.try_reserve_exact(requested).map_err(|_| error("guide allocation denied"))?;
    if output.capacity() > requested.saturating_add(1024) {
        return Err(error("guide capacity exceeded"));
    }
    for (index, value) in values.iter().enumerate() {
        if index.is_multiple_of(1024) {
            poll(control)?;
        }
        if !value.is_finite() {
            return Err(error("nonfinite existing guide coordinate"));
        }
        if selected && op == "delete" && plan.index == Some(index) {
            continue;
        }
        output.push(if selected && op == "move" && plan.index == Some(index) {
            plan.position.ok_or_else(|| error("missing prepared position"))?
        } else {
            *value
        });
    }
    if selected && op == "new" {
        poll(control)?;
        output.push(plan.position.ok_or_else(|| error("missing prepared position"))?);
    }
    poll(control)?;
    Ok(output)
}
fn guides(document: &Document, plan: Plan, op: &str, control: &Interrupt<'_>) -> Result<Guides> {
    if document.guides.horizontal.len() > 100_000 || document.guides.vertical.len() > 100_000 {
        return Err(error("guide copy work limit"));
    }
    Ok(Guides {
        horizontal: copied(&document.guides.horizontal, op, !plan.vertical, plan, control)?,
        vertical: copied(&document.guides.vertical, op, plan.vertical, plan, control)?,
    })
}
pub(crate) fn run(session: &mut Session, params: &Value, op: &str) -> Result<Value> {
    dispatch_preflight(session, params, op, false)?;
    let plan = plan(session, params, op)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let current = if plan.vertical { &baseline.guides.vertical } else { &baseline.guides.horizontal };
    if (op == "clear" && baseline.guides.vertical.is_empty() && baseline.guides.horizontal.is_empty())
        || (op == "move" && plan.index.and_then(|index| current.get(index)).zip(plan.position).is_some_and(|(old, new)| old.to_bits() == new.to_bits()))
    {
        return Ok(Value::Null);
    }
    let label = match op {
        "new" => "New Guide",
        "move" => "Move Guide",
        "delete" => "Delete Guide",
        _ => "Clear Guides",
    };
    let op = match op {
        "new" => "new",
        "move" => "move",
        "delete" => "delete",
        _ => "clear",
    };
    let work = baseline.clone();
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0., 0.95, label, |control| {
                prepare(&work, control)?;
                let prepared = guides(&work, plan, op, control)?;
                poll(control)?;
                Ok((prepared, ctx.clone()))
            })
        },
        move |session, (prepared, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id || state.revision != revision || !state.doc.shares_state(&baseline) {
                return Err(error("target changed during guide preparation"));
            }
            ctx.stage(0.95, 1., label, |control| {
                poll(control)?;
                session.edit_with_interrupt(label, control, move |document, _| {
                    document.guides = prepared;
                    Ok(())
                })?;
                Ok(Value::Null)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::Size;
    use serde_json::json;
    fn fixture() -> Session {
        let mut document = Document::with_background("guide metadata", Size::new(2, 2), ColorMode::Rgb, SampleType::F64, Color::WHITE);
        document.physical_resolution = Some(PhysicalResolution { y_dpi: 288., ..PhysicalResolution::square(144.) });
        document.resolution_dpi = 72.;
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn signed_dual_axis_guides_preserve_raw_pixels_and_resolution_one_undo() {
        let mut session = fixture();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let native = before.layers[0].surface().unwrap().try_read_interleaved_with(before.bounds(), &Interrupt::NONE).unwrap();
        assert_eq!(run(&mut session, &json!({"orientation":"vertical","position":-1.0,"unit":"in"}), "new").unwrap(), Value::Null);
        run(&mut session, &json!({"position":-25.4,"unit":"mm"}), "new").unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.doc.guides.vertical, vec![-144.]);
        assert_eq!(state.doc.guides.horizontal, vec![-288.]);
        assert_eq!(state.history.past_len(), history + 2);
        assert_eq!(state.doc.physical_resolution, before.physical_resolution);
        assert_eq!(state.doc.resolution_dpi, before.resolution_dpi);
        assert_eq!(state.doc.depth, before.depth);
        assert_eq!(state.doc.icc_profile, before.icc_profile);
        assert_eq!(state.doc.sample_encoding, before.sample_encoding);
        assert_eq!(state.doc.layers[0].surface().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), native);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.guides.horizontal.is_empty());
        assert_eq!(session.active().unwrap().doc.guides.vertical, vec![-144.]);
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.guides.horizontal, vec![-288.]);
    }
    #[test]
    fn clear_noop_and_stale_capture_keep_real_redo() {
        let mut session = fixture();
        run(&mut session, &json!({"position":12.5}), "new").unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_label().unwrap().to_string();
        run(&mut session, &Value::Null, "clear").unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
        assert!(
            dispatch_preflight(
                &session,
                &json!({"documentId":before.id.0.to_string(),"expectedRevision":revision.saturating_add(1).to_string(),"position":0}),
                "new",
                false
            )
            .is_err()
        );
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
        session.execute("edit.redo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.guides.horizontal, vec![12.5]);
    }
    #[test]
    fn controlled_copy_cancel_and_invalid_shapes_leave_guides_untouched() {
        let session = fixture();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        for params in [
            json!({"orientation":"diagonal","position":1}),
            json!({"position":"1"}),
            json!({"position":1,"unit":"bogus"}),
            json!({"index":999999,"position":1}),
            json!({"position":1,"documentId":before.id.0.to_string()}),
        ] {
            let operation = if params.get("index").is_some() { "move" } else { "new" };
            assert!(dispatch_preflight(&session, &params, operation, false).is_err());
        }
        let cancel = || true;
        let control = Interrupt::cancel_only(&cancel);
        let planned = plan(&session, &json!({"position":1}), "new").unwrap();
        assert!(matches!(prepare(&before, &control), Err(EngineError::Cancelled)));
        assert!(matches!(guides(&before, planned, "new", &control), Err(EngineError::Cancelled)));
        assert!(state.doc.shares_state(&before));
    }
    #[test]
    fn move_delete_preserve_other_axis_words_and_order() {
        let mut session = fixture();
        for position in [-0.0, 5.5, 12.5] {
            run(&mut session, &json!({"orientation":"vertical","position":position}), "new").unwrap();
        }
        run(&mut session, &json!({"position":2.25}), "new").unwrap();
        run(&mut session, &json!({"orientation":"vertical","index":1,"position":72,"unit":"pt"}), "move").unwrap();
        run(&mut session, &json!({"orientation":"vertical","index":2}), "delete").unwrap();
        let guides = &session.active().unwrap().doc.guides;
        assert_eq!(guides.vertical.iter().map(|value| value.to_bits()).collect::<Vec<_>>(), vec![(-0.0f32).to_bits(), 144f32.to_bits()]);
        assert_eq!(guides.horizontal, vec![2.25]);
    }
}
