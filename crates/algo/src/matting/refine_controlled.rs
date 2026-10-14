//! Exact ordinary Refine equations with bounded private tiles and inner polls.
//! No brush/decontamination/native-wide or imported rendering qualification.
use super::{EPS, MattingLimits, RefineParams, guided_filter_color_with_interrupt};
use crate::segment::{Region, RgbImage, Sampler};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
const MAX_WORK: u64 = 8_000_000_000;
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Clone>(n: usize, z: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    super::controlled_gray::filled(n, z, c)
}
fn copied<T: Copy>(s: &[T], c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    let mut o = Vec::new();
    o.try_reserve_exact(s.len()).map_err(|_| E::AllocationFailed)?;
    if o.capacity() > s.len().saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    for v in s.chunks(1024) {
        poll(c)?;
        o.extend_from_slice(v);
    }
    poll(c)?;
    Ok(o)
}
fn at(s: &[f32], i: usize) -> Result<f32, E> {
    s.get(i).copied().ok_or(E::LengthMismatch)
}
fn set(s: &mut [f32], i: usize, v: f32) -> Result<(), E> {
    *s.get_mut(i).ok_or(E::LengthMismatch)? = v;
    Ok(())
}
#[derive(Clone, Copy, Debug)]
pub struct RefineRequirements {
    pub bbox: Rect,
    pub halo: i32,
    pub window_width: usize,
    pub window_height: usize,
    pub output_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
fn parameters(p: &RefineParams) -> Result<(), E> {
    if ![p.radius, p.smooth, p.feather, p.contrast, p.shift_edge].iter().all(|v| v.is_finite())
        || !(0.0..=250.0).contains(&p.radius)
        || !(0.0..=100.0).contains(&p.smooth)
        || !(0.0..=1000.0).contains(&p.feather)
        || !(0.0..=100.0).contains(&p.contrast)
        || !(-100.0..=100.0).contains(&p.shift_edge)
    {
        return Err(E::InvalidSample);
    }
    Ok(())
}
pub fn requirements(content: Rect, canvas: Rect, p: &RefineParams) -> Result<RefineRequirements, E> {
    parameters(p)?;
    if canvas.x1 < canvas.x0
        || canvas.y1 < canvas.y0
        || canvas.width() > 300000
        || canvas.height() > 300000
        || [canvas.x0, canvas.x1, canvas.y0, canvas.y1, content.x0, content.y0, content.x1, content.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
    {
        return Err(E::InvalidBounds);
    }
    let content = content.intersect(&canvas);
    let bbox = if content.is_empty() { Rect::EMPTY } else { content.inflate(p.spread()).intersect(&canvas) };
    let halo = p.halo();
    let w = (bbox.width() as usize).min(256).checked_add(2 * halo as usize).ok_or(E::LimitExceeded)?.min(canvas.width() as usize);
    let h = (bbox.height() as usize).min(256).checked_add(2 * halo as usize).ok_or(E::LimitExceeded)?.min(canvas.height() as usize);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)? as u64;
    let output = u64::from(bbox.width()).checked_mul(u64::from(bbox.height())).ok_or(E::LimitExceeded)?;
    let tiles = u64::from(bbox.width()).div_ceil(256).checked_mul(u64::from(bbox.height()).div_ceil(256)).ok_or(E::LimitExceeded)?;
    let smooth = (p.smooth_sigma() * 3.).ceil() as u64;
    let feather = (p.feather_sigma() * 3.).ceil() as u64;
    let taps = smooth.checked_add(feather).and_then(|v| v.checked_mul(4)).and_then(|v| v.checked_add(4096)).ok_or(E::LimitExceeded)?;
    let work = n.checked_mul(taps).and_then(|v| v.checked_mul(tiles)).and_then(|v| v.checked_add(output.checked_mul(64)?)).ok_or(E::LimitExceeded)?;
    let bytes = n
        .checked_mul(512)
        .and_then(|v| v.checked_add(output.checked_mul(2)?))
        .and_then(|v| v.checked_add((w.max(h) as u64).checked_mul(128)?))
        .and_then(|v| v.checked_add(1 << 20))
        .ok_or(E::LimitExceeded)?;
    if bytes > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(RefineRequirements { bbox, halo, window_width: w, window_height: h, output_bytes: output, peak_resident_bytes: bytes, work_units: work })
}
fn validate(src: &[f32], w: usize, h: usize, c: &Interrupt<'_>) -> Result<(), E> {
    if src.len() != w.checked_mul(h).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    for chunk in src.chunks(1024) {
        poll(c)?;
        if chunk.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    poll(c)
}
/// Direct f32 taps, horizontal then vertical, edge-clamped: not Feather's kernel.
fn gaussian(src: &[f32], w: usize, h: usize, sigma: f32, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    if sigma < 0.1 {
        return copied(src, c);
    }
    let r = (sigma * 3.).ceil() as i64;
    let mut k = filled((2 * r + 1) as usize, 0f32, c)?;
    for (i, v) in k.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let x = i as i64 - r;
        *v = (-(x * x) as f32 / (2. * sigma * sigma)).exp();
    }
    let mut sum = 0f32;
    for v in &k {
        sum += *v;
    }
    for v in &mut k {
        *v /= sum;
    }
    let mut tmp = filled(src.len(), 0f32, c)?;
    let mut out = filled(src.len(), 0f32, c)?;
    for y in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(64) {
                poll(c)?;
            }
            let mut a = 0f32;
            for (i, v) in k.iter().enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                let xx = (x as i64 + i as i64 - r).clamp(0, w as i64 - 1) as usize;
                a += at(src, y * w + xx)? * v;
            }
            set(&mut tmp, y * w + x, a)?;
        }
    }
    for y in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(64) {
                poll(c)?;
            }
            let mut a = 0f32;
            for (i, v) in k.iter().enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                let yy = (y as i64 + i as i64 - r).clamp(0, h as i64 - 1) as usize;
                a += at(&tmp, yy * w + x)? * v;
            }
            if !a.is_finite() {
                return Err(E::InvalidSample);
            }
            set(&mut out, y * w + x, a)?;
        }
    }
    poll(c)?;
    Ok(out)
}
fn running(src: &[f32], r: usize, max: bool, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let n = src.len();
    let k = 2 * r + 1;
    let m = (n + 2 * r).div_ceil(k) * k;
    let pad = if max { f32::NEG_INFINITY } else { f32::INFINITY };
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut g = filled(m, pad, c)?;
    let mut hb = filled(m, pad, c)?;
    let p = |j: usize| if j >= r && j < r + n { at(src, j - r) } else { Ok(pad) };
    for j in 0..m {
        if j.is_multiple_of(1024) {
            poll(c)?;
        }
        let v = if j % k == 0 { p(j)? } else { pick(at(&g, j - 1)?, p(j)?) };
        set(&mut g, j, v)?;
    }
    for j in (0..m).rev() {
        if j.is_multiple_of(1024) {
            poll(c)?;
        }
        let v = if j % k == k - 1 || j == m - 1 { p(j)? } else { pick(at(&hb, j + 1)?, p(j)?) };
        set(&mut hb, j, v)?;
    }
    let mut out = filled(n, 0f32, c)?;
    for (i, v) in out.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = pick(at(&hb, i)?, at(&g, i + 2 * r)?);
    }
    Ok(out)
}
fn square(src: &[f32], w: usize, h: usize, r: usize, max: bool, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let mut tmp = filled(src.len(), 0f32, c)?;
    for y in 0..h {
        poll(c)?;
        let row = running(src.get(y * w..(y + 1) * w).ok_or(E::LengthMismatch)?, r.min(w - 1), max, c)?;
        tmp.get_mut(y * w..(y + 1) * w).ok_or(E::LengthMismatch)?.copy_from_slice(&row);
    }
    let mut out = filled(src.len(), 0f32, c)?;
    let mut col = filled(h, 0f32, c)?;
    for x in 0..w {
        poll(c)?;
        for (y, v) in col.iter_mut().enumerate() {
            if y.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = at(&tmp, y * w + x)?;
        }
        let row = running(&col, r.min(h - 1), max, c)?;
        for (y, v) in row.into_iter().enumerate() {
            if y.is_multiple_of(1024) {
                poll(c)?;
            }
            set(&mut out, y * w + x, v)?;
        }
    }
    Ok(out)
}
fn cross(src: &[f32], w: usize, h: usize, d: usize, max: bool, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut o = filled(src.len(), 0f32, c)?;
    for y in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let row = y * w;
            let v = pick(
                pick(
                    pick(pick(at(src, row + x)?, at(src, row + x.saturating_sub(d))?), at(src, row + x.saturating_add(d).min(w - 1))?),
                    at(src, y.saturating_sub(d) * w + x)?,
                ),
                at(src, y.saturating_add(d).min(h - 1) * w + x)?,
            );
            set(&mut o, row + x, v)?;
        }
    }
    Ok(o)
}
fn morph(src: &[f32], w: usize, h: usize, r: usize, max: bool, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    if r == 0 {
        return copied(src, c);
    }
    let mut negative_zero = false;
    for chunk in src.chunks(1024) {
        poll(c)?;
        negative_zero |= chunk.iter().any(|v| *v == 0. && v.is_sign_negative());
    }
    if r <= 2 || negative_zero {
        let mut cur = copied(src, c)?;
        let mut next = filled(src.len(), 0f32, c)?;
        let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
        for k in 0..r {
            for y in 0..h {
                poll(c)?;
                for x in 0..w {
                    if x.is_multiple_of(1024) {
                        poll(c)?;
                    }
                    let mut v = at(&cur, y * w + x)?;
                    for dy in -1i32..=1 {
                        for dx in -1i32..=1 {
                            if (dx != 0 && dy != 0 && k % 2 != 0) || (dx == 0 && dy == 0) {
                                continue;
                            }
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                v = pick(v, at(&cur, ny as usize * w + nx as usize)?);
                            }
                        }
                    }
                    set(&mut next, y * w + x, v)?;
                }
            }
            std::mem::swap(&mut cur, &mut next);
        }
        return Ok(cur);
    }
    let radius = (r / 2 + r % 2).min(w.max(h) - 1);
    let diamond = (r / 2).min(w + h - 2);
    let mut cur = square(src, w, h, radius, max, c)?;
    let mut built = 0usize;
    for bit in (0..usize::BITS - diamond.leading_zeros()).rev() {
        poll(c)?;
        if built > 0 {
            cur = cross(&cur, w, h, built, max, c)?;
            built *= 2;
        }
        if (diamond >> bit) & 1 != 0 {
            cur = cross(&cur, w, h, 1, max, c)?;
            built += 1;
        }
    }
    Ok(cur)
}
fn widths(img: &RgbImage, r: usize, l: MattingLimits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let (w, h) = (img.w, img.h);
    let mut y = filled(img.px.len(), 0f32, c)?;
    for (i, (v, p)) in y.iter_mut().zip(&img.px).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
    }
    let mut grad = filled(y.len(), 0f32, c)?;
    for yy in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let gx = (at(&y, yy * w + (x + 1).min(w - 1))? - at(&y, yy * w + x.saturating_sub(1))?) / 2.;
            let gy = (at(&y, (yy + 1).min(h - 1) * w + x)? - at(&y, yy.saturating_sub(1) * w + x)?) / 2.;
            set(&mut grad, yy * w + x, (gx * gx + gy * gy).sqrt())?;
        }
    }
    let gm = square(&grad, w, h, r, true, c)?;
    let hi = square(&y, w, h, r, true, c)?;
    let lo = square(&y, w, h, r, false, c)?;
    let mut out = filled(y.len(), 0f32, c)?;
    for (i, v) in out.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = (at(&hi, i)? - at(&lo, i)?) / at(&gm, i)?.max(1e-4);
    }
    let _ = l;
    Ok(out)
}
fn buffer(img: Option<&RgbImage>, m: &[f32], w: usize, h: usize, p: &RefineParams, l: MattingLimits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    validate(m, w, h, c)?;
    let mut a = copied(m, c)?;
    if p.uses_image() {
        let img = img.ok_or(E::LengthMismatch)?;
        if img.w != w || img.h != h || img.px.len() != m.len() {
            return Err(E::LengthMismatch);
        }
        let mut bin = filled(m.len(), false, c)?;
        for (i, (v, m)) in bin.iter_mut().zip(m).enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = *m >= 0.5;
        }
        let mut boundary = filled(m.len(), false, c)?;
        let mut any = false;
        for y in 0..h {
            poll(c)?;
            for x in 0..w {
                if x.is_multiple_of(1024) {
                    poll(c)?;
                }
                let i = y * w + x;
                let v = *bin.get(i).ok_or(E::LengthMismatch)?;
                let diff = (x > 0 && *bin.get(i - 1).ok_or(E::LengthMismatch)? != v)
                    || (x + 1 < w && *bin.get(i + 1).ok_or(E::LengthMismatch)? != v)
                    || (y > 0 && *bin.get(i - w).ok_or(E::LengthMismatch)? != v)
                    || (y + 1 < h && *bin.get(i + w).ok_or(E::LengthMismatch)? != v);
                *boundary.get_mut(i).ok_or(E::LengthMismatch)? = diff;
                any |= diff;
            }
        }
        if any {
            let dist = crate::selection_modify_controlled::edt_with_interrupt(
                &boundary,
                w,
                h,
                crate::selection_modify_controlled::Limits { resident_bytes: l.resident_bytes, work_units: l.work_units.min(1 << 30) },
                c,
            )?;
            let r = p.gf_radius();
            let q = guided_filter_color_with_interrupt(img, m, r, EPS, l, c)?;
            let widths = if p.smart_radius { Some(widths(img, r, l, c)?) } else { None };
            for (i, v) in a.iter_mut().enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                let reff = match &widths {
                    Some(w) => at(w, i)?.clamp(1.5, p.radius.max(1.5)),
                    None => p.radius,
                };
                let wgt = (reff + 1. - at(&dist, i)?).clamp(0., 1.);
                if wgt > 0. {
                    *v = at(m, i)? + (at(&q, i)?.clamp(0., 1.) - at(m, i)?) * wgt;
                }
            }
        }
    }
    let ss = p.smooth_sigma();
    if ss >= 0.3 {
        a = gaussian(&a, w, h, ss, c)?;
        let gain = 1. + ss;
        for (i, v) in a.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = ((*v - 0.5) * gain + 0.5).clamp(0., 1.);
        }
    }
    a = gaussian(&a, w, h, p.feather_sigma(), c)?;
    if p.contrast > 0. {
        let gain = 1. / (1. - 0.99 * p.contrast.clamp(0., 100.) / 100.);
        for (i, v) in a.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(c)?;
            }
            *v = ((*v - 0.5) * gain + 0.5).clamp(0., 1.);
        }
    }
    let amount = p.shift_amount();
    if amount.abs() >= 1e-4 {
        let mag = amount.abs();
        let whole = mag.floor() as usize;
        let frac = mag - whole as f32;
        if frac < 1e-4 {
            a = morph(&a, w, h, whole, amount > 0., c)?;
        } else {
            let lo = if whole == 0 { copied(&a, c)? } else { morph(&a, w, h, whole, amount > 0., c)? };
            let hi = morph(&a, w, h, whole + 1, amount > 0., c)?;
            for (i, v) in a.iter_mut().enumerate() {
                if i.is_multiple_of(1024) {
                    poll(c)?;
                }
                let x = at(&lo, i)?;
                *v = x + (at(&hi, i)? - x) * frac;
            }
        }
    }
    poll(c)?;
    Ok(a)
}
fn rgb(sampler: &dyn Sampler, r: Rect, c: &Interrupt<'_>) -> Result<RgbImage, E> {
    let raw = sampler.rgba(r)?;
    let w = r.width() as usize;
    let h = r.height() as usize;
    if raw.len() != w.checked_mul(h).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    let mut px = filled(raw.len(), [0f32; 3], c)?;
    for (i, (o, v)) in px.iter_mut().zip(raw).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if v.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
        let a = v[3].clamp(0., 1.);
        *o = [v[0], v[1], v[2]].map(|v| v.clamp(0., 1.) * a + 0.5 * (1. - a));
    }
    poll(c)?;
    Ok(RgbImage { w, h, px })
}

