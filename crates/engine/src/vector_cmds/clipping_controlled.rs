//! Controlled PSD clipping-path metadata; retained path geometry is never rewritten.
use super::bad;
use crate::{DocumentSnapshot, EngineError, Result, Session};
use photocraft_doc::{ClippingPath, Document, item_identity::ItemRef};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
fn error(command: &str, ctl: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if ctl.cancelled() { EngineError::Cancelled } else { bad(command, error.to_string()) }
}
fn decimal(value: &Value, nonzero: bool, command: &str) -> Result<u64> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0')))
        .ok_or_else(|| bad(command, "captured IDs must be canonical decimal strings"))?;
    let n = s.parse::<u64>().map_err(|_| bad(command, "captured ID overflow"))?;
    if nonzero && n == 0 {
        return Err(bad(command, "zero document ID"));
    }
    Ok(n)
}
struct Request<'a> {
    name: Option<&'a str>,
    flatness: f32,
}
fn request<'a>(session: &Session, command: &str, params: &'a Value, normalize: bool) -> Result<Request<'a>> {
    let set = match command {
        "path.clippingPath.set" => true,
        "path.clippingPath.clear" => false,
        _ => return Err(bad(command, "unknown clipping command")),
    };
    if !params.is_null() {
        let object = params.as_object().ok_or_else(|| bad(command, "clipping parameters must be an object or null"))?;
        if object.len() > 4
            || object.keys().any(|k| !matches!(k.as_str(), "documentId" | "expectedRevision") && !(set && matches!(k.as_str(), "name" | "flatness")))
        {
            return Err(bad(command, "unknown clipping parameter"));
        }
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let captured = match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => false,
        (Some(id), Some(rev)) if decimal(id, true, command)? == state.doc.id.0 && decimal(rev, false, command)? == state.revision => true,
        _ => return Err(bad(command, "captured clipping target changed or incomplete")),
    };
    if state.revision == u64::MAX
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
        || state.doc.accounted().is_some()
        || (state.floating.is_some() && (!normalize || captured || state.doc.xcf_state.is_some()))
        || state.doc.xcf_state.as_ref().is_some_and(|s| s.floating.is_some())
    {
        return Err(bad(command, "finish target previews/gestures/jobs before clipping metadata"));
    }
    if state.doc.paths.len() > 10_000 {
        return Err(bad(command, "saved path count bound"));
    }
    let name = if set {
        Some(
            params
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 2048 && !s.contains('\0'))
                .ok_or_else(|| bad(command, "missing or oversized saved path name"))?,
        )
    } else {
        None
    };
    let flatness = if let Some(value) = params.get("flatness") {
        value.as_f64().filter(|v| v.is_finite() && (0.0..=100.0).contains(v)).ok_or_else(|| bad(command, "flatness must be finite in 0..100"))? as f32
    } else {
        0.0
    };
    if let Some(clipping) = &state.doc.clipping_path
        && (clipping.name.is_empty()
            || clipping.name.len() > 2048
            || clipping.name.contains('\0')
            || !clipping.flatness.is_finite()
            || !(0.0..=100.0).contains(&clipping.flatness))
    {
        return Err(bad(command, "malformed current clipping metadata"));
    }
    if !set && state.doc.clipping_path.is_none() {
        return Err(bad(command, "no clipping path"));
    }
    Ok(Request { name, flatness })
}
fn text(value: &str, command: &str) -> Result<String> {
    let mut result = String::new();
    result.try_reserve_exact(value.len()).map_err(|_| bad(command, "clipping name allocation"))?;
    if result.capacity() > 4096 {
        return Err(bad(command, "clipping name capacity"));
    }
    result.push_str(value);
    Ok(result)
}
fn policy(doc: &Document, name: Option<&str>, command: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    let Some(name) = name else { return Ok(()) };
    let mut found = None;
    for path in &doc.paths {
        poll(ctl)?;
        if path.name == name && found.replace(path).is_some() {
            return Err(bad(command, "ambiguous saved clipping path name"));
        }
    }
    let path = found.ok_or_else(|| bad(command, "no saved path with clipping name"))?;
    if path.id.0 == 0 {
        return Err(bad(command, "zero saved path identity"));
    }
    let mut count = 0;
    for other in &doc.paths {
        poll(ctl)?;
        if other.id == path.id {
            count += 1;
        }
    }
    if count != 1 {
        return Err(bad(command, "ambiguous saved path identity"));
    }
    if let Some(state) = &doc.xcf_state {
        if state.items.len() > 10_000 || state.source_items.len() > 10_000 {
            return Err(bad(command, "current path metadata count bound"));
        }
        doc.validate_xcf_state_with(ctl).map_err(|e| error(command, ctl, e))?;
        let mut item = None;
        for current in &state.items {
            poll(ctl)?;
            if current.item == ItemRef::Path(path.id) && item.replace(current).is_some() {
                return Err(bad(command, "ambiguous current path relation"));
            }
        }
        if let Some(item) = item {
            if item.name != path.name {
                return Err(bad(command, "current path name relation differs"));
            }
            crate::xcf_item_locks::effective_item_locks(doc, item.item)?.require(crate::xcf_item_locks::EditKind::Content)?;
        }
        // There is no typed current clipping-path relation in XcfState. This
        // edits only the existing PSD export field, not a saved GIMP meaning.
    }
    poll(ctl)
}
fn headroom(snapshot: &DocumentSnapshot, command: &str, ctl: &Interrupt<'_>) -> Result<()> {
    poll(ctl)?;
    snapshot.validate_resident()?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, ctl).map_err(|e| error(command, ctl, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, ctl).map_err(|e| error(command, ctl, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(ctl)?;
    if model.checked_mul(8).is_none_or(|v| v > 1_000_000_000)
        || model
            .checked_mul(4)
            .and_then(|v| v.checked_add(payload))
            .and_then(|v| v.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|v| v.checked_add(16_384))
            .is_none_or(|v| v > MAX_CHECKED_RASTER_BYTES)
    {
        return Err(bad(command, "clipping metadata retained/COW work limit"));
    }
    poll(ctl)
}
pub(super) fn dispatch_preflight(session: &Session, command: &str, params: &Value, _background: bool) -> Result<()> {
    let request = request(session, command, params, true)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let name = request.name.or_else(|| state.doc.xcf_state.as_ref().and(state.doc.clipping_path.as_ref().map(|v| v.name.as_str())));
    policy(&state.doc, name, command, &Interrupt::NONE)
}
pub(super) fn run(session: &mut Session, command: &str, params: &Value) -> Result<Value> {
    let request = request(session, command, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    headroom(&state.doc, command, &Interrupt::NONE)?;
    let name = request.name.or_else(|| state.doc.xcf_state.as_ref().and(state.doc.clipping_path.as_ref().map(|v| v.name.as_str())));
    policy(&state.doc, name, command, &Interrupt::NONE)?;
    let desired = request.name.map(|name| text(name, command).map(|name| ClippingPath { name, flatness: request.flatness })).transpose()?;
    let response = desired.as_ref().map(|c| json!({"name":c.name,"flatness":c.flatness})).unwrap_or(Value::Null);
    if state.doc.clipping_path.as_ref().zip(desired.as_ref()).is_some_and(|(a, b)| a.name == b.name && a.flatness.to_bits() == b.flatness.to_bits()) {
        return Ok(response);
    }
    let baseline = state.doc.clone();
    let weak = baseline.downgrade();
    let revision = state.revision;
    let work = baseline.clone();
    let label = if desired.is_some() { "Clipping Path" } else { "Clear Clipping Path" };
    let cmd = command.to_owned();
    crate::jobs::run(
        session,
        label,
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, label, |ctl| {
                headroom(&work, &cmd, ctl)?;
                policy(
                    &work,
                    desired.as_ref().map(|v| v.name.as_str()).or_else(|| work.xcf_state.as_ref().and(work.clipping_path.as_ref().map(|v| v.name.as_str()))),
                    &cmd,
                    ctl,
                )?;
                poll(ctl)?;
                let mut document = (*work).clone();
                document.clipping_path = desired;
                poll(ctl)?;
                Ok((document, ctx.clone()))
            })
        },
        move |session, (document, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.floating.is_some()
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
                || session.command_has_job_on(current.doc.id)
            {
                return Err(bad("path.clippingPath", "target changed during clipping preparation"));
            }
            ctx.stage(0.95, 1.0, label, |ctl| {
                poll(ctl)?;
                session.edit_with_interrupt(label, ctl, move |doc, _| {
                    poll(ctl)?;
                    *doc = document;
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
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Layer, NamedPath, Path, Size};
    use photocraft_raster::Surface;
    fn session() -> Session {
        let mut doc = Document::new("Clipping", Size::new(1, 1), ColorMode::Rgb, SampleType::F64);
        doc.paths.push(NamedPath::new("Saved", Path::default()));
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let mut layer = Layer::raster("Native", format);
        *layer.surface_mut().unwrap() =
            Surface::try_with_default_bytes(format, &[(-0.0f64).to_ne_bytes(), 1.0f64.to_ne_bytes(), 2.0f64.to_ne_bytes(), 1.0f64.to_ne_bytes()].concat())
                .unwrap();
        doc.layers.push(layer);
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn default_flatness_and_clear_preserve_geometry_native_and_noop_redo() {
        let mut s = session();
        let original = s.active().unwrap().doc.clone();
        assert_eq!(run(&mut s, "path.clippingPath.set", &json!({"name":"Saved"})).unwrap(), json!({"name":"Saved","flatness":0.0}));
        assert_eq!(s.active().unwrap().doc.layers, original.layers);
        assert_eq!(s.active().unwrap().doc.paths, original.paths);
        s.edit("Redo seed", |doc, _| {
            doc.name.push_str(" changed");
            Ok(())
        })
        .unwrap();
        assert!(s.undo());
        let revision = s.active().unwrap().revision;
        run(&mut s, "path.clippingPath.set", &json!({"name":"Saved"})).unwrap();
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.redo());
        assert_eq!(run(&mut s, "path.clippingPath.clear", &Value::Null).unwrap(), Value::Null);
        assert!(s.active().unwrap().doc.clipping_path.is_none());
        assert!(s.undo());
        assert!(s.active().unwrap().doc.clipping_path.is_some());
    }
    #[test]
    fn malformed_stale_or_cancelled_requests_do_not_publish() {
        let mut s = session();
        let original = s.active().unwrap().doc.clone();
        for value in [
            json!({"name":"Saved","flatness":101}),
            json!({"name":[],"flatness":0}),
            json!({"name":"Saved","documentId":"01","expectedRevision":"0"}),
            json!({"name":"Missing"}),
        ] {
            assert!(dispatch_preflight(&s, "path.clippingPath.set", &value, false).is_err());
        }
        assert!(matches!(policy(&original, Some("Saved"), "path.clippingPath.set", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(s.active().unwrap().doc.shares_state(&original));
        assert!(!s.undo());
    }
}
