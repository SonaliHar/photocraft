//! Exact zero-byte checks for the existing qualified filter-mask envelope.
//! No coverage interpretation, mask epoch or source domain is added.
use crate::gimp_composite::Result;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};
use std::sync::atomic::{AtomicBool, Ordering};
pub(super) fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
pub(super) fn prepare<T>(control: &Interrupt<'_>, operation: impl FnOnce(&Interrupt<'_>) -> Result<T>) -> Result<T> {
    let stopped = AtomicBool::new(false);
    let cancel = || {
        if control.cancelled() {
            stopped.store(true, Ordering::Relaxed);
        }
        stopped.load(Ordering::Relaxed)
    };
    let latched = Interrupt::cancel_only(&cancel);
    poll(&latched)?;
    let result = operation(&latched);
    poll(&latched)?;
    result
}
pub(super) fn iter<'a, T>(values: &'a [T], control: &'a Interrupt<'_>) -> impl Iterator<Item = &'a T> {
    let mut values = values.iter();
    std::iter::from_fn(move || if control.cancelled() { None } else { values.next() })
}
pub(super) fn bytes_zero(bytes: &[u8], control: &Interrupt<'_>) -> Result<bool> {
    for chunk in bytes.chunks(64 << 10) {
        poll(control)?;
        if chunk.iter().any(|byte| *byte != 0) {
            return Ok(false);
        }
    }
    poll(control)?;
    Ok(true)
}
pub(super) fn tiles_zero(source: &Surface, control: &Interrupt<'_>) -> Result<bool> {
    for (_, tile) in source.tiles() {
        poll(control)?;
        if !bytes_zero(tile.bytes(), control)? {
            return Ok(false);
        }
    }
    poll(control)?;
    Ok(true)
}
pub(super) fn region_zero(source: &Surface, region: Rect, control: &Interrupt<'_>) -> Result<bool> {
    checked_region_byte_len(region, source.format())?;
    let mut native = [0u8; 40];
    let native = native.get_mut(..source.format().bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
    for y in region.y0..region.y1 {
        poll(control)?;
        for x in region.x0..region.x1 {
            poll(control)?;
            source.try_read_pixel_bytes(x, y, native)?;
            if native.iter().any(|byte| *byte != 0) {
                return Ok(false);
            }
        }
    }
    poll(control)?;
    Ok(true)
}