pub fn refine_mask_with_interrupt(
    sampler: &dyn Sampler,
    mask: &(dyn Fn(Rect) -> Result<Vec<f32>, E> + Sync),
    content: Rect,
    canvas: Rect,
    p: &RefineParams,
    l: MattingLimits,
    c: &Interrupt<'_>,
) -> Result<Option<Region>, E> {
    poll(c)?;
    let req = requirements(content, canvas, p)?;
    if req.peak_resident_bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > l.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    if req.bbox.is_empty() {
        return Ok(None);
    }
    let b = req.bbox;
    let bw = b.width() as usize;
    let mut out = filled(req.output_bytes as usize, 0u8, c)?;
    for ty in (b.y0..b.y1).step_by(256) {
        for tx in (b.x0..b.x1).step_by(256) {
            poll(c)?;
            let tile = Rect::new(tx, ty, (tx + 256).min(b.x1), (ty + 256).min(b.y1));
            let r = tile.inflate(req.halo).intersect(&canvas);
            let (w, h) = (r.width() as usize, r.height() as usize);
            let m = mask(r)?;
            validate(&m, w, h, c)?;
            let first = *m.first().ok_or(E::LengthMismatch)?;
            let mut constant = first == 0. || first == 1.;
            for chunk in m.chunks(1024) {
                poll(c)?;
                constant &= chunk.iter().all(|v| *v == first);
            }
            let data = if constant {
                None
            } else {
                let img = if p.uses_image() { Some(rgb(sampler, r, c)?) } else { None };
                Some(buffer(img.as_ref(), &m, w, h, p, l, c)?)
            };
            for y in tile.y0..tile.y1 {
                poll(c)?;
                for x in tile.x0..tile.x1 {
                    if x.rem_euclid(1024) == 0 {
                        poll(c)?;
                    }
                    let v = match &data {
                        Some(a) => at(a, (y - r.y0) as usize * w + (x - r.x0) as usize)?,
                        None => first,
                    };
                    *out.get_mut((y - b.y0) as usize * bw + (x - b.x0) as usize).ok_or(E::LengthMismatch)? = (v.clamp(0., 1.) * 255.).round() as u8;
                }
            }
        }
    }
    // Same trim row/column policy, copying the final bounded row slices fallibly.
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for (y, row) in out.chunks_exact(bw).enumerate() {
        poll(c)?;
        for (x, v) in row.iter().enumerate() {
            if x.is_multiple_of(1024) {
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
    if (x0, y0, x1, y1) == (0, 0, bw, b.height() as usize) {
        poll(c)?;
        return Ok(Some(Region { bbox: b, mask: out }));
    }
    let mut trimmed = Vec::new();
    let n = (x1 - x0).checked_mul(y1 - y0).ok_or(E::LimitExceeded)?;
    trimmed.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if trimmed.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    for y in y0..y1 {
        for chunk in out.get(y * bw + x0..y * bw + x1).ok_or(E::LengthMismatch)?.chunks(1024) {
            poll(c)?;
            trimmed.extend_from_slice(chunk);
        }
    }
    poll(c)?;
    Ok(Some(Region { bbox: Rect::new(b.x0 + x0 as i32, b.y0 + y0 as i32, b.x0 + x1 as i32, b.y0 + y1 as i32), mask: trimmed }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_refine_settings_match_current_buffer() {
        let img = RgbImage::from_fn(23, 17, |x, y| [x as f32 / 23., y as f32 / 17., (x + y) as f32 / 40.]);
        let mask = (0..391).map(|i| if i % 23 < 11 { 1. } else { 0. }).collect::<Vec<_>>();
        for p in [
            RefineParams { radius: 4., smart_radius: true, smooth: 17., feather: 2., contrast: 22., shift_edge: 12.5 },
            RefineParams { shift_edge: -100., ..Default::default() },
        ] {
            assert_eq!(
                buffer(Some(&img), &mask, 23, 17, &p, MattingLimits::default(), &Interrupt::NONE).unwrap(),
                super::super::refine_buffer(Some(&img), &mask, 23, 17, &p)
            );
        }
    }
}

#[cfg(test)]
mod tile_tests {
    use super::*;
    #[test]
    fn clipped_halo_tiles_match_current_refinement_and_late_cancel() {
        let (w, h) = (514usize, 25usize);
        let img = RgbImage::from_fn(w, h, |x, y| [(x % 23) as f32 / 23., y as f32 / 25., (x % 31) as f32 / 31.]);
        let canvas = Rect::new(-9, 5, 505, 30);
        let sampler = crate::segment::ImageSampler { img: &img, origin: (-9, 5) };
        let region = Region { bbox: canvas, mask: (0..w * h).map(|i| if (i % w) < 260 { 255 } else { 0 }).collect() };
        let p = RefineParams { radius: 4., smart_radius: true, feather: 2., shift_edge: 12.5, ..Default::default() };
        let read = super::super::region_reader(&region);
        let expected = super::super::refine_mask(&sampler, &read, canvas, canvas, &p).unwrap();
        let actual = refine_mask_with_interrupt(&sampler, &|r| Ok(read(r)), canvas, canvas, &p, MattingLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 100;
        assert!(matches!(
            refine_mask_with_interrupt(&sampler, &|r| Ok(read(r)), canvas, canvas, &p, MattingLimits::default(), &Interrupt::cancel_only(&cancel)),
            Err(E::Cancelled)
        ));
    }
}

/// Known native output and tile-scratch envelope of the existing masked-copy
/// equation. This is narrow native arithmetic, not a wide conversion policy.
pub fn masked_copy_requirements(src: &photocraft_raster::Surface, bbox: Rect) -> Result<(u64, u64), E> {
    let fmt = src.format();
    if matches!(fmt.sample, photocraft_color::SampleType::U32 | photocraft_color::SampleType::F64) || fmt.channels() == 0 || fmt.channels() > 5 {
        return Err(E::UnsupportedConversion);
    }
    if bbox.is_empty() {
        return Ok((4096, 4096));
    }
    if bbox.width() > 300000 || bbox.height() > 300000 || [bbox.x0, bbox.x1, bbox.y0, bbox.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v)) {
        return Err(E::InvalidBounds);
    }
    let out = photocraft_color::PixelFormat { alpha: true, ..fmt };
    let bpp = out.bytes_per_pixel() as u64;
    let first = photocraft_geom::TileCoord::containing(bbox.x0, bbox.y0);
    let last = photocraft_geom::TileCoord::containing(bbox.x1 - 1, bbox.y1 - 1);
    let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .map_err(|_| E::InvalidBounds)?
        .checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| E::InvalidBounds)?)
        .ok_or(E::LimitExceeded)?;
    let padded = tiles.checked_mul(65536).ok_or(E::LimitExceeded)?;
    let retained = padded.checked_mul(bpp).and_then(|v| v.checked_add(tiles.checked_mul(1024)?)).ok_or(E::LimitExceeded)?;
    let bytes = retained.checked_add(65536 * bpp).and_then(|v| v.checked_add(4096 * bpp)).and_then(|v| v.checked_add(1 << 20)).ok_or(E::LimitExceeded)?;
    let work = padded.checked_mul(bpp).and_then(|v| v.checked_mul(32)).ok_or(E::LimitExceeded)?;
    if bytes > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok((bytes, work))
}
/// Same per-channel f32 read, alpha multiply and native encoding as masked_copy.
/// The original 256-pixel core's any-visible-alpha omission is retained, including
/// hidden RGB in other samples of an accepted core. No source color conversion.
pub fn masked_copy_with_interrupt(
    src: &photocraft_raster::Surface,
    alpha: &Region,
    l: MattingLimits,
    c: &Interrupt<'_>,
) -> Result<photocraft_raster::Surface, E> {
    use photocraft_color::{PixelFormat, read_sample, write_sample};
    use photocraft_raster::staged::{StagedSurfaceBuilder, StagingBudget};
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
    poll(c)?;
    let (budget, work) = masked_copy_requirements(src, alpha.bbox)?;
    if budget > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > l.work_units.min(MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    let bbox = alpha.bbox;
    let fmt = src.format();
    let outfmt = PixelFormat { alpha: true, ..fmt };
    if alpha.mask.len() != u64::from(bbox.width()).checked_mul(u64::from(bbox.height())).and_then(|v| usize::try_from(v).ok()).ok_or(E::LimitExceeded)? {
        return Err(E::LengthMismatch);
    }
    if bbox.is_empty() {
        return Ok(photocraft_raster::Surface::new(outfmt));
    }
    let zero = [0u8; 40];
    let default = zero.get(..outfmt.bytes_per_pixel()).ok_or(E::UnsupportedConversion)?;
    let mut prepaid = Prepaid { bytes: budget, work };
    let mut builder = StagedSurfaceBuilder::new(outfmt, default, bbox, &mut prepaid, c)?;
    let n = fmt.channels();
    let m = outfmt.channels();
    let bpp = outfmt.bytes_per_pixel();
    let mut input = [0u8; 40];
    for ty in (bbox.y0..bbox.y1).step_by(256) {
        for tx in (bbox.x0..bbox.x1).step_by(256) {
            poll(c)?;
            let core = Rect::new(tx, ty, (tx + 256).min(bbox.x1), (ty + 256).min(bbox.y1));
            let cw = core.width() as usize;
            let mut bytes = filled(cw.checked_mul(core.height() as usize).and_then(|v| v.checked_mul(bpp)).ok_or(E::LimitExceeded)?, 0u8, c)?;
            let mut any = false;
            for y in core.y0..core.y1 {
                poll(c)?;
                for x in core.x0..core.x1 {
                    if x.rem_euclid(64) == 0 {
                        poll(c)?;
                    }
                    let raw = input.get_mut(..fmt.bytes_per_pixel()).ok_or(E::LengthMismatch)?;
                    src.try_read_pixel_bytes(x, y, raw)?;
                    let base = ((y - core.y0) as usize * cw + (x - core.x0) as usize).checked_mul(bpp).ok_or(E::LimitExceeded)?;
                    let pixel = bytes.get_mut(base..base + bpp).ok_or(E::LengthMismatch)?;
                    for channel in 0..m - 1 {
                        let v = read_sample(raw, fmt.sample, channel);
                        if !v.is_finite() {
                            return Err(E::InvalidSample);
                        }
                        write_sample(pixel, fmt.sample, channel, v);
                    }
                    let sa = if fmt.alpha { read_sample(raw, fmt.sample, n - 1) } else { 1. };
                    let value = sa * alpha.at(x, y);
                    if !value.is_finite() {
                        return Err(E::InvalidSample);
                    }
                    write_sample(pixel, fmt.sample, m - 1, value);
                    any |= value > 0.;
                }
            }
            for y in (core.y0..core.y1).step_by(64) {
                for x in (core.x0..core.x1).step_by(64) {
                    poll(c)?;
                    let cell = Rect::new(x, y, (x + 64).min(core.x1), (y + 64).min(core.y1));
                    let mut raw =
                        filled((cell.width() as usize).checked_mul(cell.height() as usize).and_then(|v| v.checked_mul(bpp)).ok_or(E::LimitExceeded)?, 0u8, c)?;
                    if any {
                        for yy in cell.y0..cell.y1 {
                            poll(c)?;
                            let a = ((yy - core.y0) as usize * cw + (cell.x0 - core.x0) as usize) * bpp;
                            let dst = (yy - cell.y0) as usize * cell.width() as usize * bpp;
                            let len = cell.width() as usize * bpp;
                            raw.get_mut(dst..dst + len).ok_or(E::LengthMismatch)?.copy_from_slice(bytes.get(a..a + len).ok_or(E::LengthMismatch)?);
                        }
                    }
                    builder.push_region(cell, &raw, &mut prepaid, c)?;
                }
            }
        }
    }
    let (mut out, _) = builder.finalize(&mut prepaid, c)?.into_parts();
    out.try_prune_with(c)?;
    poll(c)?;
    Ok(out)
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn masked_copy_preserves_old_hidden_rgb_core_and_native_codes() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut source = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            let area = Rect::new(-7, 3, 263, 8);
            let mut values = Vec::new();
            for y in 0..5 {
                for x in 0..270 {
                    values.extend_from_slice(&[0.23, 0.37, 0.91, if x == 5 && y == 2 { 0.4 } else { 0. }]);
                }
            }
            source.write_region(area, &values);
            let region = Region { bbox: area, mask: vec![127; 270 * 5] };
            let old = super::super::masked_copy(&source, &region);
            let actual = masked_copy_with_interrupt(&source, &region, MattingLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(actual.format(), old.format());
            assert_eq!(actual.default_bytes(), old.default_bytes());
            assert_eq!(actual.try_read_interleaved_with(area, &Interrupt::NONE).unwrap(), old.try_read_interleaved_with(area, &Interrupt::NONE).unwrap());
            assert_eq!(actual.tile_count(), old.tile_count());
            assert!(matches!(masked_copy_with_interrupt(&source, &region, MattingLimits::default(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled)));
        }
    }
}

#[path = "decontaminate_controlled.rs"]
mod decontamination;
pub use decontamination::{DecontaminateRequirements, decontaminate_with_interrupt, requirements as decontaminate_requirements};
