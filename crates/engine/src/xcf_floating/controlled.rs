//! Saved floating-anchor worker. Preparation retains the existing Normal/CMS
//! equations; source/profile admission is unchanged. Publication uses the
//! original snapshot, runtime view and one controlled history transaction.
use super::{CMD, error};
use crate::{EngineError, Result, Session};
use photocraft_doc::{Document, Layer, item_identity::ItemRef};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};

fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn decimal(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit()) && (text.len() == 1 || !text.starts_with('0')))
        .ok_or_else(|| error("captured identity must be a canonical decimal string"))?;
    text.parse().map_err(|_| error("captured identity overflow"))
}
pub(crate) fn dispatch_preflight(session: &Session, command: &str, params: &Value) -> Result<()> {
    let object = params
        .as_object()
        .filter(|object| object.len() <= 2 && object.keys().all(|key| matches!(key.as_str(), "documentId" | "expectedRevision")))
        .ok_or_else(|| EngineError::BadParams { cmd: command.into(), msg: "expected captured identity pair or empty object".into() })?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    match (object.get("documentId"), object.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id)? == state.doc.id.0 && decimal(revision)? == state.revision => {}
        _ => return Err(error("captured floating target changed")),
    }
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
        || session.command_has_job_on(state.doc.id)
        || session.command_has_pending_job()
    {
        return Err(error("saved floating edit requires an idle target"));
    }
    if state.doc.xcf_state.as_ref().and_then(|state| state.floating.as_ref()).is_none() {
        return Err(error("no saved floating attachment"));
    }
    Ok(())
}
fn admission(doc: &Document, include_anchor_pixels: bool, control: &Interrupt<'_>) -> Result<photocraft_cms::control::Limits> {
    poll(control)?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let payload = photocraft_io::retained_native_payload_bytes(doc, limits, control).map_err(error)? as u64;
    let model = photocraft_io::retained_native_model_bytes(doc, limits, control).map_err(error)? as u64;
    let raster = crate::DocumentSnapshot::resident_raster_footprint_of(doc, control)?;
    let replacement = if include_anchor_pixels {
        let state = doc.xcf_state.as_ref().ok_or_else(|| error("missing imported state"))?;
        let attachment = state.floating.as_ref().ok_or_else(|| error("missing floating attachment"))?;
        let ItemRef::Layer(target) = attachment.target else {
            return Err(error("unsupported anchor target"));
        };
        let mut source_bounds = None;
        let mut target_bounds = None;
        for item in &state.items {
            poll(control)?;
            if item.item == ItemRef::Layer(attachment.layer) {
                source_bounds = item.raster_bounds;
            }
            if item.item == ItemRef::Layer(target) {
                target_bounds = item.raster_bounds;
            }
        }
        let source_bounds = source_bounds.ok_or_else(|| error("missing floating extent"))?;
        let target_bounds = target_bounds.ok_or_else(|| error("missing target extent"))?;
        super::checked_bounds(source_bounds)?;
        super::checked_bounds(target_bounds)?;
        let overlap = source_bounds.intersect(&target_bounds);
        let cells = u64::from(overlap.width())
            .div_ceil(256)
            .checked_add(1)
            .and_then(|columns| columns.checked_mul(u64::from(overlap.height()).div_ceil(256).checked_add(1)?))
            .ok_or_else(|| error("anchor tile charge overflow"))?;
        let format = super::plain(doc.layer(target).ok_or_else(|| error("missing target raster"))?)?.format();

        cells
            .checked_mul(256 * 256)
            .and_then(|pixels| pixels.checked_mul(format.bytes_per_pixel() as u64))
            .and_then(|bytes| bytes.checked_add(cells.checked_mul(4096)?))
            .ok_or_else(|| error("anchor replacement charge overflow"))?
    } else {
        0
    };
    // Precharge replacement storage, shared retained payloads and private
    // model/map overlap. Standard allocator internals remain unchanged.
    let peak = payload
        .checked_add(model.checked_mul(6).ok_or_else(|| error("model charge overflow"))?)
        .and_then(|bytes| bytes.checked_add(raster.retained_bytes.checked_mul(2)?))
        .and_then(|bytes| bytes.checked_add(raster.clone_metadata_bytes.checked_mul(6)?))
        .and_then(|bytes| bytes.checked_add(replacement.checked_add(4 << 20)?))
        .ok_or_else(|| error("anchor aggregate charge overflow"))?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(error("anchor aggregate storage limit"));
    }
    let maximum = photocraft_cms::control::Limits::default();
    let profile_limits = photocraft_cms::control::Limits {
        resident_bytes: usize::try_from(
            MAX_CHECKED_RASTER_BYTES.checked_sub(peak).ok_or_else(|| error("anchor profile headroom"))?.min(maximum.resident_bytes as u64),
        )
        .map_err(error)?,
        work_units: maximum.work_units,
    };
    if include_anchor_pixels {
        let profiles = crate::color_cmds::document_profile_requirements_with_limits(doc, profile_limits, control)?;
        peak.checked_add(profiles.peak_resident_bytes as u64)
            .filter(|bytes| *bytes <= MAX_CHECKED_RASTER_BYTES)
            .ok_or_else(|| error("anchor aggregate profile/model/raster limit"))?;
        if profiles.work_units.checked_mul(2).is_none_or(|work| work > profile_limits.work_units) {
            return Err(error("anchor profile/working-space work limit"));
        }
    }
    poll(control)?;
    Ok(profile_limits)
}
pub(crate) fn anchor(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, CMD, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let worker = baseline.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let view = state.channel_view.clone();
    crate::jobs::run(
        session,
        "Anchor Floating Layer",
        true,
        move |context| {
            context.stage(0.0, 0.95, "Anchor Floating Layer", |control| {
                let profile_limits = admission(&worker, true, control)?;
                let plan = super::prepare_with_profile_limits(&worker, profile_limits, control)?;
                poll(control)?;
                Ok((plan, context.clone()))
            })
        },
        move |session, (plan, context)| {
            context.stage(0.95, 1.0, "Anchor Floating Layer", |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != baseline.id
                    || state.revision != revision
                    || !baseline.downgrade().is_current(&state.doc)
                    || state.active_layer != active
                    || state.channel_view != view
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                    || session.command_has_job_on(baseline.id)
                    || session.command_has_pending_job()
                {
                    return Err(error("floating anchor context changed during preparation"));
                }
                let target = plan.target;
                session.edit_with_interrupt("Anchor Floating Layer", control, move |doc, active| {
                    poll(control)?;
                    super::unlocked(doc, plan.source, false)?;
                    super::unlocked(doc, plan.target, true)?;
                    *doc.layer_mut(plan.target).and_then(Layer::surface_mut).ok_or_else(|| error("missing target raster"))? = plan.surface;
                    let state = doc.xcf_state.as_mut().ok_or_else(|| error("missing imported state"))?;
                    state.floating = None;
                    state.selection.selected.clear();
                    state.selection.selected.try_reserve(1).map_err(|_| error("selection allocation"))?;
                    state.selection.selected.push(ItemRef::Layer(plan.target));
                    state.selection.active = Some(ItemRef::Layer(plan.target));
                    doc.remove(plan.source).ok_or_else(|| error("missing floating layer"))?;
                    let mut state = doc.xcf_state.take().ok_or_else(|| error("missing imported state"))?;
                    state.reconcile_layers_with_interrupt(doc, control).map_err(error)?;
                    doc.xcf_state = Some(state);
                    photocraft_compose::gimp_render::PreparedRenderer::new_with_interrupt(doc, control).map_err(error)?;
                    *active = Some(plan.target);
                    poll(control)
                })?;
                Ok(json!({"anchored":true,"target":target.0}))
            })
        },
    )
}

