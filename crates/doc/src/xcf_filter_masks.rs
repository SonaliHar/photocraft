//! Current editable filter masks, separate from layer masks and source caches.
use crate::{
    Rect,
    item_identity::{FilterId, MaskId},
    xcf_state::XcfValidationError,
};
use photocraft_color::{ColorMode, try_read_sample_f64};
use photocraft_raster::{Interrupt, Surface};
#[path = "xcf_filter_mask_coverage_set.rs"]
mod coverage_set;
pub(crate) use coverage_set::CoverageSetProof;
pub use coverage_set::{MaskCoverageSetLease, CheckedFilterMaskCoverageSet, FilterMaskCoverageClass, MaskCoverageSetLimits, MaskCoverageSetRequirements};

#[derive(Clone, Debug, PartialEq)]
pub struct FilterMask {
    pub id: MaskId,
    pub owner: FilterId,
    pub surface: Surface,
    pub bounds: Rect,
}
// Surface equality compares encoded bytes, including floating bit patterns.
impl Eq for FilterMask {}

fn invalid(code: &'static str) -> XcfValidationError {
    XcfValidationError::new(code, "invalid filter mask or interrupted validation")
}

/// A completed controlled scan bound to this exact immutable mask. This is
/// structural validation only, never operation, source or storage authority.
pub struct CheckedFilterMaskCoverage<'a> {
    mask: &'a FilterMask,
    raw_nonempty: bool,
    logical_all_byte_zero: bool,
}
impl CheckedFilterMaskCoverage<'_> {
    pub fn raw_nonempty(&self) -> bool {
        self.raw_nonempty
    }
    pub fn logical_all_byte_zero(&self) -> bool {
        self.logical_all_byte_zero
    }
    pub(crate) fn covers(&self, mask: &FilterMask) -> bool {
        std::ptr::eq(self.mask, mask)
    }
}

impl FilterMask {
    pub fn try_new(id: MaskId, owner: FilterId, surface: Surface, bounds: Rect) -> Result<Self, XcfValidationError> {
        let mask = Self { id, owner, surface, bounds };
        mask.validate()?;
        Ok(mask)
    }

