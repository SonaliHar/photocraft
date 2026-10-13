//! Checked ordinary fill compilation/rasterization. Geometry math is shared
//! with the established rasterizer; staging, ordering and polling are explicit.
use super::*;
use crate::VectorPreparationLimits;
use photocraft_doc::{Path, Subpath};
use photocraft_raster::{Interrupt, RasterInputError as E};
use std::cmp::Ordering;

struct Control<'a, 'b> {
    interrupt: &'a Interrupt<'b>,
    limits: VectorPreparationLimits,
    live: usize,
    work: u64,
    since_poll: u64,
}
impl<'a, 'b> Control<'a, 'b> {
    fn new(limits: VectorPreparationLimits, interrupt: &'a Interrupt<'b>) -> Result<Self, E> {
        interrupt.check().map_err(|_| E::Cancelled)?;
        Ok(Self { interrupt, limits, live: 0, work: 0, since_poll: 0 })
    }
    fn tick(&mut self, n: u64) -> Result<(), E> {
        self.work = self.work.checked_add(n).ok_or(E::LimitExceeded)?;
        if self.work > self.limits.max_work {
            return Err(E::LimitExceeded);
        }
        self.since_poll = self.since_poll.saturating_add(n);
        if self.since_poll >= 256 {
            self.interrupt.check().map_err(|_| E::Cancelled)?;
            self.since_poll = 0;
        }
        Ok(())
    }
    fn reserve<T>(&mut self, vec: &mut Vec<T>, target: usize) -> Result<(), E> {
        if target <= vec.capacity() {
            return Ok(());
        }
        let bytes = target.checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        // Old allocation remains charged while a growing Vec may relocate.
        if self.live.checked_add(bytes).is_none_or(|n| n > self.limits.max_bytes || n as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES) {
            return Err(E::LimitExceeded);
        }
        self.interrupt.check().map_err(|_| E::Cancelled)?;
        let old = vec.capacity();
        vec.try_reserve_exact(target.saturating_sub(vec.len())).map_err(|_| E::AllocationFailed)?;
        let new_bytes = vec.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        if self.live.checked_add(new_bytes).is_none_or(|n| n > self.limits.max_bytes) {
            return Err(E::LimitExceeded);
        }
        self.live = self.live.checked_add((vec.capacity() - old) * std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        Ok(())
    }
    fn push<T>(&mut self, vec: &mut Vec<T>, value: T, cap: usize) -> Result<(), E> {
        self.tick(1)?;
        if vec.len() >= cap {
            return Err(E::LimitExceeded);
        }
        if vec.len() == vec.capacity() {
            let target = vec.len().saturating_add(vec.len().max(256)).min(cap);
            self.reserve(vec, target)?;
        }
        vec.push(value);
        Ok(())
    }
    fn release<T>(&mut self, vec: Vec<T>) {
        self.live = self.live.saturating_sub(vec.capacity().saturating_mul(std::mem::size_of::<T>()));
        drop(vec);
    }
    fn zeros<T: Default + Clone>(&mut self, n: usize) -> Result<Vec<T>, E> {
        let mut v = Vec::new();
        self.reserve(&mut v, n)?;
        while v.len() < n {
            self.interrupt.check().map_err(|_| E::Cancelled)?;
            v.resize(v.len().saturating_add(4096).min(n), T::default());
        }
        Ok(v)
    }
}
fn stable_sort<T: Copy>(values: &mut [T], compare: impl Fn(&T, &T) -> Ordering, ctl: &mut Control<'_, '_>) -> Result<(), E> {
    if values.len() < 2 {
        return Ok(());
    }
    let mut scratch = Vec::new();
    ctl.reserve(&mut scratch, values.len())?;
    for chunk in values.chunks(256) {
        ctl.tick(chunk.len() as u64)?;
        scratch.extend_from_slice(chunk);
    }
    let mut span = 1_usize;
    while span < values.len() {
        for start in (0..values.len()).step_by(span.saturating_mul(2)) {
            let mid = start.saturating_add(span).min(values.len());
            let end = mid.saturating_add(span).min(values.len());
            let (mut a, mut b) = (start, mid);
            for k in start..end {
                ctl.tick(1)?;
                let take_left =
                    b == end || (a < mid && compare(values.get(a).ok_or(E::LengthMismatch)?, values.get(b).ok_or(E::LengthMismatch)?) != Ordering::Greater);
                let idx = if take_left {
                    let old = a;
                    a += 1;
                    old
                } else {
                    let old = b;
                    b += 1;
                    old
                };
                *scratch.get_mut(k).ok_or(E::LengthMismatch)? = *values.get(idx).ok_or(E::LengthMismatch)?;
            }
        }
        for (out, src) in values.chunks_mut(256).zip(scratch.chunks(256)) {
            ctl.tick(out.len() as u64)?;
            out.copy_from_slice(src);
        }
        span = span.checked_mul(2).ok_or(E::LimitExceeded)?;
    }
    ctl.release(scratch);
    Ok(())
}
fn flatten(s: &Subpath, tol: f64, ctl: &mut Control<'_, '_>) -> Result<Vec<(f64, f64)>, E> {
    let mut pts = Vec::new();
    fn append(v: &mut Vec<(f64, f64)>, p: (f64, f64), ctl: &mut Control<'_, '_>) -> Result<(), E> {
        if !(p.0.is_finite() && p.1.is_finite()) {
            return Err(E::InvalidSample);
        }
        if v.last().is_some_and(|last| (last.0 - p.0).abs() < 1e-12 && (last.1 - p.1).abs() < 1e-12) {
            return Ok(());
        }
        ctl.push(v, p, ctl.limits.max_edges)
    }
    if let Some(k) = s.knots.first() {
        append(&mut pts, (k.anchor.x, k.anchor.y), ctl)?;
    }
    let count = if s.knots.len() < 2 {
        0
    } else if s.closed {
        s.knots.len()
    } else {
        s.knots.len() - 1
    };
    for i in 0..count {
        ctl.tick(1)?;
        let a = s.knots.get(i).ok_or(E::LengthMismatch)?;
        let b = s.knots.get((i + 1) % s.knots.len()).ok_or(E::LengthMismatch)?;
        let seg = [a.anchor, a.out_ctrl, b.in_ctrl, b.anchor];
        if seg.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err(E::InvalidSample);
        }
        if crate::flatten::is_line(&seg) {
            append(&mut pts, (seg[3].x, seg[3].y), ctl)?;
            continue;
        }
        let n = crate::flatten::pieces(&seg, tol);
        for j in 1..=n {
            ctl.tick(1)?;
            append(&mut pts, crate::flatten::eval(&seg, j as f64 / n as f64), ctl)?;
        }
    }
    if s.closed
        && pts.len() > 1
        && let (Some(a), Some(b)) = (pts.first(), pts.last())
        && (a.0 - b.0).abs() < 1e-12
        && (a.1 - b.1).abs() < 1e-12
    {
        pts.pop();
    }
    Ok(pts)
}
impl Rasterizer {
    pub(crate) fn try_fill_path(path: &Path, tol: f64, limits: VectorPreparationLimits, interrupt: &Interrupt<'_>) -> Result<Self, E> {
        let mut ctl = Control::new(limits, interrupt)?;
        if !tol.is_finite() || tol <= 0.0 || path.subpaths.len() > limits.max_components {
            return Err(E::LimitExceeded);
        }
        let mut r = Self::new(path.inverted);
        for (i, subpath) in path.subpaths.iter().enumerate() {
            ctl.tick(1)?;
            if i == 0 || subpath.op != PathOp::Join {
                let op = if i == 0 { PathOp::Combine } else { subpath.op };
                if op != PathOp::Combine && !r.comps.is_empty() {
                    r.union_only = false;
                }
                ctl.push(&mut r.comps, Component { op, rule: path.fill_rule }, limits.max_components)?;
            }
            let comp = u32::try_from(r.comps.len().saturating_sub(1)).map_err(|_| E::LimitExceeded)?;
            let poly = flatten(subpath, tol, &mut ctl)?;
            if poly.len() >= 2 {
                for k in 0..poly.len() {
                    ctl.tick(1)?;
                    let a = *poly.get(k).ok_or(E::LengthMismatch)?;
                    let b = *poly.get((k + 1) % poly.len()).ok_or(E::LengthMismatch)?;
                    r.bounds = Some(match r.bounds {
                        None => (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
                        Some(bb) => (bb.0.min(a.0.min(b.0)), bb.1.min(a.1.min(b.1)), bb.2.max(a.0.max(b.0)), bb.3.max(a.1.max(b.1))),
                    });
                    if a.1 == b.1 {
                        continue;
                    }
                    let (p, q, dir) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
                    let dxdy = (q.0 - p.0) / (q.1 - p.1);
                    if !dxdy.is_finite() {
                        return Err(E::InvalidSample);
                    }
                    ctl.push(&mut r.edges, Edge { x0: p.0, y0: p.1, y1: q.1, dir, comp, dxdy }, limits.max_edges)?;
                }
            }
            ctl.release(poly);
        }
        stable_sort(&mut r.edges, |a, b| a.y0.total_cmp(&b.y0), &mut ctl)?;
        interrupt.check().map_err(|_| E::Cancelled)?;
        Ok(r)
    }
    pub(crate) fn try_render_checked(&self, rect: Rect, limits: VectorPreparationLimits, interrupt: &Interrupt<'_>) -> Result<Vec<f32>, E> {
        let bytes = photocraft_raster::checked_region_byte_len(
            rect,
            photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false),
        )?;
        let mut ctl = Control::new(limits, interrupt)?;
        if self.edges.len() > limits.max_edges || self.comps.len() > limits.max_components || self.preparation_bytes() > limits.max_bytes {
            return Err(E::LimitExceeded);
        }
        ctl.live = self.preparation_bytes();
        let mut out = ctl.zeros::<f32>(bytes / 4)?;
        if bytes != 0 {
            self.render_checked_into(rect, &mut out, &mut ctl)?;
        }
        interrupt.check().map_err(|_| E::Cancelled)?;
        Ok(out)
    }
    fn render_checked_into(&self, rect: Rect, out: &mut [f32], ctl: &mut Control<'_, '_>) -> Result<(), E> {
        let w = rect.width() as usize;
        let mut st = RowState {
            active: Vec::new(),
            cuts: Vec::new(),
            band: Vec::new(),
            wind: ctl.zeros(self.comps.len().max(1))?,
            acc: ctl.zeros(w.checked_add(1).ok_or(E::LimitExceeded)?)?,
        };
        ctl.reserve(&mut st.active, self.edges.len())?;
        ctl.reserve(&mut st.cuts, self.edges.len().checked_mul(2).and_then(|n| n.checked_add(2)).ok_or(E::LimitExceeded)?)?;
        ctl.reserve(&mut st.band, self.edges.len())?;
        let top = f64::from(rect.y0);
        let mut next = self.edges.partition_point(|e| e.y0 < top);
        for i in 0..next {
            ctl.tick(1)?;
            if self.edges.get(i).ok_or(E::LengthMismatch)?.y1 > top {
                st.active.push(i);
            }
        }
        for (row, y) in (rect.y0..rect.y1).enumerate() {
            ctl.tick(1)?;
            let (ya, yb) = (f64::from(y), f64::from(y) + 1.0);
            let mut kept = 0;
            for index in 0..st.active.len() {
                ctl.tick(1)?;
                let i = *st.active.get(index).ok_or(E::LengthMismatch)?;
                if self.edges.get(i).ok_or(E::LengthMismatch)?.y1 > ya {
                    *st.active.get_mut(kept).ok_or(E::LengthMismatch)? = i;
                    kept += 1;
                }
            }
            st.active.truncate(kept);
            while next < self.edges.len() && self.edges.get(next).ok_or(E::LengthMismatch)?.y0 < yb {
                ctl.tick(1)?;
                if self.edges.get(next).ok_or(E::LengthMismatch)?.y1 > ya {
                    st.active.push(next);
                }
                next += 1;
            }
            for values in st.acc.chunks_mut(256) {
                ctl.tick(values.len() as u64)?;
                values.fill(0.0);
            }
            let start = row.checked_mul(w).ok_or(E::LimitExceeded)?;
            if st.active.is_empty() {
                for values in out.get_mut(start..start + w).ok_or(E::LengthMismatch)?.chunks_mut(256) {
                    ctl.tick(values.len() as u64)?;
                    values.fill(if self.inverted { 1.0 } else { 0.0 });
                }
                continue;
            }
            st.cuts.clear();
            st.cuts.push(ya);
            st.cuts.push(yb);
            for &i in &st.active {
                ctl.tick(1)?;
                let e = self.edges.get(i).ok_or(E::LengthMismatch)?;
                if e.y0 > ya && e.y0 < yb {
                    st.cuts.push(e.y0);
                }
                if e.y1 > ya && e.y1 < yb {
                    st.cuts.push(e.y1);
                }
            }
            stable_sort(&mut st.cuts, f64::total_cmp, ctl)?;
            let mut count = 0;
            for i in 0..st.cuts.len() {
                ctl.tick(1)?;
                let value = *st.cuts.get(i).ok_or(E::LengthMismatch)?;
                if count == 0 || st.cuts.get(count - 1).is_some_and(|v| *v != value) {
                    *st.cuts.get_mut(count).ok_or(E::LengthMismatch)? = value;
                    count += 1;
                }
            }
            st.cuts.truncate(count);
            let cuts = std::mem::take(&mut st.cuts);
            for c in cuts.windows(2) {
                self.checked_band(*c.first().ok_or(E::LengthMismatch)?, *c.get(1).ok_or(E::LengthMismatch)?, rect.x0, &mut st, 0, ctl)?;
            }
            st.cuts = cuts;
            let mut sum = 0.0_f32;
            for (o, a) in out.get_mut(start..start + w).ok_or(E::LengthMismatch)?.iter_mut().zip(&st.acc) {
                ctl.tick(1)?;
                sum += *a;
                *o = sum.clamp(0.0, 1.0);
            }
        }
        Ok(())
    }
    fn checked_inside(&self, wind: &[i32], union_count: i32, ctl: &mut Control<'_, '_>) -> Result<bool, E> {
        if self.union_only {
            return Ok((union_count > 0) ^ self.inverted);
        }
        let mut acc = false;
        for (i, c) in self.comps.iter().enumerate() {
            ctl.tick(1)?;
            let value = *wind.get(i).ok_or(E::LengthMismatch)?;
            let v = match c.rule {
                FillRule::NonZero => value != 0,
                FillRule::EvenOdd => value & 1 != 0,
            };
            acc = match if i == 0 { PathOp::Combine } else { c.op } {
                PathOp::Combine | PathOp::Join => acc | v,
                PathOp::Subtract => acc & !v,
                PathOp::Intersect => acc & v,
                PathOp::Exclude => acc ^ v,
            };
        }
        Ok(acc ^ self.inverted)
    }
    fn checked_band(&self, a: f64, b: f64, x0: i32, st: &mut RowState, depth: u32, ctl: &mut Control<'_, '_>) -> Result<(), E> {
        ctl.tick(1)?;
        if b - a <= 1e-12 {
            return Ok(());
        }
        st.band.clear();
        let mid = 0.5 * (a + b);
        for &i in &st.active {
            ctl.tick(1)?;
            let e = self.edges.get(i).ok_or(E::LengthMismatch)?;
            if e.y0 <= mid && e.y1 >= mid {
                let (xa, xb) = (e.x_at(a), e.x_at(b));
                if !xa.is_finite() || !xb.is_finite() {
                    return Err(E::InvalidSample);
                }
                st.band.push(BandEdge { xa, xb, xm: 0.5 * (xa + xb), idx: u32::try_from(i).map_err(|_| E::LimitExceeded)? });
            }
        }
        if st.band.is_empty() {
            if self.inverted {
                checked_add_full(&mut st.acc, b - a)?;
            }
            return Ok(());
        }
        stable_sort(&mut st.band, |p, q| p.xm.total_cmp(&q.xm), ctl)?;
        if depth < 24 {
            for k in 1..st.band.len() {
                ctl.tick(1)?;
                let p = *st.band.get(k - 1).ok_or(E::LengthMismatch)?;
                let q = *st.band.get(k).ok_or(E::LengthMismatch)?;
                let top = p.xa - q.xa;
                let bot = p.xb - q.xb;
                if top > 1e-9 || bot > 1e-9 {
                    let t = if (top - bot).abs() > 1e-15 { top / (top - bot) } else { 0.5 };
                    let y = a + (b - a) * t.clamp(0.0, 1.0);
                    let y = if y <= a + 1e-9 || y >= b - 1e-9 { mid } else { y };
                    self.checked_band(a, y, x0, st, depth + 1, ctl)?;
                    self.checked_band(y, b, x0, st, depth + 1, ctl)?;
                    return Ok(());
                }
            }
        }
        for values in st.wind.chunks_mut(256) {
            ctl.tick(values.len() as u64)?;
            values.fill(0);
        }
        let mut union_count = 0_i32;
        let mut inside = self.inverted;
        if inside {
            checked_add_full(&mut st.acc, b - a)?;
        }
        for k in 0..st.band.len() {
            ctl.tick(1)?;
            let be = *st.band.get(k).ok_or(E::LengthMismatch)?;
            let e = self.edges.get(be.idx as usize).ok_or(E::LengthMismatch)?;
            let c = e.comp as usize;
            if self.union_only {
                let was = self.comp_inside(c, *st.wind.get(c).ok_or(E::LengthMismatch)?);
                let wind = st.wind.get_mut(c).ok_or(E::LengthMismatch)?;
                *wind = wind.checked_add(e.dir).ok_or(E::LimitExceeded)?;
                let now = self.comp_inside(c, *wind);
                union_count = union_count.checked_add(i32::from(now) - i32::from(was)).ok_or(E::LimitExceeded)?;
            } else {
                let wind = st.wind.get_mut(c).ok_or(E::LengthMismatch)?;
                *wind = wind.checked_add(e.dir).ok_or(E::LimitExceeded)?;
            }
            let now = self.checked_inside(&st.wind, union_count, ctl)?;
            if now != inside {
                checked_draw(&mut st.acc, x0, be.xa, a, be.xb, b, if now { 1.0 } else { -1.0 }, ctl)?;
                inside = now;
            }
        }
        Ok(())
    }
    pub(crate) fn preparation_bytes(&self) -> usize {
        self.edges.capacity().saturating_mul(std::mem::size_of::<Edge>()).saturating_add(self.comps.capacity().saturating_mul(std::mem::size_of::<Component>()))
    }
}

