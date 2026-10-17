//! Borrowed native Gray selection coverage for the bounded native producer.
//! This proves sample storage, not document installation or edit authority.
use super::*;

pub(super) struct CheckedNativeSelection<'s> {
    view: CheckedRasterView<'s>,
    bytes: u64,
    _fee: QuotaLease,
}
impl<'s> CheckedNativeSelection<'s> {
    pub(super) fn new(surface: &'s Surface, source: CheckedRasterView<'s>, q: &BackingQuota, cap: u64, c: &ReadControl<'_>) -> NResult<Self> {
        c.check(1)?;
        let format = surface.format();
        if format.mode != photocraft_color::ColorMode::Grayscale
            || format.alpha
            || !(format == PixelFormat::GRAY8 || format == PixelFormat::new(photocraft_color::ColorMode::Grayscale, source.format().sample, false))
            || !matches!(format.sample, SampleType::U8 | SampleType::U16 | SampleType::U32 | SampleType::F32 | SampleType::F64)
        {
            return invalid("native selection Gray8/exact-depth coverage required");
        }
        let tile_bytes = product(65536, format.bytes_per_pixel() as u64)?;
        let physical = sum(product(surface.tile_count() as u64, tile_bytes)?, surface.default_bytes().len() as u64)?;
        if physical > photocraft_raster::MAX_CHECKED_RASTER_BYTES {
            return Err(BackingError::Length.into());
        }
        let bytes = std::mem::size_of::<Self>() as u64 + 512;
        let fee = reserve(q, bytes, cap, c)?;
        // Constructor checks actual origins under the same control. It borrows
        // the installed map/default/tiles; no Surface or native-word clone.
        let view = CheckedRasterView::resident_with_control(surface, source.logical_bounds(), c)?;
        let check = |raw: &[u8]| -> NResult<()> {
            if !raw.len().is_multiple_of(format.bytes_per_pixel()) {
                return invalid("native selection sample length");
            }
            for block in raw.chunks(65536) {
                c.check(sum(block.len() as u64, product((block.len() / format.bytes_per_pixel()) as u64, 64)?)?)?;
                for word in block.chunks_exact(format.bytes_per_pixel()) {
                    let value =
                        photocraft_color::try_read_sample_f64(word, format.sample, 0).map_err(|_| NativeStrokeError::Invalid("native selection sample"))?;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return invalid("native selection finite unit coverage");
                    }
                }
            }
            Ok(())
        };
        check(surface.default_bytes())?;
        for (_, tile) in surface.tiles() {
            c.check(1)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return invalid("native selection tile layout");
            }
            check(tile.bytes())?;
        }
        c.check(0)?;
        Ok(Self { view, bytes, _fee: fee })
    }
    pub(super) fn retained_bytes(&self) -> u64 {
        self.bytes
    }
    pub(super) fn runtime_bytes(&self) -> u64 {
        std::mem::size_of::<photocraft_raster::checked_view::CheckedTilePin>() as u64 + 512
    }
    pub(super) fn wide(&self, x: i32, y: i32, c: &ReadControl<'_>) -> NResult<f64> {
        let mut value = [0.0];
        self.view.read_logical_pixel_f64_into(x, y, &mut value, c)?;
        c.check(64)?;
        let value = *value.first().ok_or(BackingError::Length)?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return invalid("native selection finite unit coverage");
        }
        Ok(value)
    }
    pub(super) fn narrow(&self, x: i32, y: i32, c: &ReadControl<'_>) -> NResult<f32> {
        if !matches!(self.view.format().sample, SampleType::U8 | SampleType::U16 | SampleType::F32) {
            return invalid("wide selection requires native wide producer");
        }
        // Match the upstream narrow decoder exactly, including U16 rounding.
        let mut raw = [0; 4];
        let word = raw.get_mut(..self.view.format().bytes_per_pixel()).ok_or(BackingError::Length)?;
        self.view.read_logical_pixel_native_into(x, y, word, c)?;
        c.check(64)?;
        let value = read_sample(word, self.view.format().sample, 0);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return invalid("native selection finite unit coverage");
        }
        Ok(value)
    }
}

#[cfg(test)]
#[path = "native_selection_tests.rs"]
mod tests;
