//! Controlled form of the legacy fractional-coverage tracer. The numerical
//! primitives and marching-square routes remain the legacy implementation.
use super::{P, add, bezier, bezier_d1, bezier_d2, dot, len, mul, norm, sub};
use photocraft_doc::{Knot, Path, PathOp, Subpath};
use photocraft_geom::{Point, Rect};
use photocraft_raster::Interrupt;
use std::{collections::HashMap, mem::size_of, ops::Range};
type Edge = (i64, i64, u8);
const MAX_WORK: u64 = 8_000_000_000;
#[derive(Clone, Copy, Debug)]
pub struct TraceLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for TraceLimits {
    fn default() -> Self {
        Self { resident_bytes: photocraft_raster::MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceError {
    Cancelled,
    InvalidInput(&'static str),
    Limit,
    Allocation,
}
impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("path tracing cancelled"),
            Self::InvalidInput(reason) => write!(f, "invalid path tracing input: {reason}"),
            Self::Limit => f.write_str("path tracing resource limit"),
            Self::Allocation => f.write_str("path tracing allocation failed"),
        }
    }
}
impl std::error::Error for TraceError {}
type Result<T> = std::result::Result<T, TraceError>;
struct Budget<'a, 'b> {
    control: &'a Interrupt<'b>,
    work: u64,
    limit: u64,
    resident: u64,
    edges: usize,
}
impl Budget<'_, '_> {
    fn poll(&self) -> Result<()> {
        self.control.check().map_err(|_| TraceError::Cancelled)
    }
    fn tick(&mut self, units: u64) -> Result<()> {
        self.poll()?;
        self.work = self.work.checked_add(units).filter(|n| *n <= self.limit).ok_or(TraceError::Limit)?;
        Ok(())
    }
    fn vector<T>(&self, count: usize) -> Result<Vec<T>> {
        self.poll()?;
        let bytes = count.checked_mul(size_of::<T>()).ok_or(TraceError::Limit)?;
        if bytes as u64 > self.resident {
            return Err(TraceError::Limit);
        }
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| TraceError::Allocation)?;
        if values.capacity().checked_mul(size_of::<T>()).is_none_or(|n| n > bytes) {
            return Err(TraceError::Limit);
        }
        Ok(values)
    }
    fn edge(&mut self, next: &mut HashMap<Edge, Edge>, a: Edge, b: Edge) -> Result<()> {
        self.tick(1)?;
        if !next.contains_key(&a) && (next.len() >= self.edges || next.len() >= next.capacity()) {
            return Err(TraceError::Limit);
        }
        next.insert(a, b);
        Ok(())
    }
}
fn at<T: Copy>(values: &[T], index: usize) -> Result<T> {
    values.get(index).copied().ok_or(TraceError::InvalidInput("internal index"))
}
fn value(values: &[f32], w: i64, h: i64, outside: f32, x: i64, y: i64) -> Result<f32> {
    if x < 0 || y < 0 || x >= w || y >= h {
        Ok(outside)
    } else {
        let index = y.checked_mul(w).and_then(|n| n.checked_add(x)).and_then(|n| usize::try_from(n).ok()).ok_or(TraceError::Limit)?;
        at(values, index)
    }
}
fn point(values: &[f32], rect: Rect, w: i64, h: i64, outside: f32, a: (i64, i64), b: (i64, i64)) -> Result<P> {
    let (va, vb) = (value(values, w, h, outside, a.0, a.1)?, value(values, w, h, outside, b.0, b.1)?);
    let t = if (vb - va).abs() > 1e-9 { f64::from((0.5_f32 - va) / (vb - va)).clamp(0.0, 1.0) } else { 0.5 };
    let (ax, ay) = (a.0 as f64 + 0.5, a.1 as f64 + 0.5);
    let (bx, by) = (b.0 as f64 + 0.5, b.1 as f64 + 0.5);
    Ok((f64::from(rect.x0) + ax + (bx - ax) * t, f64::from(rect.y0) + ay + (by - ay) * t))
}
/// Exact legacy level-0.5 tracing for finite unit coverage, including fractional
/// interpolation and f32 saddle averaging. No binary occupancy substitution.
pub fn trace_mask_with_interrupt(values: &[f32], rect: Rect, outside: f32, tolerance: f64, limits: TraceLimits, control: &Interrupt<'_>) -> Result<Path> {
    control.check().map_err(|_| TraceError::Cancelled)?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES || limits.work_units > MAX_WORK {
        return Err(TraceError::Limit);
    }
    if !outside.is_finite() || !(0.0..=1.0).contains(&outside) || !tolerance.is_finite() {
        return Err(TraceError::InvalidInput("coverage/tolerance"));
    }
    let tol = (tolerance * 0.5).max(0.05);
    if !(tol * tol).is_finite() {
        return Err(TraceError::InvalidInput("tolerance overflow"));
    }
    let w = i64::from(rect.x1) - i64::from(rect.x0);
    let h = i64::from(rect.y1) - i64::from(rect.y0);
    let pixels = w.checked_mul(h).filter(|_| w >= 0 && h >= 0).and_then(|n| usize::try_from(n).ok()).ok_or(TraceError::InvalidInput("extent"))?;
    if values.len() != pixels {
        return Err(TraceError::InvalidInput("coverage length"));
    }
    let input = (pixels as u64).checked_mul(4).ok_or(TraceError::Limit)?;
    let cells = (w as u64).checked_add(1).and_then(|n| n.checked_mul((h as u64).checked_add(1)?)).ok_or(TraceError::Limit)?;
    if input > limits.resident_bytes || cells.checked_mul(16).is_none_or(|n| n > limits.work_units) {
        return Err(TraceError::Limit);
    }
    let mut budget = Budget { control, work: 0, limit: limits.work_units, resident: limits.resident_bytes, edges: 0 };
    for chunk in values.chunks(256) {
        budget.tick(chunk.len() as u64)?;
        if chunk.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(TraceError::InvalidInput("coverage domain"));
        }
    }
    // Count exact prospective segment publications without allocating a map.
    for y in -1..h {
        budget.poll()?;
        for x in -1..w {
            budget.tick(1)?;
            let a = value(values, w, h, outside, x, y)? >= 0.5;
            let b = value(values, w, h, outside, x + 1, y)? >= 0.5;
            let c = value(values, w, h, outside, x + 1, y + 1)? >= 0.5;
            let d = value(values, w, h, outside, x, y + 1)? >= 0.5;
            let index = u8::from(a) | u8::from(b) << 1 | u8::from(c) << 2 | u8::from(d) << 3;
            let segments = if index == 0 || index == 15 {
                0
            } else if index == 5 || index == 10 {
                2
            } else {
                1
            };
            budget.edges = budget.edges.checked_add(segments).ok_or(TraceError::Limit)?;
        }
    }
    // Contours/keys/maps plus 42 nested fitting frames and all output knots.
    // Includes input while borrowed; it is never copied by this API.
    let resident =
        input.checked_add((budget.edges as u64).checked_mul(2048).ok_or(TraceError::Limit)?).and_then(|n| n.checked_add(4096)).ok_or(TraceError::Limit)?;
    let work = cells
        .checked_mul(16)
        .and_then(|n| n.checked_add((budget.edges as u64).checked_mul(budget.edges as u64)?.checked_mul(128)?))
        .and_then(|n| n.checked_add((budget.edges as u64).checked_mul(200_000)?))
        .ok_or(TraceError::Limit)?;
    if resident > limits.resident_bytes || work > limits.work_units {
        return Err(TraceError::Limit);
    }
    let (points, ranges) = contours(values, rect, w, h, outside, &mut budget)?;
    let mut subpaths = budget.vector(ranges.len())?;
    for (i, range) in ranges.into_iter().enumerate() {
        budget.poll()?;
        let lp = points.get(range).ok_or(TraceError::InvalidInput("contour range"))?;
        let mut path = fit_closed(lp, tol, &mut budget)?;
        path.op = if i == 0 { PathOp::Combine } else { PathOp::Exclude };
        subpaths.push(path);
    }
    budget.poll()?;
    Ok(Path::new(subpaths))
}

