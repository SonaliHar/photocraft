//! Same classical Sky stages with bounded allocations and cooperative inner loops.
//! Caller Sampler::rgba must itself be bounded/tokened; the engine supplies that adapter.
use super::super::gmm::Gmm;
use super::super::{CleanupLimits, Region, RgbImage, Sampler, clean_mask_with_interrupt, scale_for};
use super::{SkyParams, WORK_PX, colour_prior, luma, smooth};
use crate::matting::{MattingLimits, box_mean_with_interrupt, guided_filter_gray_with_interrupt};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
const MAX_WORK: u64 = 8_000_000_000;
#[derive(Clone, Copy, Debug)]
pub struct SkyLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for SkyLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SkyRequirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub output_bytes: u64,
    pub step: usize,
    pub radius: usize,
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
fn filled<T: Clone>(n: usize, v: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
    if bytes as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    let mut out = reserve(n)?;
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(1024).min(n), v.clone());
    }
    poll(c)?;
    Ok(out)
}
fn reserve<T>(n: usize) -> Result<Vec<T>, E> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if v.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    Ok(v)
}
pub fn requirements(canvas: Rect, p: SkyParams) -> Result<SkyRequirements, E> {
    if canvas.x1 < canvas.x0
        || canvas.y1 < canvas.y0
        || canvas.width() == 0
        || canvas.height() == 0
        || canvas.width() > 300000
        || canvas.height() > 300000
        || [canvas.x0, canvas.y0, canvas.x1, canvas.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
    {
        return Err(E::InvalidBounds);
    }
    if !p.threshold.is_finite() || !p.softness.is_finite() {
        return Err(E::InvalidSample);
    }
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let pixels = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let step = scale_for(canvas, WORK_PX);
    let low = w.div_ceil(step).checked_mul(h.div_ceil(step)).ok_or(E::LimitExceeded)?;
    let soft = p.softness.clamp(0., 100.) / 100.;
    let radius = ((step as f32 * (1. + 2. * soft)).ceil() as usize).max(3);
    let win_w = w.min(256usize.checked_add(radius.checked_mul(2).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?);
    let win_h = h.min(256 + radius * 2);
    let window = win_w.checked_mul(win_h).ok_or(E::LimitExceeded)?;
    let tiles = w.div_ceil(256).checked_mul(h.div_ceil(256)).ok_or(E::LimitExceeded)?;
    let strip = w.checked_mul(h.min(64usize.div_ceil(step).max(1).checked_mul(step).ok_or(E::LimitExceeded)?)).ok_or(E::LimitExceeded)?;
    let bytes = add(add(mul(low as u64, 256)?, mul(pixels as u64, 2)?)?, add(add(mul(window as u64, 256)?, mul(strip as u64, 32)?)?, 16 << 20)?)?;
    let work = add(add(mul(pixels as u64, 64)?, mul(low as u64, 1024)?)?, mul(mul(tiles as u64, window as u64)?, 256)?)?;
    if bytes > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(SkyRequirements { peak_resident_bytes: bytes, work_units: work, output_bytes: pixels as u64, step, radius })
}
fn rgb(s: &dyn Sampler, r: Rect, c: &Interrupt<'_>) -> Result<RgbImage, E> {
    poll(c)?;
    let w = r.width() as usize;
    let h = r.height() as usize;
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let source = s.rgba(r)?;
    poll(c)?;
    if source.len() != n {
        return Err(E::LengthMismatch);
    }
    let mut px = filled(n, [0f32; 3], c)?;
    for (i, (out, value)) in px.iter_mut().zip(source).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if value.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        let [red, green, blue, alpha] = value;
        let a = alpha.clamp(0., 1.);
        *out = [red, green, blue].map(|v| v.clamp(0., 1.) * a + 0.5 * (1. - a));
    }
    poll(c)?;
    Ok(RgbImage { w, h, px })
}
fn scaled(s: &dyn Sampler, r: Rect, step: usize, c: &Interrupt<'_>) -> Result<RgbImage, E> {
    if step == 1 {
        return rgb(s, r, c);
    }
    let (w, h) = (r.width() as usize, r.height() as usize);
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut output = filled(ow.checked_mul(oh).ok_or(E::LimitExceeded)?, [0f32; 3], c)?;
    let rows_per = 64usize.div_ceil(step).max(1);
    for oy0 in (0..oh).step_by(rows_per) {
        poll(c)?;
        let oy1 = (oy0 + rows_per).min(oh);
        let dy0 = oy0 * step;
        let dy1 = (oy1 * step).min(h);
        let y0 = i32::try_from(i64::from(r.y0) + dy0 as i64).map_err(|_| E::InvalidBounds)?;
        let y1 = i32::try_from(i64::from(r.y0) + dy1 as i64).map_err(|_| E::InvalidBounds)?;
        let strip = rgb(s, Rect::new(r.x0, y0, r.x1, y1), c)?;
        for local_y in 0..(oy1 - oy0) {
            poll(c)?;
            for ox in 0..ow {
                let mut acc = [0f32; 3];
                let mut count = 0usize;
                let sy = local_y * step;
                let sx = ox * step;
                for y in sy..(sy + step).min(strip.h) {
                    poll(c)?;
                    for x in sx..(sx + step).min(strip.w) {
                        if x.is_multiple_of(1024) {
                            poll(c)?;
                        }
                        let value = strip.px.get(y * strip.w + x).ok_or(E::LengthMismatch)?;
                        for (a, v) in acc.iter_mut().zip(value) {
                            *a += v;
                        }
                        count += 1;
                    }
                }
                if count == 0 {
                    return Err(E::LengthMismatch);
                }
                *output.get_mut((oy0 + local_y) * ow + ox).ok_or(E::LengthMismatch)? = acc.map(|v| v / count as f32);
            }
        }
    }
    poll(c)?;
    Ok(RgbImage { w: ow, h: oh, px: output })
}
fn likelihood(img: &RgbImage, l: SkyLimits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let (w, h) = (img.w, img.h);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if img.px.len() != n {
        return Err(E::LengthMismatch);
    }
    let mut y = filled(n, 0f32, c)?;
    for (i, (y, p)) in y.iter_mut().zip(&img.px).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *y = luma(*p);
    }
    let at = |x: isize, yy: isize| -> Result<f32, E> {
        y.get(yy.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize).copied().ok_or(E::LengthMismatch)
    };
    let mut grad = filled(n, 0f32, c)?;
    for yy in 0..h as isize {
        poll(c)?;
        for x in 0..w as isize {
            if (x as usize).is_multiple_of(1024) {
                poll(c)?;
            }
            let gx = (at(x + 1, yy - 1)? + 2. * at(x + 1, yy)? + at(x + 1, yy + 1)?) - (at(x - 1, yy - 1)? + 2. * at(x - 1, yy)? + at(x - 1, yy + 1)?);
            let gy = (at(x - 1, yy + 1)? + 2. * at(x, yy + 1)? + at(x + 1, yy + 1)?) - (at(x - 1, yy - 1)? + 2. * at(x, yy - 1)? + at(x + 1, yy - 1)?);
            *grad.get_mut(yy as usize * w + x as usize).ok_or(E::LengthMismatch)? = (gx * gx + gy * gy).sqrt() / 8.;
        }
    }
    let tex = box_mean_with_interrupt(&grad, w, h, 2, MattingLimits { resident_bytes: l.resident_bytes, work_units: l.work_units }, c)?;
    let mut output = filled(n, 0f32, c)?;
    for (i, ((out, p), t)) in output.iter_mut().zip(&img.px).zip(&tex).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *out = colour_prior(*p) * (1. - smooth(0.015, 0.07, *t));
    }
    poll(c)?;
    Ok(output)
}
fn subsample<T: Copy>(values: &[T], max: usize, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    let step = if values.len() <= max { 1 } else { values.len().div_ceil(max) };
    let mut out = reserve(values.len().div_ceil(step))?;
    for (i, v) in values.iter().step_by(step).enumerate() {
        if i.is_multiple_of(64) {
            poll(c)?;
        }
        out.push(*v);
    }
    Ok(out)
}
fn low_mask(img: &RgbImage, threshold: f32, l: SkyLimits, c: &Interrupt<'_>) -> Result<Option<Vec<bool>>, E> {
    let (w, h) = (img.w, img.h);
    if w < 2 || h < 2 {
        return Ok(None);
    }
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let like = likelihood(img, l, c)?;
    let t = 0.12 + 0.6 * (threshold.clamp(0., 100.) / 100.);
    let top = (h / 40).max(1);
    let mut mask = filled(n, false, c)?;
    let mut stack = reserve(n)?;
    for i in 0..top * w {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if *like.get(i).ok_or(E::LengthMismatch)? > t {
            *mask.get_mut(i).ok_or(E::LengthMismatch)? = true;
            stack.push(i);
        }
    }
    if stack.len() * 20 < top * w {
        return Ok(None);
    }
    while let Some(i) = stack.pop() {
        poll(c)?;
        let (x, y) = (i % w, i / w);
        for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
            poll(c)?;
            if *mask.get(j).ok_or(E::LengthMismatch)? || *like.get(j).ok_or(E::LengthMismatch)? <= t {
                continue;
            }
            let a = img.px.get(i).ok_or(E::LengthMismatch)?;
            let b = img.px.get(j).ok_or(E::LengthMismatch)?;
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            if d < 0.12 {
                *mask.get_mut(j).ok_or(E::LengthMismatch)? = true;
                stack.push(j);
            }
        }
    }
    let mut samples = reserve(n)?;
    for (i, (p, m)) in img.px.iter().zip(&mask).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if *m {
            samples.push(*p);
        }
    }
    if samples.len() >= 16 {
        let fit = subsample(&samples, 4000, c)?;
        if let Some(g) = Gmm::fit_with_interrupt(&fit, 3, 1e-4, c)? {
            let probe = subsample(&samples, 2000, c)?;
            let mut nl = reserve(probe.len())?;
            for (i, p) in probe.iter().enumerate() {
                if i.is_multiple_of(64) {
                    poll(c)?;
                }
                nl.push(g.neg_log(*p));
            }
            nl.sort_unstable_by(f32::total_cmp);
            poll(c)?;
            let cut = *nl.get((nl.len() * 95 / 100).min(nl.len() - 1)).ok_or(E::LengthMismatch)?;
            for (i, m) in mask.iter_mut().enumerate() {
                if i.is_multiple_of(64) {
                    poll(c)?;
                }
                if !*m && *like.get(i).ok_or(E::LengthMismatch)? > t + 0.1 && g.neg_log(*img.px.get(i).ok_or(E::LengthMismatch)?) <= cut {
                    *m = true;
                }
            }
        }
    }
    let mask = clean_mask_with_interrupt(&mask, w, h, 0.002, 0.01, CleanupLimits { resident_bytes: l.resident_bytes, work_units: l.work_units }, c)?;
    let mut covered = 0usize;
    let bottom = (h - top.min(h)) * w..h * w;
    for (i, m) in mask.get(bottom.clone()).ok_or(E::LengthMismatch)?.iter().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        covered += usize::from(*m);
    }
    if covered * 10 >= bottom.len() * 6 {
        let (mut sat, mut count) = (0f32, 0usize);
        for (i, (p, m)) in img.px.iter().zip(&mask).enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            if *m {
                let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
                sat += if mx > 1e-4 { (mx - mn) / mx } else { 0. };
                count += 1;
            }
        }
        if count > 0 && sat / (count as f32) < 0.1 {
            return Ok(None);
        }
    }
    let mut any = false;
    for (i, m) in mask.iter().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        any |= *m;
    }
    poll(c)?;
    Ok(any.then_some(mask))
}
fn trim(region: Region, c: &Interrupt<'_>) -> Result<Option<Region>, E> {
    let w = region.bbox.width() as usize;
    let h = region.bbox.height() as usize;
    if region.mask.len() != w.checked_mul(h).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        poll(c)?;
        let mut first = None;
        let mut last = None;
        for (x, value) in region.mask.get(y * w..(y + 1) * w).ok_or(E::LengthMismatch)?.iter().enumerate() {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            if *value != 0 {
                if first.is_none() {
                    first = Some(x);
                }
                last = Some(x);
            }
        }
        if let (Some(a), Some(b)) = (first, last) {
            x0 = x0.min(a);
            x1 = x1.max(b + 1);
            y0 = y0.min(y);
            y1 = y + 1;
        }
    }
    if x0 == usize::MAX {
        return Ok(None);
    }
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        poll(c)?;
        return Ok(Some(region));
    }
    let mut mask = reserve((x1 - x0).checked_mul(y1 - y0).ok_or(E::LimitExceeded)?)?;
    for y in y0..y1 {
        poll(c)?;
        for chunk in region.mask.get(y * w + x0..y * w + x1).ok_or(E::LengthMismatch)?.chunks(4096) {
            poll(c)?;
            mask.extend_from_slice(chunk);
        }
    }
    let b = region.bbox;
    poll(c)?;
    Ok(Some(Region { bbox: Rect::new(b.x0 + x0 as i32, b.y0 + y0 as i32, b.x0 + x1 as i32, b.y0 + y1 as i32), mask }))
}
pub fn select_sky_with_interrupt(s: &dyn Sampler, canvas: Rect, p: SkyParams, l: SkyLimits, c: &Interrupt<'_>) -> Result<Option<Region>, E> {
    poll(c)?;
    let req = requirements(canvas, p)?;
    if req.peak_resident_bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > l.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    if canvas.width() < 4 || canvas.height() < 4 {
        return Ok(None);
    }
    let img = scaled(s, canvas, req.step, c)?;
    let (lw, lh) = (img.w, img.h);
    let Some(low) = low_mask(&img, p.threshold, l, c)? else { return Ok(None) };
    let mut lowf = filled(low.len(), 0f32, c)?;
    for (i, (v, m)) in lowf.iter_mut().zip(&low).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = if *m { 1. } else { 0. };
    }
    let soft = p.softness.clamp(0., 100.) / 100.;
    let eps = 1e-4 + 4e-3 * soft;
    let k = 0.08 + 0.42 * soft;
    let step = req.step;
    let r = req.radius;
    let coarse = |x: i32, y: i32| -> Result<f32, E> {
        let fx = ((x - canvas.x0) as f32 + 0.5) / step as f32 - 0.5;
        let fy = ((y - canvas.y0) as f32 + 0.5) / step as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let g =
            |xx: f32, yy: f32| lowf.get(yy.clamp(0., lh as f32 - 1.) as usize * lw + xx.clamp(0., lw as f32 - 1.) as usize).copied().ok_or(E::LengthMismatch);
        let top = g(x0, y0)? * (1. - ax) + g(x0 + 1., y0)? * ax;
        let bot = g(x0, y0 + 1.)? * (1. - ax) + g(x0 + 1., y0 + 1.)? * ax;
        Ok(top * (1. - ay) + bot * ay)
    };
    let mut mask = filled(req.output_bytes as usize, 0u8, c)?;
    let w = canvas.width() as usize;
    for ty in (canvas.y0..canvas.y1).step_by(256) {
        for tx in (canvas.x0..canvas.x1).step_by(256) {
            poll(c)?;
            let tile = Rect::new(tx, ty, tx.saturating_add(256).min(canvas.x1), ty.saturating_add(256).min(canvas.y1));
            let win = tile.inflate(r as i32).intersect(&canvas);
            let (ww, wh) = (win.width() as usize, win.height() as usize);
            let mut pm = filled(ww.checked_mul(wh).ok_or(E::LimitExceeded)?, 0f32, c)?;
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for y in 0..wh {
                poll(c)?;
                for x in 0..ww {
                    if x.is_multiple_of(1024) {
                        poll(c)?;
                    }
                    let value = coarse(win.x0 + x as i32, win.y0 + y as i32)?;
                    *pm.get_mut(y * ww + x).ok_or(E::LengthMismatch)? = value;
                    lo = lo.min(value);
                    hi = hi.max(value);
                }
            }
            if hi <= 0. || lo >= 1. {
                for y in tile.y0..tile.y1 {
                    poll(c)?;
                    let start = (y - canvas.y0) as usize * w + (tile.x0 - canvas.x0) as usize;
                    mask.get_mut(start..start + tile.width() as usize).ok_or(E::LengthMismatch)?.fill(if lo >= 1. { 255 } else { 0 });
                }
                continue;
            }
            let image = rgb(s, win, c)?;
            let mut guide = filled(image.px.len(), 0f32, c)?;
            for (i, (v, p)) in guide.iter_mut().zip(&image.px).enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                *v = luma(*p);
            }
            let q = guided_filter_gray_with_interrupt(
                &guide,
                &pm,
                ww,
                wh,
                r,
                eps,
                MattingLimits { resident_bytes: l.resident_bytes, work_units: l.work_units },
                c,
            )?;
            for y in tile.y0..tile.y1 {
                poll(c)?;
                for x in tile.x0..tile.x1 {
                    let q = *q.get((y - win.y0) as usize * ww + (x - win.x0) as usize).ok_or(E::LengthMismatch)?;
                    let value = (smooth(0.5 - k, 0.5 + k, q) * 255.).round() as u8;
                    *mask.get_mut((y - canvas.y0) as usize * w + (x - canvas.x0) as usize).ok_or(E::LengthMismatch)? = value;
                }
            }
        }
    }
    poll(c)?;
    trim(Region { bbox: canvas, mask }, c)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_landscape_and_no_sky_match_original() {
        let img = super::super::tests::landscape(160, 100);
        let sampler = super::super::super::ImageSampler { img: &img, origin: (0, 0) };
        let area = Rect::new(0, 0, 160, 100);
        for softness in [0., 50., 100.] {
            let p = SkyParams { threshold: 50., softness };
            assert_eq!(
                select_sky_with_interrupt(&sampler, area, p, SkyLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::select_sky(&sampler, area, p).unwrap()
            );
        }
        assert!(matches!(
            select_sky_with_interrupt(&sampler, area, SkyParams::default(), SkyLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(E::Cancelled)
        ));
    }
}
