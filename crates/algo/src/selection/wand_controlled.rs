//! Controlled companions to the legacy byte Magic Wand, not a wide/native
//! sampling policy. Same span order, tolerance, hidden-alpha and AA equations.
use super::{Region, SelectionMode};
use photocraft_color::{ColorMode, PixelFormat, SampleType, read_sample, write_sample};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
#[derive(Clone, Copy)]
pub struct WandLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for WandLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::LimitExceeded)
}
fn admit(bytes: u64, work: u64, l: WandLimits) -> Result<(), E> {
    if l.resident_bytes == 0 || l.resident_bytes > MAX_CHECKED_RASTER_BYTES || bytes > l.resident_bytes || work > l.work_units {
        Err(E::LimitExceeded)
    } else {
        Ok(())
    }
}
fn count(r: Rect) -> Result<usize, E> {
    usize::try_from(u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or(E::LimitExceeded)?).map_err(|_| E::LimitExceeded)
}
fn buffer<T: Copy>(n: usize, value: T, l: WandLimits, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
    admit(bytes as u64, 0, l)?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    admit(v.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)? as u64, 0, l)?;
    while v.len() < n {
        poll(c)?;
        let end = v.len().checked_add(4096).ok_or(E::LimitExceeded)?.min(n);
        v.resize(end, value);
    }
    Ok(v)
}
/// Exact legacy RGBA8 sampling, with allocation-free native pixel reads. The
/// caller retains existing source/profile authority and ordinary model policy.
pub fn try_rgba8_image_with_interrupt(s: &Surface, area: Rect, l: WandLimits, c: &Interrupt<'_>) -> Result<Vec<[u8; 4]>, E> {
    poll(c)?;
    let fmt = s.format();
    if matches!(fmt.sample, SampleType::U32 | SampleType::F64) {
        return Err(E::UnsupportedConversion);
    }
    let n = count(area)?;
    let retained = s.resident_accounted_bytes()?;
    admit(add(retained, mul(n as u64, 4)?)?, mul(n as u64, 32)?, l)?;
    s.try_validate_tile_origins_with(c)?;
    let mut out = buffer(n, [0; 4], l, c)?;
    if add(retained, mul(out.capacity() as u64, 4)?)? > l.resident_bytes {
        return Err(E::LimitExceeded);
    }
    let mut raw = [0u8; 64];
    let raw = raw.get_mut(..fmt.bytes_per_pixel()).ok_or(E::LengthMismatch)?;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut at = 0usize;
    for y in area.y0..area.y1 {
        poll(c)?;
        for x in area.x0..area.x1 {
            poll(c)?;
            s.try_read_pixel_bytes(x, y, raw)?;
            let pixel = if fmt == PixelFormat::RGBA8 {
                raw.get(..4).and_then(|b| b.try_into().ok()).ok_or(E::LengthMismatch)?
            } else {
                let mut channels = [0.0f32; 16];
                let dst = channels.get_mut(..fmt.channels()).ok_or(E::LengthMismatch)?;
                for (i, v) in dst.iter_mut().enumerate() {
                    *v = read_sample(raw, fmt.sample, i)
                }
                photocraft_raster::to_rgba(&fmt, dst).map(q)
            };
            *out.get_mut(at).ok_or(E::LengthMismatch)? = pixel;
            at = at.checked_add(1).ok_or(E::LimitExceeded)?;
        }
    }
    poll(c)?;
    Ok(out)
}
fn index(x: usize, y: usize, w: usize) -> Result<usize, E> {
    y.checked_mul(w).and_then(|n| n.checked_add(x)).ok_or(E::LimitExceeded)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn try_wand_region_with_interrupt(
    img: &[[u8; 4]],
    area: Rect,
    seed: (i32, i32),
    tolerance: f32,
    contiguous: bool,
    anti_alias: bool,
    l: WandLimits,
    c: &Interrupt<'_>,
) -> Result<Option<Region>, E> {
    poll(c)?;
    if !tolerance.is_finite() {
        return Err(E::InvalidSample);
    }
    if !area.contains(seed.0, seed.1) {
        return Ok(None);
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    let n = count(area)?;
    if img.len() < n {
        return Err(E::LengthMismatch);
    }
    let max_stack = n.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(E::LimitExceeded)?;
    let peak = add(
        mul(img.len() as u64, 4)?,
        add(mul(n as u64, 2)?, if contiguous { mul(max_stack as u64, std::mem::size_of::<(usize, usize)>() as u64)? } else { 0 })?,
    )?;
    admit(peak, mul(n as u64, 32)?, l)?;
    let (sx, sy) = (
        usize::try_from(i64::from(seed.0) - i64::from(area.x0)).map_err(|_| E::InvalidBounds)?,
        usize::try_from(i64::from(seed.1) - i64::from(area.y0)).map_err(|_| E::InvalidBounds)?,
    );
    let target = *img.get(index(sx, sy, w)?).ok_or(E::LengthMismatch)?;
    let tol = (tolerance + 1e-3).floor().max(0.0) as i32;
    let similar = |p: [u8; 4]| (target[3] == 0 && p[3] == 0) || target.iter().zip(p).all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= tol);
    let mut marks = buffer(n, 0u8, l, c)?;
    let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let work = std::cell::Cell::new(0u64);
    let tick = || -> Result<(), E> {
        poll(c)?;
        work.set(work.get().checked_add(1).ok_or(E::LimitExceeded)?);
        if work.get() > l.work_units { Err(E::LimitExceeded) } else { Ok(()) }
    };
    if contiguous {
        let mut stack = Vec::new();
        stack.try_reserve_exact(max_stack).map_err(|_| E::AllocationFailed)?;
        if add(
            add(mul(img.len() as u64, 4)?, mul(marks.capacity() as u64, 1)?)?,
            add(n as u64, mul(stack.capacity() as u64, std::mem::size_of::<(usize, usize)>() as u64)?)?,
        )? > l.resident_bytes
        {
            return Err(E::LimitExceeded);
        }
        stack.push((sx, sy));
        while let Some((x, y)) = stack.pop() {
            tick()?;
            let row = y.checked_mul(w).ok_or(E::LimitExceeded)?;
            if *marks.get(row + x).ok_or(E::LengthMismatch)? != 0 {
                continue;
            }
            let (mut left, mut right) = (x, x);
            while left > 0 {
                tick()?;
                let i = row + left - 1;
                if *marks.get(i).ok_or(E::LengthMismatch)? != 0 || !similar(*img.get(i).ok_or(E::LengthMismatch)?) {
                    break;
                }
                left -= 1;
            }
            while right.checked_add(1).is_some_and(|v| v < w) {
                tick()?;
                let i = row + right + 1;
                if *marks.get(i).ok_or(E::LengthMismatch)? != 0 || !similar(*img.get(i).ok_or(E::LengthMismatch)?) {
                    break;
                }
                right += 1;
            }
            for nx in left..=right {
                tick()?;
                *marks.get_mut(row + nx).ok_or(E::LengthMismatch)? = 255
            }
            (bx0, bx1, by0, by1) = (bx0.min(left), bx1.max(right), by0.min(y), by1.max(y));
            for ny in [y.wrapping_sub(1), y.checked_add(1).ok_or(E::LimitExceeded)?] {
                if ny >= h {
                    continue;
                }
                let mut in_run = false;
                for nx in left..=right {
                    tick()?;
                    let i = index(nx, ny, w)?;
                    let ok = *marks.get(i).ok_or(E::LengthMismatch)? == 0 && similar(*img.get(i).ok_or(E::LengthMismatch)?);
                    if ok && !in_run {
                        if stack.len() >= max_stack || stack.len() >= stack.capacity() {
                            return Err(E::LimitExceeded);
                        }
                        stack.push((nx, ny));
                    }
                    in_run = ok;
                }
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                tick()?;
                let i = index(x, y, w)?;
                if similar(*img.get(i).ok_or(E::LengthMismatch)?) {
                    *marks.get_mut(i).ok_or(E::LengthMismatch)? = 255;
                    (bx0, bx1, by0, by1) = (bx0.min(x), bx1.max(x), by0.min(y), by1.max(y));
                }
            }
        }
    }
    if bx0 == usize::MAX {
        return Ok(None);
    }
    let pad = usize::from(anti_alias);
    let (x0, y0) = (bx0.saturating_sub(pad), by0.saturating_sub(pad));
    let (x1, y1) = (bx1.checked_add(1 + pad).ok_or(E::LimitExceeded)?.min(w), by1.checked_add(1 + pad).ok_or(E::LimitExceeded)?.min(h));
    let bw = x1 - x0;
    let mut mask = buffer(bw.checked_mul(y1 - y0).ok_or(E::LimitExceeded)?, 0u8, l, c)?;
    admit(add(mul(img.len() as u64, 4)?, add(marks.capacity() as u64, mask.capacity() as u64)?)?, work.get(), l)?;
    for y in y0..y1 {
        for x in x0..x1 {
            tick()?;
            let v = *marks.get(index(x, y, w)?).ok_or(E::LengthMismatch)?;
            let mut value = v;
            if anti_alias {
                let (mut sum, mut edge) = (0u32, false);
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        tick()?;
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        let a = if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            v
                        } else {
                            *marks.get(index(nx as usize, ny as usize, w)?).ok_or(E::LengthMismatch)?
                        };
                        edge |= a != v;
                        sum += u32::from(a);
                    }
                }
                if edge {
                    let avg = sum as f32 / (9.0 * 255.0);
                    let f = if v > 127 { 0.5 + 0.5 * avg } else { 0.5 * avg };
                    value = (f * 255.0).round() as u8;
                }
            }
            *mask.get_mut(index(x - x0, y - y0, bw)?).ok_or(E::LengthMismatch)? = value;
        }
    }
    let coord = |base: i32, n: usize| base.checked_add(i32::try_from(n).map_err(|_| E::InvalidBounds)?).ok_or(E::InvalidBounds);
    let bbox = Rect::new(coord(area.x0, x0)?, coord(area.y0, y0)?, coord(area.x0, x1)?, coord(area.y0, y1)?);
    poll(c)?;
    Ok(Some(Region { bbox, mask }))
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
/// Preserve the legacy GRAY8 mixing path and ordinary Add/Subtract destination
/// depth. Wide old coverage is refused, never narrowed through f32.
pub fn try_combine_region_with_interrupt(
    old: Option<&Surface>,
    new: Option<&Region>,
    mode: SelectionMode,
    l: WandLimits,
    c: &Interrupt<'_>,
) -> Result<Option<Surface>, E> {
    poll(c)?;
    let empty = Region { bbox: Rect::EMPTY, mask: Vec::new() };
    let new = new.unwrap_or(&empty);
    let b = new.bbox;
    if new.mask.len() != count(b)? {
        return Err(E::LengthMismatch);
    }
    if matches!(mode, SelectionMode::Subtract | SelectionMode::Intersect) && old.is_none() {
        return Ok(None);
    }
    if let Some(s) = old {
        if s.format().mode != ColorMode::Grayscale || s.format().alpha || matches!(s.format().sample, SampleType::U32 | SampleType::F64) {
            return Err(E::UnsupportedConversion);
        }
        s.try_validate_tile_origins_with(c)?;
    }
    let retain_old = old.is_some() && matches!(mode, SelectionMode::Add | SelectionMode::Subtract);
    let roi = if retain_old { old.ok_or(E::LengthMismatch)?.try_content_bounds_with_interrupt(c)?.union(&b) } else { b };
    if roi.is_empty() {
        return Ok(None);
    }
    let format = if retain_old { old.ok_or(E::LengthMismatch)?.format() } else { PixelFormat::GRAY8 };
    let tile_bytes = mul(65536, format.bytes_per_pixel() as u64)?;
    let first = photocraft_geom::TileCoord::containing(roi.x0, roi.y0);
    let last = photocraft_geom::TileCoord::containing(roi.x1.checked_sub(1).ok_or(E::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(E::InvalidBounds)?);
    let tiles = mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| E::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| E::InvalidBounds)?,
    )?;
    let output = mul(tiles, add(tile_bytes, 1024)?)?;
    let retained = old.map_or(Ok(0), Surface::resident_accounted_bytes)?;
    let scratch = mul(4096, format.bytes_per_pixel() as u64)?;
    let work = add(mul(count(roi)? as u64, 32)?, mul(output, 16)?)?;
    admit(add(retained, add(output, add(scratch, new.mask.capacity() as u64)?)?)?, work, l)?;
    let zero = [0u8; 40];
    let default = if retain_old { old.ok_or(E::LengthMismatch)?.default_bytes() } else { zero.get(..format.bytes_per_pixel()).ok_or(E::LengthMismatch)? };
    let mut prepaid = Prepaid { bytes: output, work };
    let mut builder = StagedSurfaceBuilder::new(format, default, roi, &mut prepaid, c)?;
    for y0 in (roi.y0..roi.y1).step_by(64) {
        for x in (roi.x0..roi.x1).step_by(64) {
            poll(c)?;
            let part = Rect::new(x, y0, x.checked_add(64).ok_or(E::InvalidBounds)?.min(roi.x1), y0.checked_add(64).ok_or(E::InvalidBounds)?.min(roi.y1));
            let len = count(part)?.checked_mul(format.bytes_per_pixel()).ok_or(E::LimitExceeded)?;
            let mut raw = buffer(len, 0u8, l, c)?;
            if raw.capacity() as u64 > scratch {
                return Err(E::LimitExceeded);
            }
            let mut previous = [0u8; 40];
            for (i, pixel) in raw.chunks_exact_mut(format.bytes_per_pixel()).enumerate() {
                let xx = part.x0.checked_add(i32::try_from(i % part.width() as usize).map_err(|_| E::InvalidBounds)?).ok_or(E::InvalidBounds)?;
                let y = part.y0.checked_add(i32::try_from(i / part.width() as usize).map_err(|_| E::InvalidBounds)?).ok_or(E::InvalidBounds)?;
                poll(c)?;
                if retain_old {
                    old.ok_or(E::LengthMismatch)?.try_read_pixel_bytes(xx, y, pixel)?;
                }
                if !b.contains(xx, y) {
                    continue;
                }
                let n = *new
                    .mask
                    .get(index((i64::from(xx) - i64::from(b.x0)) as usize, (i64::from(y) - i64::from(b.y0)) as usize, b.width() as usize)?)
                    .ok_or(E::LengthMismatch)?;
                let a = if let Some(s) = old {
                    let data = previous.get_mut(..s.format().bytes_per_pixel()).ok_or(E::LengthMismatch)?;
                    s.try_read_pixel_bytes(xx, y, data)?;
                    (read_sample(data, s.format().sample, 0).clamp(0.0, 1.0) * 255.0).round() as u8
                } else {
                    0
                };
                let value = match mode {
                    SelectionMode::Replace => n,
                    SelectionMode::Add => a.max(n),
                    SelectionMode::Subtract => a.saturating_sub(n),
                    SelectionMode::Intersect => a.min(n),
                };
                if format == PixelFormat::GRAY8 {
                    *pixel.first_mut().ok_or(E::LengthMismatch)? = value;
                } else {
                    write_sample(pixel, format.sample, 0, f32::from(value) / 255.0);
                }
            }
            builder.push_region(part, &raw, &mut prepaid, c)?;
        }
    }
    let (mut output, ()) = builder.finalize(&mut prepaid, c)?.into_parts();
    output.try_prune_with(c)?;
    let nonempty = !output.try_content_bounds_with_interrupt(c)?.is_empty();
    poll(c)?;
    Ok(nonempty.then_some(output))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_wand_matches_legacy_span_and_aa_words() {
        let area = Rect::new(-3, 4, 6, 11);
        let pixels = (0..63).map(|i| if i % 7 < 3 { [11, 13, 17, 255] } else { [23, 29, 31, 0] }).collect::<Vec<_>>();
        for contiguous in [false, true] {
            for aa in [false, true] {
                let expected = super::super::wand_region(&pixels, area, (-2, 5), 17.0, contiguous, aa);
                let actual = try_wand_region_with_interrupt(&pixels, area, (-2, 5), 17.0, contiguous, aa, WandLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn cancellation_before_flood_keeps_input() {
        let pixels = vec![[7, 11, 13, 255]; 4096];
        let before = pixels.clone();
        assert!(matches!(
            try_wand_region_with_interrupt(
                &pixels,
                Rect::new(0, 0, 64, 64),
                (1, 1),
                32.0,
                true,
                true,
                WandLimits::default(),
                &Interrupt::cancel_only(&|| true)
            ),
            Err(E::Cancelled)
        ));
        assert_eq!(pixels, before);
    }
    #[test]
    fn ordinary_combine_matches_legacy_words_and_keeps_source() {
        let bbox = Rect::new(-2, 1, 3, 4);
        let region = Region { bbox: Rect::new(0, 2, 2, 3), mask: vec![83, 211] };
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Grayscale, sample, false);
            let mut old = Surface::new(format);
            old.fill_rect(bbox, &[0.37]);
            let before = old.clone();
            for mode in [SelectionMode::Replace, SelectionMode::Add, SelectionMode::Subtract, SelectionMode::Intersect] {
                let expected = super::super::combine_region(Some(&old), Some(&region), mode);
                let actual = try_combine_region_with_interrupt(Some(&old), Some(&region), mode, WandLimits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(
                    actual.map(|s| (s.format(), s.default_bytes().to_vec(), s.try_read_interleaved(bbox).unwrap())),
                    expected.map(|s| (s.format(), s.default_bytes().to_vec(), s.try_read_interleaved(bbox).unwrap()))
                );
                assert_eq!(old, before);
            }
        }
    }
}
