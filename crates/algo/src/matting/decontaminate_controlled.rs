//! Exact ordinary fringe color replacement, preserving all source words outside
//! touched legacy cores. It adds no source/CMS/profile or wide arithmetic policy.
use crate::matting::{MattingLimits, box_mean_with_interrupt};
use crate::segment::Region;
use photocraft_color::{SampleType, read_sample, write_sample};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
const MAX_WORK: u64 = 8_000_000_000;
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Clone>(n: usize, z: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    super::super::controlled_gray::filled(n, z, c)
}
fn at(v: &[f32], i: usize) -> Result<f32, E> {
    v.get(i).copied().ok_or(E::LengthMismatch)
}
#[derive(Clone, Copy, Debug)]
pub struct DecontaminateRequirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub output_bytes: u64,
    pub radius: usize,
}
pub fn requirements(src: &Surface, bbox: Rect, radius: f32, c: &Interrupt<'_>) -> Result<DecontaminateRequirements, E> {
    poll(c)?;
    let fmt = src.format();
    if matches!(fmt.sample, SampleType::U32 | SampleType::F64)
        || fmt.channels() == 0
        || fmt.channels() > 5
        || !radius.is_finite()
        || !(0.0..=250.0).contains(&radius)
    {
        return Err(E::UnsupportedConversion);
    }
    if bbox.x1 < bbox.x0
        || bbox.y1 < bbox.y0
        || bbox.width() > 300000
        || bbox.height() > 300000
        || [bbox.x0, bbox.x1, bbox.y0, bbox.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
    {
        return Err(E::InvalidBounds);
    }
    let rad = radius.max(3.).ceil() as usize;
    let window = 256usize.checked_add(rad.checked_mul(8).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    let n = if bbox.is_empty() { 0 } else { window.checked_mul(window).ok_or(E::LimitExceeded)? as u64 };
    let tiles = if bbox.is_empty() {
        0
    } else {
        let a = photocraft_geom::TileCoord::containing(bbox.x0, bbox.y0);
        let b = photocraft_geom::TileCoord::containing(bbox.x1 - 1, bbox.y1 - 1);
        u64::try_from(i64::from(b.tx) - i64::from(a.tx) + 1)
            .map_err(|_| E::InvalidBounds)?
            .checked_mul(u64::try_from(i64::from(b.ty) - i64::from(a.ty) + 1).map_err(|_| E::InvalidBounds)?)
            .ok_or(E::LimitExceeded)?
    };
    let output = tiles
        .checked_mul(65536u64.checked_mul(fmt.bytes_per_pixel() as u64).and_then(|v| v.checked_add(1024)).ok_or(E::LimitExceeded)?)
        .ok_or(E::LimitExceeded)?;
    src.try_validate_tile_origins_with(c)?;
    let expected = 65536usize.checked_mul(fmt.bytes_per_pixel()).ok_or(E::LimitExceeded)?;
    if src.default_bytes().len() != fmt.bytes_per_pixel() {
        return Err(E::LengthMismatch);
    }
    let mut source = src.default_bytes().len() as u64;
    for (_, tile) in src.tiles() {
        poll(c)?;
        if tile.bytes().len() != expected {
            return Err(E::LengthMismatch);
        }
        source = source.checked_add(tile.bytes().len() as u64).and_then(|v| v.checked_add(256)).ok_or(E::LimitExceeded)?;
    }
    poll(c)?;
    let map = (src.tile_count() as u64).checked_mul(256).ok_or(E::LimitExceeded)?;
    // At most four bbox-anchored legacy cores intersect one native world tile.
    // Each repeated halo is processed privately against the ORIGINAL source.
    let work = tiles
        .checked_mul(4)
        .and_then(|v| v.checked_mul(n))
        .and_then(|v| v.checked_mul(1024))
        .and_then(|v| v.checked_add(output.checked_mul(16)?))
        .and_then(|v| v.checked_add(source.checked_mul(2)?))
        .ok_or(E::LimitExceeded)?;
    let bytes = source
        .checked_add(map)
        .and_then(|v| v.checked_add(output))
        .and_then(|v| v.checked_add(n.checked_mul(256)?))
        .and_then(|v| v.checked_add(65536u64.checked_mul(fmt.bytes_per_pixel() as u64)?.checked_mul(3)?))
        .and_then(|v| v.checked_add(1 << 20))
        .ok_or(E::LimitExceeded)?;
    if bytes > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(DecontaminateRequirements { peak_resident_bytes: bytes, work_units: work, output_bytes: output, radius: rad })
}
fn core(src: &Surface, alpha: &Region, t: Rect, rad: usize, amount: f32, l: MattingLimits, c: &Interrupt<'_>) -> Result<Option<Vec<u8>>, E> {
    let soft = |a: f32| a > 0.02 && a < 0.98;
    let mut any = false;
    for y in t.y0..t.y1 {
        poll(c)?;
        for x in t.x0..t.x1 {
            if x.rem_euclid(1024) == 0 {
                poll(c)?;
            }
            any |= soft(alpha.at(x, y));
        }
    }
    if !any {
        return Ok(None);
    }
    let r = t.inflate((rad * 4) as i32);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let fmt = src.format();
    let channels = fmt.channels();
    let colors = channels - usize::from(fmt.alpha);
    let mut px = filled(n.checked_mul(channels).ok_or(E::LimitExceeded)?, 0f32, c)?;
    let mut al = filled(n, 0f32, c)?;
    let mut weights = filled(n, 0f32, c)?;
    let mut raw = [0u8; 40];
    for y in r.y0..r.y1 {
        poll(c)?;
        for x in r.x0..r.x1 {
            if x.rem_euclid(1024) == 0 {
                poll(c)?;
            }
            let i = (y - r.y0) as usize * w + (x - r.x0) as usize;
            let input = raw.get_mut(..fmt.bytes_per_pixel()).ok_or(E::LengthMismatch)?;
            src.try_read_pixel_bytes(x, y, input)?;
            for ch in 0..channels {
                let v = read_sample(input, fmt.sample, ch);
                if !v.is_finite() {
                    return Err(E::InvalidSample);
                }
                *px.get_mut(i * channels + ch).ok_or(E::LengthMismatch)? = v;
            }
            let a = alpha.at(x, y);
            *al.get_mut(i).ok_or(E::LengthMismatch)? = a;
            *weights.get_mut(i).ok_or(E::LengthMismatch)? = if a >= 0.98 { if fmt.alpha { at(&px, i * channels + channels - 1)? } else { 1. } } else { 0. };
        }
    }
    let den = box_mean_with_interrupt(&weights, w, h, rad, l, c)?;
    let den2 = box_mean_with_interrupt(&weights, w, h, rad * 4, l, c)?;
    let mut nums = Vec::new();
    nums.try_reserve_exact(colors).map_err(|_| E::AllocationFailed)?;
    if nums.capacity() > colors.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    for ch in 0..colors {
        poll(c)?;
        let mut v = filled(n, 0f32, c)?;
        for (i, v) in v.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = at(&weights, i)? * at(&px, i * channels + ch)?;
            if !v.is_finite() {
                return Err(E::InvalidSample);
            }
        }
        nums.push((box_mean_with_interrupt(&v, w, h, rad, l, c)?, box_mean_with_interrupt(&v, w, h, rad * 4, l, c)?));
    }
    let bpp = fmt.bytes_per_pixel();
    let mut out = filled((t.width() as usize).checked_mul(t.height() as usize).and_then(|v| v.checked_mul(bpp)).ok_or(E::LimitExceeded)?, 0u8, c)?;
    for y in t.y0..t.y1 {
        poll(c)?;
        for x in t.x0..t.x1 {
            if x.rem_euclid(64) == 0 {
                poll(c)?;
            }
            let i = (y - r.y0) as usize * w + (x - r.x0) as usize;
            let a = at(&al, i)?;
            let k = if soft(a) { amount * ((1. - a) * 4.).clamp(0., 1.) } else { 0. };
            let base = ((y - t.y0) as usize * t.width() as usize + (x - t.x0) as usize) * bpp;
            let pixel = out.get_mut(base..base + bpp).ok_or(E::LengthMismatch)?;
            for ch in 0..channels {
                let v = at(&px, i * channels + ch)?;
                let value = if ch >= colors || k == 0. {
                    v
                } else {
                    let pair = nums.get(ch).ok_or(E::LengthMismatch)?;
                    let f = if at(&den, i)? > 1e-4 {
                        at(&pair.0, i)? / at(&den, i)?
                    } else if at(&den2, i)? > 1e-4 {
                        at(&pair.1, i)? / at(&den2, i)?
                    } else {
                        v
                    };
                    v + (f - v) * k
                };
                if !value.is_finite() {
                    return Err(E::InvalidSample);
                }
                write_sample(pixel, fmt.sample, ch, value);
            }
        }
    }
    poll(c)?;
    Ok(Some(out))
}
struct Prepaid {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Prepaid {
    type Lease = ();
    fn reserve(&mut self, n: u64) -> Result<(), E> {
        self.bytes = self.bytes.checked_sub(n).ok_or(E::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, n: u64) -> Result<(), E> {
        self.work = self.work.checked_sub(n).ok_or(E::LimitExceeded)?;
        Ok(())
    }
}
pub fn decontaminate_with_interrupt(src: &Surface, alpha: &Region, radius: f32, amount: f32, l: MattingLimits, c: &Interrupt<'_>) -> Result<Surface, E> {
    poll(c)?;
    if !amount.is_finite() {
        return Err(E::InvalidSample);
    }
    let req = requirements(src, alpha.bbox, radius, c)?;
    if req.peak_resident_bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > l.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    let b = alpha.bbox;
    if alpha.mask.len() != u64::from(b.width()).checked_mul(u64::from(b.height())).and_then(|v| usize::try_from(v).ok()).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    src.try_validate_tile_origins_with(c)?;
    poll(c)?;
    let mut output = Surface::try_with_default_bytes(src.format(), src.default_bytes())?;
    let tile_bytes = 65536usize.checked_mul(src.format().bytes_per_pixel()).ok_or(E::LimitExceeded)?;
    for (coordinate, tile) in src.tiles() {
        poll(c)?;
        if tile.bytes().len() != tile_bytes {
            return Err(E::LengthMismatch);
        }
        output.put_tiles(std::iter::once((*coordinate, std::sync::Arc::clone(tile))));
    }
    poll(c)?;
    if b.is_empty() {
        return Ok(output);
    }
    let amount = amount.clamp(0., 100.) / 100.;
    let fmt = src.format();
    let bpp = fmt.bytes_per_pixel();
    let mut prepaid = Prepaid { bytes: req.peak_resident_bytes, work: req.work_units };
    for coordinate in b.tiles() {
        poll(c)?;
        let tile = coordinate.rect();
        let overlap = tile.intersect(&b);
        let mut raw = filled(65536usize.checked_mul(bpp).ok_or(E::LimitExceeded)?, 0u8, c)?;
        src.try_read_interleaved_into_with(tile, &mut raw, c)?;
        let x0 = (overlap.x0 - b.x0) / 256;
        let x1 = (overlap.x1 - 1 - b.x0) / 256;
        let y0 = (overlap.y0 - b.y0) / 256;
        let y1 = (overlap.y1 - 1 - b.y0) / 256;
        let mut changed = false;
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                poll(c)?;
                let x = b.x0 + cx * 256;
                let y = b.y0 + cy * 256;
                let t = Rect::new(x, y, (x + 256).min(b.x1), (y + 256).min(b.y1));
                let Some(bytes) = core(src, alpha, t, req.radius, amount, l, c)? else { continue };
                changed = true;
                let part = t.intersect(&tile);
                for yy in part.y0..part.y1 {
                    poll(c)?;
                    let width = part.width() as usize * bpp;
                    let from = ((yy - t.y0) as usize * t.width() as usize + (part.x0 - t.x0) as usize) * bpp;
                    let to = ((yy - tile.y0) as usize * TILE_SIZE as usize + (part.x0 - tile.x0) as usize) * bpp;
                    raw.get_mut(to..to + width).ok_or(E::LengthMismatch)?.copy_from_slice(bytes.get(from..from + width).ok_or(E::LengthMismatch)?);
                }
            }
        }
        if !changed {
            continue;
        }
        let mut builder = StagedSurfaceBuilder::new(fmt, src.default_bytes(), tile, &mut prepaid, c)?;
        for y in (tile.y0..tile.y1).step_by(64) {
            for x in (tile.x0..tile.x1).step_by(64) {
                poll(c)?;
                let cell = Rect::new(x, y, x + 64, y + 64);
                let mut bytes = filled(4096usize.checked_mul(bpp).ok_or(E::LimitExceeded)?, 0u8, c)?;
                for yy in y..y + 64 {
                    poll(c)?;
                    let from = ((yy - tile.y0) as usize * TILE_SIZE as usize + (x - tile.x0) as usize) * bpp;
                    let to = (yy - y) as usize * 64 * bpp;
                    let len = 64 * bpp;
                    bytes.get_mut(to..to + len).ok_or(E::LengthMismatch)?.copy_from_slice(raw.get(from..from + len).ok_or(E::LengthMismatch)?);
                }
                builder.push_region(cell, &bytes, &mut prepaid, c)?;
            }
        }
        let (replacement, _) = builder.finalize(&mut prepaid, c)?.into_parts();
        poll(c)?;
        output.put_tiles(replacement.tiles().map(|(p, t)| (*p, std::sync::Arc::clone(t))));
    }
    poll(c)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat};
    #[test]
    fn exact_old_core_math_and_off_box_defaults_survive() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut source = Surface::with_default(fmt, &[0.11, 0.23, 0.47, 0.73]);
            let area = Rect::new(-13, -5, 271, 31);
            let mut pixels = Vec::new();
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    pixels.extend_from_slice(&[if x < 120 { 0.8 } else { 0.1 }, 0.3, 0.2, if y < 9 { 0.4 } else { 1. }]);
                }
            }
            source.write_region(area, &pixels);
            let bbox = Rect::new(-7, 0, 264, 23);
            let alpha = Region { bbox, mask: (0..271 * 23).map(|i| if i % 271 < 120 { 255 } else { 127 }).collect() };
            let before = source.clone();
            let expected = super::super::super::decontaminate(&source, &alpha, 3., 75.);
            let actual = decontaminate_with_interrupt(&source, &alpha, 3., 75., MattingLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual.default_bytes(), source.default_bytes());
            assert_eq!(actual.try_read_interleaved_with(area, &Interrupt::NONE).unwrap(), expected.try_read_interleaved_with(area, &Interrupt::NONE).unwrap());
            let far = Rect::new(300, -8, 305, 0);
            assert_eq!(actual.try_read_interleaved_with(far, &Interrupt::NONE).unwrap(), source.try_read_interleaved_with(far, &Interrupt::NONE).unwrap());
            assert_eq!(source, before);
            assert!(matches!(
                decontaminate_with_interrupt(&source, &alpha, 3., 75., MattingLimits::default(), &Interrupt::cancel_only(&|| true)),
                Err(E::Cancelled)
            ));
        }
    }
    #[test]
    fn empty_high_radius_shares_without_unused_halo_refusal() {
        let source = Surface::new(PixelFormat::RGBA8);
        let alpha = Region { bbox: Rect::EMPTY, mask: Vec::new() };
        let r = requirements(&source, alpha.bbox, 250., &Interrupt::NONE).unwrap();
        assert!(r.peak_resident_bytes < 4 << 20);
        let out = decontaminate_with_interrupt(&source, &alpha, 250., 100., MattingLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(out, source);
    }
}
