//! Private, synchronous proof for the Fill transaction's alpha-lock exception.
//! No decoded comparisons, mutable model access or caller-asserted proof flags.
use photocraft_doc::{
    LayerContent, LayerId,
    accounted_document::{AccountedDocument, AccountedEdit, accounted_layer_access},
};
use photocraft_geom::Rect;
use photocraft_raster::{
    backing::{
        BackingError, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
    checked_view::{CheckedRasterView, RasterViewError},
};

/// Both borrowed immutable inputs and the exact owner stay held through staged
/// history preparation and its final gate. Only this module constructs a proof.
pub(super) struct VerifiedNativeAlpha<'a> {
    document: &'a AccountedDocument,
    layer: LayerId,
    rect: Rect,
    bytes: &'a [u8],
    _lease: QuotaLease,
}
fn view_error(error: RasterViewError) -> BackingError {
    match error {
        RasterViewError::Backed(error) | RasterViewError::Control(error) | RasterViewError::Descriptor(error) => error,
        RasterViewError::Resident(photocraft_raster::RasterInputError::Cancelled) => BackingError::Cancelled,
        RasterViewError::Length => BackingError::Length,
        RasterViewError::Bounds => BackingError::Bounds,
        _ => BackingError::Integrity,
    }
}
impl<'a> VerifiedNativeAlpha<'a> {
    pub(super) fn verify(document: &'a AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        c.check(1)?;
        let AccountedEdit::NativePixels { layer, rect, bytes } = edit else { return Err(BackingError::Unsupported) };
        // Price the fixed proof/view/native-pixel scratch before validation or
        // pinning. Tile/cache storage has the actual backing's own same-q fees.
        let fixed = std::mem::size_of::<Self>()
            .checked_add(std::mem::size_of::<CheckedRasterView<'_>>())
            .and_then(|n| n.checked_add(40 + 256))
            .ok_or(BackingError::Length)?;
        let lease = document.quota().reserve(QuotaKind::Resident, fixed as u64).map_err(|_| BackingError::Quota)?;
        document.validate_imported_native_edit(edit, c)?;
        let access = accounted_layer_access(document.document(), layer, c)?.ok_or(BackingError::Bounds)?;
        // These locks never receive an exception, even if the proposed bytes
        // happen to equal the original native region.
        if access.locks.all || access.locks.pixels || access.imported_locks.content {
            return Err(BackingError::Unsupported);
        }
        let LayerContent::BackedRaster(backed) = &access.layer.content else { return Err(BackingError::Unsupported) };
        let format = backed.format();
        if !format.alpha
            || !matches!(format.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
            || !matches!(
                format.sample,
                photocraft_doc::SampleType::U8
                    | photocraft_doc::SampleType::U16
                    | photocraft_doc::SampleType::U32
                    | photocraft_doc::SampleType::F32
                    | photocraft_doc::SampleType::F64
            )
            || !backed.quota().shares_ledger(document.quota())
        {
            return Err(BackingError::Unsupported);
        }
        let bpp = format.bytes_per_pixel();
        let sample_bytes = format.sample.bytes();
        let alpha_begin = bpp.checked_sub(sample_bytes).ok_or(BackingError::Length)?;
        if bpp > 40 || sample_bytes == 0 || alpha_begin == 0 || rect.is_empty() || !backed.operative_bounds().contains_rect(&rect) {
            return Err(BackingError::Bounds);
        }
        let width = usize::try_from(rect.width()).map_err(|_| BackingError::Length)?;
        let length = width
            .checked_mul(usize::try_from(rect.height()).map_err(|_| BackingError::Length)?)
            .and_then(|n| n.checked_mul(bpp))
            .ok_or(BackingError::Length)?;
        if bytes.len() != length {
            return Err(BackingError::Length);
        }
        let view = backed.checked_view();
        let mut raw = [0u8; 40];
        for tile in rect.tiles() {
            c.check(1)?;
            let part = tile.rect().intersect(&rect);
            let pin = view.pin_tile(tile, c).map_err(view_error)?;
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let pixel = raw.get_mut(..bpp).ok_or(BackingError::Length)?;
                    pin.read_pixel_native_into(x, y, pixel, c).map_err(view_error)?;
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackingError::Bounds)?;
                    let column = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackingError::Bounds)?;
                    let begin = row
                        .checked_mul(width)
                        .and_then(|n| n.checked_add(column))
                        .and_then(|n| n.checked_mul(bpp))
                        .and_then(|n| n.checked_add(alpha_begin))
                        .ok_or(BackingError::Length)?;
                    let end = begin.checked_add(sample_bytes).ok_or(BackingError::Length)?;
                    c.check(sample_bytes as u64)?;
                    if bytes.get(begin..end).ok_or(BackingError::Length)? != pixel.get(alpha_begin..bpp).ok_or(BackingError::Length)? {
                        return Err(BackingError::Integrity);
                    }
                }
            }
            // The next tile never overlaps this pin's lifetime.
            drop(pin);
            c.check(1)?;
        }
        c.check(1)?;
        Ok(Self { document, layer, rect, bytes, _lease: lease })
    }

    pub(super) fn verify_normal_hierarchy(document: &'a AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        c.check(1)?;
        let AccountedEdit::NativePixels { layer, rect, bytes } = edit else { return Err(BackingError::Unsupported) };
        // Price the fixed proof/view/native-pixel scratch before validation or
        // pinning. Tile/cache storage has the actual backing's own same-q fees.
        let fixed = std::mem::size_of::<Self>()
            .checked_add(std::mem::size_of::<CheckedRasterView<'_>>())
            .and_then(|n| n.checked_add(40 + 256))
            .ok_or(BackingError::Length)?;
        let lease = document.quota().reserve(QuotaKind::Resident, fixed as u64).map_err(|_| BackingError::Quota)?;
        document.validate_readonly_normal_hierarchy_native_edit(edit, c)?;
        let access = document.accounted_layer_access(layer, c)?.ok_or(BackingError::Bounds)?;
        // These locks never receive an exception, even if the proposed bytes
        // happen to equal the original native region.
        if access.locks.all || access.locks.pixels || access.imported_locks.content {
            return Err(BackingError::Unsupported);
        }
        let LayerContent::BackedRaster(backed) = &access.layer.content else { return Err(BackingError::Unsupported) };
        let format = backed.format();
        if !format.alpha
            || !matches!(format.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
            || !matches!(
                format.sample,
                photocraft_doc::SampleType::U8
                    | photocraft_doc::SampleType::U16
                    | photocraft_doc::SampleType::U32
                    | photocraft_doc::SampleType::F32
                    | photocraft_doc::SampleType::F64
            )
            || !backed.quota().shares_ledger(document.quota())
        {
            return Err(BackingError::Unsupported);
        }
        let bpp = format.bytes_per_pixel();
        let sample_bytes = format.sample.bytes();
        let alpha_begin = bpp.checked_sub(sample_bytes).ok_or(BackingError::Length)?;
        if bpp > 40 || sample_bytes == 0 || alpha_begin == 0 || rect.is_empty() || !backed.operative_bounds().contains_rect(&rect) {
            return Err(BackingError::Bounds);
        }
        let width = usize::try_from(rect.width()).map_err(|_| BackingError::Length)?;
        let length = width
            .checked_mul(usize::try_from(rect.height()).map_err(|_| BackingError::Length)?)
            .and_then(|n| n.checked_mul(bpp))
            .ok_or(BackingError::Length)?;
        if bytes.len() != length {
            return Err(BackingError::Length);
        }
        let view = backed.checked_view();
        let mut raw = [0u8; 40];
        for tile in rect.tiles() {
            c.check(1)?;
            let part = tile.rect().intersect(&rect);
            let pin = view.pin_tile(tile, c).map_err(view_error)?;
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let pixel = raw.get_mut(..bpp).ok_or(BackingError::Length)?;
                    pin.read_pixel_native_into(x, y, pixel, c).map_err(view_error)?;
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackingError::Bounds)?;
                    let column = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackingError::Bounds)?;
                    let begin = row
                        .checked_mul(width)
                        .and_then(|n| n.checked_add(column))
                        .and_then(|n| n.checked_mul(bpp))
                        .and_then(|n| n.checked_add(alpha_begin))
                        .ok_or(BackingError::Length)?;
                    let end = begin.checked_add(sample_bytes).ok_or(BackingError::Length)?;
                    c.check(sample_bytes as u64)?;
                    if bytes.get(begin..end).ok_or(BackingError::Length)? != pixel.get(alpha_begin..bpp).ok_or(BackingError::Length)? {
                        return Err(BackingError::Integrity);
                    }
                }
            }
            // The next tile never overlaps this pin's lifetime.
            drop(pin);
            c.check(1)?;
        }
        c.check(1)?;
        Ok(Self { document, layer, rect, bytes, _lease: lease })
    }

    pub(super) fn verify_stored_programme(document: &'a AccountedDocument, edit: AccountedEdit<'a>, c: &ReadControl<'_>) -> Result<Self, BackingError> {
        c.check(1)?;
        let AccountedEdit::NativePixels { layer, rect, bytes } = edit else { return Err(BackingError::Unsupported) };
        // Price the fixed proof/view/native-pixel scratch before validation or
        // pinning. Tile/cache storage has the actual backing's own same-q fees.
        let fixed = std::mem::size_of::<Self>()
            .checked_add(std::mem::size_of::<CheckedRasterView<'_>>())
            .and_then(|n| n.checked_add(40 + 256))
            .ok_or(BackingError::Length)?;
        let lease = document.quota().reserve(QuotaKind::Resident, fixed as u64).map_err(|_| BackingError::Quota)?;
        document.validate_programme_preserving_native_edit(edit, c)?;
        let access = document.accounted_layer_access(layer, c)?.ok_or(BackingError::Bounds)?;
        // These locks never receive an exception, even if the proposed bytes
        // happen to equal the original native region.
        if access.locks.all || access.locks.pixels || access.imported_locks.content {
            return Err(BackingError::Unsupported);
        }
        let LayerContent::BackedRaster(backed) = &access.layer.content else { return Err(BackingError::Unsupported) };
        let format = backed.format();
        if !format.alpha
            || !matches!(format.mode, photocraft_doc::ColorMode::Rgb | photocraft_doc::ColorMode::Grayscale)
            || !matches!(
                format.sample,
                photocraft_doc::SampleType::U8
                    | photocraft_doc::SampleType::U16
                    | photocraft_doc::SampleType::U32
                    | photocraft_doc::SampleType::F32
                    | photocraft_doc::SampleType::F64
            )
            || !backed.quota().shares_ledger(document.quota())
        {
            return Err(BackingError::Unsupported);
        }
        let bpp = format.bytes_per_pixel();
        let sample_bytes = format.sample.bytes();
        let alpha_begin = bpp.checked_sub(sample_bytes).ok_or(BackingError::Length)?;
        if bpp > 40 || sample_bytes == 0 || alpha_begin == 0 || rect.is_empty() || !backed.operative_bounds().contains_rect(&rect) {
            return Err(BackingError::Bounds);
        }
        let width = usize::try_from(rect.width()).map_err(|_| BackingError::Length)?;
        let length = width
            .checked_mul(usize::try_from(rect.height()).map_err(|_| BackingError::Length)?)
            .and_then(|n| n.checked_mul(bpp))
            .ok_or(BackingError::Length)?;
        if bytes.len() != length {
            return Err(BackingError::Length);
        }
        let view = backed.checked_view();
        let mut raw = [0u8; 40];
        for tile in rect.tiles() {
            c.check(1)?;
            let part = tile.rect().intersect(&rect);
            let pin = view.pin_tile(tile, c).map_err(view_error)?;
            for y in part.y0..part.y1 {
                for x in part.x0..part.x1 {
                    let pixel = raw.get_mut(..bpp).ok_or(BackingError::Length)?;
                    pin.read_pixel_native_into(x, y, pixel, c).map_err(view_error)?;
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackingError::Bounds)?;
                    let column = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackingError::Bounds)?;
                    let begin = row
                        .checked_mul(width)
                        .and_then(|n| n.checked_add(column))
                        .and_then(|n| n.checked_mul(bpp))
                        .and_then(|n| n.checked_add(alpha_begin))
                        .ok_or(BackingError::Length)?;
                    let end = begin.checked_add(sample_bytes).ok_or(BackingError::Length)?;
                    c.check(sample_bytes as u64)?;
                    if bytes.get(begin..end).ok_or(BackingError::Length)? != pixel.get(alpha_begin..bpp).ok_or(BackingError::Length)? {
                        return Err(BackingError::Integrity);
                    }
                }
            }
            // The next tile never overlaps this pin's lifetime.
            drop(pin);
            c.check(1)?;
        }
        c.check(1)?;
        Ok(Self { document, layer, rect, bytes, _lease: lease })
    }

    pub(super) fn authorizes(&self, document: &AccountedDocument, edit: AccountedEdit<'_>) -> bool {
        let AccountedEdit::NativePixels { layer, rect, bytes } = edit else { return false };
        self.document.shares_model(document) && self.layer == layer && self.rect == rect && std::ptr::eq(self.bytes, bytes)
    }
}

#[cfg(test)]
#[path = "accounted_native_alpha_tests.rs"]
mod tests;
