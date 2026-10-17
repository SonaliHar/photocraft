//! TEMP controlled companion to the existing polygon stroker.
//! Geometry expressions and ordered emissions follow stroke.rs; no legacy allocation fallback.
use super::*;
use crate::{Polyline, StrokeStyle};
use photocraft_doc::{LineCap, LineJoin};
type P = (f64, f64);
fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P, k: f64) -> P {
    (a.0 * k, a.1 * k)
}
fn len(a: P) -> f64 {
    a.0.hypot(a.1)
}
fn norm(a: P) -> P {
    let l = len(a);
    if l > 0.0 { (a.0 / l, a.1 / l) } else { (0.0, 0.0) }
}
fn perp(a: P) -> P {
    (a.1, -a.0)
}
fn cross(a: P, b: P) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn dot(a: P, b: P) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn point(p: P) -> Result<P, E> {
    if p.0.is_finite() && p.1.is_finite() { Ok(p) } else { Err(E::InvalidSample) }
}
fn arc_step(r: f64, tol: f64) -> f64 {
    if r <= tol { std::f64::consts::FRAC_PI_2 } else { (2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos()).clamp(0.01, std::f64::consts::FRAC_PI_2) }
}
fn area_radius(r: f64, step: f64) -> f64 {
    if step < 1e-9 { r } else { r * (step / step.sin()).sqrt() }
}
fn vec<T>(n: usize, c: &mut ShapeBudget<'_, '_>) -> Result<Vec<T>, E> {
    let mut v = Vec::new();
    c.reserve(&mut v, n)?;
    Ok(v)
}
fn push_point(v: &mut Vec<P>, p: P, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    c.tick(64)?;
    c.push(v, point(p)?, c.limits.max_edges)
}
fn emit(out: &mut Vec<Vec<P>>, mut poly: Vec<P>, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    let mut sum = 0.0;
    for (i, p) in poly.iter().enumerate() {
        c.tick(32)?;
        let q = *poly.get((i + 1) % poly.len()).ok_or(E::LengthMismatch)?;
        sum += cross(*p, q);
    }
    let a = sum * 0.5;
    if !a.is_finite() {
        return Err(E::InvalidSample);
    }
    if a.abs() < 1e-12 {
        c.release(poly);
        return Ok(());
    }
    if a < 0.0 {
        for _ in poly.chunks(256) {
            c.tick(256)?;
        }
        poly.reverse();
    }
    c.push(out, poly, c.limits.max_components)
}
fn polygon(out: &mut Vec<Vec<P>>, p: &[P], c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    let mut v = vec(p.len(), c)?;
    for &p in p {
        push_point(&mut v, p, c)?;
    }
    emit(out, v, c)
}
fn circle(out: &mut Vec<Vec<P>>, p: P, r: f64, tol: f64, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    c.tick(128)?;
    let n = ((std::f64::consts::TAU / arc_step(r, tol)).ceil() as usize).max(8);
    if n > 629 {
        return Err(E::LimitExceeded);
    }
    let rr = area_radius(r, std::f64::consts::TAU / n as f64);
    let mut v = vec(n, c)?;
    for i in 0..n {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        push_point(&mut v, (p.0 + rr * a.cos(), p.1 + rr * a.sin()), c)?;
    }
    emit(out, v, c)
}
fn cap(out: &mut Vec<Vec<P>>, p: P, dir: P, hw: f64, cap: LineCap, tol: f64, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    c.tick(128)?;
    match cap {
        LineCap::Butt => Ok(()),
        LineCap::Round => circle(out, p, hw, tol, c),
        LineCap::Square => {
            let n = mul(perp(dir), hw);
            let e = mul(dir, hw);
            polygon(out, &[add(p, n), add(add(p, n), e), add(sub(p, n), e), sub(p, n)], c)
        }
    }
}
// Explicit geometric inputs keep the existing join equation's evaluation order.
#[allow(clippy::too_many_arguments)]
fn join(out: &mut Vec<Vec<P>>, v: P, d1: P, d2: P, hw: f64, join: LineJoin, miter_limit: f64, tol: f64, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    c.tick(256)?;
    let turn = cross(d1, d2);
    let d = dot(d1, d2);
    if turn.abs() < 1e-12 && d > 0.0 {
        return Ok(());
    }
    let s = if turn > 0.0 { 1.0 } else { -1.0 };
    let n1 = mul(perp(d1), hw * s);
    let n2 = mul(perp(d2), hw * s);
    let (a, b) = (add(v, n1), add(v, n2));
    match join {
        LineJoin::Bevel => polygon(out, &[v, a, b], c),
        LineJoin::Miter => {
            let cos_half = ((1.0 + d) * 0.5).max(0.0).sqrt();
            if cos_half < 1e-9 || 1.0 / cos_half > miter_limit {
                polygon(out, &[v, a, b], c)
            } else {
                let bis = norm(add(n1, n2));
                let m = add(v, mul(bis, hw / cos_half));
                polygon(out, &[v, a, m, b], c)
            }
        }
        LineJoin::Round => {
            let a0 = n1.1.atan2(n1.0);
            let mut a1 = n2.1.atan2(n2.0);
            let mut delta = a1 - a0;
            while delta > std::f64::consts::PI {
                c.tick(1)?;
                delta -= std::f64::consts::TAU;
            }
            while delta < -std::f64::consts::PI {
                c.tick(1)?;
                delta += std::f64::consts::TAU;
            }
            if turn.abs() < 1e-12 {
                delta = std::f64::consts::PI * if cross(n1, d1) > 0.0 { 1.0 } else { -1.0 };
            }
            a1 = a0 + delta;
            let steps = ((delta.abs() / arc_step(hw, tol)).ceil() as usize).max(1);
            if steps > 315 {
                return Err(E::LimitExceeded);
            }
            let rr = area_radius(hw, delta.abs() / steps as f64);
            let mut p = vec(steps.checked_add(2).ok_or(E::LimitExceeded)?, c)?;
            push_point(&mut p, v, c)?;
            push_point(&mut p, a, c)?;
            for k in 1..steps {
                let t = a0 + (a1 - a0) * k as f64 / steps as f64;
                push_point(&mut p, (v.0 + rr * t.cos(), v.1 + rr * t.sin()), c)?;
            }
            push_point(&mut p, b, c)?;
            emit(out, p, c)
        }
    }
}
fn stroke_one(pl: &Polyline, style: &StrokeStyle, tol: f64, out: &mut Vec<Vec<P>>, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    c.tick(256)?;
    let hw = style.width * 0.5;
    let mut pts = vec::<P>(pl.pts.len(), c)?;
    let mut knots = vec::<bool>(pl.pts.len(), c)?;
    for (i, &p) in pl.pts.iter().enumerate() {
        c.tick(128)?;
        point(p)?;
        if pts.last().is_some_and(|q| len(sub(p, *q)) < 1e-9) {
            continue;
        }
        pts.push(p);
        knots.push(pl.knot.get(i).copied().unwrap_or(true));
    }
    let mut closed = pl.closed;
    if closed && pts.len() > 1 && len(sub(*pts.first().ok_or(E::LengthMismatch)?, *pts.last().ok_or(E::LengthMismatch)?)) < 1e-9 {
        pts.pop();
        knots.pop();
    }
    if pts.len() < 3 {
        closed = false;
    }
    if pts.is_empty() {
        c.release(pts);
        c.release(knots);
        return Ok(());
    }
    if pts.len() == 1 {
        let p = *pts.first().ok_or(E::LengthMismatch)?;
        match style.cap {
            LineCap::Round => circle(out, p, hw, tol, c)?,
            LineCap::Square => polygon(out, &[(p.0 - hw, p.1 - hw), (p.0 + hw, p.1 - hw), (p.0 + hw, p.1 + hw), (p.0 - hw, p.1 + hw)], c)?,
            LineCap::Butt => {}
        }
        c.release(pts);
        c.release(knots);
        return Ok(());
    }
    let n = pts.len();
    let segs = if closed { n } else { n - 1 };
    let at = |i: usize| pts.get(i).copied().ok_or(E::LengthMismatch);
    let dir = |i: usize| -> Result<P, E> { Ok(norm(sub(at((i + 1) % n)?, at(i)?))) };
    let seg_len = |i: usize| -> Result<f64, E> { Ok(len(sub(at((i + 1) % n)?, at(i)?))) };
    let cos_smooth = 25f64.to_radians().cos();
    let smooth = |j: usize| -> Result<bool, E> {
        if j == 0 || (!closed && j == n - 1) {
            return Ok(false);
        }
        let (i0, i1) = ((j + n - 1) % n, j);
        let d = dot(dir(i0)?, dir(i1)?);
        if *knots.get(j).ok_or(E::LengthMismatch)? {
            return Ok(d > 1.0 - 1e-12);
        }
        if d < cos_smooth {
            return Ok(false);
        }
        let tan_half = ((1.0 - d) / (1.0 + d)).max(0.0).sqrt();
        Ok(hw * tan_half < 0.5 * seg_len(i0)?.min(seg_len(i1)?))
    };
    let mut left = vec::<P>(n.checked_add(1).ok_or(E::LimitExceeded)?, c)?;
    let mut right = vec::<P>(n.checked_add(1).ok_or(E::LimitExceeded)?, c)?;
    for i in 0..segs {
        c.tick(512)?;
        let a = at(i)?;
        let b = at((i + 1) % n)?;
        let nrm = mul(perp(dir(i)?), hw);
        if left.is_empty() {
            push_point(&mut left, add(a, nrm), c)?;
            push_point(&mut right, sub(a, nrm), c)?;
        }
        let j = (i + 1) % n;
        if i + 1 < segs + usize::from(!closed) && smooth(j)? {
            let n2 = mul(perp(dir(j)?), hw);
            let bis = norm(add(nrm, n2));
            let cos_half = ((1.0 + dot(dir(i)?, dir(j)?)) * 0.5).max(1e-9).sqrt();
            let off = mul(bis, hw / cos_half);
            push_point(&mut left, add(b, off), c)?;
            push_point(&mut right, sub(b, off), c)?;
            continue;
        }
        push_point(&mut left, add(b, nrm), c)?;
        push_point(&mut right, sub(b, nrm), c)?;
        let mut poly = vec(left.len().checked_add(right.len()).ok_or(E::LimitExceeded)?, c)?;
        for &p in &left {
            push_point(&mut poly, p, c)?;
        }
        for &p in right.iter().rev() {
            push_point(&mut poly, p, c)?;
        }
        left.clear();
        right.clear();
        emit(out, poly, c)?;
        if closed || j != n - 1 {
            let prev = at(i)?;
            let v = at(j)?;
            let next = at((j + 1) % n)?;
            let d1 = norm(sub(v, prev));
            let d2 = norm(sub(next, v));
            let knot = *knots.get(j).ok_or(E::LengthMismatch)?;
            let jn = match (style.join, knot) {
                (LineJoin::Round, _) => LineJoin::Round,
                (jn, true) => jn,
                (_, false) => LineJoin::Miter,
            };
            join(out, v, d1, d2, hw, jn, if knot { style.miter_limit.min(500.0) } else { f64::INFINITY }, tol, c)?;
        }
    }
    if !closed {
        let d0 = norm(sub(at(1)?, at(0)?));
        let d1 = norm(sub(at(n - 1)?, at(n - 2)?));
        cap(out, at(0)?, mul(d0, -1.0), hw, style.cap, tol, c)?;
        cap(out, at(n - 1)?, d1, hw, style.cap, tol, c)?;
    }
    c.release(left);
    c.release(right);
    c.release(pts);
    c.release(knots);
    Ok(())
}

