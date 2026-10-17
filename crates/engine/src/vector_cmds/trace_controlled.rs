//! Frozen selection-to-work-path execution; existing f32 contour math only.
use crate::{EngineError, Result, Session};
use photocraft_color::{ColorMode, SampleType, read_sample};
use photocraft_doc::Path;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES};
use serde_json::{Value, json};
fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Make Work Path: {message}"))
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| EngineError::Cancelled)
}
fn controlled(control: &Interrupt<'_>, message: impl std::fmt::Display) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { error(message) }
}
fn decimal(value: &Value) -> Option<u64> {
    let text = value.as_str()?;
    if text.is_empty() || text.len() > 20 || (text.len() > 1 && text.starts_with('0')) || !text.bytes().all(|value| value.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
fn request(session: &Session, params: &Value) -> Result<f64> {
    if !params.is_null()
        && !params
            .as_object()
            .is_some_and(|object| object.len() <= 3 && object.keys().all(|key| matches!(key.as_str(), "tolerance" | "documentId" | "expectedRevision")))
    {
        return Err(error("expected bounded tolerance/captured-target parameters"));
    }
    let tolerance = params
        .get("tolerance")
        .map(|value| value.as_f64().filter(|value| value.is_finite()).ok_or_else(|| error("tolerance must be finite")))
        .transpose()?
        .unwrap_or(2.)
        .clamp(0.5, 10.);
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let document = &state.doc;
    match (params.get("documentId"), params.get("expectedRevision")) {
        (None, None) => {}
        (Some(id), Some(revision)) if decimal(id) == Some(document.id.0) && document.id.0 != 0 && decimal(revision) == Some(state.revision) => {}
        _ => return Err(error("captured document/revision changed or incomplete")),
    }
    let selection = document.selection.as_ref().ok_or_else(|| error("no selection"))?;
    let format = selection.format();
    if format.mode != ColorMode::Grayscale || format.alpha || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(error("native wide or non-Gray selection interpolation is not qualified; convert selection precision explicitly"));
    }
    // A wide image with an ordinary Gray selection reads no wide color words.
    if document.xcf_state.is_some() || document.selection_channel_id.is_some() {
        return Err(error("source-owned selection tracing policy is not qualified"));
    }
    if document.size.width == 0 || document.size.height == 0 || document.size.width > 300_000 || document.size.height > 300_000 {
        return Err(error("invalid tracing canvas"));
    }
    Ok(tolerance)
}
pub(crate) fn dispatch_preflight(session: &Session, params: &Value, _background: bool) -> Result<()> {
    request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    if state.revision == u64::MAX
        || state.floating.is_some()
        || state.coalesce.is_some()
        || session.has_jobs()
        || session.command_has_pending_job()
        || session.coalesce_request.is_some()
        || session.color_restrict.is_some()
    {
        return Err(error("idle uncoalesced selection trace context required"));
    }
    Ok(())
}
struct Admission {
    pixels: usize,
    limits: photocraft_vector::trace::TraceLimits,
}
fn admission(snapshot: &crate::DocumentSnapshot, control: &Interrupt<'_>) -> Result<Admission> {
    poll(control)?;
    snapshot.validate_resident()?;
    let pixels = (snapshot.size.width as usize).checked_mul(snapshot.size.height as usize).ok_or_else(|| error("mask sample count overflow"))?;
    let mask = (pixels as u64).checked_mul(4).ok_or_else(|| error("mask byte count overflow"))?;
    let source = snapshot.selection.as_ref().ok_or_else(|| error("missing frozen selection"))?;
    source.try_validate_tile_origins_with(control).map_err(|e| controlled(control, e))?;
    let limits = photocraft_io::NativeSaveLimits::default().preparation;
    let model = photocraft_io::retained_native_model_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let payload = photocraft_io::retained_native_payload_bytes(snapshot, limits, control).map_err(|e| controlled(control, e))? as u64;
    let raster = snapshot.resident_raster_footprint_with(control)?;
    let retained = model
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(payload))
        .and_then(|bytes| bytes.checked_add(raster.retained_bytes))
        .and_then(|bytes| bytes.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
        .and_then(|bytes| bytes.checked_add(128 * 1024))
        .ok_or_else(|| error("source/metadata residency overflow"))?;
    let resident_bytes =
        MAX_CHECKED_RASTER_BYTES.checked_sub(retained).filter(|bytes| *bytes >= mask).ok_or_else(|| error("aggregate source/mask/contour residency limit"))?;
    let cells = u64::from(snapshot.size.width.div_ceil(64))
        .checked_mul(u64::from(snapshot.size.height.div_ceil(64)))
        .ok_or_else(|| error("read cell count overflow"))?;
    let work = (pixels as u64)
        .checked_mul(48)
        .and_then(|work| work.checked_add(cells.checked_mul(source.tile_count() as u64)?.checked_mul(256)?))
        .ok_or_else(|| error("mask read work overflow"))?;
    let work_units = 8_000_000_000u64.checked_sub(work).ok_or_else(|| error("mask/trace work limit"))?;
    // Lower tracer includes this held input mask and counts actual contour edges
    // allocation-free before map/fitter/output capacity reservations. Its dynamic
    // conservative limits remain below this remaining shared envelope; no refund.
    poll(control)?;
    Ok(Admission { pixels, limits: photocraft_vector::trace::TraceLimits { resident_bytes, work_units } })
}
fn mask(snapshot: &crate::DocumentSnapshot, pixels: usize, control: &Interrupt<'_>) -> Result<Vec<f32>> {
    poll(control)?;
    let source = snapshot.selection.as_ref().ok_or_else(|| error("missing frozen selection"))?;
    let format = source.format();
    if format.mode != ColorMode::Grayscale || format.alpha || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
        return Err(error("wide selection cannot enter legacy f32 tracer"));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(pixels).map_err(|_| error("mask allocation denied"))?;
    if output.capacity() != pixels {
        return Err(error("mask capacity differs from admitted count"));
    }
    while output.len() < pixels {
        poll(control)?;
        output.resize(output.len().saturating_add(16384).min(pixels), 0.);
    }
    let area = snapshot.bounds();
    let width = snapshot.size.width as usize;
    let mut top = area.y0;
    while top < area.y1 {
        let bottom = top.checked_add(64).ok_or_else(|| error("read row overflow"))?.min(area.y1);
        let mut left = area.x0;
        while left < area.x1 {
            poll(control)?;
            let right = left.checked_add(64).ok_or_else(|| error("read column overflow"))?.min(area.x1);
            let region = Rect::new(left, top, right, bottom);
            let bytes = source.try_read_interleaved_with(region, control).map_err(|e| controlled(control, e))?;
            let columns = region.width() as usize;
            let expected = columns
                .checked_mul(region.height() as usize)
                .and_then(|count| count.checked_mul(format.bytes_per_pixel()))
                .ok_or_else(|| error("read length overflow"))?;
            if bytes.len() != expected {
                return Err(error("selection read length mismatch"));
            }
            for (index, sample) in bytes.chunks_exact(format.bytes_per_pixel()).enumerate() {
                if index.is_multiple_of(columns) {
                    poll(control)?;
                }
                let value = read_sample(sample, format.sample, 0);
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(error("selection coverage must be finite unit values"));
                }
                let x = (left as usize).checked_add(index % columns).ok_or_else(|| error("mask column overflow"))?;
                let y = (top as usize).checked_add(index / columns).ok_or_else(|| error("mask row overflow"))?;
                *output
                    .get_mut(y.checked_mul(width).and_then(|value| value.checked_add(x)).ok_or_else(|| error("mask offset overflow"))?)
                    .ok_or_else(|| error("mask offset out of bounds"))? = value;
            }
            left = right;
        }
        top = bottom;
    }
    poll(control)?;
    Ok(output)
}
fn same_path(first: &Path, second: &Path, control: &Interrupt<'_>) -> Result<bool> {
    if first.fill_rule != second.fill_rule || first.inverted != second.inverted || first.subpaths.len() != second.subpaths.len() {
        return Ok(false);
    }
    for (first, second) in first.subpaths.iter().zip(&second.subpaths) {
        poll(control)?;
        if first.closed != second.closed || first.op != second.op || first.knots.len() != second.knots.len() {
            return Ok(false);
        }
        for (first, second) in first.knots.iter().zip(&second.knots) {
            poll(control)?;
            if first.smooth != second.smooth {
                return Ok(false);
            }
            for (first, second) in [first.anchor, first.in_ctrl, first.out_ctrl].into_iter().zip([second.anchor, second.in_ctrl, second.out_ctrl]) {
                if first.x.to_bits() != second.x.to_bits() || first.y.to_bits() != second.y.to_bits() {
                    return Ok(false);
                }
            }
        }
    }
    poll(control)?;
    Ok(true)
}
pub(crate) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    dispatch_preflight(session, params, false)?;
    let tolerance = request(session, params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let revision = state.revision;
    let work = baseline.clone();
    crate::jobs::run(
        session,
        "Make Work Path",
        true,
        move |ctx| {
            ctx.stage(0., 0.95, "Make Work Path", |control| {
                let admission = admission(&work, control)?;
                let mask = mask(&work, admission.pixels, control)?;
                // 0.0 is outside coverage, not a contour threshold. The existing tracer
                // retains 0.5 interpolation/saddle/fitting math; no occupancy conversion.
                let path = photocraft_vector::trace::trace_mask_with_interrupt(&mask, work.bounds(), 0., tolerance, admission.limits, control)
                    .map_err(|e| controlled(control, e))?;
                poll(control)?;
                let mut knots = 0usize;
                for subpath in &path.subpaths {
                    poll(control)?;
                    knots = knots.checked_add(subpath.knots.len()).ok_or_else(|| error("path knot count overflow"))?;
                }
                let response = json!({"subpaths":path.subpaths.len(),"knots":knots});
                drop(mask);
                poll(control)?;
                Ok((path, response, ctx.clone()))
            })
        },
        move |session, (path, response, ctx)| {
            let state = session.active().ok_or(EngineError::NoDocument)?;
            if state.doc.id != baseline.id || state.revision != revision || !state.doc.shares_state(&baseline) {
                return Err(error("selection target changed during tracing"));
            }
            ctx.stage(0.95, 1., "Make Work Path", |control| {
                poll(control)?;
                if let Some(prior) = &baseline.work_path
                    && same_path(prior, &path, control)?
                {
                    return Ok(response);
                }
                session.edit_with_interrupt("Make Work Path", control, move |document, _| {
                    document.work_path = Some(path);
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
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{Document, Size};
    use photocraft_raster::Surface;
    fn fixture(image_depth: SampleType, selection_depth: SampleType, bytes: &[u8]) -> Session {
        let mut document = Document::with_background("selection trace", Size::new(2, 2), ColorMode::Rgb, image_depth, Color::WHITE);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, selection_depth, false));
        selection.try_write_interleaved_with(document.bounds(), bytes, &Interrupt::NONE).unwrap();
        document.selection = Some(selection);
        let mut session = Session::new();
        session.add_document(document, None).unwrap();
        session
    }
    #[test]
    fn ordinary_magnitudes_match_legacy_mask_contour_and_fitter_exactly() {
        let values = [0.0f32, 0.3, 1., 0.7];
        let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
        let mut session = fixture(SampleType::F32, SampleType::F32, &bytes);
        let before = session.active().unwrap().doc.clone();
        let old_mask = photocraft_algo::selection::mask_from_surface(before.selection.as_ref(), before.bounds());
        let expected = photocraft_vector::trace::trace_mask(&old_mask, before.bounds(), 0., 2.);
        assert_eq!(
            mask(&before, 4, &Interrupt::NONE).unwrap().iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
            old_mask.iter().map(|value| value.to_bits()).collect::<Vec<_>>()
        );
        run(&mut session, &Value::Null).unwrap();
        let state = session.active().unwrap();
        assert!(same_path(state.doc.work_path.as_ref().unwrap(), &expected, &Interrupt::NONE).unwrap());
        assert_eq!(state.doc.selection.as_ref().unwrap().try_read_interleaved_with(state.doc.bounds(), &Interrupt::NONE).unwrap(), bytes);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.work_path.is_none());
        session.execute("edit.redo", json!({})).unwrap();
        assert!(same_path(session.active().unwrap().doc.work_path.as_ref().unwrap(), &expected, &Interrupt::NONE).unwrap());
    }
    #[test]
    fn subnormal_f32_keeps_magnitude_and_wide_selection_refuses_before_read() {
        let tiny = f32::from_bits(1);
        let bytes: Vec<u8> = [tiny, 0., tiny, 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
        let session = fixture(SampleType::F64, SampleType::F32, &bytes);
        let before = session.active().unwrap().doc.clone();
        assert!(dispatch_preflight(&session, &Value::Null, false).is_ok());
        let values = mask(&before, 4, &Interrupt::NONE).unwrap();
        assert_eq!(values[0].to_bits(), 1);
        assert!(photocraft_vector::trace::trace_mask(&values, before.bounds(), 0., 2.).subpaths.is_empty());
        let wide: Vec<u8> = [f64::from_bits(1), 0., 1., 0.].into_iter().flat_map(f64::to_ne_bytes).collect();
        let session = fixture(SampleType::F64, SampleType::F64, &wide);
        let before = session.active().unwrap().doc.clone();
        assert!(dispatch_preflight(&session, &Value::Null, false).is_err());
        assert!(session.active().unwrap().doc.shares_state(&before));
    }
    #[test]
    fn repeated_exact_path_is_noop_and_retains_real_redo() {
        let mut session = fixture(SampleType::F64, SampleType::U8, &[0, 255, 255, 0]);
        run(&mut session, &json!({"tolerance":0.01})).unwrap();
        session
            .edit("redo fixture", |document, _| {
                document.name = "changed".into();
                Ok(())
            })
            .unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        let state = session.active().unwrap();
        let before = state.doc.clone();
        let revision = state.revision;
        let redo = state.history.redo_label().unwrap().to_string();
        run(&mut session, &json!({"tolerance":0.5})).unwrap();
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().history.redo_label(), Some(redo.as_str()));
    }
    #[test]
    fn stale_cancel_and_contour_budget_refusals_are_atomic() {
        let session = fixture(SampleType::F32, SampleType::U8, &[0, 255, 255, 0]);
        let state = session.active().unwrap();
        let before = state.doc.clone();
        for params in [
            json!({"tolerance":"2"}),
            json!({"tolerance":2,"documentId":before.id.0.to_string()}),
            json!({"documentId":before.id.0.to_string(),"expectedRevision":state.revision.saturating_add(1).to_string()}),
        ] {
            assert!(dispatch_preflight(&session, &params, false).is_err());
        }
        let cancel = || true;
        assert!(matches!(mask(&before, 4, &Interrupt::cancel_only(&cancel)), Err(EngineError::Cancelled)));
        let values = mask(&before, 4, &Interrupt::NONE).unwrap();
        assert!(
            photocraft_vector::trace::trace_mask_with_interrupt(
                &values,
                before.bounds(),
                0.,
                2.,
                photocraft_vector::trace::TraceLimits { resident_bytes: 1, work_units: 1 },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert!(state.doc.shares_state(&before));
    }
}
