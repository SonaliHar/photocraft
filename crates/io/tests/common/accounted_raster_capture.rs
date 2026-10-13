//! Test-only resident control -> authorized immutable backing conversion.
//! This is not default import routing or complete resident model accounting.
use photocraft_doc::BackedLayer;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_io::{
    native_plane_store::{NativeStoreLimits, VerifiedNativePlaneStore},
    xcf_source::ServiceAuthorizedScratchDirectory,
};
use photocraft_raster::{
    Surface,
    backing::{BackedRaster, BackingError, NativePlaneDescriptor, ReadControl, quota::BackingQuota},
};
use std::sync::Arc;

/// Reject physical retained pixels that cannot be represented by logical rows
/// and the exact default. Never clear or discard nondefault padding.
pub fn validate_padding(surface: &Surface, bounds: Rect, ctl: &ReadControl<'_>) -> Result<(), BackingError> {
    surface.try_validate_tile_origins_with(ctl.interrupt).map_err(|error| match error {
        photocraft_raster::RasterInputError::Cancelled => BackingError::Cancelled,
        _ => BackingError::Bounds,
    })?;
    let bpp = surface.format().bytes_per_pixel();
    for (coord, tile) in surface.tiles() {
        let x0 = coord.tx.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
        let y0 = coord.ty.checked_mul(TILE_SIZE).ok_or(BackingError::Bounds)?;
        for (index, pixel) in tile.bytes().chunks_exact(bpp).enumerate() {
            if index % 64 == 0 {
                ctl.check(64)?;
            }
            let x = x0.checked_add((index % TILE_SIZE as usize) as i32).ok_or(BackingError::Bounds)?;
            let y = y0.checked_add((index / TILE_SIZE as usize) as i32).ok_or(BackingError::Bounds)?;
            if (x < bounds.x0 || x >= bounds.x1 || y < bounds.y0 || y >= bounds.y1) && pixel != surface.default_bytes() {
                return Err(BackingError::Unsupported);
            }
        }
    }
    ctl.check(0)
}

/// The provider owns its sealed handle after this returns. The source Surface
/// and platform service remain borrowed only during bounded capture.
pub fn capture(
    surface: &Surface,
    bounds: Rect,
    service: &ServiceAuthorizedScratchDirectory<'_, '_>,
    quota: &BackingQuota,
    ctl: &ReadControl<'_>,
) -> Result<BackedLayer, BackingError> {
    validate_padding(surface, bounds, ctl)?;
    let descriptor = NativePlaneDescriptor::new(surface.format(), bounds, surface.default_bytes())?;
    let handle = service.create_owned_native_storage(descriptor, quota, ctl)?;
    let bpp = surface.format().bytes_per_pixel();
    let width = usize::try_from(i64::from(bounds.x1) - i64::from(bounds.x0)).map_err(|_| BackingError::Bounds)?;
    let store = VerifiedNativePlaneStore::capture(handle, descriptor, quota.clone(), NativeStoreLimits::default(), ctl, |offset, output, control| {
        let start = usize::try_from(offset).map_err(|_| BackingError::Length)?;
        if start % bpp != 0 || output.len() % bpp != 0 {
            return Err(BackingError::Length);
        }
        let mut pixel = start / bpp;
        let mut written = 0;
        while written < output.len() {
            control.check(1)?;
            let col = pixel % width;
            let row = pixel / width;
            let count = (width - col).min((output.len() - written) / bpp);
            let x = bounds.x0.checked_add(i32::try_from(col).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
            let y = bounds.y0.checked_add(i32::try_from(row).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
            let end_x = x.checked_add(i32::try_from(count).map_err(|_| BackingError::Bounds)?).ok_or(BackingError::Bounds)?;
            let end_y = y.checked_add(1).ok_or(BackingError::Bounds)?;
            let bytes = count.checked_mul(bpp).ok_or(BackingError::Length)?;
            surface.try_read_interleaved_into_with(Rect::new(x, y, end_x, end_y), &mut output[written..written + bytes], control.interrupt).map_err(
                |error| match error {
                    photocraft_raster::RasterInputError::Cancelled => BackingError::Cancelled,
                    _ => BackingError::Io,
                },
            )?;
            written += bytes;
            pixel += count;
        }
        Ok(())
    })?;
    BackedLayer::new(BackedRaster::new(Arc::new(store), quota.clone(), 8, ctl)?, descriptor, ctl).map_err(|error| match error {
        photocraft_doc::backed_layer::BackedLayerError::Backing(error) => error,
        photocraft_doc::backed_layer::BackedLayerError::DescriptorMismatch => BackingError::Integrity,
    })
}
