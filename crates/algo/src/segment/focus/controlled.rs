//! Same upstream Focus Area equations with private, admitted, cancellable planes.
use crate::{
    matting::{MattingLimits, box_mean_with_interrupt, guided_filter_color_with_interrupt},
    segment::{CleanupLimits, Region, RgbImage, clean_mask_with_interrupt},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
#[derive(Clone, Copy)]
pub struct FocusLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for FocusLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn buffer<T: Copy>(n: usize, v: T, l: FocusLimits, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n || (out.capacity() as u64).checked_mul(std::mem::size_of::<T>() as u64).is_none_or(|v| v > l.resident_bytes) {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), v);
    }
    Ok(out)
}
fn value(v: &[f32], i: usize) -> Result<f32, E> {
    v.get(i).copied().ok_or(E::LengthMismatch)
}
/// Admission must precede source sampling as well as every analysis allocation.
pub fn requirements(canvas: Rect, l: FocusLimits) -> Result<(usize, usize, usize), E> {
    if canvas.x1 < canvas.x0
        || canvas.y1 < canvas.y0
        || canvas.width() > 300000
        || canvas.height() > 300000
        || l.resident_bytes == 0
        || l.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || l.work_units > 8_000_000_000
    {
        return Err(E::InvalidBounds);
    }
    let fw = usize::try_from(canvas.width()).map_err(|_| E::LimitExceeded)?;
    let fh = usize::try_from(canvas.height()).map_err(|_| E::LimitExceeded)?;
    let full = fw.checked_mul(fh).ok_or(E::LimitExceeded)?;
    let step = super::super::scale_for(canvas, super::WORK_PX);
    let n = fw.div_ceil(step).checked_mul(fh.div_ceil(step)).ok_or(E::LimitExceeded)?;
    let strip = fw.checked_mul(64usize.div_ceil(step).max(1).checked_mul(step).ok_or(E::LimitExceeded)?.min(fh)).ok_or(E::LimitExceeded)?;
    let bytes = (n as u64)
        .checked_mul(384)
        .and_then(|n| n.checked_add((full as u64).checked_mul(4)?))
        .and_then(|n| n.checked_add((strip as u64).checked_mul(160)?))
        .and_then(|n| n.checked_add(65536))
        .ok_or(E::LimitExceeded)?;
    let work = (n as u64).checked_mul(2048).and_then(|n| n.checked_add((full as u64).checked_mul(128)?)).ok_or(E::LimitExceeded)?;
    if bytes > l.resident_bytes || work > l.work_units {
        return Err(E::LimitExceeded);
    }
    Ok((fw, fh, step))
}
/// Reader is caller-authorized; no profile or ownership policy is inferred here.
pub fn focus_area_with_interrupt(
    canvas: Rect,
    range: f32,
    noise: f32,
    l: FocusLimits,
    c: &Interrupt<'_>,
    mut read: impl FnMut(Rect, &Interrupt<'_>) -> Result<Vec<[f32; 4]>, E>,
) -> Result<Option<Region>, E> {
    poll(c)?;
    if !range.is_finite() || !noise.is_finite() {
        return Err(E::InvalidSample);
    }
    let (fw, fh, step) = requirements(canvas, l)?;
    if fw < 4 || fh < 4 {
        return Ok(None);
    }
    let (w, h) = (fw.div_ceil(step), fh.div_ceil(step));
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let mut img = RgbImage { w, h, px: buffer(n, [0.0; 3], l, c)? };
    // Same strip boundaries and box accumulation order as Sampler::rgb_scaled.
    let rows_per = 64usize.div_ceil(step).max(1);
    for oy0 in (0..h).step_by(rows_per) {
        poll(c)?;
        let oy1 = oy0.saturating_add(rows_per).min(h);
        let dy0 = oy0.checked_mul(step).ok_or(E::LimitExceeded)?;
        let dy1 = oy1.checked_mul(step).ok_or(E::LimitExceeded)?.min(fh);
        let y0 = i32::try_from(i64::from(canvas.y0) + i64::try_from(dy0).map_err(|_| E::InvalidBounds)?).map_err(|_| E::InvalidBounds)?;
        let y1 = i32::try_from(i64::from(canvas.y0) + i64::try_from(dy1).map_err(|_| E::InvalidBounds)?).map_err(|_| E::InvalidBounds)?;
        let rgba = read(Rect::new(canvas.x0, y0, canvas.x1, y1), c)?;
        let expected = fw.checked_mul(dy1 - dy0).ok_or(E::LimitExceeded)?;
        if rgba.len() != expected || rgba.capacity() > expected {
            return Err(E::LengthMismatch);
        }
        let mut rgb = buffer(expected, [0.0; 3], l, c)?;
        for (i, (v, p)) in rgb.iter_mut().zip(&rgba).enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            if p.iter().any(|v| !v.is_finite()) {
                return Err(E::InvalidSample);
            }
            let [r, g, b, a] = *p;
            let a = a.clamp(0.0, 1.0);
            *v = [r, g, b].map(|v| v.clamp(0.0, 1.0) * a + 0.5 * (1.0 - a));
        }
        drop(rgba);
        for oy in oy0..oy1 {
            poll(c)?;
            for ox in 0..w {
                let mut acc = [0.0f32; 3];
                let mut count = 0usize;
                for sy in (oy - oy0) * step..((oy - oy0 + 1) * step).min(dy1 - dy0) {
                    for sx in ox * step..((ox + 1) * step).min(fw) {
                        if count.is_multiple_of(1024) {
                            poll(c)?;
                        }
                        let p = rgb.get(sy * fw + sx).ok_or(E::LengthMismatch)?;
                        for (a, v) in acc.iter_mut().zip(p) {
                            *a += v;
                        }
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(E::LengthMismatch);
                }
                *img.px.get_mut(oy * w + ox).ok_or(E::LengthMismatch)? = acc.map(|v| v / count as f32);
            }
        }
    }
    let mut y = buffer(n, 0.0, l, c)?;
    for (i, (o, p)) in y.iter_mut().zip(&img.px).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let [r, g, b] = *p;
        *o = 0.299 * r + 0.587 * g + 0.114 * b;
    }
    let mut lap = buffer(n, 0.0, l, c)?;
    for yy in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let at = |x, yy| value(&y, yy * w + x);
            let center = at(x, yy)?;
            *lap.get_mut(yy * w + x).ok_or(E::LengthMismatch)? = (2.0 * center - at(x.saturating_sub(1), yy)? - at((x + 1).min(w - 1), yy)?).abs()
                + (2.0 * center - at(x, yy.saturating_sub(1))? - at(x, (yy + 1).min(h - 1))?).abs();
        }
    }
    drop(y);
    let ml = MattingLimits { resident_bytes: l.resident_bytes, work_units: l.work_units };
    let mut sharp = box_mean_with_interrupt(&lap, w, h, (w.min(h) / 60).max(3), ml, c)?;
    drop(lap);
    let mut sorted = buffer(n, 0.0, l, c)?;
    for (a, b) in sorted.chunks_mut(4096).zip(sharp.chunks(4096)) {
        poll(c)?;
        a.copy_from_slice(b);
    }
    let k = ((n as f32 * 0.99) as usize).min(n.saturating_sub(1));
    let mut cancelled = false;
    // Selection is bounded by the admitted working shape. A latched cancellation is reported before
    // any later phase; comparator polling preserves total_cmp for a live token.
    let p99 = *sorted
        .select_nth_unstable_by(k, |a, b| {
            cancelled |= c.cancelled();
            a.total_cmp(b)
        })
        .1;
    if cancelled {
        return Err(E::Cancelled);
    }
    poll(c)?;
    drop(sorted);
    let floor = noise.clamp(0.0, 1.0) * 0.5;
    let threshold = 0.05 + 0.9 * (1.0 - range.clamp(0.0, 1.0)).powi(2);
    for (i, v) in sharp.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = ((*v / p99.max(1e-6)).min(1.0) - floor).max(0.0) / (1.0 - floor).max(1e-3);
        *v = if *v >= threshold { 1.0 } else { 0.0 };
    }
    let smooth = guided_filter_color_with_interrupt(&img, &sharp, (w.min(h) / 50).max(2), 1e-2, ml, c)?;
    drop(img);
    drop(sharp);
    let mut mask = buffer(n, false, l, c)?;
    for (i, (m, v)) in mask.iter_mut().zip(&smooth).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *m = *v >= 0.5;
    }
    let mask = clean_mask_with_interrupt(&mask, w, h, 0.05, 0.02, CleanupLimits { resident_bytes: l.resident_bytes, work_units: l.work_units }, c)?;
    let mut any = false;
    for chunk in mask.chunks(4096) {
        poll(c)?;
        any |= chunk.iter().any(|v| *v);
    }
    if !any {
        return Ok(None);
    }
    let mut field = buffer(n, 0.0, l, c)?;
    for (i, ((o, m), v)) in field.iter_mut().zip(mask).zip(smooth).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = if m { v.clamp(0.5, 1.0) } else { v.clamp(0.0, 0.5) };
    }
    let mut out = buffer(fw.checked_mul(fh).ok_or(E::LimitExceeded)?, 0u8, l, c)?;
    let st = step as f32;
    for yy in 0..fh {
        poll(c)?;
        let fy = ((yy as f32 + 0.5) / st - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(h - 1);
        for xx in 0..fw {
            if xx.is_multiple_of(1024) {
                poll(c)?;
            }
            let fx = ((xx as f32 + 0.5) / st - 0.5).clamp(0.0, (w - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(w - 1);
            let top = value(&field, y0 * w + x0)? * (1.0 - tx) + value(&field, y0 * w + x1)? * tx;
            let bot = value(&field, y1 * w + x0)? * (1.0 - tx) + value(&field, y1 * w + x1)? * tx;
            let v = top * (1.0 - ty) + bot * ty;
            *out.get_mut(yy * fw + xx).ok_or(E::LengthMismatch)? = (((v - 0.5) * 3.0 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    trim(Region { bbox: canvas, mask: out }, l, c)
}
fn trim(r: Region, l: FocusLimits, c: &Interrupt<'_>) -> Result<Option<Region>, E> {
    let w = r.bbox.width() as usize;
    let h = r.bbox.height() as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        poll(c)?;
        let row = r.mask.get(y * w..(y + 1) * w).ok_or(E::LengthMismatch)?;
        for (x, v) in row.iter().enumerate() {
            if x.is_multiple_of(4096) {
                poll(c)?;
            }
            if *v != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y + 1;
            }
        }
    }
    if x0 == usize::MAX {
        return Ok(None);
    }
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        return Ok(Some(r));
    }
    let nw = x1 - x0;
    let mut mask = buffer(nw.checked_mul(y1 - y0).ok_or(E::LimitExceeded)?, 0u8, l, c)?;
    for (y, row) in mask.chunks_mut(nw).enumerate() {
        poll(c)?;
        let source = r.mask.get((y + y0) * w + x0..(y + y0) * w + x1).ok_or(E::LengthMismatch)?;
        for (a, b) in row.chunks_mut(4096).zip(source.chunks(4096)) {
            poll(c)?;
            a.copy_from_slice(b);
        }
    }
    let coord = |base: i32, d: usize| i32::try_from(i64::from(base) + i64::try_from(d).map_err(|_| E::InvalidBounds)?).map_err(|_| E::InvalidBounds);
    Ok(Some(Region { bbox: Rect::new(coord(r.bbox.x0, x0)?, coord(r.bbox.y0, y0)?, coord(r.bbox.x0, x1)?, coord(r.bbox.y0, y1)?), mask }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{ImageSampler, Sampler};
    #[test]
    fn exact_focus_region_matches_upstream_equations() {
        let img = RgbImage::from_fn(73, 49, |x, y| {
            let v = ((x * 37 + y * 17 + x * y) % 251) as f32 / 251.0;
            [v, v * 0.7, 1.0 - v]
        });
        let sampler = ImageSampler { img: &img, origin: (-7, 3) };
        let rect = Rect::new(-7, 3, 66, 52);
        for (range, noise) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.2)] {
            let expected = super::super::focus_area(&sampler, rect, range, noise).unwrap();
            let got = focus_area_with_interrupt(rect, range, noise, FocusLimits::default(), &Interrupt::NONE, |r, _| sampler.rgba(r)).unwrap();
            assert_eq!(got.as_ref().map(|r| r.bbox), expected.as_ref().map(|r| r.bbox));
            assert_eq!(got.as_ref().map(|r| &r.mask), expected.as_ref().map(|r| &r.mask));
        }
    }
    #[test]
    fn refusal_and_cancellation_precede_reader() {
        let rect = Rect::new(0, 0, 32, 32);
        let limited = FocusLimits { resident_bytes: 1, work_units: 1 };
        assert!(matches!(
            focus_area_with_interrupt(rect, 0.5, 0.0, limited, &Interrupt::NONE, |_, _| panic!("reader called before admission")),
            Err(E::LimitExceeded)
        ));
        assert!(matches!(
            focus_area_with_interrupt(rect, 0.5, 0.0, FocusLimits::default(), &Interrupt::cancel_only(&|| true), |_, _| panic!("reader called after cancel")),
            Err(E::Cancelled)
        ));
    }
    #[test]
    fn cancellation_during_analysis_does_not_return_a_region() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) > 100;
        let c = Interrupt::cancel_only(&cancel);
        let rect = Rect::new(0, 0, 64, 64);
        let result = focus_area_with_interrupt(rect, 0.5, 0.0, FocusLimits::default(), &c, |r, _| {
            Ok(vec![[0.25, 0.5, 0.75, 1.0]; r.width() as usize * r.height() as usize])
        });
        assert!(matches!(result, Err(E::Cancelled)));
    }
}
