//! Bounded exact Refine Edge Brush construction and color-line estimate.
use super::{EdgeBrush, EdgeStroke, MAX_SWEPT_AREA, capsule, dab_bounds};
use crate::matting::{MattingLimits, box_mean_with_interrupt};
use crate::segment::RgbImage;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
const MAX_WORK: u64 = 8_000_000_000;
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Clone>(n: usize, z: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    crate::matting::controlled_gray::filled(n, z, c)
}
fn add(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::LimitExceeded)
}
fn area(r: Rect) -> Result<u64, E> {
    mul(u64::from(r.width()), u64::from(r.height()))
}
fn at(v: &[f32], i: usize) -> Result<f32, E> {
    v.get(i).copied().ok_or(E::LengthMismatch)
}
fn bounds(s: &EdgeStroke, canvas: Rect, c: &Interrupt<'_>) -> Result<Rect, E> {
    let r = s.size.clamp(1., 5000.) / 2.;
    let mut b = Rect::EMPTY;
    for p in &s.points {
        poll(c)?;
        b = b.union(&dab_bounds(*p, r));
    }
    Ok(b.intersect(&canvas))
}
/// Visit the exact kept polyline, including the final endpoint and single dab.
/// No subdivision Vec is retained; callback computes the same original pieces.
fn lines(s: &EdgeStroke, c: &Interrupt<'_>, mut f: impl FnMut((f32, f32), (f32, f32)) -> Result<(), E>) -> Result<(), E> {
    let first = *s.points.first().ok_or(E::LengthMismatch)?;
    let r = s.size.clamp(1., 5000.) / 2.;
    let min = (r / 4.).max(0.5);
    let mut last = first;
    let mut count = 1usize;
    for p in s.points.iter().skip(1) {
        poll(c)?;
        if (p.0 - last.0).hypot(p.1 - last.1) >= min {
            f(last, *p)?;
            last = *p;
            count += 1;
        }
    }
    let end = *s.points.last().ok_or(E::LengthMismatch)?;
    if last != end {
        f(last, end)?;
        count += 1;
    }
    if count == 1 {
        f(first, first)?;
    }
    poll(c)
}
fn pieces(a: (f32, f32), b: (f32, f32), r: f32) -> usize {
    ((b.0 - a.0).hypot(b.1 - a.1) / r.max(2.)).ceil().clamp(1., 1e6) as usize
}
#[derive(Clone, Copy, Debug)]
pub struct BrushRequirements {
    pub bounds: Rect,
    pub possible_added_bounds: Rect,
    pub window: usize,
    pub parameter_bytes: u64,
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
impl EdgeBrush {
    /// Borrowed bounded census before the first coverage/segment allocation.
    pub fn requirements_with_interrupt(strokes: &[EdgeStroke], canvas: Rect, c: &Interrupt<'_>) -> Result<BrushRequirements, E> {
        poll(c)?;
        if strokes.len() > 4096
            || canvas.x1 < canvas.x0
            || canvas.y1 < canvas.y0
            || canvas.width() > 300000
            || canvas.height() > 300000
            || [canvas.x0, canvas.x1, canvas.y0, canvas.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
        {
            return Err(E::InvalidBounds);
        }
        let (mut count, mut bbox, mut possible, mut largest, mut work, mut swept, mut max_add) = (0usize, Rect::EMPTY, Rect::EMPTY, 0u64, 0u64, 0f64, 0f32);
        for s in strokes {
            poll(c)?;
            if s.points.is_empty() || !s.size.is_finite() || !(1.0..=5000.0).contains(&s.size) {
                return Err(E::InvalidSample);
            }
            count = count.checked_add(s.points.len()).filter(|v| *v <= 100000).ok_or(E::LimitExceeded)?;
            for p in &s.points {
                poll(c)?;
                if !p.0.is_finite() || !p.1.is_finite() || p.0.abs() > 1e7 || p.1.abs() > 1e7 {
                    return Err(E::InvalidSample);
                }
            }
            let mut length = 0f64;
            for p in s.points.windows(2) {
                poll(c)?;
                let a = *p.first().ok_or(E::LengthMismatch)?;
                let b = *p.get(1).ok_or(E::LengthMismatch)?;
                length += f64::from((b.0 - a.0).hypot(b.1 - a.1));
            }
            let size = f64::from(s.size);
            swept += length * size + size * size;
            if !swept.is_finite() || swept > MAX_SWEPT_AREA {
                return Err(E::LimitExceeded);
            }
            let sb = bounds(s, canvas, c)?;
            bbox = bbox.union(&sb);
            if sb.is_empty() {
                continue;
            }
            if s.add {
                possible = possible.union(&sb);
                max_add = max_add.max(s.size);
            }
            let n = area(sb)?;
            largest = largest.max(n);
            work = add(work, mul(n, 4)?)?;
            let r = s.size / 2.;
            let mut visits = 0u64;
            lines(s, c, |a, b| {
                let count = pieces(a, b, r);
                // The original caps subdivisions at 1e6; long lines can thus
                // produce pieces longer than max_step. Charge that real bound.
                let length = (b.0 - a.0).hypot(b.1 - a.1) / count as f32;
                let side = (length + 2. * r + 16.).ceil() as u64;
                visits = add(visits, mul(count as u64, add(mul(side, side)?, 4)?)?)?;
                if visits > MAX_WORK {
                    return Err(E::LimitExceeded);
                }
                Ok(())
            })?;
            work = add(work, visits)?;
        }
        let retained = area(bbox)?;
        let parameters = add(mul(count as u64, 8)?, mul(strokes.len() as u64, std::mem::size_of::<EdgeStroke>() as u64)?)?;
        let bytes = add(add(retained, mul(largest, 4)?)?, add(parameters, 65536)?)?;
        work = add(work, add(mul(retained, 8)?, mul(count as u64, 16)?)?)?;
        if bytes > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
            return Err(E::LimitExceeded);
        }
        poll(c)?;
        Ok(BrushRequirements {
            bounds: bbox,
            possible_added_bounds: possible,
            window: max_add.ceil().clamp(4., 64.) as usize,
            parameter_bytes: parameters,
            retained_bytes: retained,
            peak_resident_bytes: bytes,
            work_units: work,
        })
    }
    pub fn new_with_interrupt(strokes: &[EdgeStroke], canvas: Rect, l: MattingLimits, c: &Interrupt<'_>) -> Result<Option<Self>, E> {
        let req = Self::requirements_with_interrupt(strokes, canvas, c)?;
        if req.peak_resident_bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > l.work_units.min(MAX_WORK) {
            return Err(E::LimitExceeded);
        }
        let bbox = req.bounds;
        if bbox.is_empty() {
            return Ok(None);
        }
        let bw = bbox.width() as usize;
        let mut v = filled(req.retained_bytes as usize, 0i8, c)?;
        let mut remaining = req.work_units;
        let mut tick = || -> Result<(), E> {
            poll(c)?;
            remaining = remaining.checked_sub(1).ok_or(E::LimitExceeded)?;
            Ok(())
        };
        for s in strokes {
            poll(c)?;
            let sb = bounds(s, canvas, c)?;
            if sb.is_empty() {
                continue;
            }
            let r = s.size / 2.;
            let sw = sb.width() as usize;
            let mut cov = filled(area(sb)? as usize, 0f32, c)?;
            lines(s, c, |a, b| {
                let count = pieces(a, b, r);
                let at = |k: usize| {
                    let t = k as f32 / count as f32;
                    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
                };
                for k in 0..count {
                    tick()?;
                    let (a, b) = (at(k), at(k + 1));
                    let seg = dab_bounds(a, r).union(&dab_bounds(b, r)).intersect(&sb);
                    for y in seg.y0..seg.y1 {
                        poll(c)?;
                        for x in seg.x0..seg.x1 {
                            tick()?;
                            let c = capsule((x as f32 + 0.5, y as f32 + 0.5), a, b, r);
                            let slot = cov.get_mut((y - sb.y0) as usize * sw + (x - sb.x0) as usize).ok_or(E::LengthMismatch)?;
                            if c > *slot {
                                *slot = c;
                            }
                        }
                    }
                }
                Ok(())
            })?;
            let target = if s.add { 1. } else { -1. };
            for y in sb.y0..sb.y1 {
                poll(c)?;
                for x in sb.x0..sb.x1 {
                    tick()?;
                    let c = *cov.get((y - sb.y0) as usize * sw + (x - sb.x0) as usize).ok_or(E::LengthMismatch)?;
                    if c <= 0. {
                        continue;
                    }
                    let slot = v.get_mut((y - bbox.y0) as usize * bw + (x - bbox.x0) as usize).ok_or(E::LengthMismatch)?;
                    let cur = f32::from(*slot) / 127.;
                    *slot = ((cur + (target - cur) * c) * 127.).round().clamp(-127., 127.) as i8;
                }
            }
        }
        let mut any = false;
        for chunk in v.chunks(1024) {
            poll(c)?;
            any |= chunk.iter().any(|v| *v != 0);
        }
        if !any {
            return Ok(None);
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (row, line) in v.chunks_exact(bw).enumerate() {
            poll(c)?;
            let (mut first, mut last) = (None, None);
            for (x, value) in line.iter().enumerate() {
                if x.is_multiple_of(1024) {
                    poll(c)?;
                }
                if *value > 0 {
                    if first.is_none() {
                        first = Some(x);
                    }
                    last = Some(x);
                }
            }
            if let (Some(a), Some(b)) = (first, last) {
                let y = bbox.y0 + row as i32;
                (x0, x1, y0, y1) = (x0.min(bbox.x0 + a as i32), x1.max(bbox.x0 + b as i32 + 1), y0.min(y), y1.max(y + 1));
            }
        }
        let added = if x0 < x1 { Rect::new(x0, y0, x1, y1) } else { Rect::EMPTY };
        poll(c)?;
        Ok(Some(Self { bbox, v, added, window: req.window }))
    }
    pub fn read_with_interrupt(&self, r: Rect, l: MattingLimits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
        poll(c)?;
        if r.x1 < r.x0 || r.y1 < r.y0 || r.width() > 300000 || r.height() > 300000 {
            return Err(E::InvalidBounds);
        }
        let n = area(r)?;
        if mul(n, 4)? > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || n > l.work_units.min(MAX_WORK) {
            return Err(E::LimitExceeded);
        }
        let mut out = filled(n as usize, 0f32, c)?;
        let w = r.width() as usize;
        for y in r.y0..r.y1 {
            poll(c)?;
            for x in r.x0..r.x1 {
                if x.rem_euclid(1024) == 0 {
                    poll(c)?;
                }
                *out.get_mut((y - r.y0) as usize * w + (x - r.x0) as usize).ok_or(E::LengthMismatch)? = self.at(x, y);
            }
        }
        poll(c)?;
        Ok(out)
    }
}
fn means(img: &RgbImage, wt: &[f32], r: usize, l: MattingLimits, c: &Interrupt<'_>) -> Result<(Vec<f32>, [Vec<f32>; 3]), E> {
    let n = img.px.len();
    let channel = |ch: usize| -> Result<Vec<f32>, E> {
        let mut v = filled(n, 0f32, c)?;
        for (i, value) in v.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *value = img.px.get(i).and_then(|p| p.get(ch)).copied().ok_or(E::LengthMismatch)? * at(wt, i)?;
        }
        box_mean_with_interrupt(&v, img.w, img.h, r, l, c)
    };
    let sums = [channel(0)?, channel(1)?, channel(2)?];
    Ok((box_mean_with_interrupt(wt, img.w, img.h, r, l, c)?, sums))
}
pub(crate) fn estimate_with_interrupt(
    img: &RgbImage,
    m: &[f32],
    brush: &[f32],
    w: usize,
    h: usize,
    window: usize,
    l: MattingLimits,
    c: &Interrupt<'_>,
) -> Result<Vec<f32>, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if n == 0 || w > 300000 || h > 300000 || img.w != w || img.h != h || img.px.len() != n || m.len() != n || brush.len() != n || !(4..=64).contains(&window) {
        return Err(E::LengthMismatch);
    }
    if mul(n as u64, 256)? > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || mul(n as u64, 1024)? > l.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    let (mut wf, mut wb) = (filled(n, 0f32, c)?, filled(n, 0f32, c)?);
    for i in 0..n {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let a = at(m, i)?;
        let b = at(brush, i)?;
        if !a.is_finite() || !b.is_finite() || img.px.get(i).ok_or(E::LengthMismatch)?.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        let known = b <= 0.;
        *wf.get_mut(i).ok_or(E::LengthMismatch)? = if known && a >= 0.98 { 1. } else { 0. };
        *wb.get_mut(i).ok_or(E::LengthMismatch)? = if known && a <= 0.02 { 1. } else { 0. };
    }
    let near = [means(img, &wf, window, l, c)?, means(img, &wb, window, l, c)?];
    let far = [means(img, &wf, window * 4, l, c)?, means(img, &wb, window * 4, l, c)?];
    let mean = |kind: usize, i: usize| -> Result<Option<[f32; 3]>, E> {
        for pair in [near.get(kind), far.get(kind)] {
            let pair = pair.ok_or(E::LengthMismatch)?;
            let d = at(&pair.0, i)?;
            if d >= 1e-4 {
                return Ok(Some([at(&pair.1[0], i)? / d, at(&pair.1[1], i)? / d, at(&pair.1[2], i)? / d]));
            }
        }
        Ok(None)
    };
    let mut out = filled(n, 0f32, c)?;
    for (i, o) in out.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = at(m, i)?;
        if at(brush, i)? <= 0. {
            continue;
        }
        let (Some(fm), Some(bm)) = (mean(0, i)?, mean(1, i)?) else { continue };
        let px = img.px.get(i).ok_or(E::LengthMismatch)?;
        let (mut num, mut den) = (0f32, 0f32);
        for ((f, b), p) in fm.iter().zip(&bm).zip(px) {
            num += (p - b) * (f - b);
            den += (f - b) * (f - b);
        }
        if den > 1e-4 {
            *o = (num / den).clamp(0., 1.);
        }
        if !o.is_finite() {
            return Err(E::InvalidSample);
        }
    }
    poll(c)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simplify_last_endpoint_subdivision_and_order_match_original() {
        let canvas = Rect::new(-3, -7, 127, 71);
        let add = EdgeStroke { points: vec![(2., 4.), (2.1, 4.1), (22., 17.), (90., 55.), (90.2, 55.1)], size: 13., add: true };
        let subtract = EdgeStroke { points: vec![(19., 14.), (63., 39.)], size: 21., add: false };
        let off = EdgeStroke { points: vec![(-500., -700.)], size: 3., add: true };
        let strokes = vec![add, subtract, off];
        let original = EdgeBrush::new(&strokes, canvas).unwrap();
        let controlled = EdgeBrush::new_with_interrupt(&strokes, canvas, MattingLimits::default(), &Interrupt::NONE).unwrap().unwrap();
        assert_eq!(controlled, original);
        assert_eq!(controlled.read_with_interrupt(canvas, MattingLimits::default(), &Interrupt::NONE).unwrap(), original.read(canvas));
    }
    #[test]
    fn one_dab_color_line_near_far_and_late_cancel_are_real() {
        let w = 120;
        let h = 60;
        let img = RgbImage::from_fn(w, h, |x, _| if x < 40 || ((60..84).contains(&x) && x % 4 == 0) { [0.9, 0.7, 0.2] } else { [0.1, 0.3, 0.8] });
        let canvas = Rect::new(0, 0, 120, 60);
        let strokes = [EdgeStroke { points: vec![(70., 30.)], size: 30., add: true }];
        let brush = EdgeBrush::new_with_interrupt(&strokes, canvas, MattingLimits::default(), &Interrupt::NONE).unwrap().unwrap();
        let b = brush.read(canvas);
        let m = (0..w * h).map(|i| if i % w < 40 { 1. } else { 0. }).collect::<Vec<_>>();
        let out = estimate_with_interrupt(&img, &m, &b, w, h, brush.window(), MattingLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(out, super::super::estimate(&img, &m, &b, w, h, brush.window()));
        let count = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 100;
        assert!(matches!(EdgeBrush::new_with_interrupt(&strokes, canvas, MattingLimits::default(), &Interrupt::cancel_only(&cancel)), Err(E::Cancelled)));
    }
    #[test]
    fn low_caps_refuse_before_the_first_subdivision_walk() {
        let s = [EdgeStroke { points: vec![(-1e7, 0.), (1e7, 0.)], size: 1., add: true }];
        let req = EdgeBrush::requirements_with_interrupt(&s, Rect::new(0, 0, 300, 30), &Interrupt::NONE).unwrap();
        assert!(req.work_units > 1000000);
        assert!(matches!(
            EdgeBrush::new_with_interrupt(
                &s,
                Rect::new(0, 0, 300, 30),
                MattingLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1000 },
                &Interrupt::NONE
            ),
            Err(E::LimitExceeded)
        ));
    }
}
