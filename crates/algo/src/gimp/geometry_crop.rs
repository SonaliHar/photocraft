//! Exact raw crop/rebase infrastructure, not a qualified `gegl:crop` evaluator.
//! The retained stock assignment has no selected crop schema; no reset-origin,
//! fractional rounding, profile, alpha or serialized-version policy is inferred.
use super::roi::{Abyss, Halo, RoiPlan, rect_from_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropOrigin {
    KeepCoordinates,
    RebaseToZero,
}
#[derive(Clone, Copy, Debug)]
pub struct CropLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
/// The surface retains the exact source encoded default. Consumers must honor
/// extent; an infinite Surface's outside-default is not the cropped image extent.
pub struct CroppedRaster {
    pub surface: Surface,
    pub extent: Rect,
}

/// Crop wholly inside an explicit caller source extent, with no conversion.
/// Integral finite half-open edges only; partial/external crops refuse instead
/// of inventing an abyss/default composition policy. Raw NaN/HDR/low bits survive.
pub fn try_crop_raw(
    source: &Surface,
    source_extent: Rect,
    crop_edges: [f64; 4],
    origin: CropOrigin,
    limits: CropLimits,
    interrupt: &Interrupt<'_>,
) -> Result<CroppedRaster, RasterInputError> {
    poll(interrupt)?;
    let format = source.format();
    if crop_edges.iter().any(|edge| edge.is_finite() && edge.fract() != 0.0) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let crop = rect_from_f64(crop_edges, format)?;
    let plan = RoiPlan::new(crop, source_extent, Halo::default(), Abyss::None, format, interrupt)?;
    if crop.is_empty() || plan.source_read() != crop {
        return Err(RasterInputError::InvalidBounds);
    }
    let extent = match origin {
        CropOrigin::KeepCoordinates => crop,
        CropOrigin::RebaseToZero => {
            let dx = crop.x0.checked_neg().ok_or(RasterInputError::InvalidBounds)?;
            let dy = crop.y0.checked_neg().ok_or(RasterInputError::InvalidBounds)?;
            Rect::new(
                crop.x0.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?,
                crop.y0.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?,
                crop.x1.checked_add(dx).ok_or(RasterInputError::InvalidBounds)?,
                crop.y1.checked_add(dy).ok_or(RasterInputError::InvalidBounds)?,
            )
        }
    };
    let copied = checked_region_byte_len(extent, format)? as u64;
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    // Admission is constant-time: never enumerate a hostile ROI to learn its size.
    let output_tiles = tile_count(extent)?;
    let input_tiles = tile_count(crop)?;
    let source_tiles = source.tile_count() as u64;
    let work = copied
        .checked_mul(2)
        .and_then(|n| n.checked_add(output_tiles.checked_mul(tile_bytes)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
        .and_then(|n| n.checked_add(input_tiles))
        .ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units || limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    if source.default_bytes().len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    // Every valid tile has exactly this payload length; malformed lengths refuse
    // in the reserved traversal after peak admission, rather than changing its cost.
    let retained = source_tiles
        .checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(source.default_bytes().len() as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    let replacements = output_tiles.checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let peak = retained
        .checked_add(copied)
        .and_then(|n| n.checked_add(replacements))
        .and_then(|n| n.checked_add((source.default_bytes().len() as u64).checked_mul(2)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes {
        return Err(RasterInputError::LimitExceeded);
    }
    for (_, tile) in source.tiles() {
        poll(interrupt)?;
        if tile.bytes().len() as u64 != tile_bytes {
            return Err(RasterInputError::LengthMismatch);
        }
    }
    source.try_validate_tile_origins_with(interrupt)?;
    let mut result = Surface::try_with_default_bytes(format, source.default_bytes())?;
    // Preserve an all-default crop sparsely. No encoded default is decoded or
    // compared numerically (including signed zeros and NaN payloads).
    let mut populated = false;
    for coordinate in crop.tiles() {
        poll(interrupt)?;
        populated |= source.tile(coordinate).is_some();
    }
    if populated {
        // Existing checked raw-copy boundary uses fallible buffer/tile reserves
        // and private COW staging; one full bounded copy avoids quadratic map clones.
        result.try_copy_native_region_from(source, crop, extent, interrupt)?;
    }
    poll(interrupt)?;
    Ok(CroppedRaster { surface: result, extent })
}
fn tile_count(rect: Rect) -> Result<u64, RasterInputError> {
    if rect.is_empty() {
        return Ok(0);
    }
    let x_last = rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let y_last = rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let span = |first: i32, last: i32| {
        let count = i64::from(last.div_euclid(TILE_SIZE))
            .checked_sub(i64::from(first.div_euclid(TILE_SIZE)))
            .and_then(|n| n.checked_add(1))
            .ok_or(RasterInputError::LimitExceeded)?;
        u64::try_from(count).map_err(|_| RasterInputError::InvalidBounds)
    };
    span(rect.x0, x_last)?.checked_mul(span(rect.y0, y_last)?).ok_or(RasterInputError::LimitExceeded)
}
fn poll(interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    interrupt.check().map_err(|_| RasterInputError::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    fn limits() -> CropLimits {
        CropLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
    #[test]
    fn negative_rebase_retains_u32_words_and_source() {
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U32, alpha: false };
        let extent = Rect::new(-2, -1, 0, 0);
        let raw: Vec<u8> = [1u32, 0xffff_fffe].into_iter().flat_map(u32::to_ne_bytes).collect();
        let source = Surface::try_from_interleaved(format, extent, &raw).unwrap();
        let before = source.clone();
        let cropped = try_crop_raw(&source, extent, [-2., -1., 0., 0.], CropOrigin::RebaseToZero, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(cropped.extent, Rect::new(0, 0, 2, 1));
        assert_eq!(cropped.surface.try_read_interleaved(cropped.extent).unwrap(), raw);
        assert_eq!(source, before);
    }
    #[test]
    fn default_f64_nan_payload_stays_sparse_exact() {
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let raw = 0x7ff8_1234_5678_9abcu64.to_ne_bytes();
        let source = Surface::try_with_default_bytes(format, &raw).unwrap();
        let result = try_crop_raw(&source, Rect::new(-4, -4, 4, 4), [-1., -1., 1., 1.], CropOrigin::KeepCoordinates, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(result.surface.default_bytes(), raw);
        assert_eq!(result.surface.tile_count(), 0);
    }
    #[test]
    fn fractional_invalid_external_and_low_budget_refuse() {
        let source = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F64, alpha: false });
        let extent = Rect::new(0, 0, 3, 3);
        for crop in [[f64::NAN, 0., 1., 1.], [0.5, 0., 1., 1.], [-1., 0., 1., 1.], [2., 0., 1., 1.]] {
            assert!(try_crop_raw(&source, extent, crop, CropOrigin::KeepCoordinates, limits(), &Interrupt::NONE).is_err());
        }
        assert!(
            try_crop_raw(&source, extent, [0., 0., 1., 1.], CropOrigin::KeepCoordinates, CropLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE)
                .is_err()
        );
        assert_eq!(source.tile_count(), 0);
    }
    #[test]
    fn thin_low_work_refuses_before_scheduler_tile_visits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::SeqCst) >= 5;
        let interrupt = Interrupt::cancel_only(&cancel);
        let source = Surface::new(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false });
        let extent = Rect::new(-17, -131072, -16, 131072);
        let result = try_crop_raw(
            &source,
            extent,
            [-17., -131072., -16., 131072.],
            CropOrigin::KeepCoordinates,
            CropLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 0 },
            &interrupt,
        );
        // The old enumerating admission reached cancellation instead of the
        // already-known work refusal. No source or output allocation is needed.
        assert!(matches!(result, Err(RasterInputError::LimitExceeded)));
        assert_eq!(source.tile_count(), 0);
    }
    #[test]
    fn late_cancellation_never_changes_source_or_returns_output() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, Ordering::SeqCst) >= 7;
        let interrupt = Interrupt::cancel_only(&cancel);
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let source = Surface::try_from_interleaved(format, Rect::new(0, 0, 2, 1), &[1.0f64.to_ne_bytes(), (-0.0f64).to_ne_bytes()].concat()).unwrap();
        let before = source.clone();
        assert!(matches!(
            try_crop_raw(&source, Rect::new(0, 0, 2, 1), [0., 0., 2., 1.], CropOrigin::RebaseToZero, limits(), &interrupt),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(source, before);
    }
}
