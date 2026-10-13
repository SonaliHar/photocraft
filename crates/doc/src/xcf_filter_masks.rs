//! Current editable filter masks, separate from layer masks and source caches.
use crate::{
    Rect,
    item_identity::{FilterId, MaskId},
    xcf_state::XcfValidationError,
};
use photocraft_color::{ColorMode, try_read_sample_f64};
use photocraft_raster::{Interrupt, Surface};

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

impl FilterMask {
    pub fn try_new(id: MaskId, owner: FilterId, surface: Surface, bounds: Rect) -> Result<Self, XcfValidationError> {
        let mask = Self { id, owner, surface, bounds };
        mask.validate()?;
        Ok(mask)
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
