//! Paid singleton neighborhood evaluation. Kernel equations are the existing
//! lower operations; each concurrently live Surface/temporary keeps a real fee.
use super::*;
use crate::backed_normal::BackedNormalError as E;
use photocraft_raster::backing::{
    BackingError, ReadControl,
    quota::{BackingQuota, QuotaKind, QuotaLease},
};
pub(crate) struct OwnedNeighborhoodSurface {
    pub(crate) surface: Surface,
    _lease: QuotaLease,
}
fn invalid() -> E {
    E::Invalid("owned neighborhood resource envelope")
}
fn lower(e: photocraft_raster::RasterInputError) -> E {
    match e {
        photocraft_raster::RasterInputError::Cancelled => E::Backing(BackingError::Cancelled),
        other => E::Color(other.into()),
    }
}
fn fee(q: &BackingQuota, bytes: u64) -> std::result::Result<QuotaLease, E> {
    q.reserve(QuotaKind::Resident, bytes).map_err(E::Quota)
}
fn surface_bytes(surface: &Surface) -> std::result::Result<u64, E> {
    surface
        .resident_payload_bytes()
        .map_err(lower)?
        .checked_mul(2)
        .and_then(|v| (surface.tile_count() as u64).checked_mul(512).and_then(|m| v.checked_add(m)))
        .and_then(|v| v.checked_add(std::mem::size_of::<Surface>() as u64 + 256))
        .ok_or_else(invalid)
}
fn checked(surface: Surface, lease: QuotaLease) -> std::result::Result<OwnedNeighborhoodSurface, E> {
    let owned = OwnedNeighborhoodSurface { surface, _lease: lease };
    if surface_bytes(&owned.surface)? > owned._lease.bytes() {
        return Err(E::Allocation);
    }
    Ok(owned)
}

