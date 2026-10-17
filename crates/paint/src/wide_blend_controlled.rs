//! Explicit unit-domain native Multiply/Screen painting in binary64.
//! Caller establishes matching working encoding/profile. No CMS/TRC conversion,
//! imported-operation mapping, HDR blend claim, or exact-rational arithmetic.
use crate::render::StrokeLimits;
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, RasterInputError, Surface, checked_region_byte_len};
use std::sync::Arc;
type Result<T> = std::result::Result<T, RasterInputError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeUnitBlend {
    Multiply,
    Screen,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn unit(value: f64) -> Result<f64> {
    if value.is_finite() && (0.0..=1.0).contains(&value) { Ok(value) } else { Err(RasterInputError::InvalidSample) }
}
fn tiles(rect: Rect) -> Result<u64> {
    if rect.is_empty() {
        return Ok(0);
    }
    let x = rect.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.div_euclid(TILE_SIZE);
    let y = rect.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.div_euclid(TILE_SIZE);
    let w = u64::try_from(i64::from(x) - i64::from(rect.x0.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let h = u64::try_from(i64::from(y) - i64::from(rect.y0.div_euclid(TILE_SIZE)) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    mul(w, h)
}
fn retained(surface: &Surface, limit: u64, control: &Interrupt<'_>) -> Result<u64> {
    poll(control)?;
    let format = surface.format();
    let expected =
        (TILE_SIZE as usize).checked_mul(TILE_SIZE as usize).and_then(|n| n.checked_mul(format.bytes_per_pixel())).ok_or(RasterInputError::LimitExceeded)?;
    let mut bytes = add(surface.default_bytes().len() as u64, mul(surface.tile_count() as u64, 256)?)?;
    if bytes > limit {
        return Err(RasterInputError::LimitExceeded);
    }
    surface.try_validate_tile_origins_with(control)?;
    if surface.default_bytes().len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    for (_, tile) in surface.tiles() {
        poll(control)?;
        if tile.bytes().len() != expected {
            return Err(RasterInputError::LengthMismatch);
        }
        bytes = add(bytes, tile.bytes().len() as u64)?;
        if bytes > limit {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    Ok(bytes)
}
/// Coverage/opacity/selection modulate source alpha once, after existing stroke
/// coverage union. Selection is declared Gray coverage without alpha, at any
/// native sample depth. Colors are native straight finite-unit values. Locked alpha
/// retains original encoded bits; zero-weight/source-alpha/locked-zero-alpha
/// skips leave every destination byte untouched before color interpretation.
#[allow(clippy::too_many_arguments)]
pub fn try_apply_unit_blend_coverage(
    target: &mut Surface,
    paint: &Surface,
    rect: Rect,
    coverage: &[f64],
    opacity: f64,
    selection: Option<&Surface>,
    lock_transparency: bool,
    mode: NativeUnitBlend,
    limits: StrokeLimits,
    control: &Interrupt<'_>,
) -> Result<()> {
    poll(control)?;
    let standard = StrokeLimits::default();
    let format = target.format();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if !matches!(format.sample, SampleType::U32 | SampleType::F64)
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || format.mode != paint.format().mode
        || format.alpha != paint.format().alpha
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    unit(opacity)?;
    let bpp = format.bytes_per_pixel();
    let bytes = checked_region_byte_len(rect, format)?;
    let count = bytes / bpp;
    if coverage.len() != count {
        return Err(RasterInputError::LengthMismatch);
    }
    if selection.is_some_and(|s| s.format().mode != ColorMode::Grayscale || s.format().alpha) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let output_tiles = tiles(rect)?;
    let source_tiles = add(add(target.tile_count() as u64, paint.tile_count() as u64)?, selection.map_or(0, |s| s.tile_count() as u64))?;
    let coverage_bytes = mul(count as u64, 8)?;
    let native_tile = mul((TILE_SIZE * TILE_SIZE) as u64, bpp as u64)?;
    let scratch = mul(native_tile, 4)?;
    let output = add(mul(output_tiles, add(native_tile, 512)?)?, add(mul(target.tile_count() as u64, 512)?, target.default_bytes().len() as u64)?)?;
    let work = add(mul(count as u64, 256)?, mul(mul(output_tiles, add(source_tiles, output_tiles)?)?, 256)?)?;
    if add(coverage_bytes, add(output, scratch)?)? > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut any = false;
    for chunk in coverage.chunks(256) {
        poll(control)?;
        for value in chunk {
            unit(*value)?;
            any |= *value != 0.0;
        }
    }
    if count == 0 || opacity == 0.0 || !any {
        return Ok(());
    }
    let mut held = coverage_bytes;
    for surface in [Some(&*target), Some(paint), selection].into_iter().flatten() {
        held = add(held, retained(surface, limits.resident_bytes, control)?)?;
        if held > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    if add(held, add(output, scratch)?)? > limits.resident_bytes {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut staged = Surface::try_with_default_bytes(format, target.default_bytes())?;
    // Standard BTree nodes are prepaid; immutable tile payload ownership stays
    // shared until a checked native write replaces the affected private tile.
    for (position, tile) in target.tiles() {
        poll(control)?;
        staged.put_tiles(std::iter::once((*position, Arc::clone(tile))));
    }
    let width = usize::try_from(rect.width()).map_err(|_| RasterInputError::InvalidBounds)?;
    let channels = format.channels();
    let color_channels = format.mode.color_channels();
    for tile in rect.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&rect);
        let part_width = usize::try_from(part.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        if part_width == 0 {
            return Err(RasterInputError::InvalidBounds);
        }
        let mut raw = target.try_read_interleaved_with(part, control)?;
        if add(held, add(output, add(raw.capacity() as u64, mul(native_tile, 3)?)?)?)? > limits.resident_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut changed = false;
        for (index, pixel) in raw.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(part_width) || index.is_multiple_of(256) {
                poll(control)?;
            }
            let x =
                part.x0.checked_add(i32::try_from(index % part_width).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
            let y =
                part.y0.checked_add(i32::try_from(index / part_width).map_err(|_| RasterInputError::InvalidBounds)?).ok_or(RasterInputError::InvalidBounds)?;
            let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let column = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
            let coverage_index = row.checked_mul(width).and_then(|v| v.checked_add(column)).ok_or(RasterInputError::LimitExceeded)?;
            let mut weight = *coverage.get(coverage_index).ok_or(RasterInputError::LengthMismatch)? * opacity;
            if weight == 0.0 {
                continue;
            }
            if let Some(mask) = selection {
                let mut values = [0.0; 5];
                mask.try_read_pixel_f64(x, y, values.get_mut(..mask.channels()).ok_or(RasterInputError::LengthMismatch)?)?;
                weight *= unit(*values.first().ok_or(RasterInputError::LengthMismatch)?)?;
            }
            if weight == 0.0 {
                continue;
            }
            unit(weight)?;
            let da_raw =
                if format.alpha { try_read_sample_f64(pixel, format.sample, color_channels).map_err(|_| RasterInputError::InvalidSample)? } else { 1.0 };
            if lock_transparency && format.alpha && da_raw == 0.0 {
                continue;
            }
            let mut source = [0.0; 5];
            paint.try_read_pixel_f64(x, y, source.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?)?;
            let source_alpha = if format.alpha { unit(*source.get(color_channels).ok_or(RasterInputError::LengthMismatch)?)? } else { 1.0 };
            if source_alpha == 0.0 {
                continue;
            }
            let da = unit(da_raw)?;
            let sa = unit(source_alpha * weight)?;
            if sa == 0.0 {
                continue;
            }
            let oa = unit(sa + da * (1.0 - sa))?;
            let mut original = [0u8; 32];
            original.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
            for channel in 0..color_channels {
                let s = unit(*source.get(channel).ok_or(RasterInputError::LengthMismatch)?)?;
                let value = if da == 0.0 {
                    s
                } else {
                    let d = unit(try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?)?;
                    let blend = match mode {
                        NativeUnitBlend::Multiply => d * s,
                        NativeUnitBlend::Screen => d + s - d * s,
                    };
                    // Unfused binary64, in the upstream general paint equation's
                    // term order. One final native sample quantization follows.
                    ((1.0 - sa) * da * d + (1.0 - da) * sa * s + sa * da * blend) / oa
                };
                unit(value)?;
                try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
            }
            if format.alpha && !lock_transparency {
                try_write_sample_f64(pixel, format.sample, color_channels, oa).map_err(|_| RasterInputError::InvalidSample)?;
            }
            changed |= pixel != original.get(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        }
        if changed {
            staged.try_write_interleaved_with(part, &raw, control)?;
        }
    }
    poll(control)?;
    *target = staged;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    fn surface(format: PixelFormat, values: &[f64]) -> Surface {
        let mut raw = vec![0; format.bytes_per_pixel()];
        for (channel, value) in values.iter().enumerate() {
            try_write_sample_f64(&mut raw, format.sample, channel, *value).unwrap();
        }
        Surface::try_with_default_bytes(format, &raw).unwrap()
    }
    fn apply(target: &mut Surface, paint: &Surface, coverage: f64, lock: bool, mode: NativeUnitBlend) -> Result<()> {
        try_apply_unit_blend_coverage(target, paint, Rect::new(0, 0, 1, 1), &[coverage], 1.0, None, lock, mode, StrokeLimits::default(), &Interrupt::NONE)
    }
    #[test]
    fn native_low_bits_and_adjacent_u32_codes_survive_blend() {
        for sample in [SampleType::U32, SampleType::F64] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let d = if sample == SampleType::U32 { f64::from(0x8000_0001u32) / f64::from(u32::MAX) } else { f64::from_bits(0x3fe0_0000_0000_0001) };
            for mode in [NativeUnitBlend::Multiply, NativeUnitBlend::Screen] {
                // Exact endpoint identity exercises the final native writer;
                // adjacent integer codes and binary64 low bits cannot use f32.
                let s = if mode == NativeUnitBlend::Multiply { 1.0 } else { 0.0 };
                let mut target = surface(format, &[d]);
                let before = target.default_bytes().to_vec();
                let paint = surface(PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), &[s]);
                apply(&mut target, &paint, 1.0, false, mode).unwrap();
                assert_eq!(target.try_read_interleaved_with(Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap(), before);
            }
        }
    }
    #[test]
    fn straight_alpha_blends_and_locked_alpha_bits() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let paint = surface(format, &[0.5, 0.5]);
        for (mode, expected) in [(NativeUnitBlend::Multiply, 0.275_f64), (NativeUnitBlend::Screen, 0.375_f64)] {
            let mut target = surface(format, &[0.25, 0.5]);
            apply(&mut target, &paint, 0.5, true, mode).unwrap();
            let mut pixel = [0.0; 2];
            target.try_read_pixel_f64(0, 0, &mut pixel).unwrap();
            // sa=.25, da=.5, oa=.625. Alpha lock changes only publication
            // of alpha, not the upstream general color numerator.
            assert_eq!(pixel[0].to_bits(), expected.to_bits());
            assert_eq!(pixel[1].to_bits(), 0.5f64.to_bits());
        }
    }
    #[test]
    fn skipped_samples_keep_nan_and_signed_zero_raw() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let raw = [f64::from_bits(0x7ff8_1234_5678_9abc), -0.0].into_iter().flat_map(f64::to_le_bytes).collect::<Vec<_>>();
        for (coverage, lock, source_alpha) in [(0.0, false, 1.0), (1.0, true, 1.0), (1.0, false, 0.0)] {
            let mut target = Surface::try_with_default_bytes(format, &raw).unwrap();
            let before = target.clone();
            let paint = surface(format, &[0.5, source_alpha]);
            apply(&mut target, &paint, coverage, lock, NativeUnitBlend::Multiply).unwrap();
            assert_eq!(target, before);
        }
    }
    #[test]
    fn touched_hdr_cancel_and_tiny_budget_refuse_atomically() {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let paint = surface(format, &[0.5, 0.5, 0.5, 1.0]);
        let mut target = surface(format, &[0.25, 2.0, 0.75, 1.0]);
        let before = target.clone();
        assert!(matches!(apply(&mut target, &paint, 1.0, false, NativeUnitBlend::Screen), Err(RasterInputError::InvalidSample)));
        assert_eq!(target, before);
        let stop = || true;
        assert!(matches!(
            try_apply_unit_blend_coverage(
                &mut target,
                &paint,
                Rect::new(0, 0, 1, 1),
                &[1.0],
                1.0,
                None,
                false,
                NativeUnitBlend::Multiply,
                StrokeLimits::default(),
                &Interrupt::cancel_only(&stop)
            ),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(target, before);
        assert!(
            try_apply_unit_blend_coverage(
                &mut target,
                &paint,
                Rect::new(0, 0, 1, 1),
                &[1.0],
                1.0,
                None,
                false,
                NativeUnitBlend::Multiply,
                StrokeLimits { resident_bytes: 1, ..Default::default() },
                &Interrupt::NONE
            )
            .is_err()
        );
        assert_eq!(target, before);
        let mut valid = surface(format, &[0.25, 0.5, 0.75, 1.0]);
        let original = valid.clone();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop_late = || {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            n > 12
        };
        let coverage = vec![1.0; 512];
        assert!(matches!(
            try_apply_unit_blend_coverage(
                &mut valid,
                &paint,
                Rect::new(0, 0, 512, 1),
                &coverage,
                1.0,
                None,
                false,
                NativeUnitBlend::Multiply,
                StrokeLimits::default(),
                &Interrupt::cancel_only(&stop_late)
            ),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(valid, original);
    }
    #[test]
    fn selection_requires_gray_coverage_without_alpha_before_copy() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let paint = surface(format, &[0.5, 1.0]);
        for mask_format in [PixelFormat::RGBA32F, PixelFormat::CMYKA8, PixelFormat::GRAYA8] {
            let mask = Surface::new(mask_format);
            let mut target = surface(format, &[0.25, 1.0]);
            let before = target.clone();
            assert!(matches!(
                try_apply_unit_blend_coverage(
                    &mut target,
                    &paint,
                    Rect::new(0, 0, 1, 1),
                    &[1.0],
                    1.0,
                    Some(&mask),
                    false,
                    NativeUnitBlend::Multiply,
                    StrokeLimits::default(),
                    &Interrupt::NONE
                ),
                Err(RasterInputError::UnsupportedConversion)
            ));
            assert_eq!(target, before);
        }
        for sample in SampleType::ALL {
            let mask = surface(PixelFormat::new(ColorMode::Grayscale, sample, false), &[1.0]);
            let mut target = surface(format, &[0.25, 1.0]);
            try_apply_unit_blend_coverage(
                &mut target,
                &paint,
                Rect::new(0, 0, 1, 1),
                &[1.0],
                1.0,
                Some(&mask),
                false,
                NativeUnitBlend::Multiply,
                StrokeLimits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            let mut actual = [0.0; 2];
            target.try_read_pixel_f64(0, 0, &mut actual).unwrap();
            assert_eq!(actual, [0.125, 1.0]);
        }
    }
}
