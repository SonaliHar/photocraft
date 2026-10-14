//! Closed alpha-only picking. Native color words stay opaque; no CMS or rendering.
#[cfg(test)]
#[path = "backed_alpha_tests.rs"]
mod tests;
use crate::{EngineError, Result};
use photocraft_doc::{BackedLayer, Document, Layer};
use photocraft_raster::{
    Interrupt,
    backing::{BackingError, ReadControl, quota::QuotaKind},
};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_WORK: u64 = 1 << 30;
pub(super) struct PickReadWork {
    used: AtomicU64,
}
impl PickReadWork {
    pub(super) fn new() -> Self {
        Self { used: AtomicU64::new(0) }
    }
    fn consume(&self, amount: u64, control: &Interrupt<'_>) -> std::result::Result<(), BackingError> {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            control.check().map_err(|_| BackingError::Cancelled)?;
            let next = current.checked_add(amount).filter(|n| *n <= MAX_WORK).ok_or(BackingError::Work)?;
            match self.used.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return control.check().map_err(|_| BackingError::Cancelled),
                Err(actual) => current = actual,
            }
        }
    }
}
fn unsupported() -> EngineError {
    EngineError::Other("layer.pickAt.backed-pixel-sampling-unsupported".into())
}
fn read_error(error: impl std::fmt::Display, control: &Interrupt<'_>) -> EngineError {
    if control.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("layer.pickAt.backed-alpha-read: {error}")) }
}
/// Actual backing quota owns provider tile/cache allocations. The job-local work
/// owner is shared across every backed hit in one walk, without reset or refund.
pub(super) fn hit(doc: &Document, layer: &Layer, backed: &BackedLayer, x: i32, y: i32, work: &PickReadWork, control: &Interrupt<'_>) -> Result<bool> {
    control.check().map_err(|_| EngineError::Cancelled)?;
    if !backed.operative_bounds().contains(x, y) {
        return Ok(false);
    }
    if doc.xcf_state.is_some()
        || doc.source_color_context.is_some()
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || layer.import_compositing.is_some()
        || layer.indexed_source.is_some()
        || layer.video.is_some()
        || layer.fill_cache.is_some()
    {
        return Err(unsupported());
    }
    work.consume(1, control).map_err(|e| read_error(e, control))?;
    let format = backed.format();
    // Alpha-free pixel layouts are opaque inside operative bounds. Opacity/fill
    // are deliberately ignored, matching the existing ordinary picker policy.
    if !format.alpha {
        return Ok(true);
    }
    let envelope = std::mem::size_of::<PickReadWork>()
        + std::mem::size_of::<ReadControl<'_>>()
        + std::mem::size_of::<photocraft_raster::checked_view::CheckedRasterView<'_>>()
        + 40
        + 4 * std::mem::size_of::<usize>();
    let _header = backed.quota().reserve(QuotaKind::Resident, envelope as u64).map_err(|e| read_error(e, control))?;
    let charge = |amount| work.consume(amount, control);
    let read = ReadControl { interrupt: control, work: &charge };
    let mut raw = [0u8; 40];
    let encoded = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(unsupported)?;
    backed.checked_view().read_logical_pixel_native_into(x, y, encoded, &read).map_err(|e| read_error(e, control))?;
    // Decode only the runtime alpha slot. NaN/HDR/raw signed-zero color samples
    // neither require conversion nor change the hit decision.
    let alpha = photocraft_color::try_read_sample_f64(encoded, format.sample, format.mode.color_channels()).map_err(|e| read_error(e, control))?;
    if !alpha.is_finite() {
        return Err(EngineError::Other("layer.pickAt.backed-alpha-nonfinite".into()));
    }
    work.consume(1, control).map_err(|e| read_error(e, control))?;
    Ok(alpha > 0.0)
}