fn contours(values: &[f32], rect: Rect, w: i64, h: i64, outside: f32, budget: &mut Budget<'_, '_>) -> Result<(Vec<P>, Vec<Range<usize>>)> {
    let get = |x: i64, y: i64| value(values, w, h, outside, x, y);
    let lvl = 0.5_f32;
    let mut next: HashMap<Edge, Edge> = HashMap::new();
    next.try_reserve(budget.edges).map_err(|_| TraceError::Allocation)?;
    let map_envelope = (budget.edges as u64).checked_mul(512).and_then(|n| n.checked_add(4096)).ok_or(TraceError::Limit)?;
    if (next.capacity() as u64).checked_mul(128).is_none_or(|n| n > map_envelope || n > budget.resident) {
        return Err(TraceError::Limit);
    }
    for cy in -1..h {
        budget.poll()?;
        for cx in -1..w {
            budget.tick(1)?;
            let tl = get(cx, cy)? >= lvl;
            let tr = get(cx + 1, cy)? >= lvl;
            let br = get(cx + 1, cy + 1)? >= lvl;
            let bl = get(cx, cy + 1)? >= lvl;
            let idx = u8::from(tl) | u8::from(tr) << 1 | u8::from(br) << 2 | u8::from(bl) << 3;
            if idx == 0 || idx == 15 {
                continue;
            }
            let top = (cx, cy, 0u8);
            let bottom = (cx, cy + 1, 0u8);
            let left = (cx, cy, 1u8);
            let right = (cx + 1, cy, 1u8);
            // Walking along the contour with the inside on the left-hand side.
            let mut seg = |a: Edge, b: Edge| budget.edge(&mut next, a, b);
            match idx {
                1 => seg(left, top)?,
                2 => seg(top, right)?,
                3 => seg(left, right)?,
                4 => seg(right, bottom)?,
                5 => {
                    let centre = (get(cx, cy)? + get(cx + 1, cy)? + get(cx + 1, cy + 1)? + get(cx, cy + 1)?) / 4.0;
                    if centre >= lvl {
                        seg(left, bottom)?;
                        seg(right, top)?;
                    } else {
                        seg(left, top)?;
                        seg(right, bottom)?;
                    }
                }
                6 => seg(top, bottom)?,
                7 => seg(left, bottom)?,
                8 => seg(bottom, left)?,
                9 => seg(bottom, top)?,
                10 => {
                    let centre = (get(cx, cy)? + get(cx + 1, cy)? + get(cx + 1, cy + 1)? + get(cx, cy + 1)?) / 4.0;
                    if centre >= lvl {
                        seg(top, left)?;
                        seg(bottom, right)?;
                    } else {
                        seg(top, right)?;
                        seg(bottom, left)?;
                    }
                }
                11 => seg(bottom, right)?,
                12 => seg(right, left)?,
                13 => seg(right, top)?,
                14 => seg(top, left)?,
                _ => {}
            }
        }
    }

    let mut keys = budget.vector(next.len())?;
    for key in next.keys() {
        budget.tick(1)?;
        keys.push(*key);
    }
    sort_edges(&mut keys, budget)?;
    let mut points: Vec<P> = budget.vector(budget.edges)?;
    let mut ranges = budget.vector(budget.edges / 3 + 1)?;
    for start in keys {
        budget.tick(1)?;
        if !next.contains_key(&start) {
            continue;
        }
        let begin = points.len();
        let mut cur = start;
        while let Some(n) = next.remove(&cur) {
            budget.tick(1)?;
            let p = if cur.2 == 0 {
                point(values, rect, w, h, outside, (cur.0, cur.1), (cur.0 + 1, cur.1))?
            } else {
                point(values, rect, w, h, outside, (cur.0, cur.1), (cur.0, cur.1 + 1))?
            };
            // Same consecutive deduplication as the legacy completed loop.
            if points.len() == begin || points.last().is_none_or(|q| (p.0 - q.0).abs() >= 1e-9 || (p.1 - q.1).abs() >= 1e-9) {
                if points.len() >= points.capacity() {
                    return Err(TraceError::Limit);
                }
                points.push(p);
            }
            cur = n;
            if cur == start {
                break;
            }
        }
        if points.len() - begin >= 3 {
            if ranges.len() >= ranges.capacity() {
                return Err(TraceError::Limit);
            }
            ranges.push(begin..points.len());
        } else {
            points.truncate(begin);
        }
    }
    Ok((points, ranges))
}
fn sort_edges(values: &mut [Edge], budget: &mut Budget<'_, '_>) -> Result<()> {
    fn key(v: Edge) -> (i64, i64, u8) {
        (v.1, v.0, v.2)
    }
    fn sift(values: &mut [Edge], mut root: usize, end: usize, budget: &mut Budget<'_, '_>) -> Result<()> {
        loop {
            budget.tick(1)?;
            let Some(left) = root.checked_mul(2).and_then(|n| n.checked_add(1)) else {
                return Err(TraceError::Limit);
            };
            if left >= end {
                return Ok(());
            }
            let mut child = left;
            let right = left.checked_add(1).ok_or(TraceError::Limit)?;
            if right < end && key(at(values, right)?) > key(at(values, left)?) {
                child = right;
            }
            if key(at(values, root)?) >= key(at(values, child)?) {
                return Ok(());
            }
            values.swap(root, child);
            root = child;
        }
    }
    for start in (0..values.len() / 2).rev() {
        sift(values, start, values.len(), budget)?;
    }
    for end in (1..values.len()).rev() {
        budget.tick(1)?;
        values.swap(0, end);
        sift(values, 0, end, budget)?;
    }
    Ok(())
}
fn walk(lp: &[P], i: usize, window: f64, fwd: bool, budget: &mut Budget<'_, '_>) -> Result<P> {
    let n = lp.len();
    if n == 0 {
        return Err(TraceError::InvalidInput("empty contour"));
    }
    let mut acc = 0.0;
    let mut cur = i;
    for _ in 0..n {
        budget.tick(1)?;
        let nx = if fwd { (cur + 1) % n } else { (cur + n - 1) % n };
        let d = len(sub(at(lp, nx)?, at(lp, cur)?));
        if acc + d >= window {
            let t = (window - acc) / d.max(1e-12);
            return Ok(add(at(lp, cur)?, mul(sub(at(lp, nx)?, at(lp, cur)?), t)));
        }
        acc += d;
        cur = nx;
    }
    at(lp, cur)
}
fn corners(lp: &[P], budget: &mut Budget<'_, '_>) -> Result<Vec<usize>> {
    let n = lp.len();
    let window = 1.5;
    let cos_thresh = (60f64).to_radians().cos();
    let mut turn = budget.vector(n)?;
    for i in 0..n {
        budget.tick(1)?;
        let a = norm(sub(at(lp, i)?, walk(lp, i, window, false, budget)?));
        let b = norm(sub(walk(lp, i, window, true, budget)?, at(lp, i)?));
        turn.push(dot(a, b));
    }
    let mut out = budget.vector(n)?;
    for i in 0..n {
        budget.tick(1)?;
        if at(&turn, i)? >= cos_thresh {
            continue;
        }
        let mut is_min = true;
        let mut acc = 0.0;
        let mut j = i;
        while acc < window {
            budget.tick(1)?;
            let nx = (j + 1) % n;
            acc += len(sub(at(lp, nx)?, at(lp, j)?));
            j = nx;
            if j == i {
                break;
            }
            if at(&turn, j)? < at(&turn, i)? {
                is_min = false;
                break;
            }
        }
        acc = 0.0;
        j = i;
        while is_min && acc < window {
            budget.tick(1)?;
            let pv = (j + n - 1) % n;
            acc += len(sub(at(lp, j)?, at(lp, pv)?));
            j = pv;
            if j == i {
                break;
            }
            if at(&turn, j)? <= at(&turn, i)? {
                is_min = false;
            }
        }
        if is_min {
            out.push(i);
        }
    }
    Ok(out)
}
fn fit_closed(lp: &[P], tol: f64, budget: &mut Budget<'_, '_>) -> Result<Subpath> {
    let n = lp.len();
    if n < 3 {
        return Err(TraceError::InvalidInput("short contour"));
    }
    let cs = corners(lp, budget)?;
    let capacity = n.checked_add(1).ok_or(TraceError::Limit)?;
    let mut segs: Vec<[P; 4]> = budget.vector(capacity)?;
    let mut corner_at_start: Vec<bool> = budget.vector(capacity)?;
    if cs.is_empty() {
        let mut pts = budget.vector(capacity)?;
        for point in lp {
            budget.tick(1)?;
            pts.push(*point);
        }
        pts.push(at(lp, 0)?);
        let t = norm(sub(at(lp, 1 % n)?, at(lp, n - 1)?));
        let before = segs.len();
        fit_cubic(&pts, t, mul(t, -1.0), tol * tol, &mut segs, 0, budget)?;
        for _ in before..segs.len() {
            budget.tick(1)?;
            corner_at_start.push(false);
        }
    } else {
        for (k, &c) in cs.iter().enumerate() {
            budget.tick(1)?;
            let end = at(&cs, (k + 1) % cs.len())?;
            let mut pts = budget.vector(capacity)?;
            pts.push(at(lp, c)?);
            let mut j = c;
            loop {
                budget.tick(1)?;
                j = (j + 1) % n;
                if pts.len() >= pts.capacity() {
                    return Err(TraceError::Limit);
                }
                pts.push(at(lp, j)?);
                if j == end {
                    break;
                }
            }
            let m = pts.len();
            let t1 = norm(sub(at(&pts, 1.min(m - 1))?, at(&pts, 0)?));
            let t2 = norm(sub(at(&pts, m.saturating_sub(2))?, at(&pts, m - 1)?));
            let before = segs.len();
            fit_cubic(&pts, t1, t2, tol * tol, &mut segs, 0, budget)?;
            for (q, _) in (before..segs.len()).enumerate() {
                budget.tick(1)?;
                if corner_at_start.len() >= corner_at_start.capacity() {
                    return Err(TraceError::Limit);
                }
                corner_at_start.push(q == 0);
            }
        }
    }
    let mut knots: Vec<Knot> = budget.vector(segs.len())?;
    let pt = |p: P| Point::new(p.0, p.1);
    for (i, s) in segs.iter().enumerate() {
        budget.tick(1)?;
        if s.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return Err(TraceError::InvalidInput("nonfinite fitted curve"));
        }
        knots.push(Knot { anchor: pt(s[0]), in_ctrl: pt(s[0]), out_ctrl: pt(s[1]), smooth: !at(&corner_at_start, i)? });
    }
    let m = segs.len();
    for (i, segment) in segs.iter().enumerate() {
        budget.tick(1)?;
        let knot = knots.get_mut((i + 1) % m).ok_or(TraceError::InvalidInput("knot index"))?;
        knot.in_ctrl = pt(segment[2]);
    }
    Ok(Subpath { closed: true, knots, op: PathOp::Combine })
}
fn chord_params(pts: &[P], budget: &mut Budget<'_, '_>) -> Result<Vec<f64>> {
    if pts.is_empty() {
        return Err(TraceError::InvalidInput("empty fit"));
    }
    let mut u = budget.vector(pts.len())?;
    for _ in pts {
        budget.tick(1)?;
        u.push(0.0);
    }
    for i in 1..pts.len() {
        budget.tick(1)?;
        let v = at(&u, i - 1)? + len(sub(at(pts, i)?, at(pts, i - 1)?));
        *u.get_mut(i).ok_or(TraceError::InvalidInput("parameter index"))? = v;
    }
    let total = at(&u, pts.len() - 1)?;
    if total > 0.0 {
        for v in &mut u {
            budget.tick(1)?;
            *v /= total;
        }
    }
    Ok(u)
}
fn generate(pts: &[P], u: &[f64], t1: P, t2: P, budget: &mut Budget<'_, '_>) -> Result<[P; 4]> {
    let last = pts.len().checked_sub(1).ok_or(TraceError::InvalidInput("empty fit"))?;
    let (p0, p3) = (at(pts, 0)?, at(pts, last)?);
    let mut c = [[0.0f64; 2]; 2];
    let mut x = [0.0f64; 2];
    for (i, &t) in u.iter().enumerate() {
        budget.tick(1)?;
        let mt = 1.0 - t;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let a1 = mul(t1, b1);
        let a2 = mul(t2, b2);
        c[0][0] += dot(a1, a1);
        c[0][1] += dot(a1, a2);
        c[1][1] += dot(a2, a2);
        let b0 = mt * mt * mt;
        let b3 = t * t * t;
        let tmp = sub(at(pts, i)?, add(mul(p0, b0 + b1), mul(p3, b2 + b3)));
        x[0] += dot(a1, tmp);
        x[1] += dot(a2, tmp);
    }
    c[1][0] = c[0][1];
    let det = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det0 = c[0][0] * x[1] - c[1][0] * x[0];
    let det1 = x[0] * c[1][1] - x[1] * c[0][1];
    let (mut al, mut ar) = if det.abs() > 1e-12 { (det1 / det, det0 / det) } else { (0.0, 0.0) };
    let seg = len(sub(p3, p0));
    let eps = 1e-6 * seg;
    if al < eps || ar < eps {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    Ok([p0, add(p0, mul(t1, al)), add(p3, mul(t2, ar)), p3])
}
fn max_error(pts: &[P], b: &[P; 4], u: &[f64], budget: &mut Budget<'_, '_>) -> Result<(f64, usize)> {
    let end = pts.len().checked_sub(1).ok_or(TraceError::InvalidInput("empty fit"))?;
    let mut best = (0.0, pts.len() / 2);
    for i in 1..end {
        budget.tick(1)?;
        let d = sub(bezier(b, at(u, i)?), at(pts, i)?);
        let e = dot(d, d);
        if e >= best.0 {
            best = (e, i);
        }
    }
    Ok(best)
}
fn reparam(pts: &[P], u: &[f64], b: &[P; 4], budget: &mut Budget<'_, '_>) -> Result<Vec<f64>> {
    let mut out = budget.vector(pts.len().min(u.len()))?;
    for (p, &t) in pts.iter().zip(u) {
        budget.tick(1)?;
        let q = bezier(b, t);
        let d1 = bezier_d1(b, t);
        let d2 = bezier_d2(b, t);
        let num = dot(sub(q, *p), d1);
        let den = dot(d1, d1) + dot(sub(q, *p), d2);
        out.push(if den.abs() < 1e-12 { t } else { (t - num / den).clamp(0.0, 1.0) });
    }
    Ok(out)
}
fn emit(out: &mut Vec<[P; 4]>, value: [P; 4], budget: &mut Budget<'_, '_>) -> Result<()> {
    budget.tick(1)?;
    if out.len() >= out.capacity() {
        return Err(TraceError::Limit);
    }
    out.push(value);
    Ok(())
}
fn fit_cubic(pts: &[P], t1: P, t2: P, err2: f64, out: &mut Vec<[P; 4]>, depth: u32, budget: &mut Budget<'_, '_>) -> Result<()> {
    budget.poll()?;
    let n = pts.len();
    if n < 2 {
        return Err(TraceError::InvalidInput("short fit"));
    }
    // Exact legacy depth guard and fallback curve, rather than a substitute fit.
    if n == 2 || depth > 40 {
        let first = at(pts, 0)?;
        let last = at(pts, n - 1)?;
        let d = len(sub(last, first)) / 3.0;
        return emit(out, [first, add(first, mul(t1, d)), add(last, mul(t2, d)), last], budget);
    }
    let mut u = chord_params(pts, budget)?;
    let mut b = generate(pts, &u, t1, t2, budget)?;
    let (mut e, mut split) = max_error(pts, &b, &u, budget)?;
    if e < err2 {
        return emit(out, b, budget);
    }
    if e < err2 * 16.0 {
        for _ in 0..6 {
            budget.poll()?;
            let u2 = reparam(pts, &u, &b, budget)?;
            let b2 = generate(pts, &u2, t1, t2, budget)?;
            let (e2, s2) = max_error(pts, &b2, &u2, budget)?;
            u = u2;
            b = b2;
            e = e2;
            split = s2;
            if e < err2 {
                return emit(out, b, budget);
            }
        }
    }
    let split = split.clamp(1, n - 2);
    let tc = norm(sub(at(pts, split - 1)?, at(pts, split + 1)?));
    let tc = if len(tc) < 0.5 { norm(sub(at(pts, split - 1)?, at(pts, split)?)) } else { tc };
    fit_cubic(pts.get(..=split).ok_or(TraceError::InvalidInput("fit split"))?, t1, tc, err2, out, depth + 1, budget)?;
    fit_cubic(pts.get(split..).ok_or(TraceError::InvalidInput("fit split"))?, mul(tc, -1.0), t2, err2, out, depth + 1, budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_matches_legacy_fractional_saddles_order_and_recursive_fitting() {
        for pattern in 0..16u8 {
            let values = [
                if pattern & 1 != 0 { 0.83 } else { 0.17 },
                if pattern & 2 != 0 { 0.71 } else { 0.31 },
                if pattern & 8 != 0 { 0.92 } else { 0.21 },
                if pattern & 4 != 0 { 0.61 } else { 0.44 },
            ];
            for outside in [0.0, 1.0] {
                for tolerance in [0.05, 0.2, 1.0, 10.0] {
                    let rect = Rect::new(-7, 5, -5, 7);
                    let expected = super::super::trace_mask(&values, rect, outside, tolerance);
                    assert_eq!(trace_mask_with_interrupt(&values, rect, outside, tolerance, TraceLimits::default(), &Interrupt::NONE).unwrap(), expected);
                }
            }
        }
        let mut values = Vec::new();
        for y in 0..12 {
            for x in 0..14 {
                let radius = ((x as f32 - 6.0).powi(2) + (y as f32 - 5.0).powi(2)).sqrt();
                values.push((5.4 - radius).clamp(0.0, 1.0));
            }
        }
        let rect = Rect::new(-20, -11, -6, 1);
        assert_eq!(
            trace_mask_with_interrupt(&values, rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::NONE).unwrap(),
            super::super::trace_mask(&values, rect, 0.0, 0.1)
        );
    }
    #[test]
    fn cancellation_and_limits_never_return_a_partial_path_or_modify_input() {
        let values = [0.2, 0.8, 0.9, 0.1];
        let before = values;
        let rect = Rect::new(0, 0, 2, 2);
        use std::sync::atomic::{AtomicUsize, Ordering};
        // No edges means no randomized HashMap iteration/heapsort poll count.
        // Replay the last check deterministically to exercise the publication fence.
        let empty = [0.0; 4];
        let empty_before = empty;
        let calls = AtomicUsize::new(0);
        let observe = || {
            calls.fetch_add(1, Ordering::Relaxed);
            false
        };
        let complete = trace_mask_with_interrupt(&empty, rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::cancel_only(&observe)).unwrap();
        assert!(complete.subpaths.is_empty());
        let last = calls.load(Ordering::Relaxed);
        assert!(last > 1);
        calls.store(0, Ordering::Relaxed);
        let cancel = || calls.fetch_add(1, Ordering::Relaxed) + 1 >= last;
        assert_eq!(
            trace_mask_with_interrupt(&empty, rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::cancel_only(&cancel)).unwrap_err(),
            TraceError::Cancelled
        );
        assert_eq!(empty, empty_before);
        assert_eq!(values, before);
        // A nonempty contour must also refuse promptly without publishing a path.
        assert_eq!(
            trace_mask_with_interrupt(&values, rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::cancel_only(&|| true)).unwrap_err(),
            TraceError::Cancelled
        );
        assert!(matches!(
            trace_mask_with_interrupt(&values, rect, 0.0, 0.1, TraceLimits { resident_bytes: 15, work_units: MAX_WORK }, &Interrupt::NONE),
            Err(TraceError::Limit)
        ));
        assert!(trace_mask_with_interrupt(&values[..3], rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::NONE).is_err());
        assert!(trace_mask_with_interrupt(&[f32::NAN, 0.0, 1.0, 0.5], rect, 0.0, 0.1, TraceLimits::default(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn controlled_fitter_retains_the_legacy_depth_guard_curve() {
        let points = [(0.0, 0.0), (0.3, 0.7), (1.0, 1.0)];
        let t1 = (1.0, 0.0);
        let t2 = (0.0, -1.0);
        let mut expected = Vec::new();
        super::super::fit_cubic(&points, t1, t2, 0.01, &mut expected, 41);
        let mut budget = Budget { control: &Interrupt::NONE, work: 0, limit: MAX_WORK, resident: TraceLimits::default().resident_bytes, edges: 3 };
        let mut actual = budget.vector(3).unwrap();
        fit_cubic(&points, t1, t2, 0.01, &mut actual, 41, &mut budget).unwrap();
        assert_eq!(actual, expected);
    }
}
