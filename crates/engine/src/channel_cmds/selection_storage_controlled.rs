//! Explicit raw selection storage transfer, not coverage arithmetic.
//! Preserves all retained tile words/defaults, including off-canvas storage and NaN payloads.
use crate::{DocumentSnapshot, EngineError, Result};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, Surface};
fn error(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| EngineError::Cancelled)
}
pub(super) fn copy(
    source: &DocumentSnapshot,
    target: &DocumentSnapshot,
    surface: &Surface,
    format: PixelFormat,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    poll(ctl)?;
    if source.xcf_state.is_some() || target.xcf_state.is_some() || source.source_color_context.is_some() || target.source_color_context.is_some() {
        return Err(error(cmd, "rawStorage source-carrier context is not qualified"));
    }
    copy_admitted(source, target, surface, format, cmd, ctl)
}
pub(super) fn copy_canonical_load(
    source: &DocumentSnapshot,
    target: &DocumentSnapshot,
    surface: &Surface,
    format: PixelFormat,
    proof: &super::load_selection_controlled::canonical_selection_storage::LoadProof,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    proof.check_copy(source, target, surface, format, ctl)?;
    copy_admitted(source, target, surface, format, cmd, ctl)
}
pub(super) fn copy_canonical_save(
    source: &DocumentSnapshot,
    target: &DocumentSnapshot,
    surface: &Surface,
    format: PixelFormat,
    proof: &super::load_selection_controlled::canonical_selection_storage::SaveProof,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    proof.check_copy(source, target, surface, format, ctl)?;
    copy_admitted(source, target, surface, format, cmd, ctl)
}
pub(super) fn copy_canonical_new(
    document: &DocumentSnapshot,
    surface: &Surface,
    format: PixelFormat,
    proof: &super::load_selection_controlled::canonical_selection_storage::NewProof,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    proof.check_copy(document, surface, format, ctl)?;
    copy_admitted(document, document, surface, format, cmd, ctl)
}
fn copy_admitted(
    source: &DocumentSnapshot,
    target: &DocumentSnapshot,
    surface: &Surface,
    format: PixelFormat,
    cmd: &str,
    ctl: &Interrupt<'_>,
) -> Result<Surface> {
    if surface.format() != format || format.mode != ColorMode::Grayscale || format.alpha {
        return Err(error(cmd, "rawStorage requires identical Gray depth without alpha"));
    }
    source.validate_resident()?;
    target.validate_resident()?;
    surface.try_validate_tile_origins_with(ctl)?;
    let tile_len = (photocraft_geom::TILE_SIZE as usize)
        .checked_mul(photocraft_geom::TILE_SIZE as usize)
        .and_then(|n| n.checked_mul(format.bytes_per_pixel()))
        .ok_or_else(|| error(cmd, "rawStorage tile length"))?;
    let mut bytes = 0u64;
    let mut work = 0u64;
    for document in [source, target] {
        poll(ctl)?;
        let limits = photocraft_io::NativeSaveLimits::default().preparation;
        let model = photocraft_io::retained_native_model_bytes(document, limits, ctl)
            .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })? as u64;
        let payload = photocraft_io::retained_native_payload_bytes(document, limits, ctl)
            .map_err(|e| if ctl.cancelled() { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })? as u64;
        let raster = document.resident_raster_footprint_with(ctl)?;
        bytes = bytes
            .checked_add(raster.retained_bytes)
            .and_then(|n| n.checked_add(raster.clone_metadata_bytes.checked_mul(4)?))
            .and_then(|n| n.checked_add(model.checked_mul(4)?))
            .and_then(|n| n.checked_add(payload))
            .ok_or_else(|| error(cmd, "rawStorage model budget"))?;
        work = work.checked_add(model).and_then(|n| n.checked_add(payload)).ok_or_else(|| error(cmd, "rawStorage work budget"))?;
    }
    for (_, tile) in surface.tiles() {
        poll(ctl)?;
        if tile.bytes().len() != tile_len {
            return Err(error(cmd, "rawStorage tile shape"));
        }
        bytes = bytes.checked_add(512).filter(|n| *n <= MAX_CHECKED_RASTER_BYTES).ok_or_else(|| error(cmd, "rawStorage map budget"))?;
        work = work.checked_add(32).filter(|n| *n <= 1 << 32).ok_or_else(|| error(cmd, "rawStorage work limit"))?;
    }
    if bytes > MAX_CHECKED_RASTER_BYTES || work > 1 << 32 {
        return Err(error(cmd, "rawStorage retained limit"));
    }
    poll(ctl)?;
    let mut output = Surface::try_with_default_bytes(format, surface.default_bytes())?;
    for (position, tile) in surface.tiles() {
        poll(ctl)?;
        output.put_tiles(std::iter::once((*position, std::sync::Arc::clone(tile))));
    }
    poll(ctl)?;
    Ok(output)
}
/// Controlled exact storage equality for raw replacement no-ops, not sampled equality.
pub(super) fn equal_storage(a: &Surface, b: &Surface, ctl: &Interrupt<'_>) -> Result<bool> {
    poll(ctl)?;
    let work = a
        .resident_payload_bytes()?
        .checked_add(b.resident_payload_bytes()?)
        .filter(|n| *n <= 1 << 32)
        .ok_or_else(|| error("select.saveSelection", "raw equality work limit"))?;
    let _ = work;
    if a.format() != b.format() || a.default_bytes() != b.default_bytes() || a.tile_count() != b.tile_count() {
        poll(ctl)?;
        return Ok(false);
    }
    for ((ap, at), (bp, bt)) in a.tiles().zip(b.tiles()) {
        poll(ctl)?;
        if ap != bp || at.bytes().len() != bt.bytes().len() {
            return Ok(false);
        }
        if std::sync::Arc::ptr_eq(at, bt) {
            continue;
        }
        for (left, right) in at.bytes().chunks(32768).zip(bt.bytes().chunks(32768)) {
            poll(ctl)?;
            if left != right {
                return Ok(false);
            }
        }
    }
    poll(ctl)?;
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::SampleType;
    use photocraft_doc::Document;
    use photocraft_geom::{Rect, Size};
    #[test]
    fn raw_defaults_offcanvas_nan_and_tile_identity_survive() {
        let doc = DocumentSnapshot::from_resident(std::sync::Arc::new(Document::new("Raw draft", Size::new(2, 2), ColorMode::Rgb, SampleType::F64))).unwrap();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let nan = 0x7ff8000000000042u64.to_ne_bytes();
        let mut surface = Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap();
        surface.try_write_interleaved(Rect::new(-7, 9, -6, 10), &nan).unwrap();
        let output = copy(&doc, &doc, &surface, format, "draft", &Interrupt::NONE).unwrap();
        assert_eq!(output.default_bytes(), surface.default_bytes());
        assert_eq!(output.try_read_interleaved(Rect::new(-7, 9, -6, 10)).unwrap(), nan);
        assert!(surface.tiles().zip(output.tiles()).all(|((a, b), (c, d))| a == c && std::sync::Arc::ptr_eq(b, d)));
        assert!(matches!(copy(&doc, &doc, &surface, format, "draft", &Interrupt::cancel_only(&|| true)), Err(EngineError::Cancelled)));
    }
}