    /// Validate the closed checked-Float coverage domain without altering raw
    /// words. Defaults and physical padding are byte zero. Logical nonempty is
    /// measured before Float conversion, including positive tiny F64 values.
    pub fn validate_checked_f32_coverage_with(
        &self,
        c: &photocraft_raster::backing::ReadControl<'_>,
    ) -> Result<CheckedFilterMaskCoverage<'_>, photocraft_raster::backing::BackingError> {
        self.scan_checked_f32_coverage(&|units| c.check(units))
    }
    /// Synchronous job-local equivalent for an existing non-Sync source budget.
    /// The callback never escapes this call. It changes no mask/source authority.
    pub fn validate_checked_f32_coverage_with_local_work(
        &self,
        interrupt: &Interrupt<'_>,
        work: &dyn Fn(u64) -> Result<(), photocraft_raster::backing::BackingError>,
    ) -> Result<CheckedFilterMaskCoverage<'_>, photocraft_raster::backing::BackingError> {
        self.scan_checked_f32_coverage(&|units| {
            interrupt.check().map_err(|_| photocraft_raster::backing::BackingError::Cancelled)?;
            work(units)?;
            interrupt.check().map_err(|_| photocraft_raster::backing::BackingError::Cancelled)
        })
    }
    pub(super) fn scan_checked_f32_coverage(
        &self,
        check: &dyn Fn(u64) -> Result<(), photocraft_raster::backing::BackingError>,
    ) -> Result<CheckedFilterMaskCoverage<'_>, photocraft_raster::backing::BackingError> {
        use photocraft_raster::backing::BackingError as E;
        check(1)?;
        let f = self.surface.format();
        let w = i64::from(self.bounds.x1) - i64::from(self.bounds.x0);
        let h = i64::from(self.bounds.y1) - i64::from(self.bounds.y0);
        if self.id.0 == 0
            || self.owner.0 == 0
            || self.bounds.x0 != 0
            || self.bounds.y0 != 0
            || w <= 0
            || h <= 0
            || w > 262144
            || h > 262144
            || w.checked_mul(h).ok_or(E::Length)? > 1 << 30
            || f.mode != ColorMode::Grayscale
            || f.alpha
            || !matches!(f.sample, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
        {
            return Err(E::Unsupported);
        }
        let sample = f.sample.bytes();
        let side = u64::try_from(photocraft_geom::TILE_SIZE).map_err(|_| E::Length)?;
        let tile_bytes = side.checked_mul(side).and_then(|n| n.checked_mul(sample as u64)).ok_or(E::Length)?;
        let count = self.surface.tile_count() as u64;
        let bytes = count.checked_mul(tile_bytes).and_then(|n| n.checked_add(sample as u64)).ok_or(E::Length)?;
        let columns = (w as u64).checked_add(side - 1).ok_or(E::Length)? / side;
        let rows = (h as u64).checked_add(side - 1).ok_or(E::Length)? / side;
        if bytes > 16 << 20 || count > columns.checked_mul(rows).ok_or(E::Length)? {
            return Err(E::Length);
        }
        // Pay byte traversal, fixed bookkeeping and conservative per-sample
        // coordinate/decode/cast/finite checks before walking any map/payload.
        // Chunk debits retain the original caller ledger and cancellation token.
        let samples = bytes.checked_div(sample as u64).ok_or(E::Length)?;
        let mut work = bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(samples.checked_mul(96)?))
            .and_then(|n| n.checked_add(count.checked_mul(16)?))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>() as u64))
            .and_then(|n| n.checked_add(std::mem::size_of::<CheckedFilterMaskCoverage<'_>>() as u64))
            .ok_or(E::Length)?;
        while work != 0 {
            let debit = work.min(65536);
            check(debit)?;
            work = work.checked_sub(debit).ok_or(E::Length)?;
        }
        if self.surface.default_bytes().len() != sample || self.surface.default_bytes().iter().any(|b| *b != 0) {
            return Err(E::Integrity);
        }
        let mut nonempty = false;
        let mut byte_zero = true;
        for (coord, tile) in self.surface.tiles() {
            check(0)?;
            let tx = u64::try_from(coord.tx).map_err(|_| E::Bounds)?;
            let ty = u64::try_from(coord.ty).map_err(|_| E::Bounds)?;
            if tx >= columns || ty >= rows || tile.bytes().len() as u64 != tile_bytes {
                return Err(E::Bounds);
            }
            let x0 = tx.checked_mul(side).ok_or(E::Bounds)?;
            let y0 = ty.checked_mul(side).ok_or(E::Bounds)?;
            for (i, word) in tile.bytes().chunks_exact(sample).enumerate() {
                if i % 256 == 0 {
                    check(0)?;
                }
                let i = i as u64;
                let x = x0.checked_add(i % side).ok_or(E::Bounds)?;
                let y = y0.checked_add(i / side).ok_or(E::Bounds)?;
                if x >= w as u64 || y >= h as u64 {
                    if word.iter().any(|b| *b != 0) {
                        return Err(E::Integrity);
                    }
                    continue;
                }
                let raw = try_read_sample_f64(word, f.sample, 0).map_err(|_| E::Integrity)?;
                let value = raw as f32;
                if !raw.is_finite() || raw < 0.0 || !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(E::Bounds);
                }
                nonempty |= raw != 0.0;
                byte_zero &= word.iter().all(|b| *b == 0);
            }
        }
        check(0)?;
        Ok(CheckedFilterMaskCoverage { mask: self, raw_nonempty: nonempty, logical_all_byte_zero: byte_zero })
    }

    pub fn validate(&self) -> Result<(), XcfValidationError> {
        self.validate_with(&Interrupt::NONE)
    }

    /// Validate all owned samples and defaults without narrowing or making a copy.
    pub fn validate_with(&self, interrupt: &Interrupt<'_>) -> Result<(), XcfValidationError> {
        self.scan(interrupt).map(|_| ())
    }

    /// This is only a pixel-state observation; the caller must qualify how its
    /// source operation interprets an empty selection mask.
    pub fn is_empty_selection_with(&self, interrupt: &Interrupt<'_>) -> Result<bool, XcfValidationError> {
        self.scan(interrupt)
    }

    fn scan(&self, interrupt: &Interrupt<'_>) -> Result<bool, XcfValidationError> {
        interrupt.check().map_err(|_| invalid("cancelled"))?;
        let width = i64::from(self.bounds.x1) - i64::from(self.bounds.x0);
        let height = i64::from(self.bounds.y1) - i64::from(self.bounds.y0);
        let format = self.surface.format();
        if self.id.0 == 0
            || self.owner.0 == 0
            || width <= 0
            || height <= 0
            || width > 262144
            || height > 262144
            || [self.bounds.x0, self.bounds.y0, self.bounds.x1, self.bounds.y1].iter().any(|v| !(-(1 << 29)..=(1 << 29)).contains(v))
            || format.mode != ColorMode::Grayscale
            || format.alpha
        {
            return Err(invalid("filter_mask_bounds"));
        }
        self.surface.resident_payload_bytes().map_err(|_| invalid("filter_mask_limit"))?;
        let mut empty = true;
        let mut scan = |bytes: &[u8]| -> Result<(), XcfValidationError> {
            if !bytes.len().is_multiple_of(format.sample.bytes()) {
                return Err(invalid("filter_mask_samples"));
            }
            for (index, sample) in bytes.chunks_exact(format.sample.bytes()).enumerate() {
                if index % 256 == 0 {
                    interrupt.check().map_err(|_| invalid("cancelled"))?;
                }
                let value = try_read_sample_f64(sample, format.sample, 0).map_err(|_| invalid("filter_mask_samples"))?;
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(invalid("filter_mask_samples"));
                }
                empty &= value == 0.0;
            }
            Ok(())
        };
        scan(self.surface.default_bytes())?;
        for (_, tile) in self.surface.tiles() {
            scan(tile.bytes())?;
        }
        interrupt.check().map_err(|_| invalid("cancelled"))?;
        Ok(empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PixelFormat, SampleType};
    #[test]
    fn exact_default_and_low_bits_are_current_state() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut surface = Surface::try_with_default_bytes(format, &(-0.0_f64).to_ne_bytes()).unwrap();
        let empty = FilterMask::try_new(MaskId(1), FilterId(2), surface.clone(), Rect::new(0, 0, 2, 1)).unwrap();
        assert!(empty.is_empty_selection_with(&Interrupt::NONE).unwrap());
        assert_eq!(empty.surface.default_bytes(), &(-0.0_f64).to_ne_bytes());
        let value = 0.5 + 2.0_f64.powi(-40);
        surface.try_write_pixel_f64(0, 0, &[value]).unwrap();
        let mask = FilterMask::try_new(MaskId(1), FilterId(2), surface, Rect::new(0, 0, 2, 1)).unwrap();
        assert!(!mask.is_empty_selection_with(&Interrupt::NONE).unwrap());
        let mut actual = [0.0];
        mask.surface.try_read_pixel_f64(0, 0, &mut actual).unwrap();
        assert_eq!(actual[0].to_bits(), value.to_bits());
    }
    #[test]
    fn invalid_bounds_and_nonfinite_defaults_fail_without_mutation() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            let surface = Surface::try_with_default_bytes(format, &value.to_ne_bytes()).unwrap();
            assert!(FilterMask::try_new(MaskId(1), FilterId(2), surface, Rect::new(0, 0, 1, 1)).is_err());
        }
        let surface = Surface::new(format);
        assert!(FilterMask::try_new(MaskId(1), FilterId(2), surface, Rect::new(0, 0, 0, 1)).is_err());
    }

    #[test]
    fn cancelled_scan_keeps_all_current_sample_bits() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let surface = Surface::try_with_default_bytes(format, &1_u32.to_ne_bytes()).unwrap();
        let mask = FilterMask { id: MaskId(1), owner: FilterId(2), surface, bounds: Rect::new(0, 0, 1, 1) };
        let before = mask.clone();
        let cancel = || true;
        assert_eq!(mask.validate_with(&Interrupt::cancel_only(&cancel)).unwrap_err().code, "cancelled");
        assert_eq!(mask, before);
    }
}

#[cfg(test)]
#[path = "xcf_filter_mask_coverage_tests.rs"]
mod checked_coverage_tests;
