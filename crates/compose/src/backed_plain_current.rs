//! Controlled CURRENT validation for the existing implicit plain imported lane.
//! This retains its existing renderer/profile policy and grants no source,
//! geometry-edit, native-write or assigned-profile authority.
use crate::{
    backed_document::{BackedDocumentSinkError, PreparedBackedDocument},
    backed_normal::BackedNormalError as Error,
};
use photocraft_doc::{ColorMode, LayerContent, WorkingProfileInterpretation, accounted_document::AccountedDocument};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    backing::{BackingError, ReadControl, quota::QuotaKind},
    checked_view::{CheckedRasterView, CheckedTilePin, RasterViewError},
};

fn scope() -> Error {
    Error::Unsupported("current implicit plain imported Normal storage")
}
fn unit(bytes: &[u8], format: photocraft_color::PixelFormat, control: &ReadControl<'_>) -> Result<(), Error> {
    control.check(format.channels() as u64).map_err(Error::Backing)?;
    for channel in 0..format.channels() {
        let value = photocraft_color::try_read_sample_f64(bytes, format.sample, channel).map_err(|e| Error::View(RasterViewError::Sample(e)))?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(scope());
        }
    }
    Ok(())
}
fn scan(view: CheckedRasterView<'_>, control: &ReadControl<'_>) -> Result<(), Error> {
    let format = view.format();
    unit(view.default_bytes(), format, control)?;
    let bounds = view.logical_bounds();
    for coord in bounds.tiles() {
        control.check(1).map_err(Error::Backing)?;
        let x = coord.tx.checked_mul(TILE_SIZE).ok_or(Error::Backing(BackingError::Bounds))?;
        let y = coord.ty.checked_mul(TILE_SIZE).ok_or(Error::Backing(BackingError::Bounds))?;
        let tile = Rect::new(
            x,
            y,
            x.checked_add(TILE_SIZE).ok_or(Error::Backing(BackingError::Bounds))?,
            y.checked_add(TILE_SIZE).ok_or(Error::Backing(BackingError::Bounds))?,
        )
        .intersect(&bounds);
        let pin = view.pin_tile(coord, control).map_err(Error::View)?;
        for y in tile.y0..tile.y1 {
            for x in tile.x0..tile.x1 {
                let mut raw = [0; 40];
                let pixel = raw.get_mut(..format.bytes_per_pixel()).ok_or_else(scope)?;
                pin.read_pixel_native_into(x, y, pixel, control).map_err(Error::View)?;
                unit(pixel, format, control)?;
            }
        }
    }
    Ok(())
}
/// Validate the existing paid implicit plain recipe and complete current canvas.
/// Hidden and off-canvas operative leaves are independently scanned before
/// rendering; skipped coverage cannot hide malformed current native samples.
/// All providers, temporary pins and returned bands use the actual owner ledger
/// and this same caller control. No receipt or source capability is synthesized.
pub fn validate_current_plain_import_storage(owner: &AccountedDocument, control: &ReadControl<'_>) -> Result<(), Error> {
    control.check(1).map_err(Error::Backing)?;
    // The shared display factory has additional class-specific consumers.
    // Those consumers do not acquire this Normal-only editing prerequisite.
    if owner.is_readonly_flat_composite_storage() {
        return Err(scope());
    }
    let doc = owner.document();
    owner.validate_selection_replacement(control).map_err(Error::Backing)?;
    let builtin = match (&doc.working_profile_interpretation, doc.mode) {
        (WorkingProfileInterpretation::ImplicitSrgb { .. }, ColorMode::Rgb) => photocraft_cms::Builtin::Srgb,
        (WorkingProfileInterpretation::ImplicitSGray { .. }, ColorMode::Grayscale) => photocraft_cms::Builtin::SGray,
        _ => return Err(scope()),
    };
    // Bound the route before the shared factory so other runtime classes cannot
    // select a different compositor or acquire plain validation by retry.
    if doc.layers.is_empty() || doc.layers.len() > 4096 || doc.xcf_state.as_ref().is_none_or(|s| !s.filter_stacks.is_empty() || !s.filter_masks.is_empty()) {
        return Err(scope());
    }
    for layer in &doc.layers {
        control.check(1).map_err(Error::Backing)?;
        if !matches!(layer.content, LayerContent::BackedRaster(_)) || layer.mask.is_some() {
            return Err(scope());
        }
    }
    // Builtin OnceLock initialization remains an outer-polled provider boundary.
    control.check(1).map_err(Error::Backing)?;
    let profile = builtin.profile();
    control.check(1).map_err(Error::Backing)?;
    let prepared = PreparedBackedDocument::new_accounted(owner, profile, Default::default(), control)?;
    let _scratch = owner.quota().reserve(QuotaKind::Resident, (std::mem::size_of::<CheckedTilePin>() + 40 + 128) as u64).map_err(Error::Quota)?;
    for layer in &doc.layers {
        control.check(1).map_err(Error::Backing)?;
        let LayerContent::BackedRaster(backed) = &layer.content else {
            return Err(scope());
        };
        scan(backed.checked_view(), control)?;
    }
    prepared
        .render_bands(doc.bounds(), control, |band| {
            drop(band);
            Ok::<(), std::convert::Infallible>(())
        })
        .map_err(|e| match e {
            BackedDocumentSinkError::Render(error) => error,
            BackedDocumentSinkError::Sink(never) => match never {},
        })?;
    control.check(0).map_err(Error::Backing)
}

#[cfg(test)]
#[path = "backed_plain_current_tests.rs"]
mod tests;
