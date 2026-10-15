//! Private sealed native INPUT executor. Stock output remains wide: its typed
//! pipeline owns the exact existing U8/U32-only current-buffer rounding policy.
//! No arbitrary callback, CMS inference or source/runtime/default grant.
use super::*;
use crate::{
    backed_normal::BackedNormalError,
    backed_stock_point_context::{OwnedStockPointWideBand, PreparedStockPointPipeline},
};
pub(super) enum KernelRef<'a> {
    Native { kernel: &'a PreparedKernel, enabled: bool },
    Stock(&'a PreparedStockPointPipeline),
}
pub(super) enum OutputRef<'a> {
    Native(Option<&'a mut [u8]>),
    Stock(Option<&'a mut [[f64; 4]]>),
}
pub(super) enum WalkError {
    Point(Error),
    Stock(BackedNormalError),
}
fn index(rect: Rect, x: i32, y: i32) -> Result<usize, Error> {
    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| length())?;
    let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| length())?;
    row.checked_mul(rect.width() as usize).and_then(|n| n.checked_add(col)).ok_or_else(length)
}
/// One actual checked native reader, one pin at a time, shared by the exact old
/// public native Point path and the sealed Stock wide path. Stack/CMS work uses
/// the same supplied ReadControl in both preflight and output passes.
pub(super) fn walk(layer: &BackedLayer, rect: Rect, kernel: KernelRef<'_>, mut output: OutputRef<'_>, control: &ReadControl<'_>) -> Result<(), WalkError> {
    if !matches!((&kernel, &output), (KernelRef::Native { .. }, OutputRef::Native(_)) | (KernelRef::Stock(_), OutputRef::Stock(_))) {
        return Err(WalkError::Point(Error::Invalid("sealed point kernel/output mismatch")));
    }
    let view = layer.checked_view();
    let format = layer.format();
    let bpp = format.bytes_per_pixel();
    for coord in rect.tiles() {
        control.check(0).map_err(|e| WalkError::Point(Error::Backing(e)))?;
        let overlap = tile_rect(coord).map_err(WalkError::Point)?.intersect(&rect);
        let pin = view.pin_tile(coord, control).map_err(|e| WalkError::Point(Error::View(e)))?;
        for y in overlap.y0..overlap.y1 {
            for x in overlap.x0..overlap.x1 {
                control.check(0).map_err(|e| WalkError::Point(Error::Backing(e)))?;
                let mut raw = [0u8; 40];
                let source = raw.get_mut(..bpp).ok_or_else(|| WalkError::Point(length()))?;
                pin.read_pixel_native_into(x, y, source, control).map_err(|e| WalkError::Point(Error::View(e)))?;
                match &kernel {
                    KernelRef::Native { kernel, enabled } => {
                        let mut evaluated = [0u8; 40];
                        let pixel = evaluated.get_mut(..bpp).ok_or_else(|| WalkError::Point(length()))?;
                        if *enabled {
                            kernel.evaluate(source, format, pixel).map_err(|e| WalkError::Point(Error::Operation(e)))?;
                        } else {
                            pixel.copy_from_slice(source);
                        }
                        if let OutputRef::Native(Some(bytes)) = &mut output {
                            let start = index(rect, x, y).map_err(WalkError::Point)?.checked_mul(bpp).ok_or_else(|| WalkError::Point(length()))?;
                            bytes.get_mut(start..add(start, bpp).map_err(WalkError::Point)?).ok_or_else(|| WalkError::Point(length()))?.copy_from_slice(pixel);
                        }
                    }
                    KernelRef::Stock(pipeline) => {
                        let pixel = pipeline.evaluate_native(source, format, control).map_err(WalkError::Stock)?;
                        if let OutputRef::Stock(Some(pixels)) = &mut output {
                            *pixels.get_mut(index(rect, x, y).map_err(WalkError::Point)?).ok_or_else(|| WalkError::Point(length()))? = pixel;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
fn normal(error: WalkError) -> BackedNormalError {
    match error {
        WalkError::Stock(error) => error,
        WalkError::Point(Error::Backing(error)) => BackedNormalError::Backing(error),
        WalkError::Point(Error::View(error)) => BackedNormalError::View(error),
        WalkError::Point(Error::Operation(_)) => BackedNormalError::Invalid("native point operation selected for stock wide output"),
        WalkError::Point(Error::Invalid(message)) => BackedNormalError::Invalid(message),
    }
}
fn invalid() -> BackedNormalError {
    BackedNormalError::Invalid("stock wide band size or coordinate overflow")
}
/// Private typed caller only. The pipeline/context retains all actual model,
/// profile, source/mask and preparation fees. This function owns only the new
/// band and bounded input scratch; it cannot qualify those external semantics.
pub(crate) fn render_stock_wide_band(
    layer: &BackedLayer,
    expected: NativePlaneDescriptor,
    limits: NormalBandLimits,
    pipeline: &PreparedStockPointPipeline,
    rect: Rect,
    control: &ReadControl<'_>,
) -> Result<OwnedStockPointWideBand, BackedNormalError> {
    control.check(1).map_err(BackedNormalError::Backing)?;
    let maximum = NormalBandLimits::default();
    if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
        return Err(BackedNormalError::Invalid("lower-only nonzero stock wide band limits"));
    }
    if layer.descriptor() != expected || pipeline.format() != layer.format() || !pipeline.shares_quota(layer.quota()) {
        return Err(BackedNormalError::Invalid("stock native input descriptor, format or quota mismatch"));
    }
    let bounds = layer.operative_bounds();
    if rect.is_empty() || rect.intersect(&bounds) != rect || rect.height() > limits.max_rows {
        return Err(BackedNormalError::Invalid("stock wide band containment or row limit"));
    }
    let count = u64::from(rect.width()).checked_mul(u64::from(rect.height())).filter(|n| *n <= limits.max_pixels).ok_or_else(invalid)?;
    let count = usize::try_from(count).map_err(|_| invalid())?;
    let capacity = count.checked_mul(2).ok_or_else(invalid)?;
    let header = std::mem::size_of::<OwnedStockPointWideBand>().checked_add(128).ok_or_else(invalid)?;
    let payload = capacity.checked_mul(std::mem::size_of::<[f64; 4]>()).ok_or_else(invalid)?;
    let bytes = payload.checked_add(header).ok_or_else(invalid)?;
    let mut lease = layer.quota().reserve(QuotaKind::Resident, u64::try_from(bytes).map_err(|_| invalid())?).map_err(BackedNormalError::Quota)?;
    // Shared input reader + evaluated wide pixel + one real native pin. The
    // pipeline's numerical/CMS temporary reservations remain separately real.
    let scratch = std::mem::size_of::<[u8; 40]>()
        .checked_add(std::mem::size_of::<[f64; 4]>())
        .and_then(|n| n.checked_add(std::mem::size_of::<CheckedTilePin>()))
        .and_then(|n| n.checked_add(128))
        .ok_or_else(invalid)?;
    let _scratch = layer.quota().reserve(QuotaKind::Resident, u64::try_from(scratch).map_err(|_| invalid())?).map_err(BackedNormalError::Quota)?;
    // Known two-pass reader/loop/index/init/copy work is prepaid once. Native
    // provider and actual WideWork additionally debit this same caller control.
    let bridge_work = (count as u64).checked_mul(2 * PIXEL_WORK + 128).and_then(|n| n.checked_add(payload as u64)).ok_or_else(invalid)?;
    control.check(bridge_work).map_err(BackedNormalError::Backing)?;
    walk(layer, rect, KernelRef::Stock(pipeline), OutputRef::Stock(None), control).map_err(normal)?;
    let mut pixels = Vec::new();
    control.check(0).map_err(BackedNormalError::Backing)?;
    pixels.try_reserve_exact(count).map_err(|_| BackedNormalError::Allocation)?;
    if pixels.capacity() > capacity {
        return Err(BackedNormalError::Allocation);
    }
    let actual = pixels.capacity().checked_mul(std::mem::size_of::<[f64; 4]>()).and_then(|n| n.checked_add(header)).ok_or_else(invalid)?;
    lease.reconcile_capacity(u64::try_from(actual).map_err(|_| invalid())?).map_err(BackedNormalError::Quota)?;
    while pixels.len() < count {
        control.check(0).map_err(BackedNormalError::Backing)?;
        let end = pixels.len().checked_add((count - pixels.len()).min((64 << 10) / std::mem::size_of::<[f64; 4]>())).ok_or_else(invalid)?;
        pixels.resize(end, [0.; 4]);
    }
    walk(layer, rect, KernelRef::Stock(pipeline), OutputRef::Stock(Some(&mut pixels)), control).map_err(normal)?;
    let band = OwnedStockPointWideBand::from_accounted_pixels(rect, pixels, lease);
    control.check(0).map_err(BackedNormalError::Backing)?;
    Ok(band)
}
#[cfg(test)]
#[path = "stock_wide_tests.rs"]
mod tests;
