//! Exact current Quick Selection solver with bounded private storage and cancellation.
//! Sequential increasing-window attempts retain the first kept legacy window;
//! speculative larger attempts have no mathematical effect on that winner.
use super::{Tuning, working_step};
use crate::segment::{
    FREE, HARD_BG, HARD_FG, Region, RgbImage, Sampler,
    controlled_cut::{Budget, cut},
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
use std::cell::Cell;
const MAX_WORK: u64 = 8_000_000_000;
#[derive(Clone, Copy, Debug)]
pub struct QuickLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for QuickLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct QuickRequirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub attempts: usize,
    pub step: usize,
}
fn plus(a: u64, b: u64) -> Result<u64, E> {
    a.checked_add(b).ok_or(E::LimitExceeded)
}
fn times(a: u64, b: u64) -> Result<u64, E> {
    a.checked_mul(b).ok_or(E::LimitExceeded)
}
fn at<T: Copy>(v: &[T], i: usize) -> Result<T, E> {
    v.get(i).copied().ok_or(E::LengthMismatch)
}
fn bbox(canvas: Rect, points: &[(f32, f32)], size: f32, c: &Interrupt<'_>) -> Result<Rect, E> {
    c.check().map_err(|_| E::Cancelled)?;
    if points.is_empty()
        || points.len() > 100000
        || !size.is_finite()
        || !(1.0..=5000.0).contains(&size)
        || canvas.x1 < canvas.x0
        || canvas.y1 < canvas.y0
        || canvas.width() > 300000
        || canvas.height() > 300000
        || [canvas.x0, canvas.y0, canvas.x1, canvas.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
    {
        return Err(E::InvalidBounds);
    }
    let r = size.max(1.) / 2.;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        c.check().map_err(|_| E::Cancelled)?;
        if !x.is_finite() || !y.is_finite() || x.abs() > 1e7 || y.abs() > 1e7 {
            return Err(E::InvalidSample);
        }
        x0 = x0.min(x - r);
        y0 = y0.min(y - r);
        x1 = x1.max(x + r);
        y1 = y1.max(y + r);
    }
    let clamp = |v: f32| v.clamp(i32::MIN as f32 / 2., i32::MAX as f32 / 2.) as i32;
    Ok(Rect::new(clamp(x0.floor()), clamp(y0.floor()), clamp(x1.ceil()).saturating_add(1), clamp(y1.ceil()).saturating_add(1)).intersect(&canvas))
}
/// Metadata-only first-window admission. Later doubled windows are admitted
/// individually against caller-authorized lower-only limits before any sampling.
pub fn requirements(canvas: Rect, points: &[(f32, f32)], size: f32, c: &Interrupt<'_>) -> Result<QuickRequirements, E> {
    let bb = bbox(canvas, points, size, c)?;
    let step = working_step(canvas.width() as usize, canvas.height() as usize);
    if bb.is_empty() {
        return Ok(QuickRequirements { peak_resident_bytes: 0, work_units: 1, attempts: 0, step });
    }
    let rw = size / 2. / step as f32;
    let margin = 48f32.max(2. * rw) as i32 * step as i32;
    let (first_bytes, first_work) = window_requirements(window(bb, canvas, margin, step), step, points.len())?;
    let mut margin = margin;
    let mut attempts = 0usize;
    loop {
        c.check().map_err(|_| E::Cancelled)?;
        attempts += 1;
        if attempts > 32 {
            return Err(E::LimitExceeded);
        }
        if window(bb, canvas, margin, step) == canvas {
            break;
        }
        margin = margin.checked_mul(2).ok_or(E::LimitExceeded)?;
    }
    Ok(QuickRequirements { peak_resident_bytes: first_bytes, work_units: first_work, attempts, step })
}
fn window_requirements(win: Rect, step: usize, point_count: usize) -> Result<(u64, u64), E> {
    let n = times(win.width().div_ceil(step as u32) as u64, win.height().div_ceil(step as u32) as u64)?;
    let full = times(win.width() as u64, win.height() as u64)?;
    let peak = plus(plus(times(n, 640)?, times(full, 20)?)?, plus(times(point_count as u64, 16)?, 65536)?)?;
    let work = plus(times(n, 4096)?, plus(times(full, 16)?, times(n, point_count.saturating_sub(1).max(1) as u64)?)?)?;
    if peak > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok((peak, work))
}