fn map(
    source: &Surface,
    roi: Rect,
    q: &BackingQuota,
    c: &ReadControl<'_>,
    mut f: impl FnMut([f64; 4]) -> Result<[f64; 4]>,
) -> std::result::Result<OwnedNeighborhoodSurface, E> {
    let count = u64::from(roi.width()).checked_mul(u64::from(roi.height())).and_then(|v| v.checked_mul(4)).ok_or_else(invalid)?;
    let cap = count.checked_mul(2).ok_or_else(invalid)?;
    let tiles = roi.tiles().count() as u64;
    let envelope = tiles
        .checked_mul(256 * 256 * 32 * 2 + 512)
        .and_then(|v| cap.checked_mul(8).and_then(|n| v.checked_add(n)))
        .and_then(|v| v.checked_add(512))
        .ok_or_else(invalid)?;
    let lease = fee(q, envelope)?;
    // Tile zero fill/map metadata and destination Vec admission precede alloc.
    c.check(envelope).map_err(E::Backing)?;
    let mut failure = None;
    let mut poll = |event| {
        let result = match event {
            WideWork::SampleUnits(n) => c.check(n),
            _ => Err(BackingError::Integrity),
        };
        if let Err(e) = result {
            failure = Some(e);
            return Err(CompositionError::Invalid("owned neighborhood caller work"));
        }
        Ok(())
    };
    let result = map_pixels_at_bounded(source, roi, &mut poll, c.interrupt, |_, p| f(p), Some(usize::try_from(cap).map_err(|_| invalid())?));
    if let Some(error) = failure {
        return Err(E::Backing(error));
    }
    checked(result.map_err(E::Color)?, lease)
}
pub(super) fn evaluate(plan: &OrderedStack, source: &Surface, q: &BackingQuota, c: &ReadControl<'_>) -> std::result::Result<OwnedNeighborhoodSurface, E> {
    use photocraft_algo::gimp::neighborhood as n;
    c.check(1).map_err(E::Backing)?;
    let roi = plan.output_extent;
    let input = map(source, plan.extent, q, c, Ok)?;
    if plan.stock_disabled {
        if !plan.stages.is_empty() || !plan.spaces.is_empty() || roi != plan.extent {
            return Err(invalid());
        }
        return Ok(input);
    }
    let [stage] = plan.stages.as_slice() else { return Err(invalid()) };
    let working = match stage {
        Stage::BoxBlur { working, .. } | Stage::MotionBlur { working, .. } | Stage::Pixelize { working, .. } | Stage::GaussianFinite { working, .. } => {
            Some(plan.spaces.get(*working).ok_or_else(invalid)?)
        }
        Stage::MedianBlur { .. } => None,
        _ => return Err(invalid()),
    };
    let linear = if let Some(space) = working {
        Some(map(&input.surface, plan.extent, q, c, |p| {
            if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
                return Err(CompositionError::Unsupported("neighborhood finite color/unit alpha"));
            }
            let rgb = space.convert([p[0], p[1], p[2]], true)?;
            Ok([rgb[0], rgb[1], rgb[2], p[3]])
        })?)
    } else {
        None
    };
    let native = linear.as_ref().map_or(&input.surface, |v| &v.surface);
    // Lower helpers repeat bounded Surface metadata validation. Both their
    // prospective and constructor walks are prepaid on the same caller ledger.
    c.check((native.tile_count() as u64 + roi.tiles().count() as u64 + 1).checked_mul(1024).ok_or_else(invalid)?).map_err(E::Backing)?;
    let req = match stage {
        Stage::BoxBlur { radius, .. } => n::box_blur_requirements(native, roi, plan.extent, *radius, c.interrupt),
        Stage::MotionBlur { params, .. } => n::motion_blur_requirements(native, roi, plan.extent, *params, c.interrupt),
        Stage::Pixelize { params, .. } => n::pixelize_requirements(native, roi, plan.extent, *params, c.interrupt),
        Stage::MedianBlur { params } => n::median_requirements(native, roi, plan.extent, *params, c.interrupt),
        Stage::GaussianFinite { params, filter, .. } => n::gaussian_finite_requirements(native, plan.extent, plan.extent, *params, *filter, c.interrupt),
        _ => return Err(invalid()),
    }
    .map_err(lower)?;
    if req.peak_resident_bytes > CACHE_BYTES {
        return Err(invalid());
    }
    let extra = (roi.tiles().count() as u64).checked_mul(512).and_then(|v| v.checked_add(512)).ok_or_else(invalid)?;
    let lease = fee(q, req.peak_resident_bytes.checked_mul(2).and_then(|v| v.checked_add(extra)).ok_or_else(invalid)?)?;
    c.check(req.work_units.checked_add(req.peak_resident_bytes).ok_or_else(invalid)?).map_err(E::Backing)?;
    let limits = GaussianLimits { resident_bytes: CACHE_BYTES, work_units: req.work_units };
    let filtered = match stage {
        Stage::BoxBlur { radius, .. } => n::try_box_blur(native, roi, plan.extent, *radius, limits, c.interrupt),
        Stage::MotionBlur { params, .. } => n::try_motion_blur_linear(native, roi, plan.extent, *params, limits, c.interrupt),
        Stage::Pixelize { params, .. } => n::try_pixelize_square(native, roi, plan.extent, *params, limits, c.interrupt),
        Stage::MedianBlur { params } => {
            if params.high_precision {
                n::try_median_blur(native, roi, plan.extent, *params, limits, c.interrupt)
            } else {
                n::try_median_blur_quantized_encoded(native, roi, plan.extent, *params, limits, c.interrupt)
            }
        }
        Stage::GaussianFinite { params, filter, .. } => n::try_gaussian_finite_clamp(native, plan.extent, plan.extent, *params, *filter, limits, c.interrupt),
        _ => return Err(invalid()),
    }
    .map_err(lower)?;
    let filtered = checked(filtered, lease)?;
    if let Some(space) = working {
        map(&filtered.surface, roi, q, c, |p| {
            let rgb = space.convert([p[0], p[1], p[2]], false)?;
            Ok([rgb[0], rgb[1], rgb[2], p[3]])
        })
    } else {
        Ok(filtered)
    }
}
