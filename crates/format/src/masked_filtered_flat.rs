//! Closed native selected-mask metadata admission; no source/render authority.
use crate::{FormatError, Result, manifest::DocM};
use photocraft_raster::backing::ReadControl;
pub(crate) fn preflight(m: &DocM, c: &ReadControl<'_>) -> Result<()> {
    c.check(1).map_err(crate::owned_document::error)?;
    if m.xcf_filter_masks.len() > 4096 {
        return Err(FormatError::LimitExceeded("selected masks".into()));
    }
    let mut total = 0u64;
    for mask in &m.xcf_filter_masks {
        c.check(1).map_err(crate::owned_document::error)?;
        let s = &mask.surface;
        let f = s.format;
        if f.mode != photocraft_color::ColorMode::Grayscale
            || f.alpha
            || !matches!(f.sample, photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
            || s.default.len() != f.bytes_per_pixel().checked_mul(2).ok_or_else(|| FormatError::LimitExceeded("mask default".into()))?
        {
            return Err(FormatError::Unsupported("selected mask format".into()));
        }
        let bytes = (s.tiles.len() as u64)
            .checked_mul(256 * 256)
            .and_then(|n| n.checked_mul(f.bytes_per_pixel() as u64))
            .and_then(|n| n.checked_add(f.bytes_per_pixel() as u64))
            .ok_or_else(|| FormatError::LimitExceeded("selected mask bytes".into()))?;
        if bytes > 16 << 20 {
            return Err(FormatError::LimitExceeded("selected mask bytes".into()));
        }
        total = total.checked_add(bytes).ok_or_else(|| FormatError::LimitExceeded("selected mask aggregate".into()))?;
        if total > 64 << 20 {
            return Err(FormatError::LimitExceeded("selected mask aggregate".into()));
        }
    }
    Ok(())
}
