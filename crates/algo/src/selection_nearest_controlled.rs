//! Raw Gray native nearest homography transport. This is numeric storage
//! preservation, not interpolation, source-operation or coverage qualification.
use crate::{transform::Homography, wide_geometry::GeometryLimits};
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
type Result<T> = std::result::Result<T, RasterInputError>;
#[derive(Clone, Copy, Debug)]
pub struct NativeNearestRequirements {
    pub output_bounds: Rect,
    pub retained_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub work_units: u64,
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn cap(l: GeometryLimits) -> Result<()> {
    let standard = GeometryLimits::default();
    if l.resident_bytes > standard.resident_bytes || l.work_units > standard.work_units { Err(RasterInputError::LimitExceeded) } else { Ok(()) }
}
fn tile_count(r: Rect) -> Result<u64> {
    if r.is_empty() {
        return Ok(0);
    }
    let w = i64::from(r.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.div_euclid(TILE_SIZE)) - i64::from(r.x0.div_euclid(TILE_SIZE)) + 1;
    let h = i64::from(r.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.div_euclid(TILE_SIZE)) - i64::from(r.y0.div_euclid(TILE_SIZE)) + 1;
    mul(u64::try_from(w).map_err(|_| RasterInputError::InvalidBounds)?, u64::try_from(h).map_err(|_| RasterInputError::InvalidBounds)?)
}
fn source_charge(source: &Surface, l: GeometryLimits, c: &Interrupt<'_>) -> Result<(u64, u64)> {
    poll(c)?;
    cap(l)?;
    let fmt = source.format();
    if fmt.mode != ColorMode::Grayscale || fmt.alpha || !matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if source.tile_count() > 100_000 {
        return Err(RasterInputError::LimitExceeded);
    }
    if source.default_bytes().len() != fmt.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    let tile = mul((TILE_SIZE * TILE_SIZE) as u64, fmt.bytes_per_pixel() as u64)?;
    let retained = add(source.default_bytes().len() as u64, mul(source.tile_count() as u64, add(tile, 512)?)?)?;
    // Includes repeated bounded raw occupancy scans at engine capture/result
    // boundaries plus origin/length validation; never decodes sample values.
    let work = mul(mul(source.tile_count() as u64, tile)?, 8)?;
    if retained > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > l.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    source.try_validate_tile_origins_with(c)?;
    for (_, t) in source.tiles() {
        poll(c)?;
        if t.bytes().len() as u64 != tile {
            return Err(RasterInputError::LengthMismatch);
        }
    }
    Ok((retained, work))
}
/// Admission before the allocation-free exact encoded occupancy scan.
pub fn source_bounds_with_limits(source: &Surface, l: GeometryLimits, c: &Interrupt<'_>) -> Result<Rect> {
    source_charge(source, l, c)?;
    source.try_content_bounds_with_interrupt(c)
}
fn geometry(bounds: Rect, h: &Homography) -> Result<(Rect, Homography)> {
    if h.0.iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let inverse = h.inverse().filter(|m| m.0.iter().all(|v| v.is_finite())).ok_or(RasterInputError::InvalidBounds)?;
    if bounds.is_empty() {
        return Ok((Rect::EMPTY, inverse));
    }
    let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x1, bounds.y1), (bounds.x0, bounds.y1)];
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut sign = 0.0;
    for (x, y) in corners {
        if i64::from(x).abs() > 1_000_000 || i64::from(y).abs() > 1_000_000 {
            return Err(RasterInputError::InvalidBounds);
        }
        let d = h.0[6] * f64::from(x) + h.0[7] * f64::from(y) + h.0[8];
        if !d.is_finite() || d.abs() < 1e-12 || (sign != 0.0 && d.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = d.signum();
        let (u, v) = h.apply(f64::from(x), f64::from(y));
        if !u.is_finite() || !v.is_finite() || u.abs() > 1_000_000.0 || v.abs() > 1_000_000.0 {
            return Err(RasterInputError::InvalidBounds);
        }
        lo[0] = lo[0].min(u);
        lo[1] = lo[1].min(v);
        hi[0] = hi[0].max(u);
        hi[1] = hi[1].max(v);
    }
    // Same +/- one-pixel support padding as the controlled legacy sampler.
    let dst = Rect::new(
        (lo[0].floor() as i32).checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        (lo[1].floor() as i32).checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        (hi[0].ceil() as i32).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
        (hi[1].ceil() as i32).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
    );
    sign = 0.0;
    for (x, y) in [(dst.x0, dst.y0), (dst.x1, dst.y0), (dst.x1, dst.y1), (dst.x0, dst.y1)] {
        let d = inverse.0[6] * f64::from(x) + inverse.0[7] * f64::from(y) + inverse.0[8];
        if !d.is_finite() || d.abs() < 1e-12 || (sign != 0.0 && d.signum() != sign) {
            return Err(RasterInputError::InvalidBounds);
        }
        sign = d.signum();
        let (u, v) = inverse.apply(f64::from(x), f64::from(y));
        if !u.is_finite() || !v.is_finite() || u.abs() > f64::from(i32::MAX - 2) || v.abs() > f64::from(i32::MAX - 2) {
            return Err(RasterInputError::InvalidBounds);
        }
    }
    Ok((dst, inverse))
}
/// Explicit source rectangle is caller-authoritative. Engine supplies exact raw
/// occupancy, including off-canvas words; no unit/finite-value interpretation.
pub fn requirements(source: &Surface, bounds: Rect, h: &Homography, l: GeometryLimits, c: &Interrupt<'_>) -> Result<NativeNearestRequirements> {
    let (retained, source_work) = source_charge(source, l, c)?;
    checked_region_byte_len(bounds, source.format())?;
    let (dst, _) = geometry(bounds, h)?;
    let tiles = tile_count(dst)?;
    let tile = mul((TILE_SIZE * TILE_SIZE) as u64, source.format().bytes_per_pixel() as u64)?;
    // Raw writer stages a second sparse map before its final swap. Payloads
    // stay shared; both map owners are charged, not merely the returned map.
    let output = add(source.default_bytes().len() as u64, mul(tiles, add(tile, 1024)?)?)?;
    let scratch = if dst.is_empty() { 0 } else { add(mul(tile, 4)?, 64)? };
    let work = add(source_work, add(mul(mul(tiles, tile)?, 32)?, mul(mul(tiles, add(source.tile_count() as u64, tiles)?)?, 256)?)?)?;
    if add(retained, add(output, scratch)?)? > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > l.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(c)?;
    Ok(NativeNearestRequirements { output_bounds: dst, retained_bytes: retained, scratch_bytes: scratch, output_bytes: output, work_units: work })
}
/// Half-open input cell nearest: floor(inverse(output x+.5, y+.5)). Minification
/// stays point-sampled, as legacy Nearest; no prefilter or premultiplication.
pub fn try_warp_gray_nearest_raw_with_limits(source: &Surface, bounds: Rect, h: &Homography, l: GeometryLimits, c: &Interrupt<'_>) -> Result<Surface> {
    let req = requirements(source, bounds, h, l, c)?;
    let (_, inverse) = geometry(bounds, h)?;
    let fmt = source.format();
    let bpp = fmt.bytes_per_pixel();
    let mut out = Surface::try_with_default_bytes(fmt, source.default_bytes())?;
    for tile in req.output_bounds.tiles() {
        poll(c)?;
        let area = tile.rect().intersect(&req.output_bounds);
        let len = checked_region_byte_len(area, fmt)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
        if raw.capacity() != len || raw.capacity() as u64 > req.scratch_bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut nondefault = false;
        for y in area.y0..area.y1 {
            poll(c)?;
            for x in area.x0..area.x1 {
                if x.rem_euclid(64) == 0 {
                    poll(c)?;
                }
                let (u, v) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                if !u.is_finite() || !v.is_finite() || u.abs() > f64::from(i32::MAX - 2) || v.abs() > f64::from(i32::MAX - 2) {
                    return Err(RasterInputError::InvalidBounds);
                }
                let (px, py) = (u.floor() as i32, v.floor() as i32);
                let mut sample = [0u8; 8];
                let sample = sample.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                if bounds.contains(px, py) {
                    source.try_read_pixel_bytes(px, py, sample)?;
                } else {
                    sample.copy_from_slice(source.default_bytes());
                }
                nondefault |= sample != source.default_bytes();
                if raw.len().checked_add(bpp).is_none_or(|n| n > raw.capacity()) {
                    return Err(RasterInputError::LimitExceeded);
                }
                raw.extend_from_slice(sample);
            }
        }
        if raw.len() != len {
            return Err(RasterInputError::LengthMismatch);
        }
        if nondefault {
            out.try_write_interleaved_with(area, &raw, c)?;
        }
    }
    // A tile is inserted only after finding a non-default native word, and
    // each destination tile is visited once. No output-wide prune scan or
    // its allocation-bearing removal inventory is necessary.
    poll(c)?;
    Ok(out)
}