fn window(bb: Rect, canvas: Rect, margin: i32, step: usize) -> Rect {
    let grown = bb.inflate(margin).intersect(&canvas);
    let s = step as i32;
    let snap = |v: i32, o: i32| o + (v - o).div_euclid(s) * s;
    Rect::new(snap(grown.x0, canvas.x0), snap(grown.y0, canvas.y0), grown.x1, grown.y1)
}
fn rgb(s: &dyn Sampler, r: Rect, b: &Budget<'_, '_>) -> Result<RgbImage, E> {
    let (w, h, n) = crate::segment::sample_shape(r, 16)?;
    b.ensure_work((n as u64).checked_mul(2).ok_or(E::LimitExceeded)?)?;
    b.work(n as u64)?;
    let rgba = s.rgba_with_interrupt(r, b.control)?;
    if rgba.len() != n {
        return Err(E::LengthMismatch);
    }
    let mut px = b.reserve(n)?;
    for p in rgba {
        b.work(1)?;
        if p.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        let a = p[3].clamp(0., 1.);
        px.push([p[0], p[1], p[2]].map(|v| v.clamp(0., 1.) * a + 0.5 * (1. - a)));
    }
    Ok(RgbImage { w, h, px })
}
// Same legacy 64-source-row strip grouping and f32 cell summation order.
fn scaled(s: &dyn Sampler, r: Rect, step: usize, b: &Budget<'_, '_>) -> Result<RgbImage, E> {
    if step == 0 {
        return Err(E::InvalidBounds);
    }
    if step == 1 {
        return rgb(s, r, b);
    }
    let (w, h, _) = crate::segment::sample_shape(r, 1)?;
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut px = b.reserve(ow.checked_mul(oh).ok_or(E::LimitExceeded)?)?;
    let rows = 64usize.div_ceil(step).max(1);
    for oy0 in (0..oh).step_by(rows) {
        b.work(1)?;
        let oy1 = oy0.saturating_add(rows).min(oh);
        let dy0 = oy0.checked_mul(step).ok_or(E::LimitExceeded)?.min(h);
        let dy1 = oy1.checked_mul(step).ok_or(E::LimitExceeded)?.min(h);
        let y = |v: usize| i32::try_from(i64::from(r.y0) + i64::try_from(v).map_err(|_| E::InvalidBounds)?).map_err(|_| E::InvalidBounds);
        let strip = rgb(s, Rect::new(r.x0, y(dy0)?, r.x1, y(dy1)?), b)?;
        for sy in (0..strip.h).step_by(step) {
            for sx in (0..w).step_by(step) {
                let mut acc = [0f32; 3];
                let mut count = 0usize;
                for yy in sy..sy.saturating_add(step).min(strip.h) {
                    for xx in sx..sx.saturating_add(step).min(w) {
                        b.work(1)?;
                        for (a, v) in acc.iter_mut().zip(at(&strip.px, yy * strip.w + xx)?) {
                            *a += v;
                        }
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(E::LengthMismatch);
                }
                px.push(acc.map(|v| v / count as f32));
            }
        }
    }
    if px.len() != ow.checked_mul(oh).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    Ok(RgbImage { w: ow, h: oh, px })
}

fn seeds(points: &[(f32, f32)], radius: f32, w: usize, h: usize, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let mut m = b.filled(w.checked_mul(h).ok_or(E::LimitExceeded)?, false)?;
    let mut segment = |a: (f32, f32), z: (f32, f32)| -> Result<(), E> {
        let x0 = (a.0.min(z.0) - radius).floor().max(0.) as usize;
        let y0 = (a.1.min(z.1) - radius).floor().max(0.) as usize;
        let x1 = ((a.0.max(z.0) + radius).ceil().max(0.) as usize).min(w);
        let y1 = ((a.1.max(z.1) + radius).ceil().max(0.) as usize).min(h);
        let (dx, dy) = (z.0 - a.0, z.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in y0..y1 {
            for x in x0..x1 {
                b.work(1)?;
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 > 0. { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0., 1.) } else { 0. };
                let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
                if qx * qx + qy * qy <= radius * radius {
                    *m.get_mut(y * w + x).ok_or(E::LengthMismatch)? = true;
                }
            }
        }
        Ok(())
    };
    if points.len() == 1 {
        let a = at(points, 0)?;
        segment(a, a)?;
    } else {
        for pair in points.windows(2) {
            segment(at(pair, 0)?, at(pair, 1)?)?;
        }
    }
    for &(x, y) in points {
        b.work(1)?;
        if x >= 0. && y >= 0. && (x as usize) < w && (y as usize) < h {
            *m.get_mut(y as usize * w + x as usize).ok_or(E::LengthMismatch)? = true;
        }
    }
    Ok(m)
}
fn distance(seeds: &[bool], w: usize, h: usize, b: &Budget<'_, '_>) -> Result<Vec<f32>, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let work = times(n as u64, 128)?;
    b.work(work)?;
    crate::selection_modify_controlled::edt_with_interrupt(seeds, w, h, crate::selection_modify_controlled::Limits::default(), b.control)
}
fn working(img: &RgbImage, seeds: &[bool], radius: f32, t: &Tuning, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let d = distance(seeds, img.w, img.h, b)?;
    let grow = (t.grow + t.grow_frac * radius).max(0.);
    let mut cf = b.reserve(d.len())?;
    let mut fixed = b.reserve(d.len())?;
    for v in d {
        b.work(1)?;
        cf.push(t.area * (1. + t.dist * (v - grow).max(0.).powf(t.pow)));
        fixed.push(if v <= grow { HARD_FG } else { FREE });
    }
    let sigma = t.sigma.max(1e-4);
    let weight = |d2: f32| t.floor + 1. / (1. + (d2.sqrt() / sigma).powf(t.k));
    levels(img, &fixed, &cf, &weight, 1., t.direct_px.max(1024), 0, b)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
fn levels(
    img: &RgbImage,
    fixed: &[u8],
    cf: &[f32],
    weight: &dyn Fn(f32) -> f32,
    scale: f32,
    direct: usize,
    depth: usize,
    b: &Budget<'_, '_>,
) -> Result<Vec<bool>, E> {
    b.work(1)?;
    if depth > 32 {
        return Err(E::LimitExceeded);
    }
    let (w, h) = (img.w, img.h);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let zeros = b.filled(n, 0f32)?;
    if n <= direct || w < 8 || h < 8 {
        return cut(img, cf, &zeros, fixed, &|d2| scale * weight(d2), b);
    }
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let cn = cw.checked_mul(ch).ok_or(E::LimitExceeded)?;
    let mut px = b.filled(cn, [0f32; 3])?;
    // Identical y/x/channel box-average order of RgbImage::downsample(2).
    for oy in 0..ch {
        for ox in 0..cw {
            let mut acc = [0f32; 3];
            let mut count = 0f32;
            for y in oy * 2..((oy + 1) * 2).min(h) {
                for x in ox * 2..((ox + 1) * 2).min(w) {
                    b.work(1)?;
                    let v = at(&img.px, y * w + x)?;
                    for (a, v) in acc.iter_mut().zip(v) {
                        *a += v;
                    }
                    count += 1.;
                }
            }
            *px.get_mut(oy * cw + ox).ok_or(E::LengthMismatch)? = acc.map(|v| v / count);
        }
    }
    let coarse_img = RgbImage { w: cw, h: ch, px };
    let mut cfix = b.filled(cn, FREE)?;
    let mut ccost = b.filled(cn, 0f32)?;
    for y in 0..h {
        for x in 0..w {
            b.work(1)?;
            let ci = y / 2 * cw + x / 2;
            *ccost.get_mut(ci).ok_or(E::LengthMismatch)? += at(cf, y * w + x)?;
            if at(fixed, y * w + x)? == HARD_FG {
                *cfix.get_mut(ci).ok_or(E::LengthMismatch)? = HARD_FG;
            }
        }
    }
    let coarse = levels(&coarse_img, &cfix, &ccost, weight, scale * 2., direct, depth + 1, b)?;
    let mut proj = b.reserve(n)?;
    for i in 0..n {
        b.work(1)?;
        proj.push(at(&coarse, i / w / 2 * cw + i % w / 2)?);
    }
    let mut edge = b.reserve(n)?;
    let mut any = false;
    for i in 0..n {
        b.work(1)?;
        let (x, y) = (i % w, i / w);
        let v = at(&proj, i)?;
        let e = (x > 0 && at(&proj, i - 1)? != v)
            || (x + 1 < w && at(&proj, i + 1)? != v)
            || (y > 0 && at(&proj, i - w)? != v)
            || (y + 1 < h && at(&proj, i + w)? != v);
        any |= e;
        edge.push(e);
    }
    if !any {
        for (i, v) in proj.iter_mut().enumerate() {
            b.work(1)?;
            *v |= at(fixed, i)? == HARD_FG;
        }
        return Ok(proj);
    }
    let near = distance(&edge, w, h, b)?;
    let mut band = b.reserve(n)?;
    for i in 0..n {
        b.work(1)?;
        band.push(if at(fixed, i)? == HARD_FG {
            HARD_FG
        } else if at(&near, i)? <= super::BAND {
            FREE
        } else if at(&proj, i)? {
            HARD_FG
        } else {
            HARD_BG
        });
    }
    cut(img, cf, &zeros, &band, &|d2| scale * weight(d2), b)
}
fn upsample(low: &[bool], lw: usize, lh: usize, step: usize, w: usize, h: usize, b: &Budget<'_, '_>) -> Result<Vec<u8>, E> {
    if lw == 0 || lh == 0 || low.len() != lw.checked_mul(lh).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    let mut out = b.reserve(w.checked_mul(h).ok_or(E::LimitExceeded)?)?;
    let read =
        |x: i64, y: i64| -> Result<f32, E> { Ok(if at(low, y.clamp(0, lh as i64 - 1) as usize * lw + x.clamp(0, lw as i64 - 1) as usize)? { 1. } else { 0. }) };
    for y in 0..h {
        for x in 0..w {
            b.work(1)?;
            let value = if step == 1 {
                if at(low, y * lw + x)? { 255 } else { 0 }
            } else {
                let s = step as f32;
                let fy = (y as f32 + 0.5) / s - 0.5;
                let (iy, ty) = (fy.floor() as i64, fy - fy.floor());
                let fx = (x as f32 + 0.5) / s - 0.5;
                let (ix, tx) = (fx.floor() as i64, fx - fx.floor());
                let v = (read(ix, iy)? * (1. - tx) + read(ix + 1, iy)? * tx) * (1. - ty) + (read(ix, iy + 1)? * (1. - tx) + read(ix + 1, iy + 1)? * tx) * ty;
                (v * 255.).round() as u8
            };
            out.push(value);
        }
    }
    Ok(out)
}
fn trim(region: Region, b: &Budget<'_, '_>) -> Result<Option<Region>, E> {
    let w = region.bbox.width() as usize;
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for (i, v) in region.mask.iter().enumerate() {
        b.work(1)?;
        if *v != 0 {
            x0 = x0.min(i % w);
            x1 = x1.max(i % w + 1);
            y0 = y0.min(i / w);
            y1 = y1.max(i / w + 1);
        }
    }
    if x0 == usize::MAX {
        return Ok(None);
    }
    let mut out = b.reserve((x1 - x0).checked_mul(y1 - y0).ok_or(E::LimitExceeded)?)?;
    for y in y0..y1 {
        b.work((x1 - x0) as u64)?;
        out.extend_from_slice(region.mask.get(y * w + x0..y * w + x1).ok_or(E::LengthMismatch)?);
    }
    Ok(Some(Region {
        bbox: Rect::new(region.bbox.x0 + x0 as i32, region.bbox.y0 + y0 as i32, region.bbox.x0 + x1 as i32, region.bbox.y0 + y1 as i32),
        mask: out,
    }))
}
pub fn quick_select_with_interrupt(
    sampler: &dyn Sampler,
    canvas: Rect,
    points: &[(f32, f32)],
    size: f32,
    limits: QuickLimits,
    control: &Interrupt<'_>,
) -> Result<Option<Region>, E> {
    let remaining = Cell::new(limits.work_units.min(MAX_WORK));
    quick_select_accounted_with_interrupt(sampler, canvas, points, size, limits, control, &remaining, &|_, _, _| Ok(()))
}
/// Shared work ledger plus bounded pre-window accounting hook for rendering callers.
/// Hook runs before sampling/allocation; it must not alter numerical inputs.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn quick_select_accounted_with_interrupt(
    sampler: &dyn Sampler,
    canvas: Rect,
    points: &[(f32, f32)],
    size: f32,
    limits: QuickLimits,
    control: &Interrupt<'_>,
    remaining: &Cell<u64>,
    before_window: &dyn Fn(Rect, u64, u64) -> Result<(), E>,
) -> Result<Option<Region>, E> {
    if remaining.get() > limits.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    let req = requirements(canvas, points, size, control)?;
    if req.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > limits.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    if req.attempts == 0 {
        return Ok(None);
    }
    let b = Budget { left: remaining, control };
    let bb = bbox(canvas, points, size, control)?;
    let step = req.step;
    let rw = size / 2. / step as f32;
    let t = Tuning::default();
    let mut margin = 48f32.max(2. * rw) as i32 * step as i32;
    for _ in 0..req.attempts {
        b.work(1)?;
        let win = window(bb, canvas, margin, step);
        let (peak, required_work) = window_requirements(win, step, points.len())?;
        if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
            return Err(E::LimitExceeded);
        }
        let previous_work = remaining.get();
        before_window(win, peak, required_work)?;
        if remaining.get() > previous_work || remaining.get() > limits.work_units.min(MAX_WORK) {
            return Err(E::LimitExceeded);
        }
        // Lower allocations/loops debit this shared ledger; the complete next
        // window envelope must fit the remaining work before reading pixels.
        b.ensure_work(required_work)?;
        let img = scaled(sampler, win, step, &b)?;
        let mut pts = b.reserve(points.len())?;
        for &(x, y) in points {
            b.work(1)?;
            pts.push(((x - win.x0 as f32) / step as f32, (y - win.y0 as f32) / step as f32));
        }
        let seeds = seeds(&pts, rw.max(0.5), img.w, img.h, &b)?;
        let mut any = false;
        for v in &seeds {
            b.work(1)?;
            any |= *v;
        }
        if !any {
            return Ok(None);
        }
        let low = working(&img, &seeds, rw, &t, &b)?;
        let (w, h) = (img.w, img.h);
        let mut touches = false;
        if win.x0 > canvas.x0 {
            for y in 0..h {
                b.work(1)?;
                touches |= at(&low, y * w)?;
            }
        }
        if win.y0 > canvas.y0 {
            for x in 0..w {
                b.work(1)?;
                touches |= at(&low, x)?;
            }
        }
        if win.x1 < canvas.x1 {
            for y in 0..h {
                b.work(1)?;
                touches |= at(&low, y * w + w - 1)?;
            }
        }
        if win.y1 < canvas.y1 {
            for x in 0..w {
                b.work(1)?;
                touches |= at(&low, (h - 1) * w + x)?;
            }
        }
        if !touches {
            let mask = upsample(&low, w, h, step, win.width() as usize, win.height() as usize, &b)?;
            let result = trim(Region { bbox: win, mask }, &b)?;
            b.work(1)?;
            return Ok(result);
        }
        margin = margin.checked_mul(2).ok_or(E::LimitExceeded)?;
    }
    Err(E::LimitExceeded)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_stroke_window_oracle_and_cancel() {
        let img = RgbImage { w: 64, h: 48, px: (0..64 * 48).map(|i| if i % 64 < 32 { [0.8, 0.1, 0.2] } else { [0.1, 0.2, 0.8] }).collect() };
        let s = crate::segment::ImageSampler { img: &img, origin: (0, 0) };
        for pts in [vec![(16., 20.)], vec![(16., 20.), (20., 20.), (22., 28.)]] {
            let area = Rect::new(0, 0, 64, 48);
            assert_eq!(
                quick_select_with_interrupt(&s, area, &pts, 12., QuickLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::quick_select(&s, area, &pts, 12., super::super::WORK_PX).unwrap()
            );
            assert!(matches!(quick_select_with_interrupt(&s, area, &pts, 12., QuickLimits::default(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled)));
        }
    }
}

#[cfg(test)]
mod half_resolution_tests {
    use super::*;
    #[test]
    fn half_resolution_strip_order_matches_upstream() {
        let img = RgbImage { w: 514, h: 511, px: (0..514 * 511).map(|i| if i % 514 < 200 { [0.8, 0.1, 0.2] } else { [0.1, 0.2, 0.8] }).collect() };
        let s = crate::segment::ImageSampler { img: &img, origin: (0, 0) };
        let canvas = Rect::new(0, 0, 514, 511);
        let points = [(120., 1.), (130., 1.)];
        assert_eq!(
            quick_select_with_interrupt(&s, canvas, &points, 12., QuickLimits::default(), &Interrupt::NONE).unwrap(),
            super::super::quick_select(&s, canvas, &points, 12., super::super::WORK_PX).unwrap()
        );
    }
}

#[cfg(test)]
mod dynamic_window_tests {
    use super::*;
    #[test]
    fn early_settling_large_canvas_does_not_require_full_canvas_graph() {
        // A bounded contrasting object settles inside the first window. A
        // uniform canvas has no such boundary and legitimately grows windows.
        let mut surface = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::RGBA8, &[0., 0., 1., 1.]);
        surface.fill_rect(Rect::new(2980, 1980, 3020, 2020), &[1., 0., 0., 1.]);
        let sampler = crate::segment::SurfaceSampler(&surface);
        let canvas = Rect::new(0, 0, 6000, 4000);
        let points = [(3000., 2000.)];
        let limits = QuickLimits { resident_bytes: 16 << 20, work_units: 50_000_000 };
        let req = requirements(canvas, &points, 12., &Interrupt::NONE).unwrap();
        assert!(req.peak_resident_bytes < limits.resident_bytes);
        assert!(quick_select_with_interrupt(&sampler, canvas, &points, 12., limits, &Interrupt::NONE).unwrap().is_some());
    }
    #[test]
    fn callback_cannot_refund_work_or_read_after_refused_window() {
        let surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        let s = crate::segment::SurfaceSampler(&surface);
        let remaining = Cell::new(50_000_000);
        let calls = Cell::new(0);
        let refuse = |_: Rect, _: u64, _: u64| {
            calls.set(calls.get() + 1);
            Err(E::LimitExceeded)
        };
        assert!(matches!(
            quick_select_accounted_with_interrupt(
                &s,
                Rect::new(0, 0, 64, 48),
                &[(20., 20.)],
                12.,
                QuickLimits { resident_bytes: 16 << 20, work_units: 50_000_000 },
                &Interrupt::NONE,
                &remaining,
                &refuse
            ),
            Err(E::LimitExceeded)
        ));
        assert_eq!(calls.get(), 1);
    }
}