#[derive(Default, Clone, Copy)]
struct DashCounts {
    points: usize,
    fragments: usize,
}
trait DashSink {
    fn start(&mut self, p: P, c: &mut ShapeBudget<'_, '_>) -> Result<(), E>;
    fn point(&mut self, p: P, knot: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E>;
    fn finish(&mut self, keep_single: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E>;
}
struct CountSink {
    totals: DashCounts,
    current: usize,
}
impl DashSink for CountSink {
    fn start(&mut self, _: P, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        c.tick(1)?;
        self.current = 1;
        Ok(())
    }
    fn point(&mut self, _: P, _: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        c.tick(1)?;
        self.current = self.current.checked_add(1).ok_or(E::LimitExceeded)?;
        Ok(())
    }
    fn finish(&mut self, keep_single: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        c.tick(1)?;
        if self.current > 1 || (keep_single && self.current > 0) {
            self.totals.points = self.totals.points.checked_add(self.current).ok_or(E::LimitExceeded)?;
            self.totals.fragments = self.totals.fragments.checked_add(1).ok_or(E::LimitExceeded)?;
        }
        self.current = 0;
        Ok(())
    }
}
struct EmitSink {
    out: Vec<Polyline>,
    current: Option<Polyline>,
}
impl DashSink for EmitSink {
    fn start(&mut self, p: P, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        let mut pl = Polyline { pts: Vec::new(), knot: Vec::new(), closed: false };
        push_point(&mut pl.pts, p, c)?;
        c.push(&mut pl.knot, true, c.limits.max_edges)?;
        self.current = Some(pl);
        Ok(())
    }
    fn point(&mut self, p: P, knot: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        if let Some(pl) = self.current.as_mut() {
            push_point(&mut pl.pts, p, c)?;
            c.push(&mut pl.knot, knot, c.limits.max_edges)?;
        }
        Ok(())
    }
    fn finish(&mut self, keep_single: bool, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
        if let Some(pl) = self.current.take() {
            if pl.pts.len() > 1 || keep_single {
                c.push(&mut self.out, pl, c.limits.max_components)?;
            } else {
                c.release(pl.pts);
                c.release(pl.knot);
            }
        }
        Ok(())
    }
}
// A scalar-only first invocation computes the bound before emitted dash allocations.
// The second invocation repeats every original step and pays independently.
fn dash_walk(pl: &Polyline, pattern: &[f64], offset: f64, sink: &mut impl DashSink, c: &mut ShapeBudget<'_, '_>) -> Result<(), E> {
    if pattern.is_empty() {
        return Err(E::InvalidSample);
    }
    if pattern.len() > 32 || !offset.is_finite() {
        return Err(E::InvalidSample);
    }
    let mut pat = [0.0; 64];
    let n = if pattern.len().is_multiple_of(2) { pattern.len() } else { pattern.len() * 2 };
    for i in 0..n {
        c.tick(64)?;
        let d = *pattern.get(i % pattern.len()).ok_or(E::LengthMismatch)?;
        if !d.is_finite() || d < 0.0 {
            return Err(E::InvalidSample);
        }
        *pat.get_mut(i).ok_or(E::LengthMismatch)? = d;
    }
    let mut total = 0.0;
    for &d in pat.get(..n).ok_or(E::LengthMismatch)? {
        c.tick(1)?;
        total += d;
    }
    for &p in &pl.pts {
        c.tick(32)?;
        point(p)?;
    }
    if !total.is_finite() || total <= 1e-9 {
        return Err(E::InvalidSample);
    }
    if pl.pts.len() < 2 {
        return Ok(());
    }
    let mut idx = 0;
    let mut left = *pat.first().ok_or(E::LengthMismatch)?;
    let mut skip = offset.rem_euclid(total);
    while skip > 0.0 {
        c.tick(64)?;
        if skip >= left {
            skip -= left;
            idx = (idx + 1) % n;
            left = *pat.get(idx).ok_or(E::LengthMismatch)?;
        } else {
            left -= skip;
            skip = 0.0;
        }
    }
    let mut on = idx.is_multiple_of(2);
    if on {
        sink.start(*pl.pts.first().ok_or(E::LengthMismatch)?, c)?;
    }
    let segs = pl.pts.len() - 1 + usize::from(pl.closed);
    let mut guard = 0;
    for wi in 0..segs {
        c.tick(128)?;
        let a = *pl.pts.get(wi).ok_or(E::LengthMismatch)?;
        let next = (wi + 1) % pl.pts.len();
        let b = *pl.pts.get(next).ok_or(E::LengthMismatch)?;
        let b_knot = if next == 0 { true } else { pl.knot.get(next).copied().unwrap_or(true) };
        let seg = len(sub(b, a));
        if !seg.is_finite() {
            return Err(E::InvalidSample);
        }
        let mut pos = 0.0;
        while seg - pos > 1e-12 {
            c.tick(256)?;
            guard += 1;
            if guard > 100_000 {
                return Err(E::LimitExceeded);
            }
            let step = left.min(seg - pos);
            if step > 0.0 && pos + step == pos {
                return Err(E::InvalidSample);
            }
            pos += step;
            left -= step;
            let at_end = seg - pos <= 1e-12;
            let p = if at_end { b } else { add(a, mul(sub(b, a), pos / seg)) };
            point(p)?;
            if on {
                sink.point(p, !at_end || b_knot, c)?;
            }
            if left <= 1e-12 {
                idx = (idx + 1) % n;
                left = *pat.get(idx).ok_or(E::LengthMismatch)?;
                if idx % 2 == 1 {
                    if on {
                        sink.finish(true, c)?;
                    }
                    on = false;
                } else {
                    sink.start(p, c)?;
                    on = true;
                }
            }
        }
    }
    if on {
        sink.finish(false, c)?;
    }
    Ok(())
}
fn dashes(pl: &Polyline, style: &StrokeStyle, c: &mut ShapeBudget<'_, '_>) -> Result<Vec<Polyline>, E> {
    let mut count = CountSink { totals: DashCounts::default(), current: 0 };
    dash_walk(pl, &style.dashes, style.dash_offset, &mut count, c)?;
    if count.totals.points > c.limits.max_edges || count.totals.fragments > c.limits.max_components {
        return Err(E::LimitExceeded);
    }
    let mut sink = EmitSink { out: vec(count.totals.fragments, c)?, current: None };
    dash_walk(pl, &style.dashes, style.dash_offset, &mut sink, c)?;
    if pl.closed {
        c.tick(128)?;
        let seam = pl.pts.first().copied();
        let wraps = sink.out.first().and_then(|d| d.pts.first()).copied() == seam && sink.out.last().and_then(|d| d.pts.last()).copied() == seam;
        if wraps && sink.out.len() == 1 {
            if let Some(d) = sink.out.first_mut() {
                d.closed = true;
            }
        } else if wraps && sink.out.len() > 1 {
            let mut tail = sink.out.pop().ok_or(E::LengthMismatch)?;
            let first = sink.out.first_mut().ok_or(E::LengthMismatch)?;
            let target = tail.pts.len().checked_add(first.pts.len().saturating_sub(1)).ok_or(E::LimitExceeded)?;
            c.reserve(&mut tail.pts, target)?;
            c.reserve(&mut tail.knot, target)?;
            for (&p, &k) in first.pts.iter().skip(1).zip(first.knot.iter().skip(1)) {
                c.tick(64)?;
                tail.pts.push(p);
                tail.knot.push(k);
            }
            let old = std::mem::replace(first, tail);
            c.release(old.pts);
            c.release(old.knot);
        }
    }
    Ok(sink.out)
}
pub(super) fn polygons(lines: &[Polyline], style: &StrokeStyle, tol: f64, c: &mut ShapeBudget<'_, '_>) -> Result<Vec<Vec<P>>, E> {
    if style.width <= 0.0 || !style.width.is_finite() || !style.miter_limit.is_finite() || style.miter_limit < 1.0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for pl in lines {
        c.tick(128)?;
        if style.dashes.is_empty() {
            stroke_one(pl, style, tol, &mut out, c)?;
        } else {
            let mut dashed = dashes(pl, style, c)?;
            for d in &dashed {
                stroke_one(d, style, tol, &mut out, c)?;
            }
            for _ in &dashed {
                c.tick(1)?;
            }
            for d in dashed.drain(..) {
                c.release(d.pts);
                c.release(d.knot);
            }
            c.release(dashed);
        }
    }
    Ok(out)
}
