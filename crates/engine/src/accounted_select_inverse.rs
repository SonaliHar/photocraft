//! Native, guarded selection inversion. Existing resident inverse stays literal.
use crate::{EngineError, Result, Session, accounted_selection_session::AccountedSelectionTarget};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::accounted_selection::{AccountedNativeSelectionBandBuilder, AccountedSelection};
use photocraft_geom::Rect;
use photocraft_raster::{
    backing::{BackingError, ReadControl, quota::QuotaKind},
    checked_view::{CheckedRasterView, RasterViewError},
};
use serde_json::Value;
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: "select.inverse".into(), msg: message.into() }
}
pub(crate) fn validate_request(session: &Session, params: &Value) -> Result<()> {
    crate::selection_inverse_controlled::dispatch_preflight(session, params, false)?;
    let doc = &session.active().ok_or(EngineError::NoDocument)?.doc;
    if doc.accounted().is_none() || doc.selection_channel_id.is_some() {
        return Err(bad("native Inverse requires anonymous guarded selection"));
    }
    let source = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    if source.format().mode != ColorMode::Grayscale
        || source.format().alpha
        || !matches!(source.format().sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
    {
        return Err(BackingError::Unsupported.into());
    }
    Ok(())
}
fn invert(source: &[u8], sample: SampleType, output: &mut [u8]) -> std::result::Result<(), BackingError> {
    let width = match sample {
        SampleType::U8 => 1,
        SampleType::U16 => 2,
        SampleType::U32 | SampleType::F32 => 4,
        SampleType::F64 => 8,
    };
    if source.len() != width || output.len() != width {
        return Err(BackingError::Length);
    }
    match sample {
        SampleType::U8 => {
            let value = *source.first().ok_or(BackingError::Length)?;
            output.first_mut().ok_or(BackingError::Length)?.clone_from(&(u8::MAX - value));
        }
        SampleType::U16 => output.copy_from_slice(&(u16::MAX - u16::from_ne_bytes(source.try_into().map_err(|_| BackingError::Length)?)).to_ne_bytes()),
        SampleType::U32 => output.copy_from_slice(&(u32::MAX - u32::from_ne_bytes(source.try_into().map_err(|_| BackingError::Length)?)).to_ne_bytes()),
        SampleType::F32 => {
            let v = f32::from_ne_bytes(source.try_into().map_err(|_| BackingError::Length)?);
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(BackingError::Unsupported);
            }
            output.copy_from_slice(&(1.0f32 - v).to_ne_bytes());
        }
        SampleType::F64 => {
            let v = f64::from_ne_bytes(source.try_into().map_err(|_| BackingError::Length)?);
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(BackingError::Unsupported);
            }
            output.copy_from_slice(&(1.0f64 - v).to_ne_bytes());
        }
    }
    Ok(())
}
fn view_error(error: RasterViewError) -> EngineError {
    match error {
        RasterViewError::Control(e) | RasterViewError::Backed(e) | RasterViewError::Descriptor(e) => e.into(),
        _ => BackingError::Integrity.into(),
    }
}
fn prepare(owner: &photocraft_doc::accounted_document::AccountedDocument, c: &ReadControl<'_>) -> Result<Option<AccountedSelection>> {
    let doc = owner.document();
    let source = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let bounds = doc.bounds();
    let view = CheckedRasterView::resident_with_control(source, bounds, c).map_err(view_error)?;
    let row = usize::try_from(bounds.width()).map_err(|_| BackingError::Length)?.checked_mul(bpp).ok_or(BackingError::Length)?;
    let envelope = row.checked_mul(2).and_then(|n| n.checked_add(4096)).ok_or(BackingError::Length)?;
    let _scratch = owner.quota().reserve(QuotaKind::Scratch, envelope as u64).map_err(|_| BackingError::Quota)?;
    c.check(row as u64)?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(row).map_err(|_| BackingError::Allocation)?;
    if raw.capacity() > row.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    raw.resize(row, 0);
    let mut builder = AccountedNativeSelectionBandBuilder::new(format, bounds, owner.quota(), c)?;
    for y in bounds.y0..bounds.y1 {
        c.check(1)?;
        for (x, out) in (bounds.x0..bounds.x1).zip(raw.chunks_exact_mut(bpp)) {
            c.check(64)?;
            let mut input = [0; 8];
            let input = input.get_mut(..bpp).ok_or(BackingError::Length)?;
            view.read_pixel_native_into(x, y, input, c).map_err(view_error)?;
            invert(input, format.sample, out)?;
        }
        builder.push_band(Rect::new(bounds.x0, y, bounds.x1, y.checked_add(1).ok_or(BackingError::Bounds)?), &raw, c)?;
    }
    Ok(builder.finish(c)?)
}
pub(crate) fn run_captured(session: &mut Session, params: &Value, target: AccountedSelectionTarget, c: &ReadControl<'_>) -> Result<Value> {
    c.check(1)?;
    validate_request(session, params)?;
    target.validate_with_control(session, c)?;
    let baseline = session.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let owner = baseline.accounted().ok_or(BackingError::Unsupported)?;
    if let Some(kind) = crate::accounted_programme_current::kind(owner) {
        owner.validate_programme_selection_replacement(c)?;
        crate::accounted_programme_current::validate(owner, kind, c)?;
    } else {
        owner.validate_selection_replacement(c)?;
    }
    let selection = prepare(owner, c)?;
    let prior = baseline.selection.as_ref().ok_or(BackingError::Integrity)?;
    let comparison = (prior.tile_count() as u64)
        .checked_mul(65536)
        .and_then(|n| n.checked_mul(prior.format().bytes_per_pixel() as u64))
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add((prior.tile_count() as u64).checked_mul(64)?))
        .and_then(|n| n.checked_add(128))
        .ok_or(BackingError::Length)?;
    c.check(comparison)?;
    if selection.as_ref().map(|next| next.matches_native_surface(baseline.selection.as_ref().ok_or(BackingError::Integrity)?, c)).transpose()?.unwrap_or(false)
    {
        // Exact full native storage equality, not a numeric midpoint shortcut.
        // The shared staged model still receives the full Current proof.
        if let Some(kind) = crate::accounted_programme_current::kind(owner) {
            crate::accounted_programme_current::validate(owner, kind, c)?;
        } else {
            owner.validate_selection_replacement(c)?;
        }
        target.validate_with_control(session, c)?;
        validate_request(session, params)?;
        c.check(1)?;
        return Ok(Value::Null);
    }

    target.validate_with_control(session, c)?;
    validate_request(session, params)?;
    session.replace_accounted_selection(target, "Inverse", selection.as_ref(), c)?;
    Ok(Value::Null)
}

#[cfg(test)]
#[path = "accounted_select_inverse_tests.rs"]
mod tests;
