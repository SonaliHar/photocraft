//! Additive checked Shape ALPHA only. Existing geometry/alpha equations/order;
//! one caller-owned work sink is retained only for this synchronous operation.
use super::*;
use crate::VectorPreparationLimits;
use photocraft_doc::{Path, Subpath};
use photocraft_raster::{Interrupt, RasterInputError as E};
use std::cmp::Ordering;

struct ShapeBudget<'a, 'b> {
    interrupt: &'a Interrupt<'b>,
    limits: VectorPreparationLimits,
    live: usize,
    work: u64,
    since_poll: u64,
    peak: usize,
    original_work: &'a mut dyn FnMut(u64) -> Result<(), E>,
}
impl<'a, 'b> ShapeBudget<'a, 'b> {
    fn new(limits: VectorPreparationLimits, interrupt: &'a Interrupt<'b>, original_work: &'a mut dyn FnMut(u64) -> Result<(), E>) -> Result<Self, E> {
        interrupt.check().map_err(|_| E::Cancelled)?;
        let headers =
            std::mem::size_of::<Self>().checked_add(std::mem::size_of::<CheckedShapeAlpha>()).and_then(|n| n.checked_add(4096)).ok_or(E::LimitExceeded)?;
        if headers > limits.max_bytes {
            return Err(E::LimitExceeded);
        }
        // Fixed 4096 covers synchronous stroke/dash stack arrays and value headers;
        // every variable vector/header payload is charged separately below.
        Ok(Self { interrupt, limits, live: headers, work: 0, since_poll: 0, peak: headers, original_work })
    }
    fn tick(&mut self, n: u64) -> Result<(), E> {
        self.work = self.work.checked_add(n).ok_or(E::LimitExceeded)?;
        if self.work > self.limits.max_work {
            return Err(E::LimitExceeded);
        }
        (self.original_work)(n)?;
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
        if self
            .live
            .checked_add(bytes.checked_mul(2).ok_or(E::LimitExceeded)?)
            .is_none_or(|n| n > self.limits.max_bytes || n as u64 > photocraft_raster::MAX_CHECKED_RASTER_BYTES)
        {
            return Err(E::LimitExceeded);
        }
        self.tick(64)?;
        self.interrupt.check().map_err(|_| E::Cancelled)?;
        let old = vec.capacity();
        vec.try_reserve_exact(target.saturating_sub(vec.len())).map_err(|_| E::AllocationFailed)?;
        let new_bytes = vec.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        if new_bytes > bytes.checked_mul(2).ok_or(E::LimitExceeded)? || self.live.checked_add(new_bytes).is_none_or(|n| n > self.limits.max_bytes) {
            return Err(E::LimitExceeded);
        }
        self.peak = self.peak.max(self.live.checked_add(new_bytes).ok_or(E::LimitExceeded)?);
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
            let end = v.len().saturating_add(256).min(n);
            self.tick((end - v.len()) as u64)?;
            v.resize(end, T::default());
        }
        Ok(v)
    }
}
fn stable_sort<T: Copy>(values: &mut [T], compare: impl Fn(&T, &T) -> Ordering, ctl: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
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
fn flatten(s: &Subpath, tol: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<Vec<(f64, f64)>, E> {
    let mut pts = Vec::new();
    fn append(v: &mut Vec<(f64, f64)>, p: (f64, f64), ctl: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
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
    fn shape_fill(path: &Path, tol: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<Self, E> {
        let limits = ctl.limits;
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
            let poly = flatten(subpath, tol, ctl)?;
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
        stable_sort(&mut r.edges, |a, b| a.y0.total_cmp(&b.y0), ctl)?;
        ctl.interrupt.check().map_err(|_| E::Cancelled)?;
        Ok(r)
    }
    fn shape_render(&self, rect: Rect, ctl: &mut ShapeBudget<'_, '_>) -> Result<Vec<f32>, E> {
        let limits = ctl.limits;
        let bytes = photocraft_raster::checked_region_byte_len(
            rect,
            photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false),
        )?;

        if self.edges.len() > limits.max_edges || self.comps.len() > limits.max_components || self.preparation_bytes() > limits.max_bytes {
            return Err(E::LimitExceeded);
        }
        // All retained geometry is already live in the one parent budget.
        let mut out = ctl.zeros::<f32>(bytes / 4)?;
        if bytes != 0 {
            self.shape_render_into(rect, &mut out, ctl)?;
        }
        ctl.interrupt.check().map_err(|_| E::Cancelled)?;
        Ok(out)
    }
    fn shape_render_into(&self, rect: Rect, out: &mut [f32], ctl: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
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
                self.shape_band(*c.first().ok_or(E::LengthMismatch)?, *c.get(1).ok_or(E::LengthMismatch)?, rect.x0, &mut st, 0, ctl)?;
            }
            st.cuts = cuts;
            let mut sum = 0.0_f32;
            for (o, a) in out.get_mut(start..start + w).ok_or(E::LengthMismatch)?.iter_mut().zip(&st.acc) {
                ctl.tick(1)?;
                sum += *a;
                *o = if sum >= super::RESIDUE { sum.min(1.0) } else { 0.0 };
            }
        }
        ctl.release(st.active);
        ctl.release(st.cuts);
        ctl.release(st.band);
        ctl.release(st.wind);
        ctl.release(st.acc);
        Ok(())
    }
    fn shape_inside(&self, wind: &[i32], union_count: i32, ctl: &mut ShapeBudget<'_, '_>) -> Result<bool, E> {
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
    fn shape_band(&self, a: f64, b: f64, x0: i32, st: &mut RowState, depth: u32, ctl: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
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
                    self.shape_band(a, y, x0, st, depth + 1, ctl)?;
                    self.shape_band(y, b, x0, st, depth + 1, ctl)?;
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
            let now = self.shape_inside(&st.wind, union_count, ctl)?;
            if now != inside {
                checked_draw(&mut st.acc, x0, be.xa, a, be.xb, b, if now { 1.0 } else { -1.0 }, ctl)?;
                inside = now;
            }
        }
        Ok(())
    }
}

fn checked_add_full(acc: &mut [f32], h: f64) -> Result<(), E> {
    *acc.first_mut().ok_or(E::LengthMismatch)? += h as f32;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn checked_draw(acc: &mut [f32], x0: i32, xa: f64, ya: f64, xb: f64, yb: f64, weight: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
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

#[path = "shape_stroke_controlled.rs"]
mod shape_stroke_controlled;
use crate::{ShapeLayer, StrokeAlign};
use photocraft_doc::{Fill, GradientStyle};

// This represents actual appearance ALPHA, not shape geometry alone or RGBA.
// RGB conversion and dither cannot affect the old CompiledShape alpha equation.
enum AlphaPaint {
    None,
    Solid(f32),
    Gradient {
        stops: Vec<(f32, f32)>,
        midpoints: Vec<f32>,
        opacity_stops: Vec<(f32, f32)>,
        style: GradientStyle,
        layout: (f32, f32, (f32, f32)),
        reverse: bool,
        frame: (f64, f64, f64, f64),
    },
}
/// Controlled alpha of an ordinary shape's actual fill and stroke appearance.
/// No RGB/native color plane or Source/storage authority is represented.
pub struct CheckedShapeAlpha {
    fill: Option<(Rasterizer, AlphaPaint)>,
    stroke: Option<(Rasterizer, AlphaPaint, f32)>,
    align: Option<(Rasterizer, bool)>,
    bounds: Option<Rect>,
}
fn alpha_paint(fill: &Fill, bounds: (f64, f64, f64, f64), canvas: (f64, f64, f64, f64), ctl: &mut ShapeBudget<'_, '_>) -> Result<AlphaPaint, E> {
    ctl.tick(256)?;
    Ok(match fill {
        Fill::Solid(color) => {
            if !color.alpha.is_finite() {
                return Err(E::InvalidSample);
            }
            AlphaPaint::Solid(color.alpha)
        }
        // This is the exact old Paint::None behavior for a pattern shape fill.
        Fill::Pattern { .. } => AlphaPaint::None,
        Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither: _, align } => {
            // Validate stored coordinates before any gradient allocation. The
            // legacy layout can return unchanged malformed inputs, and NaN t
            // would otherwise silently select the last finite alpha stop.
            let frame = if *align { bounds } else { canvas };
            if !angle.is_finite()
                || !scale.is_finite()
                || !offset.0.is_finite()
                || !offset.1.is_finite()
                || ![frame.0, frame.1, frame.2, frame.3, frame.2 - frame.0, frame.3 - frame.1].iter().all(|v| v.is_finite())
            {
                return Err(E::InvalidSample);
            }
            let layout = crate::gradient_layout(*style, *angle, *scale, *offset, frame);
            if ![layout.0, layout.1, layout.2.0, layout.2.1].iter().all(|v| v.is_finite()) {
                return Err(E::InvalidSample);
            }
            let mut colors = Vec::new();
            ctl.reserve(&mut colors, stops.len())?;
            for &(position, color) in stops {
                ctl.tick(128)?;
                if !position.is_finite() || !color.alpha.is_finite() {
                    return Err(E::InvalidSample);
                }
                colors.push((position, color.alpha));
            }
            stable_sort(&mut colors, |a, b| a.0.total_cmp(&b.0), ctl)?;
            let mut opacity = Vec::new();
            ctl.reserve(&mut opacity, opacity_stops.len())?;
            for &p in opacity_stops {
                ctl.tick(32)?;
                if !p.0.is_finite() || !p.1.is_finite() {
                    return Err(E::InvalidSample);
                }
                opacity.push(p);
            }
            stable_sort(&mut opacity, |a, b| a.0.total_cmp(&b.0), ctl)?;
            let mut middle = Vec::new();
            ctl.reserve(&mut middle, midpoints.len())?;
            for &p in midpoints {
                ctl.tick(16)?;
                if !p.is_finite() {
                    return Err(E::InvalidSample);
                }
                middle.push(p);
            }
            AlphaPaint::Gradient { stops: colors, midpoints: middle, opacity_stops: opacity, style: *style, layout, reverse: *reverse, frame }
        }
    })
}
fn alpha_paint_bytes(p: &AlphaPaint) -> Result<usize, E> {
    Ok(match p {
        AlphaPaint::None | AlphaPaint::Solid(_) => 0,
        AlphaPaint::Gradient { stops, midpoints, opacity_stops, .. } => stops
            .capacity()
            .checked_mul(8)
            .and_then(|n| n.checked_add(midpoints.capacity().checked_mul(4)?))
            .and_then(|n| n.checked_add(opacity_stops.capacity().checked_mul(8)?))
            .ok_or(E::LimitExceeded)?,
    })
}
fn alpha_at(p: &AlphaPaint, x: f64, y: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<f32, E> {
    // A constant alpha performs no gradient-coordinate/trigonometric work.
    // Keep the full fixed gradient assignment plus each actual stop traversal.
    ctl.tick(if matches!(p, AlphaPaint::None | AlphaPaint::Solid(_)) { 32 } else { 512 })?;
    let alpha = match p {
        AlphaPaint::None => 0.0,
        AlphaPaint::Solid(alpha) => *alpha,
        AlphaPaint::Gradient { stops, midpoints, opacity_stops, style, layout, reverse, frame } => {
            let t = crate::gradient_t(*style, layout.0, layout.1, *reverse, layout.2, *frame, x, y);
            if !t.is_finite() {
                return Err(E::InvalidSample);
            }
            let mut alpha = if let (Some(first), Some(last)) = (stops.first(), stops.last()) {
                if t <= first.0 {
                    first.1
                } else {
                    let mut result = last.1;
                    for (i, w) in stops.windows(2).enumerate() {
                        ctl.tick(64)?;
                        let a = w.first().ok_or(E::LengthMismatch)?;
                        let b = w.get(1).ok_or(E::LengthMismatch)?;
                        if t <= b.0 {
                            let u = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
                            let k = crate::midpoint_remap(u, midpoints.get(i).copied().unwrap_or(0.5));
                            result = a.1 + (b.1 - a.1) * k;
                            break;
                        }
                    }
                    result
                }
            } else {
                0.0
            };
            if let (Some(first), Some(last)) = (opacity_stops.first(), opacity_stops.last()) {
                let opacity = if t <= first.0 {
                    first.1
                } else {
                    let mut result = last.1;
                    for w in opacity_stops.windows(2) {
                        ctl.tick(64)?;
                        let a = w.first().ok_or(E::LengthMismatch)?;
                        let b = w.get(1).ok_or(E::LengthMismatch)?;
                        if t <= b.0 {
                            let u = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
                            result = a.1 + (b.1 - a.1) * u;
                            break;
                        }
                    }
                    result
                };
                alpha *= opacity;
            }
            alpha
        }
    };
    if !alpha.is_finite() {
        return Err(E::InvalidSample);
    }
    Ok(alpha)
}

// RENDER_BODY (appended after the preparation implementation in the final child).
impl CheckedShapeAlpha {
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }
    /// Old appearance alpha in row-major order, with simultaneous fill/stroke/
    /// alignment raster scratch and returned band included in the one allowance.
    /// Repeated bands must use the same caller work sink; none of it is retained.
    pub fn try_render_with_control(
        &self,
        rect: Rect,
        limits: VectorPreparationLimits,
        interrupt: &Interrupt<'_>,
        original_work: &mut dyn FnMut(u64) -> Result<(), E>,
    ) -> Result<(Vec<f32>, usize), E> {
        lower_limits(limits)?;
        let mut ctl = ShapeBudget::new(limits, interrupt, original_work)?;
        ctl.live = ctl.live.checked_add(self.checked_preparation_bytes()?).ok_or(E::LimitExceeded)?;
        if ctl.live > limits.max_bytes {
            return Err(E::LimitExceeded);
        }
        ctl.peak = ctl.live;
        let format = photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false);
        let bytes = photocraft_raster::checked_region_byte_len(rect, format)?;
        let mut out = ctl.zeros::<f32>(bytes / 4)?;
        let w = rect.width() as usize;
        if let Some((r, paint)) = &self.fill {
            let cov = r.shape_render(rect, &mut ctl)?;
            for (i, (o, &coverage)) in out.iter_mut().zip(&cov).enumerate() {
                ctl.tick(32)?;
                if coverage > 0.0 {
                    let (x, y) = ((rect.x0 + (i % w) as i32) as f64 + 0.5, (rect.y0 + (i / w) as i32) as f64 + 0.5);
                    let mut alpha = alpha_at(paint, x, y, &mut ctl)?;
                    alpha *= coverage;
                    *o = alpha;
                }
            }
            ctl.release(cov);
        }
        if let Some((r, paint, opacity)) = &self.stroke {
            let mut cov = r.shape_render(rect, &mut ctl)?;
            if let Some((area, inside)) = &self.align {
                let coverage = area.shape_render(rect, &mut ctl)?;
                for (c, &f) in cov.iter_mut().zip(&coverage) {
                    ctl.tick(16)?;
                    *c *= if *inside { f } else { 1.0 - f };
                }
                ctl.release(coverage);
            }
            for (i, (o, &coverage)) in out.iter_mut().zip(&cov).enumerate() {
                ctl.tick(32)?;
                if coverage <= 0.0 {
                    continue;
                }
                let (x, y) = ((rect.x0 + (i % w) as i32) as f64 + 0.5, (rect.y0 + (i / w) as i32) as f64 + 0.5);
                let sample_alpha = alpha_at(paint, x, y, &mut ctl)?;
                let sa = sample_alpha * coverage * opacity;
                let da = *o;
                let a = sa + da * (1.0 - sa);
                if !a.is_finite() {
                    return Err(E::InvalidSample);
                }
                *o = a;
            }
            ctl.release(cov);
        }
        interrupt.check().map_err(|_| E::Cancelled)?;
        Ok((out, ctl.peak))
    }
}

fn flatten_stroke(path: &Path, tol: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<Vec<crate::Polyline>, E> {
    // The checked flattener's established 256/knot +64/output-point +64/subpath
    // schedule is independently assigned before its own repeated census/emission.
    // This planning walk itself pays before touching every knot/segment.
    let mut knots = 0usize;
    let mut points = 0usize;
    let mut source = path.subpaths.capacity().checked_mul(std::mem::size_of::<Subpath>()).ok_or(E::LimitExceeded)?;
    for s in &path.subpaths {
        ctl.tick(64)?;
        source =
            source.checked_add(s.knots.capacity().checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
        knots = knots.checked_add(s.knots.len()).filter(|n| *n <= ctl.limits.max_edges).ok_or(E::LimitExceeded)?;
        for k in &s.knots {
            ctl.tick(256)?;
            for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
                if !p.x.is_finite() || !p.y.is_finite() {
                    return Err(E::InvalidSample);
                }
            }
        }
        points = points.checked_add(usize::from(!s.knots.is_empty())).ok_or(E::LimitExceeded)?;
        let count = if s.knots.len() < 2 {
            0
        } else if s.closed {
            s.knots.len()
        } else {
            s.knots.len() - 1
        };
        for i in 0..count {
            ctl.tick(512)?;
            let a = s.knots.get(i).ok_or(E::LengthMismatch)?;
            let b = s.knots.get((i + 1) % s.knots.len()).ok_or(E::LengthMismatch)?;
            let seg = [a.anchor, a.out_ctrl, b.in_ctrl, b.anchor];
            // Same finite-intermediate refusal as flatten_controlled::count.
            let [p0, c1, c2, p1] = seg;
            let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
            let len2 = dx * dx + dy * dy;
            if !len2.is_finite() {
                return Err(E::InvalidSample);
            }
            for p in [c1, c2] {
                let (vx, vy) = (p.x - p0.x, p.y - p0.y);
                if !vx.is_finite() || !vy.is_finite() || !(vx * dx + vy * dy).is_finite() || !(vx * dy - vy * dx).is_finite() {
                    return Err(E::InvalidSample);
                }
            }
            let n = if crate::flatten::is_line(&seg) {
                1
            } else {
                let d1 = ((p0.x - 2.0 * c1.x + c2.x).powi(2) + (p0.y - 2.0 * c1.y + c2.y).powi(2)).sqrt();
                let d2 = ((c1.x - 2.0 * c2.x + p1.x).powi(2) + (c1.y - 2.0 * c2.y + p1.y).powi(2)).sqrt();
                let size = (c1.x - p0.x).hypot(c1.y - p0.y) + (c2.x - c1.x).hypot(c2.y - c1.y) + (p1.x - c2.x).hypot(p1.y - c2.y);
                let effective = tol.min(size * 1e-3).max(1e-4);
                if !d1.is_finite() || !d2.is_finite() || !size.is_finite() || !(0.75 * d1.max(d2) / effective).is_finite() {
                    return Err(E::InvalidSample);
                }
                crate::flatten::pieces(&seg, tol)
            };
            points = points.checked_add(n).filter(|n| *n <= ctl.limits.max_edges).ok_or(E::LimitExceeded)?;
        }
    }
    let work = (knots as u64)
        .checked_mul(256)
        .and_then(|n| n.checked_add((points as u64).checked_mul(64)?))
        .and_then(|n| n.checked_add((path.subpaths.len() as u64).checked_mul(64)?))
        .ok_or(E::LimitExceeded)?;
    let output =
        path.subpaths.len().checked_mul(std::mem::size_of::<crate::Polyline>()).and_then(|n| n.checked_add(points.checked_mul(17)?)).ok_or(E::LimitExceeded)?;
    let allowance = source.checked_add(output.checked_mul(2).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    if ctl.live.checked_add(allowance).is_none_or(|n| n > ctl.limits.max_bytes) {
        return Err(E::LimitExceeded);
    }
    ctl.tick(work)?;
    let lines = crate::flatten_path_with_interrupt(path, tol, VectorPreparationLimits { max_bytes: allowance, max_work: work, ..ctl.limits }, ctl.interrupt)?;
    let mut actual = lines.capacity().checked_mul(std::mem::size_of::<crate::Polyline>()).ok_or(E::LimitExceeded)?;
    for pl in &lines {
        ctl.tick(64)?;
        actual = actual
            .checked_add(pl.pts.capacity().checked_mul(16).ok_or(E::LimitExceeded)?)
            .and_then(|n| n.checked_add(pl.knot.capacity()))
            .ok_or(E::LimitExceeded)?;
    }
    ctl.peak = ctl.peak.max(ctl.live.checked_add(allowance).ok_or(E::LimitExceeded)?);
    ctl.live = ctl.live.checked_add(actual).filter(|n| *n <= ctl.limits.max_bytes).ok_or(E::LimitExceeded)?;
    Ok(lines)
}
fn stroke_raster(path: &Path, style: &crate::StrokeStyle, tol: f64, ctl: &mut ShapeBudget<'_, '_>) -> Result<Rasterizer, E> {
    let lines = flatten_stroke(path, (tol * 4.0).min(style.width.max(0.01) * 0.1).max(1e-3), ctl)?;
    let mut polys = shape_stroke_controlled::polygons(&lines, style, tol, ctl)?;
    let mut r = Rasterizer::new(false);
    ctl.push(&mut r.comps, Component { op: PathOp::Combine, rule: FillRule::NonZero }, ctl.limits.max_components)?;
    for poly in &polys {
        ctl.tick(1)?;
        if poly.len() < 2 {
            continue;
        }
        for i in 0..poly.len() {
            ctl.tick(64)?;
            let a = *poly.get(i).ok_or(E::LengthMismatch)?;
            let b = *poly.get((i + 1) % poly.len()).ok_or(E::LengthMismatch)?;
            if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
                return Err(E::InvalidSample);
            }
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
            ctl.push(&mut r.edges, Edge { x0: p.0, y0: p.1, y1: q.1, dir, comp: 0, dxdy }, ctl.limits.max_edges)?;
        }
    }
    stable_sort(&mut r.edges, |a, b| a.y0.total_cmp(&b.y0), ctl)?;
    for poly in polys.drain(..) {
        ctl.release(poly);
    }
    ctl.release(polys);
    let mut lines = lines;
    for line in lines.drain(..) {
        ctl.release(line.pts);
        ctl.release(line.knot);
    }
    ctl.release(lines);
    Ok(r)
}
fn clone_raster(r: &Rasterizer, ctl: &mut ShapeBudget<'_, '_>) -> Result<Rasterizer, E> {
    let mut edges = Vec::new();
    ctl.reserve(&mut edges, r.edges.len())?;
    for &e in &r.edges {
        ctl.tick(1)?;
        edges.push(e);
    }
    let mut comps = Vec::new();
    ctl.reserve(&mut comps, r.comps.len())?;
    for &c in &r.comps {
        ctl.tick(1)?;
        comps.push(c);
    }
    Ok(Rasterizer { edges, comps, inverted: r.inverted, bounds: r.bounds, union_only: r.union_only })
}
fn drop_raster(r: Rasterizer, ctl: &mut ShapeBudget<'_, '_>) {
    ctl.release(r.edges);
    ctl.release(r.comps);
}
fn lower_limits(limits: VectorPreparationLimits) -> Result<(), E> {
    let d = VectorPreparationLimits::default();
    if limits.max_bytes > d.max_bytes || limits.max_edges > d.max_edges || limits.max_components > d.max_components || limits.max_work > d.max_work {
        return Err(E::LimitExceeded);
    }
    Ok(())
}

impl CheckedShapeAlpha {
    /// Additive checked preparation. The caller must retain a real prospective
    /// `limits.max_bytes` temporary envelope through this call and its returned
    /// geometry; returned usize is the conservative simultaneous payload peak.
    /// Every actual phase charges the borrowed original work sink independently.
    /// The sink and interrupt are never retained in the compiled geometry.
    /// Alpha uses Color.alpha only; no RGB conversion, CMS preparation or RGB result is claimed.
    pub fn try_new_with_control(
        shape: &ShapeLayer,
        tol: f64,
        canvas: Rect,
        limits: VectorPreparationLimits,
        interrupt: &Interrupt<'_>,
        original_work: &mut dyn FnMut(u64) -> Result<(), E>,
    ) -> Result<(Self, usize), E> {
        lower_limits(limits)?;
        let mut ctl = ShapeBudget::new(limits, interrupt, original_work)?;
        if !tol.is_finite() || tol <= 0.0 {
            return Err(E::InvalidSample);
        }
        // Bind control bounds using the exact original visit/order, with bounded polling.
        let mut pb: Option<(f64, f64, f64, f64)> = None;
        let mut all_closed = true;
        if shape.path.subpaths.len() > limits.max_components {
            return Err(E::LimitExceeded);
        }
        let mut total_knots = 0usize;
        for s in &shape.path.subpaths {
            ctl.tick(64)?;
            all_closed &= s.closed;
            total_knots = total_knots.checked_add(s.knots.len()).filter(|n| *n <= limits.max_edges).ok_or(E::LimitExceeded)?;
            for k in &s.knots {
                ctl.tick(128)?;
                for p in [k.anchor, k.in_ctrl, k.out_ctrl] {
                    if !p.x.is_finite() || !p.y.is_finite() {
                        return Err(E::InvalidSample);
                    }
                    pb = Some(match pb {
                        None => (p.x, p.y, p.x, p.y),
                        Some(b) => (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y)),
                    });
                }
            }
        }
        let pb = pb.unwrap_or((0.0, 0.0, 0.0, 0.0));
        let cb = (f64::from(canvas.x0), f64::from(canvas.y0), f64::from(canvas.x1), f64::from(canvas.y1));
        // Reject malformed stored stroke scalars before the activity predicate:
        // NaN must not turn an invalid checked request into an absent stroke.
        if let Some(stroke) = &shape.stroke {
            ctl.tick(256)?;
            if !stroke.width.is_finite()
                || !stroke.opacity.is_finite()
                || !stroke.miter_limit.is_finite()
                || !stroke.dash_offset.is_finite()
                || stroke.dashes.len() > 32
            {
                return Err(E::InvalidSample);
            }
            for &dash in &stroke.dashes {
                ctl.tick(64)?;
                if !dash.is_finite() || dash < 0.0 {
                    return Err(E::InvalidSample);
                }
            }
        }
        let area = Rasterizer::shape_fill(&shape.path, tol, &mut ctl)?;
        let mut bounds = None;
        let fill = if let Some(f) = &shape.fill {
            bounds = area.pixel_bounds();
            Some((clone_raster(&area, &mut ctl)?, alpha_paint(f, pb, cb, &mut ctl)?))
        } else {
            None
        };
        let mut align = None;
        let stroke = if let Some(s) = shape.stroke.as_ref().filter(|s| s.width > 0.0 && s.opacity > 0.0) {
            ctl.tick(256)?;
            if !s.width.is_finite() || !s.opacity.is_finite() || !s.miter_limit.is_finite() || !s.dash_offset.is_finite() || s.dashes.len() > 32 {
                return Err(E::InvalidSample);
            }
            let w = f64::from(s.width.max(0.0));
            let mut dashes = Vec::new();
            ctl.reserve(&mut dashes, s.dashes.len())?;
            for &d in &s.dashes {
                ctl.tick(64)?;
                let d = f64::from(d) * w;
                if !d.is_finite() || d < 0.0 {
                    return Err(E::InvalidSample);
                }
                dashes.push(d);
            }
            let mut st = crate::StrokeStyle {
                width: w,
                cap: s.cap,
                join: s.join,
                miter_limit: f64::from(s.miter_limit.max(1.0)),
                dashes,
                dash_offset: f64::from(s.dash_offset) * w,
            };
            if all_closed && s.align != StrokeAlign::Center {
                st.width *= 2.0;
                align = Some((clone_raster(&area, &mut ctl)?, s.align == StrokeAlign::Inside));
            }
            let r = stroke_raster(&shape.path, &st, tol, &mut ctl)?;
            let sb = r.pixel_bounds();
            bounds = match (bounds, sb) {
                (Some(a), Some(b)) => Some(a.union(&b)),
                (a, b) => a.or(b),
            };
            ctl.release(st.dashes);
            Some((r, alpha_paint(&s.paint, pb, cb, &mut ctl)?, s.opacity.clamp(0.0, 1.0)))
        } else {
            None
        };
        drop_raster(area, &mut ctl);
        if shape.path.inverted && shape.fill.is_some() {
            bounds = Some(Rect::new(i32::MIN / 4, i32::MIN / 4, i32::MAX / 4, i32::MAX / 4));
        }
        interrupt.check().map_err(|_| E::Cancelled)?;
        Ok((Self { fill, stroke, align, bounds }, ctl.peak))
    }
    /// Known retained geometry/Paint capacities, for a caller's actual lease.
    pub fn checked_preparation_bytes(&self) -> Result<usize, E> {
        let mut n = std::mem::size_of::<Self>();
        if let Some((r, p)) = &self.fill {
            n = n.checked_add(r.preparation_bytes()).and_then(|n| n.checked_add(alpha_paint_bytes(p).ok()?)).ok_or(E::LimitExceeded)?;
        }
        if let Some((r, p, _)) = &self.stroke {
            n = n.checked_add(r.preparation_bytes()).and_then(|n| n.checked_add(alpha_paint_bytes(p).ok()?)).ok_or(E::LimitExceeded)?;
        }
        if let Some((r, _)) = &self.align {
            n = n.checked_add(r.preparation_bytes()).ok_or(E::LimitExceeded)?;
        }
        Ok(n)
    }
}

#[cfg(test)]
#[path = "shape_alpha_tests.rs"]
mod tests;
