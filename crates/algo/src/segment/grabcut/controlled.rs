//! Exact rectangle GrabCut companion. No point/Subject/source qualification.
use super::{BG, GAMMA, K, MAX_SAMPLES, PR_BG, PR_FG, is_fg};
use crate::segment::{
    Region, RgbImage, Sampler,
    gmm::{DEFAULT_REG, Gmm},
    maxflow::Graph,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
use std::cell::Cell;
#[derive(Clone, Copy, Debug)]
pub struct ObjectSelectionLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for ObjectSelectionLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
pub(crate) struct Budget<'a, 'b> {
    pub(crate) left: Cell<u64>,
    pub(crate) control: &'a Interrupt<'b>,
}
impl Budget<'_, '_> {
    pub(crate) fn ensure_work(&self, n: u64) -> Result<(), E> {
        self.control.check().map_err(|_| E::Cancelled)?;
        if n > self.left.get() {
            return Err(E::LimitExceeded);
        }
        Ok(())
    }
    pub(crate) fn work(&self, n: u64) -> Result<(), E> {
        self.control.check().map_err(|_| E::Cancelled)?;
        self.left.set(self.left.get().checked_sub(n).ok_or(E::LimitExceeded)?);
        Ok(())
    }
    pub(crate) fn filled<T: Clone>(&self, n: usize, value: T) -> Result<Vec<T>, E> {
        let mut out = self.reserve(n)?;
        while out.len() < n {
            let end = out.len().saturating_add(1024).min(n);
            self.work((end - out.len()) as u64)?;
            out.resize(end, value.clone());
        }
        Ok(out)
    }
    pub(crate) fn reserve<T>(&self, n: usize) -> Result<Vec<T>, E> {
        self.work(1)?;
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
        if out.capacity() > n.saturating_add(16) {
            return Err(E::AllocationFailed);
        }
        Ok(out)
    }
}
pub(crate) fn index(x: usize, y: usize, w: usize) -> Result<usize, E> {
    y.checked_mul(w).and_then(|v| v.checked_add(x)).ok_or(E::LimitExceeded)
}
fn at(img: &RgbImage, x: usize, y: usize) -> Result<[f32; 3], E> {
    img.px.get(index(x, y, img.w)?).copied().ok_or(E::LengthMismatch)
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
pub(crate) fn scaled(s: &dyn Sampler, r: Rect, step: usize, b: &Budget<'_, '_>) -> Result<RgbImage, E> {
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
                        for (a, v) in acc.iter_mut().zip(at(&strip, xx, yy)?) {
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
fn beta(img: &RgbImage, b: &Budget<'_, '_>) -> Result<f32, E> {
    let (mut sum, mut n) = (0f64, 0usize);
    for y in 0..img.h {
        for x in 0..img.w {
            b.work(4)?;
            let a = at(img, x, y)?;
            for (dx, dy, _) in crate::segment::NB {
                let nx = x as i64 + i64::from(dx);
                let ny = y as i64 + i64::from(dy);
                if nx >= 0 && ny >= 0 && nx < img.w as i64 && ny < img.h as i64 {
                    sum += f64::from(crate::segment::d2(a, at(img, nx as usize, ny as usize)?));
                    n += 1;
                }
            }
        }
    }
    Ok(if n == 0 || sum <= 1e-12 { 0. } else { (n as f64 / (2. * sum)) as f32 })
}
fn cut(img: &RgbImage, cf: &[f32], cb: &[f32], fixed: &[u8], beta: f32, b: &Budget<'_, '_>) -> Result<Vec<bool>, E> {
    let n = img.w.checked_mul(img.h).ok_or(E::LimitExceeded)?;
    if [img.px.len(), cf.len(), cb.len(), fixed.len()].iter().any(|v| *v != n) {
        return Err(E::LengthMismatch);
    }
    b.ensure_work((n as u64).checked_mul(256).ok_or(E::LimitExceeded)?)?;
    let mut idx = b.filled(n, u32::MAX)?;
    let mut count = 0usize;
    let mut out = b.reserve(n)?;
    for (i, f) in fixed.iter().enumerate() {
        b.work(1)?;
        out.push(*f == crate::segment::HARD_FG);
        if *f == crate::segment::FREE {
            *idx.get_mut(i).ok_or(E::LengthMismatch)? = u32::try_from(count).map_err(|_| E::LimitExceeded)?;
            count += 1;
        }
    }
    if count == 0 {
        return Ok(out);
    }
    let mut graph = Graph::try_with_capacity(count, count.checked_mul(4).ok_or(E::LimitExceeded)?, b.control)?;
    for i in 0..n {
        b.work(1)?;
        let id = *idx.get(i).ok_or(E::LengthMismatch)?;
        if id != u32::MAX {
            graph.add_tweights(id as usize, *cb.get(i).ok_or(E::LengthMismatch)?, *cf.get(i).ok_or(E::LengthMismatch)?);
        }
    }
    // Same optimized sparse edge-start ordering as the legacy grid cut.
    let mut free = b.reserve(count)?;
    let sparse = count < n / 8;
    if sparse {
        for (i, id) in idx.iter().enumerate() {
            b.work(1)?;
            if *id != u32::MAX {
                free.push(i);
            }
        }
    }
    let mut visit = |i: usize| -> Result<(), E> {
        let x = i % img.w;
        let y = i / img.w;
        let a = at(img, x, y)?;
        for (dx, dy, len) in crate::segment::NB {
            b.work(1)?;
            let nx = x as i64 + i64::from(dx);
            let ny = y as i64 + i64::from(dy);
            if nx < 0 || ny < 0 || nx >= img.w as i64 || ny >= img.h as i64 {
                continue;
            }
            let j = index(nx as usize, ny as usize, img.w)?;
            let ia = *idx.get(i).ok_or(E::LengthMismatch)?;
            let ib = *idx.get(j).ok_or(E::LengthMismatch)?;
            if ia == u32::MAX && ib == u32::MAX {
                continue;
            }
            let wt = GAMMA * (-beta * crate::segment::d2(a, at(img, nx as usize, ny as usize)?)).exp() / len;
            match (ia != u32::MAX, ib != u32::MAX) {
                (true, true) => graph.add_edge(ia as usize, ib as usize, wt, wt),
                (true, false) => crate::segment::add_fixed_link(&mut graph, ia as usize, *fixed.get(j).ok_or(E::LengthMismatch)?, wt),
                (false, true) => crate::segment::add_fixed_link(&mut graph, ib as usize, *fixed.get(i).ok_or(E::LengthMismatch)?, wt),
                _ => (),
            }
        }
        Ok(())
    };
    if sparse {
        for i in crate::segment::edge_starts::Starts::new(&free, img.w) {
            visit(i)?;
        }
    } else {
        for i in 0..n {
            visit(i)?;
        }
    }
    let stopped = Cell::new(None);
    let cancel = || match b.work(1) {
        Ok(()) => false,
        Err(e) => {
            stopped.set(Some(e));
            true
        }
    };
    if graph.maxflow_controlled(&cancel).is_none() {
        return Err(stopped.get().unwrap_or(E::AllocationFailed));
    }
    for (i, id) in idx.iter().enumerate() {
        b.work(1)?;
        if *id != u32::MAX {
            *out.get_mut(i).ok_or(E::LengthMismatch)? = graph.in_source(*id as usize);
        }
    }
    Ok(out)
}
fn costs(img: &RgbImage, fg: &Gmm, bg: &Gmm, b: &Budget<'_, '_>) -> Result<(Vec<f32>, Vec<f32>), E> {
    let mut cf = b.reserve(img.px.len())?;
    let mut cb = b.reserve(img.px.len())?;
    for p in &img.px {
        b.work(256)?;
        cf.push(fg.neg_log(*p));
        cb.push(bg.neg_log(*p));
    }
    Ok((cf, cb))
}
fn fit_samples(img: &RgbImage, trimap: &[u8], foreground: bool, b: &Budget<'_, '_>) -> Result<Vec<[f32; 3]>, E> {
    let count = trimap.iter().try_fold(0usize, |n, t| {
        b.work(1)?;
        Ok::<_, E>(n + usize::from(is_fg(*t) == foreground))
    })?;
    let stride = count.div_ceil(MAX_SAMPLES).max(1);
    let mut out = b.reserve(count.div_ceil(stride))?;
    let mut ordinal = 0usize;
    for (p, t) in img.px.iter().zip(trimap) {
        b.work(1)?;
        if is_fg(*t) == foreground {
            if ordinal.is_multiple_of(stride) {
                out.push(*p);
            }
            ordinal += 1;
        }
    }
    Ok(out)
}
pub(crate) fn iterations(img: &RgbImage, trimap: &mut [u8], iters: usize, b: &Budget<'_, '_>) -> Result<Option<(Gmm, Gmm)>, E> {
    let beta = beta(img, b)?;
    let mut fixed = b.reserve(trimap.len())?;
    for t in trimap.iter() {
        b.work(1)?;
        fixed.push(match *t {
            super::FG => crate::segment::HARD_FG,
            BG => crate::segment::HARD_BG,
            _ => crate::segment::FREE,
        });
    }
    let mut models: Option<(Gmm, Gmm)> = None;
    for _ in 0..iters.max(1) {
        b.work(1)?;
        let fs = fit_samples(img, trimap, true, b)?;
        let bs = fit_samples(img, trimap, false, b)?;
        // Exact existing fit/refit recipe; charge its bounded deterministic loops before allocation.
        b.work((fs.len() + bs.len()) as u64 * 4096)?;
        let (fg, bg) = match &models {
            None => {
                match (Gmm::fit_grabcut_with_interrupt(&fs, K, DEFAULT_REG, b.control)?, Gmm::fit_grabcut_with_interrupt(&bs, K, DEFAULT_REG, b.control)?) {
                    (Some(f), Some(g)) => (f, g),
                    _ => return Ok(None),
                }
            }
            Some((f, g)) => (
                match f.refit_grabcut_with_interrupt(&fs, b.control)? {
                    Some(v) => v,
                    None => f.clone_grabcut_with_interrupt(b.control)?,
                },
                match g.refit_grabcut_with_interrupt(&bs, b.control)? {
                    Some(v) => v,
                    None => g.clone_grabcut_with_interrupt(b.control)?,
                },
            ),
        };
        let (cf, cb) = costs(img, &fg, &bg, b)?;
        let result = cut(img, &cf, &cb, &fixed, beta, b)?;
        let mut changed = 0usize;
        for (t, v) in trimap.iter_mut().zip(&result) {
            b.work(1)?;
            if *t == PR_BG || *t == PR_FG {
                let next = if *v { PR_FG } else { PR_BG };
                changed += usize::from(next != *t);
                *t = next;
            }
        }
        models = Some((fg, bg));
        if changed.checked_mul(2000).ok_or(E::LimitExceeded)? < trimap.len() {
            break;
        }
    }
    Ok(models)
}

// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish(
    s: &dyn Sampler,
    window: Rect,
    step: usize,
    low: &[bool],
    lw: usize,
    lh: usize,
    fg: &Gmm,
    bg: &Gmm,
    b: &Budget<'_, '_>,
) -> Result<Option<Region>, E> {
    let (w, h, n) = crate::segment::sample_shape(window, 1)?;
    if step == 0 || lw != w.div_ceil(step) || lh != h.div_ceil(step) || lw.checked_mul(lh) != Some(low.len()) {
        return Err(E::LengthMismatch);
    }
    b.ensure_work((n as u64).checked_mul(24).ok_or(E::LimitExceeded)?)?;
    let mut mask = b.filled(n, 0u8)?;
    for y in 0..h {
        b.work(1)?;
        let ly = (y / step).min(lh - 1);
        for x in 0..w {
            b.work(1)?;
            *mask.get_mut(index(x, y, w)?).ok_or(E::LengthMismatch)? =
                if *low.get(index((x / step).min(lw - 1), ly, lw)?).ok_or(E::LengthMismatch)? { 255 } else { 0 };
        }
    }
    if step > 1 {
        let mut band = b.filled(low.len(), false)?;
        for y in 0..lh {
            for x in 0..lw {
                let value = *low.get(index(x, y, lw)?).ok_or(E::LengthMismatch)?;
                let mut edge = false;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        b.work(1)?;
                        let nx = x as i64 + dx;
                        let ny = y as i64 + dy;
                        if nx >= 0
                            && ny >= 0
                            && nx < lw as i64
                            && ny < lh as i64
                            && *low.get(index(nx as usize, ny as usize, lw)?).ok_or(E::LengthMismatch)? != value
                        {
                            edge = true;
                        }
                    }
                }
                *band.get_mut(index(x, y, lw)?).ok_or(E::LengthMismatch)? = edge;
            }
        }
        // Same 192-pixel core/12-pixel halo and core ordering; each core is committed
        // privately as soon as finished, rather than retaining every parallel result.
        const T: i32 = 192;
        const HALO: i32 = 12;
        let mut ty = window.y0;
        while ty < window.y1 {
            let mut tx = window.x0;
            while tx < window.x1 {
                b.work(1)?;
                let tile = Rect::new(tx, ty, tx.saturating_add(T).min(window.x1), ty.saturating_add(T).min(window.y1));
                let bx0 = (tile.x0 - window.x0) as usize / step;
                let by0 = (tile.y0 - window.y0) as usize / step;
                let bx1 = ((tile.x1 - 1 - window.x0) as usize / step).min(lw - 1);
                let by1 = ((tile.y1 - 1 - window.y0) as usize / step).min(lh - 1);
                let mut touches = false;
                for yy in by0..=by1 {
                    for xx in bx0..=bx1 {
                        b.work(1)?;
                        touches |= *band.get(index(xx, yy, lw)?).ok_or(E::LengthMismatch)?;
                    }
                }
                if touches {
                    let r = tile.inflate(HALO).intersect(&window);
                    let halo_pixels = u64::from(r.width()).checked_mul(u64::from(r.height())).ok_or(E::LimitExceeded)?;
                    b.ensure_work(halo_pixels.checked_mul(800).ok_or(E::LimitExceeded)?)?;
                    let img = rgb(s, r, b)?;
                    let rn = img.w.checked_mul(img.h).ok_or(E::LimitExceeded)?;
                    let mut fixed = b.filled(rn, crate::segment::FREE)?;
                    let mut cf = b.filled(rn, 0f32)?;
                    let mut cb = b.filled(rn, 0f32)?;
                    for yy in 0..img.h {
                        for xx in 0..img.w {
                            b.work(256)?;
                            let i = index(xx, yy, img.w)?;
                            let gx = i64::from(r.x0) + xx as i64 - i64::from(window.x0);
                            let gy = i64::from(r.y0) + yy as i64 - i64::from(window.y0);
                            let block = index((gx as usize / step).min(lw - 1), (gy as usize / step).min(lh - 1), lw)?;
                            let label = *low.get(block).ok_or(E::LengthMismatch)?;
                            if !*band.get(block).ok_or(E::LengthMismatch)? {
                                *fixed.get_mut(i).ok_or(E::LengthMismatch)? = if label { crate::segment::HARD_FG } else { crate::segment::HARD_BG };
                                continue;
                            }
                            let p = *img.px.get(i).ok_or(E::LengthMismatch)?;
                            *cf.get_mut(i).ok_or(E::LengthMismatch)? = fg.neg_log(p) + if label { 0. } else { 0.4 };
                            *cb.get_mut(i).ok_or(E::LengthMismatch)? = bg.neg_log(p) + if label { 0.4 } else { 0. };
                        }
                    }
                    let result = cut(&img, &cf, &cb, &fixed, beta(&img, b)?, b)?;
                    for yy in tile.y0..tile.y1 {
                        for xx in tile.x0..tile.x1 {
                            b.work(1)?;
                            let value = *result.get(index((xx - r.x0) as usize, (yy - r.y0) as usize, img.w)?).ok_or(E::LengthMismatch)?;
                            *mask.get_mut(index((xx - window.x0) as usize, (yy - window.y0) as usize, w)?).ok_or(E::LengthMismatch)? =
                                if value { 255 } else { 0 };
                        }
                    }
                }
                tx = tx.saturating_add(T).min(window.x1);
            }
            ty = ty.saturating_add(T).min(window.y1);
        }
    }
    // Exact antialias_u8 neighborhood math, with a fallible private source copy.
    let mut src = b.reserve(n)?;
    for chunk in mask.chunks(1024) {
        b.work(chunk.len() as u64)?;
        src.extend_from_slice(chunk);
    }
    for y in 0..h {
        for x in 0..w {
            b.work(9)?;
            let i = index(x, y, w)?;
            let value = *src.get(i).ok_or(E::LengthMismatch)?;
            let (mut sum, mut edge) = (0u32, false);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    let v = if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        value
                    } else {
                        *src.get(index(nx as usize, ny as usize, w)?).ok_or(E::LengthMismatch)?
                    };
                    sum += u32::from(v);
                    edge |= v != value;
                }
            }
            if edge {
                let avg = sum as f32 / (9. * 255.);
                let f = if value > 127 { 0.5 + 0.5 * avg } else { 0.5 * avg };
                *mask.get_mut(i).ok_or(E::LengthMismatch)? = (f * 255.).round() as u8;
            }
        }
    }
    drop(src);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            b.work(1)?;
            if *mask.get(index(x, y, w)?).ok_or(E::LengthMismatch)? != 0 {
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
        return Ok(Some(Region { bbox: window, mask }));
    }
    let mut trimmed = b.reserve((x1 - x0).checked_mul(y1 - y0).ok_or(E::LimitExceeded)?)?;
    for y in y0..y1 {
        b.work((x1 - x0) as u64)?;
        trimmed.extend_from_slice(mask.get(index(x0, y, w)?..index(x1, y, w)?).ok_or(E::LengthMismatch)?);
    }
    let coord = |base: i32, n: usize| base.checked_add(i32::try_from(n).map_err(|_| E::InvalidBounds)?).ok_or(E::InvalidBounds);
    b.work(1)?;
    Ok(Some(Region { bbox: Rect::new(coord(window.x0, x0)?, coord(window.y0, y0)?, coord(window.x0, x1)?, coord(window.y0, y1)?), mask: trimmed }))
}
/// Bounded same-math rectangle Object Selection. Caller retains source pixels;
/// the selected window/result and all intermediate graph/model storage are private.
pub fn object_select_with_interrupt(
    sampler: &dyn Sampler,
    canvas: Rect,
    rect: Rect,
    work_px: usize,
    limits: ObjectSelectionLimits,
    control: &Interrupt<'_>,
) -> Result<Option<Region>, E> {
    control.check().map_err(|_| E::Cancelled)?;
    let defaults = ObjectSelectionLimits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units {
        return Err(E::LimitExceeded);
    }
    let req = requirements(canvas, rect, work_px)?;
    if req.window.is_empty() {
        return Ok(None);
    }
    let rect = rect.intersect(&canvas);
    let window = req.window;
    let step = req.step;
    let (w, h, _) = crate::segment::sample_shape(window, 1)?;
    let lw = w.div_ceil(step);
    let lh = h.div_ceil(step);
    let low_n = lw.checked_mul(lh).ok_or(E::LimitExceeded)?;
    if req.peak_resident_bytes > limits.resident_bytes || req.work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    let b = Budget { left: Cell::new(limits.work_units), control };
    b.work(1)?;
    let img = scaled(sampler, window, step, &b)?;
    if img.w != lw || img.h != lh {
        return Err(E::LengthMismatch);
    }
    let mut trimap = b.filled(low_n, BG)?;
    let mut any_bg = false;
    for y in 0..lh {
        for x in 0..lw {
            b.work(1)?;
            let cx = i64::from(window.x0) + x.checked_mul(step).and_then(|v| v.checked_add(step / 2)).ok_or(E::LimitExceeded)? as i64;
            let cy = i64::from(window.y0) + y.checked_mul(step).and_then(|v| v.checked_add(step / 2)).ok_or(E::LimitExceeded)? as i64;
            let inside = rect.contains(i32::try_from(cx).map_err(|_| E::InvalidBounds)?, i32::try_from(cy).map_err(|_| E::InvalidBounds)?);
            if inside {
                *trimap.get_mut(index(x, y, lw)?).ok_or(E::LengthMismatch)? = PR_FG;
            } else {
                any_bg = true;
            }
        }
    }
    if !any_bg {
        let ring = (lw.min(lh) / 50).max(1);
        for y in 0..lh {
            for x in 0..lw {
                b.work(1)?;
                if x < ring || y < ring || x + ring >= lw || y + ring >= lh {
                    *trimap.get_mut(index(x, y, lw)?).ok_or(E::LengthMismatch)? = BG;
                }
            }
        }
    }
    let Some((fg, bg)) = iterations(&img, &mut trimap, 8, &b)? else { return Ok(None) };
    let mut low = b.reserve(low_n)?;
    for t in &trimap {
        b.work(1)?;
        low.push(is_fg(*t));
    }
    let cleanup_work = (low_n as u64).checked_mul(192).ok_or(E::LimitExceeded)?;
    b.work(cleanup_work)?;
    let low = crate::segment::clean_mask_with_interrupt(
        &low,
        lw,
        lh,
        0.1,
        0.002,
        crate::segment::CleanupLimits { resident_bytes: limits.resident_bytes, work_units: cleanup_work },
        control,
    )?;
    let result = finish(sampler, window, step, &low, lw, lh, &fg, &bg, &b)?;
    b.work(1)?;
    Ok(result)
}