pub(crate) fn release(session: &mut Session, params: &Value, delete: bool) -> Result<Value> {
    dispatch_preflight(session, if delete { "xcf.floating.delete" } else { "xcf.floating.toLayer" }, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    state.doc.validate_resident()?;
    let baseline = state.doc.clone();
    let worker = baseline.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let view = state.channel_view.clone();
    let label = if delete { "Delete Floating Layer" } else { "Floating Selection to Layer" };
    crate::jobs::run(
        session,
        label,
        true,
        move |context| {
            context.stage(0.0, 0.95, label, |control| {
                let _ = admission(&worker, false, control)?;
                poll(control)?;
                // Model capacity is admitted above. This standard Document clone is
                // bounded and boundary-polled, not an all-allocator guarantee.
                let mut prepared = (*worker).clone();
                poll(control)?;
                let mut selected = active;
                let layer = super::release_on_document(&mut prepared, &mut selected, delete, control)?;
                poll(control)?;
                Ok((prepared, layer, context.clone()))
            })
        },
        move |session, (prepared, layer, context)| {
            context.stage(0.95, 1.0, label, |control| {
                poll(control)?;
                let state = session.active().ok_or(EngineError::NoDocument)?;
                if state.doc.id != baseline.id
                    || state.revision != revision
                    || !baseline.downgrade().is_current(&state.doc)
                    || state.active_layer != active
                    || state.channel_view != view
                    || state.floating.is_some()
                    || state.coalesce.is_some()
                    || session.coalesce_request.is_some()
                    || session.color_restrict.is_some()
                    || session.command_has_job_on(baseline.id)
                    || session.command_has_pending_job()
                {
                    return Err(error("floating release context changed during preparation"));
                }
                session.edit_with_interrupt(label, control, move |doc, active| {
                    poll(control)?;
                    *doc = prepared;
                    *active = Some(layer);
                    poll(control)
                })?;
                Ok(json!({"deleted":delete,"layer":layer.0}))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_identity_rejects_normalized_numbers_and_leading_zeroes() {
        assert_eq!(decimal(&json!("42")).unwrap(), 42);
        for value in [json!(42), json!("042"), json!("+42"), json!("18446744073709551616")] {
            assert!(decimal(&value).is_err());
        }
    }
}
