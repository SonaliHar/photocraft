//! One calculation/load/duplicate preparation ledger. Known outer phases and
//! checked Shape alpha are charged independently; legacy renderer/provider
//! private phases retain their documented Interrupt/ceiling-only contract.
use super::*;
use photocraft_raster::{Interrupt, RasterInputError};
use std::sync::atomic::{AtomicU64, Ordering};
pub(super) const WORK: u64 = 8_000_000_000;
pub(super) struct SourcePreparation<'a, 'i> {
    control: &'a Interrupt<'i>,
    cmd: &'a str,
    remaining: AtomicU64,
    base_peak: AtomicU64,
}
impl<'a, 'i> SourcePreparation<'a, 'i> {
    pub(super) fn new(cmd: &'a str, control: &'a Interrupt<'i>) -> Result<Self> {
        source_poll(control)?;
        Ok(Self { control, cmd, remaining: AtomicU64::new(WORK), base_peak: AtomicU64::new(0) })
    }
    pub(super) fn debit(&self, work: u64) -> Result<()> {
        source_poll(self.control)?;
        self.remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |old| old.checked_sub(work))
            .map_err(|_| bad(self.cmd, "aggregate source preparation Work limit"))?;
        Ok(())
    }
    pub(super) fn pixels(&self, count: usize, factor: u64) -> Result<()> {
        self.debit(u64::try_from(count).ok().and_then(|n| n.checked_mul(factor)).ok_or_else(|| bad(self.cmd, "source preparation Work overflow"))?)
    }
    /// Prospective whole physical tile initialization/COW/encode/prune and
    /// growing-map clone visits, independently of logical sample arithmetic.
    /// A one-pixel canvas still initializes/scans a full 256x256 native tile.
    pub(super) fn physical_output(&self, tiles: u64, bytes_per_pixel: u64) -> Result<()> {
        let tile = 65536_u64
            .checked_mul(
                64_u64
                    .checked_add(bytes_per_pixel.checked_mul(4).ok_or_else(|| bad(self.cmd, "physical output Work overflow"))?)
                    .ok_or_else(|| bad(self.cmd, "physical output Work overflow"))?,
            )
            .and_then(|n| n.checked_add(1024))
            .ok_or_else(|| bad(self.cmd, "physical output Work overflow"))?;
        let work = tiles
            .checked_mul(tile)
            .and_then(|n| n.checked_add(tiles.checked_mul(tiles.checked_add(1)?)?.checked_mul(256)?))
            .ok_or_else(|| bad(self.cmd, "physical output map Work overflow"))?;
        self.debit(work)
    }
    pub(super) fn admit_peak(&self, bytes: u64) -> Result<()> {
        source_poll(self.control)?;
        if bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
            return Err(bad(self.cmd, "aggregate source preparation Resident limit"));
        }
        self.base_peak.fetch_max(bytes, Ordering::Relaxed);
        Ok(())
    }
    pub(super) fn census(&self, doc: &Document) -> Result<(u64, u64)> {
        let work = |n| {
            self.debit(n)
                .map_err(|e| if self.control.cancelled() { photocraft_io::IoError::Cancelled } else { photocraft_io::IoError::Unsupported(e.to_string()) })
        };
        let limits = photocraft_io::NativeSaveLimits::default().preparation;
        let model = photocraft_io::retained_native_model_census_with_work(doc, limits, self.control, &work)
            .map_err(|e| if self.control.cancelled() { EngineError::Cancelled } else { bad(self.cmd, e.to_string()) })?;
        let payload = photocraft_io::retained_native_payload_census_with_work(doc, limits, self.control, &work)
            .map_err(|e| if self.control.cancelled() { EngineError::Cancelled } else { bad(self.cmd, e.to_string()) })?;
        Ok((model.retained_bytes() as u64, payload.retained_bytes() as u64))
    }
    pub(super) fn alpha(&self, shape: &photocraft_doc::ShapeLayer, canvas: Rect, plane: &mut [f32]) -> Result<()> {
        // The entire plane is already in the parent admission. Additional live
        // geometry, alpha bands and raster scratch share the remaining envelope.
        let available = photocraft_raster::MAX_CHECKED_RASTER_BYTES
            .checked_sub(self.base_peak.load(Ordering::Relaxed))
            .ok_or_else(|| bad(self.cmd, "Shape alpha Resident overlap"))?;
        let mut limits = photocraft_vector::VectorPreparationLimits::default();
        limits.max_bytes = limits.max_bytes.min(usize::try_from(available).map_err(|_| bad(self.cmd, "Shape alpha Resident overflow"))?);
        limits.max_work = limits.max_work.min(self.remaining.load(Ordering::Relaxed));
        let mut work = |n| self.debit(n).map_err(|_| if self.control.cancelled() { RasterInputError::Cancelled } else { RasterInputError::LimitExceeded });
        let (compiled, _) = photocraft_vector::raster::CheckedShapeAlpha::try_new_with_control(
            shape,
            photocraft_vector::DEFAULT_TOLERANCE,
            canvas,
            limits,
            self.control,
            &mut work,
        )
        .map_err(|e| if self.control.cancelled() { EngineError::Cancelled } else { bad(self.cmd, e.to_string()) })?;
        // Keep the whole-area horizontal raster origin and prefix-sum order.
        // Splitting x folds all coverage left of a cell into one f32 value and
        // changes native alpha bits. Rows are independent, so lower only the
        // strip height to keep about4096 samples live (one full row if wider).
        // Every actual vector/header allocation is still admitted by the same
        // checked renderer and original remaining Resident/Work envelope.
        let width = usize::try_from(canvas.width()).map_err(|_| bad(self.cmd, "Shape alpha canvas width"))?;
        if width == 0 {
            return source_poll(self.control);
        }
        let rows = (4096 / width).clamp(1, 64);
        for y in (canvas.y0..canvas.y1).step_by(rows) {
            self.debit(64)?;
            let cell = Rect::new(canvas.x0, y, canvas.x1, y.saturating_add(rows as i32).min(canvas.y1));
            let (alpha, _) = compiled
                .try_render_with_control(cell, limits, self.control, &mut work)
                .map_err(|e| if self.control.cancelled() { EngineError::Cancelled } else { bad(self.cmd, e.to_string()) })?;
            for (row, values) in alpha.chunks(width).enumerate() {
                self.pixels(values.len(), 2)?;
                let oy = usize::try_from(i64::from(y) - i64::from(canvas.y0))
                    .ok()
                    .and_then(|n| n.checked_add(row))
                    .ok_or_else(|| bad(self.cmd, "Shape alpha row overflow"))?;
                let start = oy.checked_mul(width).ok_or_else(|| bad(self.cmd, "Shape alpha offset overflow"))?;
                let end = start.checked_add(values.len()).ok_or_else(|| bad(self.cmd, "Shape alpha end overflow"))?;
                plane.get_mut(start..end).ok_or_else(|| bad(self.cmd, "Shape alpha plane mismatch"))?.copy_from_slice(values);
            }
        }
        source_poll(self.control)
    }
}

#[cfg(test)]
#[path = "source_preparation_tests.rs"]
mod tests;