/// Exact native sample/default equality, ignoring allocated all-default padding
/// tiles. Used for no-op history admission; no numeric coverage classification.
pub fn raw_equal_with_limits(a: &Surface, b: &Surface, l: GeometryLimits, c: &Interrupt<'_>) -> Result<bool> {
    poll(c)?;
    let (ar, aw) = source_charge(a, l, c)?;
    let (br, bw) = source_charge(b, l, c)?;
    if add(ar, br)? > l.resident_bytes || add(aw, bw)? > l.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if a.format() != b.format() || a.default_bytes() != b.default_bytes() {
        return Ok(false);
    }
    let default = a.default_bytes();
    let bpp = default.len();
    for (coord, tile) in a.tiles() {
        poll(c)?;
        if let Some(other) = b.tile(*coord) {
            for (left, right) in tile.bytes().chunks(4096).zip(other.bytes().chunks(4096)) {
                poll(c)?;
                if left != right {
                    return Ok(false);
                }
            }
        } else {
            for chunk in tile.bytes().chunks(bpp * 256) {
                poll(c)?;
                if chunk.chunks_exact(bpp).any(|px| px != default) {
                    return Ok(false);
                }
            }
        }
    }
    for (coord, tile) in b.tiles() {
        poll(c)?;
        if a.tile(*coord).is_none() {
            for chunk in tile.bytes().chunks(bpp * 256) {
                poll(c)?;
                if chunk.chunks_exact(bpp).any(|px| px != default) {
                    return Ok(false);
                }
            }
        }
    }
    poll(c)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    fn fmt(sample: SampleType) -> PixelFormat {
        PixelFormat::new(ColorMode::Grayscale, sample, false)
    }
    #[test]
    fn native_nan_signed_zero_default_and_offcanvas_words_move_raw() {
        let default = 0x7ff8_0000_0000_0061u64.to_ne_bytes();
        let mut source = Surface::try_with_default_bytes(fmt(SampleType::F64), &default).unwrap();
        let words = [0x7ff8_0000_0000_0042u64, (-0.0f64).to_bits(), 0.5f64.to_bits() + 1];
        let bytes: Vec<u8> = words.into_iter().flat_map(u64::to_ne_bytes).collect();
        let r = Rect::new(-2, 3, 1, 4);
        source.try_write_interleaved_with(r, &bytes, &Interrupt::NONE).unwrap();
        let h = Homography([1.0, 0.0, -7.0, 0.0, 1.0, 2.0, 0.0, 0.0, 1.0]);
        let output = try_warp_gray_nearest_raw_with_limits(&source, r, &h, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.format(), source.format());
        assert_eq!(output.default_bytes(), default);
        assert_eq!(output.try_read_interleaved_with(Rect::new(-9, 5, -6, 6), &Interrupt::NONE).unwrap(), bytes);
        let mut raw = [0u8; 8];
        output.try_read_pixel_bytes(-2, 3, &mut raw).unwrap();
        assert_eq!(raw, default, "vacated pixels retain original raw default");
        assert_eq!(source.try_read_interleaved_with(r, &Interrupt::NONE).unwrap(), bytes);
    }
    #[test]
    fn nearest_minification_uses_containing_cell_without_prefilter() {
        let mut source = Surface::try_with_default_bytes(fmt(SampleType::U32), &99u32.to_ne_bytes()).unwrap();
        let bytes: Vec<u8> = [1u32, 2, 3, 4].into_iter().flat_map(u32::to_ne_bytes).collect();
        let r = Rect::new(0, 0, 4, 1);
        source.try_write_interleaved_with(r, &bytes, &Interrupt::NONE).unwrap();
        let h = Homography([0.5, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let output = try_warp_gray_nearest_raw_with_limits(&source, r, &h, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(
            output.try_read_interleaved_with(Rect::new(0, 0, 2, 1), &Interrupt::NONE).unwrap(),
            [2u32, 4].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()
        );
        let req = requirements(&source, r, &h, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(req.output_bounds, Rect::new(-1, -1, 3, 2));
    }
    #[test]
    fn empty_native_background_and_failure_are_not_numerically_classified() {
        let source = Surface::try_with_default_bytes(fmt(SampleType::F64), &(-0.0f64).to_ne_bytes()).unwrap();
        let h = Homography([1.0, 0.0, 3.0, 0.0, 1.0, -2.0, 0.0, 0.0, 1.0]);
        let output = try_warp_gray_nearest_raw_with_limits(&source, Rect::EMPTY, &h, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert!(raw_equal_with_limits(&source, &output, GeometryLimits::default(), &Interrupt::NONE).unwrap());
        assert_eq!(output.default_bytes(), source.default_bytes());
        let cancel = || true;
        assert!(matches!(
            try_warp_gray_nearest_raw_with_limits(&source, Rect::EMPTY, &h, GeometryLimits::default(), &Interrupt::cancel_only(&cancel)),
            Err(RasterInputError::Cancelled)
        ));
        assert!(
            try_warp_gray_nearest_raw_with_limits(&source, Rect::new(0, 0, 10, 10), &h, GeometryLimits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE)
                .is_err()
        );
        let horizon = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        assert!(try_warp_gray_nearest_raw_with_limits(&source, Rect::new(-1, -1, 1, 1), &horizon, GeometryLimits::default(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn partial_destination_tiles_do_not_need_default_padding_prune() {
        let mut source = Surface::new(fmt(SampleType::U32));
        let rect = Rect::new(0, 0, 513, 1);
        let raw: Vec<u8> = (1u32..=513).flat_map(u32::to_ne_bytes).collect();
        source.try_write_interleaved_with(rect, &raw, &Interrupt::NONE).unwrap();
        let output = try_warp_gray_nearest_raw_with_limits(&source, rect, &Homography::IDENTITY, GeometryLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved_with(rect, &Interrupt::NONE).unwrap(), raw);
        assert_eq!(output.tile_count(), source.tile_count(), "padding-only destination tiles must never be inserted");
        assert!(raw_equal_with_limits(&source, &output, GeometryLimits::default(), &Interrupt::NONE).unwrap());
    }
}
