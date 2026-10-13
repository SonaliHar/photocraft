//! Metadata-only controlled profile assignment. Uses the existing ColorState
//! resolver (including its existing explicit path/working-space authority), not
//! stored source locators. Resolver/ICC encoder internals have boundary polls;
//! this is no new interruptible filesystem or universal allocator contract.
use super::{mode_space, profile_from_bytes, validate_analytic_xcf_profile_change_with_interrupt};
use crate::{EngineError, Result, Session};
use photocraft_cms::Profile;
use photocraft_doc::{Document, SampleEncoding, WorkingProfileInterpretation};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
use std::sync::Arc;
const CMD: &str = "edit.assignProfile";
const MAX_PROFILE_BYTES: usize = 16 << 20;
fn bad(message: impl std::fmt::Display) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.to_string() }
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, error: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { bad(error) }
}
fn decimal(value: &Value, nonzero: bool) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0')))
        .ok_or_else(|| bad("captured IDs must be canonical decimal strings"))?;
    let number = text.parse().map_err(|_| bad("captured ID exceeds range"))?;
    if nonzero && number == 0 {
        return Err(bad("documentId must be nonzero"));
    }
    Ok(number)
}
/// Allocation-free borrowed envelope before floating normalization/params cloning.
pub(super) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    let object = params
        .as_object()
        .filter(|object| object.len() <= 3 && object.keys().all(|key| matches!(key.as_str(), "profile" | "documentId" | "expectedRevision")))
        .ok_or_else(|| bad("expected bounded profile and optional captured target"))?;
    let _ = object;
    if !params.get("profile").and_then(Value::as_str).is_some_and(|profile| !profile.is_empty() && profile.len() <= 4096 && !profile.contains('\0')) {
        return Err(bad("profile must be a bounded nonempty string"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let captured = match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => false,
        (Some(document), Some(revision)) if decimal(document, true)? == state.doc.id.0 && decimal(revision, false)? == state.revision => true,
        _ => return Err(bad("captured target changed or incomplete")),
    };
    // Persisted XCF attachments never receive the ordinary auto-anchor exception.
    if state.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
        || (captured && state.floating.is_some())
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.command_has_job_on(state.doc.id)
    {
        return Err(bad("finish transient edits or pending work before assigning a profile"));
    }
    if state.revision == u64::MAX || state.doc.accounted().is_some() {
        return Err(bad("resident revision/headroom required for profile assignment"));
    }
    Ok(())
}
fn copy_description(description: &str, control: &Interrupt<'_>) -> Result<String> {
    if description.len() > MAX_PROFILE_BYTES {
        return Err(bad("profile description limit"));
    }
    poll(control)?;
    let mut result = String::new();
    result.try_reserve_exact(description.len()).map_err(|_| bad("profile description allocation"))?;
    if result.capacity() > MAX_PROFILE_BYTES {
        return Err(bad("profile description capacity"));
    }
    result.push_str(description);
    poll(control)?;
    Ok(result)
}
fn prepare(doc: &Document, profile: Option<&Profile>, control: &Interrupt<'_>) -> Result<(Option<Arc<Vec<u8>>>, String)> {
    poll(control)?;
    let preparation = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, preparation, control).map_err(|e| controlled(control, e))? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, preparation, control).map_err(|e| controlled(control, e))? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let base = model
        .checked_mul(4)
        .and_then(|n| n.checked_add(payload))
        .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or_else(|| bad("retained/model/COW assignment headroom limit"))?;
    if payload.checked_add(model).and_then(|n| n.checked_mul(4)).is_none_or(|n| n > 8_000_000_000) {
        return Err(bad("profile assignment validation work limit"));
    }
    // Keep the original model check before the active analytic compatibility check.
    let bytes = match profile {
        None => {
            validate_analytic_xcf_profile_change_with_interrupt(doc, None, control)?;
            None
        }
        Some(profile) => {
            if profile.description.len() > MAX_PROFILE_BYTES || profile.encoded_bytes().is_some_and(|bytes| bytes.len() > MAX_PROFILE_BYTES) {
                return Err(bad("profile metadata/data exceeds standard ICC limit"));
            }
            if profile.color_space != mode_space(doc.mode) {
                return Err(EngineError::Other(format!("profile `{}` is {:?}; the document is {:?}", profile.description, profile.color_space, doc.mode)));
            }
            validate_analytic_xcf_profile_change_with_interrupt(doc, Some(profile), control)?;
            poll(control)?;
            let bytes = profile.to_bytes();
            poll(control)?;
            if bytes.len() > MAX_PROFILE_BYTES || bytes.capacity() > MAX_PROFILE_BYTES {
                return Err(bad("encoded profile capacity exceeds standard ICC limit"));
            }
            if base
                .checked_add((bytes.capacity() as u64).checked_mul(4).ok_or_else(|| bad("profile headroom overflow"))?)
                .and_then(|n| n.checked_add((profile.description.len() as u64).checked_mul(3)?))
                .is_none_or(|n| n > MAX_CHECKED_RASTER_BYTES)
            {
                return Err(bad("profile/data/response assignment headroom limit"));
            }
            Some(bytes)
        }
    };
    poll(control)?;
    // Retain the old encoded-profile description behavior, including its empty
    // response fallback on reparsing failure; it never substitutes profile bytes.
    let description = match &bytes {
        Some(bytes) => match profile_from_bytes(bytes) {
            Ok(parsed) => copy_description(&parsed.description, control)?,
            Err(_) => String::new(),
        },
        None => copy_description("none", control)?,
    };
    poll(control)?;
    Ok((bytes, description))
}
pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, session.command_is_background())?;
    let spec = params.get("profile").and_then(Value::as_str).ok_or_else(|| bad("missing profile"))?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let weak = baseline.downgrade();
    // Preserve the existing resolver and exact working/none/document semantics.
    let resolved = if spec == "none" { None } else { Some(session.color.resolve(spec, Some(&baseline), None)?) };
    let work_base = baseline.clone();
    crate::jobs::run(
        session,
        "Assign Profile",
        true,
        move |ctx| {
            ctx.stage(0.0, 0.95, "Assign Profile", |control| {
                let (bytes, description) = prepare(&work_base, resolved.as_deref(), control)?;
                poll(control)?;
                let mut document = (*work_base).clone();
                document.icc_profile = bytes;
                document.working_profile_interpretation = WorkingProfileInterpretation::Icc;
                if document.sample_encoding.is_some() {
                    document.sample_encoding = Some(SampleEncoding::NonlinearProfile);
                }
                poll(control)?;
                Ok((document, description, ctx.clone()))
            })
        },
        move |session, (document, description, ctx)| {
            let current = session.active().ok_or(EngineError::NoDocument)?;
            if current.doc.id != baseline.id
                || current.revision != revision
                || !weak.is_current(&current.doc)
                || !current.doc.shares_state(&baseline)
                || current.floating.is_some()
                || current.doc.xcf_state.as_ref().is_some_and(|state| state.floating.is_some())
                || current.coalesce.is_some()
                || session.coalesce_request.is_some()
                || session.color_restrict.is_some()
            {
                return Err(bad("target changed while preparing profile assignment"));
            }
            ctx.stage(0.95, 1.0, "Assign Profile", |control| {
                poll(control)?;
                session.edit_with_interrupt("Assign Profile", control, move |current, _| {
                    poll(control)?;
                    *current = document;
                    Ok(())
                })?;
                Ok(json!({"profile":description}))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Layer, LayerContent, SampleType, Size};
    fn source(sample: SampleType) -> Session {
        let mut doc = Document::new("Assign raw", Size::new(1, 1), ColorMode::Rgb, sample);
        let mut surface = photocraft_raster::Surface::new(doc.pixel_format());
        let raw: Vec<_> = match sample {
            SampleType::U32 => [1u32, 2, u32::MAX - 1, u32::MAX].into_iter().flat_map(u32::to_le_bytes).collect(),
            _ => [(0.5f64).to_bits() + 1, (-0f64).to_bits(), 3f64.to_bits(), 1f64.to_bits()].into_iter().flat_map(u64::to_le_bytes).collect(),
        };
        surface.try_write_interleaved_with(photocraft_geom::Rect::new(0, 0, 1, 1), &raw, &Interrupt::NONE).unwrap();
        doc.layers.push(Layer::new("raw", LayerContent::Raster(surface)));
        let mut session = Session::new();
        session.add_document(doc, None).unwrap();
        session
    }
    #[test]
    fn assignment_preserves_wide_samples_native_metadata_and_history() {
        for sample in [SampleType::U32, SampleType::F64] {
            let mut session = source(sample);
            let before = session.active().unwrap().doc.clone();
            let revision = session.active().unwrap().revision;
            run(&mut session, &json!({"profile":"working","documentId":before.id.0.to_string(),"expectedRevision":revision.to_string()})).unwrap();
            let current = session.active().unwrap().doc.clone();
            assert_eq!(current.layers[0].surface(), before.layers[0].surface());
            assert_eq!(current.source_color_context, before.source_color_context);
            assert_eq!(current.working_profile_interpretation, WorkingProfileInterpretation::Icc);
            assert!(current.icc_profile.is_some());
            let encoded = photocraft_format::save_to_bytes(&current, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&encoded).unwrap();
            assert_eq!(reopened.icc_profile, current.icc_profile);
            assert_eq!(reopened.layers[0].surface(), before.layers[0].surface());
            session.execute("edit.undo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.icc_profile, before.icc_profile);
            assert_eq!(session.active().unwrap().doc.layers[0].surface(), before.layers[0].surface());
            session.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(session.active().unwrap().doc.icc_profile, current.icc_profile);
        }
    }
    #[test]
    fn stale_partial_wrong_type_and_space_fail_without_consuming_real_redo() {
        let mut session = source(SampleType::U32);
        session
            .edit("seed", |doc, _| {
                doc.name = "seed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", Value::Null).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>();
        for params in [
            json!({"profile":false}),
            json!({"profile":"working","documentId":before.id.0.to_string()}),
            json!({"profile":"working","documentId":before.id.0.to_string(),"expectedRevision":(revision+1).to_string()}),
            json!({"profile":"working-cmyk"}),
            json!({"profile":"x".repeat(4097)}),
        ] {
            assert!(run(&mut session, &params).is_err());
            let state = session.active().unwrap();
            assert!(state.doc.shares_state(&before));
            assert_eq!(state.revision, revision);
            assert_eq!(state.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), redo);
        }
    }
    #[test]
    fn discard_and_noop_keep_raw_bits_and_cancel_does_not_prepare_new_metadata() {
        let mut session = source(SampleType::F64);
        run(&mut session, &json!({"profile":"working"})).unwrap();
        let assigned = session.active().unwrap().doc.clone();
        assert!(matches!(prepare(&assigned, None, &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
        assert!(session.active().unwrap().doc.shares_state(&assigned));
        run(&mut session, &json!({"profile":"none"})).unwrap();
        let cleared = session.active().unwrap().doc.clone();
        assert!(cleared.icc_profile.is_none());
        assert_eq!(cleared.layers[0].surface(), assigned.layers[0].surface());
        run(&mut session, &json!({"profile":"none"})).unwrap();
        assert_eq!(session.active().unwrap().doc.layers[0].surface(), cleared.layers[0].surface());
        assert_eq!(session.active().unwrap().doc.working_profile_interpretation, WorkingProfileInterpretation::Icc);
    }
}
