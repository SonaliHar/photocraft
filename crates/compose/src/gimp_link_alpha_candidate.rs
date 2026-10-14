//! Explicit alpha-only candidate consumer. Never called by imported-link dispatch.
//! No RGB reconstruction, CMS, source qualification or complete LoHalo claim.
use photocraft_algo::gimp::lohalo_alpha_candidate::{self as alpha, Limits, Request};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, RasterInputError as Error, Surface, checked_region_byte_len};
pub fn prepare_alpha(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, Error> {
    control.check().map_err(|_| Error::Cancelled)?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > 8_000_000_000 {
        return Err(Error::LimitExceeded);
    }
    let lower = alpha::requirements(request)?;
    let r = request.output_extent;
    // Use the same coordinate/length admission as the final native Surface
    // writer before allocating or reconstructing the private alpha plane.
    checked_region_byte_len(r, request.output_format)?;
    let first = TileCoord::containing(r.x0, r.y0);
    let last = TileCoord::containing(r.x1.checked_sub(1).ok_or(Error::InvalidBounds)?, r.y1.checked_sub(1).ok_or(Error::InvalidBounds)?);
    let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|x| x.checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok()?))
        .ok_or(Error::LimitExceeded)?;
    // Native tiles, overlapping maps/scheduler and fixed default storage. Standard
    // map/Arc allocations remain managed by Surface, not a complete RSS guarantee.
    let native = tiles
        .checked_mul((TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_add(2048)).ok_or(Error::LimitExceeded)?)
        .and_then(|n| n.checked_add(4096))
        .ok_or(Error::LimitExceeded)?;
    let staging_work = native.checked_mul(16).and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?)).ok_or(Error::LimitExceeded)?;
    if lower.peak_resident_bytes.checked_add(native).is_none_or(|n| n > limits.resident_bytes)
        || lower.work_units.checked_add(staging_work).is_none_or(|n| n > limits.work_units)
    {
        return Err(Error::LimitExceeded);
    }
    let plane = alpha::try_reconstruct(
        request,
        Limits {
            resident_bytes: limits.resident_bytes.checked_sub(native).ok_or(Error::LimitExceeded)?,
            work_units: limits.work_units.checked_sub(staging_work).ok_or(Error::LimitExceeded)?,
        },
        control,
    )?;
    let mut output = Surface::try_with_default_bytes(plane.format, &[0])?;
    output.try_write_interleaved_with(plane.extent, &plane.bytes, control)?;
    control.check().map_err(|_| Error::Cancelled)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::gimp::lohalo_alpha_candidate::Magnification;
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;

    // Unrun regression: lower dense arithmetic accepts these i32 coordinates,
    // while the Surface consumer must reject before reconstruction/reservation.
    // Zero quotas distinguish this early InvalidBounds from a later budget error.
    #[test]
    fn extreme_world_extent_refuses_before_alpha_staging() {
        let source = [0u8; 12 * 12 * 4];
        for origin in [(1 << 29) + 1, -(1 << 29) - 2] {
            let extent = Rect::from_xywh(origin, 1, 12, 12);
            let output = alpha::adjusted_extent(extent, Magnification::One).unwrap();
            let request = Request {
                raw_source: &source,
                source_format: PixelFormat::RGBA8,
                output_format: PixelFormat::GRAY8,
                source_extent: extent,
                output_extent: output,
                magnification: Magnification::One,
            };
            assert!(matches!(prepare_alpha(request, Limits { resident_bytes: 0, work_units: 0 }, &Interrupt::NONE), Err(Error::InvalidBounds)));
            assert_eq!(source, [0; 12 * 12 * 4]);
        }
    }
}