#[derive(Clone, Copy, Debug)]
pub struct ObjectSelectionRequirements {
    pub window: Rect,
    pub step: usize,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
/// Allocation-free bounds/known storage and deterministic preparation admission.
/// Graph solve has a dynamic work debit as well; this is not an unbounded solver promise.
pub fn requirements(canvas: Rect, rect: Rect, work_px: usize) -> Result<ObjectSelectionRequirements, E> {
    let rect = rect.intersect(&canvas);
    if rect.width() < 2 || rect.height() < 2 {
        return Ok(ObjectSelectionRequirements { window: Rect::EMPTY, step: 1, peak_resident_bytes: 0, work_units: 1 });
    }
    let margin = ((rect.width().max(rect.height()) as f32 * 0.15).ceil() as i32).max(8);
    let window = rect.inflate(margin).intersect(&canvas);
    let step = crate::segment::scale_for(window, work_px);
    let (w, h, n) = crate::segment::sample_shape(window, 1)?;
    let low_n = w.div_ceil(step).checked_mul(h.div_ceil(step)).ok_or(E::LimitExceeded)?;
    let peak_resident_bytes = (n as u64)
        .checked_mul(3)
        .and_then(|v| v.checked_add((low_n as u64).checked_mul(320)?))
        .and_then(|v| v.checked_add((w as u64).checked_mul((h.min(64usize.saturating_add(step)) as u64).checked_mul(28)?)?))
        .and_then(|v| v.checked_add(216 * 216 * 320 + 8 * 1024 * 1024))
        .ok_or(E::LimitExceeded)?;
    let work_units = (n as u64)
        .checked_mul(24)
        .and_then(|v| v.checked_add((low_n as u64).checked_mul(512)?))
        .and_then(|v| v.checked_add((low_n.min(2 * MAX_SAMPLES) as u64).checked_mul(4096 * 8)?))
        .ok_or(E::LimitExceeded)?;
    Ok(ObjectSelectionRequirements { window, step, peak_resident_bytes, work_units })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_matches_old_disc_region_and_cancel() {
        let image = crate::segment::RgbImage::from_fn(80, 64, |x, y| {
            if (x as i32 - 40).pow(2) + (y as i32 - 32).pow(2) < 18 * 18 { [0.9, 0.1, 0.1] } else { [0.1, 0.3, 0.8] }
        });
        let source = crate::segment::ImageSampler { img: &image, origin: (0, 0) };
        let canvas = Rect::from_xywh(0, 0, 80, 64);
        let rect = Rect::from_xywh(18, 10, 44, 44);
        let expected = super::super::object_select(&source, canvas, rect, 160_000).unwrap();
        let actual = object_select_with_interrupt(&source, canvas, rect, 160_000, ObjectSelectionLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            object_select_with_interrupt(&source, canvas, rect, 160_000, ObjectSelectionLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(E::Cancelled)
        );
    }
}

mod point;
pub use point::{object_at_with_interrupt, point_requirements};
