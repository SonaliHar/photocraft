//! Accounted Select All using upstream canvas and native-format policy.
//! No source, profile, mask or imported selection authority is inferred.
use crate::{EngineError, Result, Session, accounted_selection_session::AccountedSelectionTarget};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::accounted_selection::{AccountedSelection, CoverageDomain};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    MAX_CHECKED_RASTER_BYTES, Surface,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use serde_json::Value;
const CMD: &str = "select.all";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
/// Borrowed dispatch validation, before journal/parameter copies. Full ownership
/// census runs under the actual caller ReadControl in run().
pub(crate) fn validate_request(session: &Session, params: &Value) -> Result<()> {
    crate::selection_inverse_controlled::storage_dispatch_preflight(session, CMD, params, false)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let doc = &state.doc;
    if doc.accounted().is_none() || doc.selection_channel_id.is_some() {
        return Err(bad("accounted Select All requires an anonymous current pixel selection"));
    }
    if doc.size.width == 0 || doc.size.height == 0 || doc.size.width > 262144 || doc.size.height > 262144 {
        return Err(bad("accounted Select All canvas exceeds native selection bounds"));
    }
    output_format(doc.selection.as_ref())?;
    Ok(())
}
fn output_format(prior: Option<&Surface>) -> Result<PixelFormat> {
    if let Some(prior) = prior {
        let format = prior.format();
        if format.mode != ColorMode::Grayscale || format.alpha {
            return Err(bad("selection must be Gray without alpha"));
        }
        if matches!(format.sample, SampleType::U32 | SampleType::F64) {
            return Ok(format);
        }
    }
    Ok(PixelFormat::GRAY8)
}
fn unit(format: PixelFormat) -> Result<([u8; 8], usize)> {
    let mut word = [0; 8];
    let length = match format.sample {
        SampleType::U8 => {
            word[0] = 255;
            1
        }
        SampleType::U32 => {
            word.get_mut(..4).ok_or(BackingError::Length)?.copy_from_slice(&u32::MAX.to_ne_bytes());
            4
        }
        SampleType::F64 => {
            word.copy_from_slice(&1f64.to_ne_bytes());
            8
        }
        _ => return Err(bad("unsupported Select All output format")),
    };
    Ok((word, length))
}
/// Exact complete retained storage equality, including default and padded tails.
/// No allocation or numeric interpretation is used for the old selection.
fn is_all(prior: Option<&Surface>, area: Rect, format: PixelFormat, one: &[u8], control: &ReadControl<'_>) -> Result<bool> {
    let Some(prior) = prior else { return Ok(false) };
    if prior.format() != format || prior.default_bytes().iter().any(|v| *v != 0) {
        return Ok(false);
    }
    let bpp = format.bytes_per_pixel();
    let across = u64::from(area.width()).div_ceil(TILE_SIZE as u64);
    let down = u64::from(area.height()).div_ceil(TILE_SIZE as u64);
    let count = across.checked_mul(down).ok_or(BackingError::Length)?;
    if prior.tile_count() as u64 != count {
        return Ok(false);
    }
    let tile_length = (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(bpp)).ok_or(BackingError::Length)?;
    for (coordinate, tile) in prior.tiles() {
        control.check(1)?;
        let x0 = coordinate.tx.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
        let y0 = coordinate.ty.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
        let x1 = x0.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?;
        let y1 = y0.checked_add(TILE_SIZE).ok_or(BackingError::Bounds)?;
        if Rect::new(x0, y0, x1, y1).intersect(&area).is_empty() || tile.bytes().len() != tile_length {
            return Ok(false);
        }
        for (block_index, block) in tile.bytes().chunks(4096 * bpp).enumerate() {
            control.check(block.len() as u64)?;
            let first = block_index.checked_mul(4096).ok_or(BackingError::Length)?;
            for (i, word) in block.chunks_exact(bpp).enumerate() {
                let index = first.checked_add(i).ok_or(BackingError::Length)?;
                let x = x0.checked_add(i32::try_from(index % TILE_SIZE as usize).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
                let y = y0.checked_add(i32::try_from(index / TILE_SIZE as usize).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
                if area.contains(x, y) {
                    if word != one {
                        return Ok(false);
                    }
                } else if word.iter().any(|v| *v != 0) {
                    return Ok(false);
                }
            }
        }
    }
    control.check(1)?;
    Ok(true)
}
/// Actual registered transaction body; caller supplies its existing work ledger.
/// Response is prebuilt Null, and publication is the last fallible operation.
pub(crate) fn run(session: &mut Session, params: &Value, control: &ReadControl<'_>) -> Result<Value> {
    control.check(1)?;
    validate_request(session, params)?;
    let target = AccountedSelectionTarget::capture(session)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let baseline = state.doc.clone();
    let owner = baseline.accounted().ok_or(BackingError::Unsupported)?;
    // The paid document gate alone admits plain imported metadata/storage.
    // This dispatch check never clears source state or supplies an ownership proof.
    owner.validate_selection_replacement(control)?;
    if !state.history.current()?.shares_state(&baseline) {
        return Err(BackingError::Integrity.into());
    }
    let area = baseline.bounds();
    let format = output_format(baseline.selection.as_ref())?;
    let (word, bpp) = unit(format)?;
    let one = word.get(..bpp).ok_or(BackingError::Length)?;
    if is_all(baseline.selection.as_ref(), area, format, one, control)? {
        validate_request(session, params)?;
        let state = session.active().ok_or(EngineError::NoDocument)?;
        if state.revision != target.revision
            || state.view_revision != target.view_revision
            || !baseline.shares_state(&state.doc)
            || !state.history.current()?.shares_state(&state.doc)
        {
            return Err(bad("accounted Select All target changed"));
        }
        control.check(1)?;
        return Ok(Value::Null);
    }
    let bytes = u64::from(area.width())
        .checked_mul(u64::from(area.height()))
        .and_then(|n| n.checked_mul(bpp as u64))
        .filter(|n| *n <= MAX_CHECKED_RASTER_BYTES)
        .ok_or(BackingError::Length)?;
    let capacity = usize::try_from(bytes).map_err(|_| BackingError::Length)?;
    let envelope = bytes.checked_mul(2).and_then(|n| n.checked_add(128)).ok_or(BackingError::Length)?;
    let input_lease = owner.quota().reserve(QuotaKind::Scratch, envelope).map_err(|_| BackingError::Quota)?;
    control.check(bytes)?;
    let mut native = Vec::new();
    native.try_reserve_exact(capacity).map_err(|_| BackingError::Allocation)?;
    if native.capacity() > capacity.checked_mul(2).ok_or(BackingError::Length)? {
        return Err(BackingError::Allocation.into());
    }
    while native.len() < capacity {
        let next = native.len().checked_add(65536).ok_or(BackingError::Length)?.min(capacity);
        control.check((next - native.len()) as u64)?;
        native.resize(next, 0);
    }
    for block in native.chunks_mut(65536) {
        control.check(block.len() as u64)?;
        for output in block.chunks_exact_mut(bpp) {
            output.copy_from_slice(one);
        }
    }
    let zero = [0; 8];
    let selection = AccountedSelection::try_from_native(
        format,
        area,
        zero.get(..bpp).ok_or(BackingError::Length)?,
        &native,
        CoverageDomain::FiniteUnit,
        owner.quota(),
        control,
    )?;
    // Borrowed input and its real lease remain alive through typed staging.
    let result = session.replace_accounted_selection(target, "Select All", Some(&selection), control);
    drop(native);
    drop(input_lease);
    result?;
    Ok(Value::Null)
}
#[cfg(test)]
#[path = "accounted_select_all_tests.rs"]
mod tests;