fn checked_add_full(acc: &mut [f32], h: f64) -> Result<(), E> {
    *acc.first_mut().ok_or(E::LengthMismatch)? += h as f32;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn checked_draw(acc: &mut [f32], x0: i32, xa: f64, ya: f64, xb: f64, yb: f64, weight: f64, ctl: &mut Control<'_, '_>) -> Result<(), E> {
    let width = acc.len().checked_sub(1).ok_or(E::LengthMismatch)? as f64;
    let left = f64::from(x0);
    let (mut ua, mut ub) = (xa - left, xb - left);
    let dy = yb - ya;
    if dy <= 0.0 {
        return Ok(());
    }
    if ua > ub {
        std::mem::swap(&mut ua, &mut ub);
    }
    let dx = ub - ua;
    let h_of = |u0: f64, u1: f64| if dx > 1e-12 { dy * (u1 - u0) / dx } else { dy };
    if ua < 0.0 {
        let cut = ub.min(0.0);
        *acc.first_mut().ok_or(E::LengthMismatch)? += (weight * h_of(ua, cut)) as f32;
        if ub <= 0.0 {
            return Ok(());
        }
        ua = 0.0;
    }
    if ua >= width {
        return Ok(());
    }
    let end = ub.min(width);
    if dx <= 1e-12 {
        let c = ua.floor();
        let f = ua - c;
        let ci = c as usize;
        *acc.get_mut(ci).ok_or(E::LengthMismatch)? += (weight * dy * (1.0 - f)) as f32;
        *acc.get_mut(ci + 1).ok_or(E::LengthMismatch)? += (weight * dy * f) as f32;
        return Ok(());
    }
    let mut u = ua;
    while u < end {
        ctl.tick(1)?;
        let c = u.floor();
        let v = (c + 1.0).min(end);
        let h = h_of(u, v);
        let m = 0.5 * (u + v) - c;
        let ci = c as usize;
        *acc.get_mut(ci).ok_or(E::LengthMismatch)? += (weight * h * (1.0 - m)) as f32;
        *acc.get_mut(ci + 1).ok_or(E::LengthMismatch)? += (weight * h * m) as f32;
        if v <= u {
            break;
        }
        u = v;
    }
    Ok(())
}
