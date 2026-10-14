//! Same fixed Select Subject saliency/GrabCut recipe, privately prepared.
use super::{Item, SIGMA_CLR, SIGMA_SPA, WORK_PX};
use crate::segment::{
    Region, RgbImage, Sampler,
    grabcut::{
        self,
        controlled::{Budget, finish, index, iterations, scaled},
    },
    slic::rgb_to_lab,
};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError as E};
use std::{cell::Cell, collections::BinaryHeap};
mod slic;
pub type SubjectLimits = grabcut::ObjectSelectionLimits;
pub type SubjectRequirements = grabcut::ObjectSelectionRequirements;
pub fn requirements(canvas: Rect) -> Result<SubjectRequirements, E> {
    if canvas.width() < 4 || canvas.height() < 4 {
        return Ok(SubjectRequirements { window: Rect::EMPTY, step: 1, peak_resident_bytes: 0, work_units: 1 });
    }
    let mut r = grabcut::requirements(canvas, canvas, WORK_PX)?;
    let n = (canvas.width() as usize).div_ceil(r.step).checked_mul((canvas.height() as usize).div_ceil(r.step)).ok_or(E::LimitExceeded)?;
    r.peak_resident_bytes = r.peak_resident_bytes.checked_add((n as u64).checked_mul(128).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    r.work_units = r.work_units.checked_add((n as u64).checked_mul(8192).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    Ok(r)
}
fn clone_vec<T: Clone>(v: &[T], b: &Budget<'_, '_>) -> Result<Vec<T>, E> {
    let mut out = b.reserve(v.len())?;
    for chunk in v.chunks(256) {
        b.work(chunk.len() as u64)?;
        out.extend_from_slice(chunk);
    }
    Ok(out)
}
fn blur(lab: &[[f32; 3]], w: usize, h: usize, b: &Budget<'_, '_>) -> Result<Vec<[f32; 3]>, E> {
    let mut cur = clone_vec(lab, b)?;
    for _ in 0..2 {
        let mut tmp = clone_vec(&cur, b)?;
        for y in 0..h {
            for x in 0..w {
                b.work(16)?;
                let l = *cur.get(index(x.saturating_sub(1), y, w)?).ok_or(E::LengthMismatch)?;
                let c = *cur.get(index(x, y, w)?).ok_or(E::LengthMismatch)?;
                let r = *cur.get(index((x + 1).min(w - 1), y, w)?).ok_or(E::LengthMismatch)?;
                *tmp.get_mut(index(x, y, w)?).ok_or(E::LengthMismatch)? = [0, 1, 2].map(|k| (l[k] + 2. * c[k] + r[k]) / 4.);
            }
        }
        for y in 0..h {
            for x in 0..w {
                b.work(16)?;
                let u = *tmp.get(index(x, y.saturating_sub(1), w)?).ok_or(E::LengthMismatch)?;
                let c = *tmp.get(index(x, y, w)?).ok_or(E::LengthMismatch)?;
                let d = *tmp.get(index(x, (y + 1).min(h - 1), w)?).ok_or(E::LengthMismatch)?;
                *cur.get_mut(index(x, y, w)?).ok_or(E::LengthMismatch)? = [0, 1, 2].map(|k| (u[k] + 2. * c[k] + d[k]) / 4.);
            }
        }
    }
    Ok(cur)
}
fn distance(src: usize, adj: &[Vec<usize>], means: &[[f32; 3]], dist: &mut [f32], heap: &mut BinaryHeap<Item>, b: &Budget<'_, '_>) -> Result<(), E> {
    for v in dist.iter_mut() {
        b.work(1)?;
        *v = f32::INFINITY;
    }
    *dist.get_mut(src).ok_or(E::LengthMismatch)? = 0.;
    heap.clear();
    heap.push(Item(0., src));
    while let Some(Item(d, u)) = heap.pop() {
        b.work(32)?;
        if d > *dist.get(u).ok_or(E::LengthMismatch)? {
            continue;
        }
        for &v in adj.get(u).ok_or(E::LengthMismatch)? {
            b.work(32)?;
            let weight = crate::segment::d2(*means.get(u).ok_or(E::LengthMismatch)?, *means.get(v).ok_or(E::LengthMismatch)?).sqrt();
            let nd = d + weight;
            if nd < *dist.get(v).ok_or(E::LengthMismatch)? {
                *dist.get_mut(v).ok_or(E::LengthMismatch)? = nd;
                if heap.len() == heap.capacity() {
                    return Err(E::LimitExceeded);
                }
                heap.push(Item(nd, v));
            }
        }
    }
    Ok(())
}
fn saliency(img: &RgbImage, b: &Budget<'_, '_>, extra_resident: u64) -> Result<Vec<f32>, E> {
    let (w, h) = (img.w, img.h);
    let pixels = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if img.px.len() != pixels || pixels == 0 {
        return Err(E::LengthMismatch);
    }
    let mut lab = b.reserve(pixels)?;
    for p in &img.px {
        b.work(64)?;
        lab.push(rgb_to_lab(*p));
    }
    let target = (pixels / 150).clamp(16, 400);
    let sp = slic::slic_lab(&lab, w, h, target, 10., 8, b)?;
    let n = sp.count;
    if n == 0 || sp.labels.len() != pixels || sp.w != w || sp.h != h {
        return Err(E::LengthMismatch);
    }
    // Actual post-connectivity count controls storage; no inferred target-count cap.
    // Full row capacities +weights and worst heap relaxations are retained together.
    let pairs = n.checked_mul(n).ok_or(E::LimitExceeded)?;
    let bytes =
        (pairs as u64).checked_mul(32).and_then(|v| v.checked_add((n as u64).checked_mul(512)?)).and_then(|v| v.checked_add(4096)).ok_or(E::LimitExceeded)?;
    if bytes > extra_resident {
        return Err(E::LimitExceeded);
    }
    b.ensure_work((pixels as u64).checked_mul(32).and_then(|v| v.checked_add((pairs as u64).checked_mul(256)?)).ok_or(E::LimitExceeded)?)?;
    let mut mean = b.filled(n, [0f64; 3])?;
    let mut pos = b.filled(n, [0f64; 2])?;
    let mut cnt = b.filled(n, 0f64)?;
    let mut on_border = b.filled(n, false)?;
    for y in 0..h {
        for x in 0..w {
            b.work(8)?;
            let i = index(x, y, w)?;
            let l = *sp.labels.get(i).ok_or(E::LengthMismatch)? as usize;
            let c = *lab.get(i).ok_or(E::LengthMismatch)?;
            let m = mean.get_mut(l).ok_or(E::LengthMismatch)?;
            for (a, v) in m.iter_mut().zip(c) {
                *a += f64::from(v);
            }
            let p = pos.get_mut(l).ok_or(E::LengthMismatch)?;
            p[0] += x as f64 / w as f64;
            p[1] += y as f64 / h as f64;
            *cnt.get_mut(l).ok_or(E::LengthMismatch)? += 1.;
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                *on_border.get_mut(l).ok_or(E::LengthMismatch)? = true;
            }
        }
    }
    let mut means = b.reserve(n)?;
    let mut positions = b.reserve(n)?;
    for ((m, p), c) in mean.iter().zip(&pos).zip(&cnt) {
        b.work(8)?;
        means.push(m.map(|v| (v / c.max(1.)) as f32));
        positions.push(p.map(|v| (v / c.max(1.)) as f32));
    }
    drop(mean);
    drop(pos);
    drop(cnt);
    let mut adj = b.reserve(n)?;
    for _ in 0..n {
        adj.push(b.reserve::<usize>(n)?);
    }
    let mut link = |a: usize, c: usize| -> Result<(), E> {
        b.work(adj.get(a).ok_or(E::LengthMismatch)?.len() as u64 + 1)?;
        if a != c && !adj.get(a).ok_or(E::LengthMismatch)?.contains(&c) {
            adj.get_mut(a).ok_or(E::LengthMismatch)?.push(c);
            adj.get_mut(c).ok_or(E::LengthMismatch)?.push(a);
        }
        Ok(())
    };
    for y in 0..h {
        for x in 0..w {
            let l = *sp.labels.get(index(x, y, w)?).ok_or(E::LengthMismatch)? as usize;
            if x + 1 < w {
                link(l, *sp.labels.get(index(x + 1, y, w)?).ok_or(E::LengthMismatch)? as usize)?;
            }
            if y + 1 < h {
                link(l, *sp.labels.get(index(x, y + 1, w)?).ok_or(E::LengthMismatch)? as usize)?;
            }
        }
    }
    let mut heap = BinaryHeap::new();
    let heap_len = pairs.checked_add(1).ok_or(E::LimitExceeded)?;
    heap.try_reserve_exact(heap_len).map_err(|_| E::AllocationFailed)?;
    if heap.capacity() > heap_len.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    let mut wbg = b.filled(n, 0f32)?;
    let mut geo = b.filled(n, f32::INFINITY)?;
    for (p, wb) in wbg.iter_mut().enumerate() {
        distance(p, &adj, &means, &mut geo, &mut heap, b)?;
        let (mut area, mut len) = (0f32, 0f32);
        for (q, g) in geo.iter().enumerate() {
            b.work(32)?;
            let s = (-*g * *g / (2. * SIGMA_CLR * SIGMA_CLR)).exp();
            area += s;
            if *on_border.get(q).ok_or(E::LengthMismatch)? {
                len += s;
            }
        }
        let bnd = len / area.max(1e-6).sqrt();
        *wb = 1. - (-bnd * bnd / 2.).exp();
    }
    let dapp = |a: usize, c: usize| -> Result<f32, E> {
        Ok(crate::segment::d2(*means.get(a).ok_or(E::LengthMismatch)?, *means.get(c).ok_or(E::LengthMismatch)?).sqrt())
    };
    let mut wctr = b.filled(n, 0f32)?;
    for p in 0..n {
        let mut sum = 0f32;
        for q in 0..n {
            b.work(64)?;
            let pp = *positions.get(p).ok_or(E::LengthMismatch)?;
            let pq = *positions.get(q).ok_or(E::LengthMismatch)?;
            let (dx, dy) = (pp[0] - pq[0], pp[1] - pq[1]);
            let wspa = (-(dx * dx + dy * dy) / (2. * SIGMA_SPA * SIGMA_SPA)).exp();
            sum += dapp(p, q)? * wspa * *wbg.get(q).ok_or(E::LengthMismatch)?;
        }
        *wctr.get_mut(p).ok_or(E::LengthMismatch)? = sum;
    }
    let mut mx = 0f32;
    for v in &wctr {
        b.work(1)?;
        mx = mx.max(*v);
    }
    mx = mx.max(1e-6);
    let mut wfg = b.reserve(n)?;
    for v in &wctr {
        b.work(1)?;
        wfg.push(v / mx);
    }
    let mut wij = b.reserve(n)?;
    for (p, neighbors) in adj.iter().enumerate() {
        let mut row = b.reserve(neighbors.len())?;
        for &q in neighbors {
            b.work(64)?;
            row.push((-dapp(p, q)?.powi(2) / (2. * SIGMA_CLR * SIGMA_CLR)).exp() + 0.1);
        }
        wij.push(row);
    }
    let mut s = clone_vec(&wfg, b)?;
    for _ in 0..200 {
        for p in 0..n {
            b.work(1)?;
            let (mut num, mut den) = (*wfg.get(p).ok_or(E::LengthMismatch)?, *wbg.get(p).ok_or(E::LengthMismatch)? + *wfg.get(p).ok_or(E::LengthMismatch)?);
            for (q, wq) in adj.get(p).ok_or(E::LengthMismatch)?.iter().zip(wij.get(p).ok_or(E::LengthMismatch)?) {
                b.work(8)?;
                num += wq * *s.get(*q).ok_or(E::LengthMismatch)?;
                den += wq;
            }
            *s.get_mut(p).ok_or(E::LengthMismatch)? = num / den.max(1e-6);
        }
    }
    let mut mean_all = [0f64; 3];
    for c in &lab {
        b.work(3)?;
        for (a, v) in mean_all.iter_mut().zip(c) {
            *a += f64::from(*v);
        }
    }
    let mean_all = mean_all.map(|v| (v / lab.len() as f64) as f32);
    let blurred = blur(&lab, w, h, b)?;
    let mut ft = b.reserve(pixels)?;
    for c in blurred {
        b.work(16)?;
        ft.push(crate::segment::d2(c, mean_all).sqrt());
    }
    let mut ftmax = 0f32;
    for v in &ft {
        b.work(1)?;
        ftmax = ftmax.max(*v);
    }
    ftmax = ftmax.max(1e-6);
    let (mut smin, mut smax) = (f32::MAX, f32::MIN);
    for v in &s {
        b.work(1)?;
        smin = smin.min(*v);
        smax = smax.max(*v);
    }
    let srange = (smax - smin).max(1e-6);
    let mut out = b.reserve(pixels)?;
    for (l, f) in sp.labels.iter().zip(&ft) {
        b.work(8)?;
        out.push(0.75 * (*s.get(*l as usize).ok_or(E::LengthMismatch)? - smin) / srange + 0.25 * f / ftmax);
    }
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in &out {
        b.work(1)?;
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let range = (hi - lo).max(1e-6);
    for v in &mut out {
        b.work(1)?;
        *v = (*v - lo) / range;
    }
    Ok(out)
}
fn otsu(v: &[f32], b: &Budget<'_, '_>) -> Result<f32, E> {
    let mut hist = [0u64; 256];
    for x in v {
        b.work(1)?;
        let i = (x.clamp(0., 1.) * 255.).round() as usize;
        let n = hist.get_mut(i).ok_or(E::InvalidSample)?;
        *n = n.checked_add(1).ok_or(E::LimitExceeded)?;
    }
    let total = v.len() as f64;
    let mut sum = 0f64;
    for (i, c) in hist.iter().enumerate() {
        b.work(1)?;
        sum += i as f64 * *c as f64;
    }
    let (mut wb, mut sb, mut best, mut bt) = (0f64, 0f64, -1f64, 0usize);
    for (t, c) in hist.iter().enumerate() {
        b.work(1)?;
        wb += *c as f64;
        if wb == 0. {
            continue;
        }
        let wf = total - wb;
        if wf == 0. {
            break;
        }
        sb += t as f64 * *c as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let between = wb * wf * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            bt = t;
        }
    }
    Ok((bt as f32 + 0.5) / 255.)
}
pub fn select_subject_with_interrupt(sampler: &dyn Sampler, canvas: Rect, limits: SubjectLimits, control: &Interrupt<'_>) -> Result<Option<Region>, E> {
    control.check().map_err(|_| E::Cancelled)?;
    let defaults = SubjectLimits::default();
    if limits.resident_bytes > defaults.resident_bytes || limits.work_units > defaults.work_units {
        return Err(E::LimitExceeded);
    }
    let req = requirements(canvas)?;
    if req.window.is_empty() {
        return Ok(None);
    }
    let extra = limits.resident_bytes.checked_sub(req.peak_resident_bytes).ok_or(E::LimitExceeded)?;
    if req.work_units > limits.work_units {
        return Err(E::LimitExceeded);
    }
    let b = Budget { left: Cell::new(limits.work_units), control };
    let img = scaled(sampler, canvas, req.step, &b)?;
    let (w, h) = (img.w, img.h);
    let sal = saliency(&img, &b, extra)?;
    let threshold = otsu(&sal, &b)?.clamp(0.15, 0.85);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let mut trimap = b.filled(n, grabcut::PR_BG)?;
    let mut any_fg = false;
    for y in 0..h {
        for x in 0..w {
            b.work(1)?;
            let i = index(x, y, w)?;
            let value = *sal.get(i).ok_or(E::LengthMismatch)?;
            let border = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            *trimap.get_mut(i).ok_or(E::LengthMismatch)? = if value >= threshold {
                any_fg = true;
                grabcut::PR_FG
            } else if border || value < 0.4 * threshold {
                grabcut::BG
            } else {
                grabcut::PR_BG
            };
        }
    }
    if !any_fg {
        return Ok(None);
    }
    let Some((fg, bg)) = iterations(&img, &mut trimap, 6, &b)? else { return Ok(None) };
    let mut low = b.reserve(n)?;
    for t in &trimap {
        b.work(1)?;
        low.push(*t == grabcut::PR_FG || *t == grabcut::FG);
    }
    let work = (n as u64).checked_mul(192).ok_or(E::LimitExceeded)?;
    b.work(work)?;
    let low = crate::segment::clean_mask_with_interrupt(
        &low,
        w,
        h,
        0.2,
        0.05,
        crate::segment::CleanupLimits { resident_bytes: limits.resident_bytes, work_units: work },
        control,
    )?;
    let found = low.iter().try_fold(false, |found, v| {
        b.work(1)?;
        Ok::<_, E>(found || *v)
    })?;
    if !found {
        return Ok(None);
    }
    let result = finish(sampler, canvas, req.step, &low, w, h, &fg, &bg, &b)?;
    b.work(1)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(w: usize, h: usize) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            if (x as i32 - w as i32 / 2).pow(2) + (y as i32 - h as i32 / 2).pow(2) < (h as i32 / 4).pow(2) { [0.9, 0.15, 0.1] } else { [0.1, 0.35, 0.8] }
        })
    }
    #[test]
    fn saliency_and_otsu_keep_exact_upstream_arrays() {
        for (w, h) in [(40, 32), (75, 60), (1, 37)] {
            let img = image(w, h);
            let b = Budget { left: Cell::new(8_000_000_000), control: &Interrupt::NONE };
            let actual = saliency(&img, &b, 512 << 20).unwrap();
            assert_eq!(actual, super::super::saliency(&img));
            assert_eq!(otsu(&actual, &b).unwrap(), super::super::otsu(&actual));
        }
    }
    #[test]
    fn subject_whole_recipe_matches_and_cancellation_is_error() {
        let img = image(80, 64);
        let source = crate::segment::ImageSampler { img: &img, origin: (0, 0) };
        let canvas = Rect::from_xywh(0, 0, 80, 64);
        let expected = super::super::select_subject(&source, canvas).unwrap();
        let actual = select_subject_with_interrupt(&source, canvas, SubjectLimits::default(), &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(select_subject_with_interrupt(&source, canvas, SubjectLimits::default(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
    }
}
