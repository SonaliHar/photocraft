//! Additive controlled version of the established uniform-piece Bézier flattener.
//! No rasterizer, curve fitting or alternative subdivision equations are used.
use crate::{Polyline, VectorPreparationLimits};
use photocraft_doc::{Path, Subpath};
use photocraft_geom::Point;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};

fn poll(control: &Interrupt<'_>) -> Result<(), E> {
    control.check().map_err(|_| E::Cancelled)
}
fn segment(s: &Subpath, i: usize) -> Result<[Point; 4], E> {
    let n = s.knots.len();
    let a = s.knots.get(i).ok_or(E::InvalidBounds)?;
    let next = i.checked_add(1).ok_or(E::InvalidBounds)?;
    let b = s.knots.get(if next == n && s.closed { 0 } else { next }).ok_or(E::InvalidBounds)?;
    Ok([a.anchor, a.out_ctrl, b.in_ctrl, b.anchor])
}
fn segments(s: &Subpath) -> usize {
    if s.knots.len() < 2 {
        0
    } else if s.closed {
        s.knots.len()
    } else {
        s.knots.len() - 1
    }
}
fn count(seg: &[Point; 4], tol: f64) -> Result<usize, E> {
    // Refuse nonfinite arithmetic rather than relying on the legacy invalid-input fallback.
    let [p0, c1, c2, p1] = seg;
    let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
    let len2 = dx * dx + dy * dy;
    if !len2.is_finite() {
        return Err(E::InvalidSample);
    }
    for c in [c1, c2] {
        let (vx, vy) = (c.x - p0.x, c.y - p0.y);
        if !vx.is_finite() || !vy.is_finite() || !(vx * dx + vy * dy).is_finite() || !(vx * dy - vy * dx).is_finite() {
            return Err(E::InvalidSample);
        }
    }
    if crate::flatten::is_line(seg) {
        return Ok(1);
    }
    let d1 = ((p0.x - 2.0 * c1.x + c2.x).powi(2) + (p0.y - 2.0 * c1.y + c2.y).powi(2)).sqrt();
    let d2 = ((c1.x - 2.0 * c2.x + p1.x).powi(2) + (c1.y - 2.0 * c2.y + p1.y).powi(2)).sqrt();
    let size = (c1.x - p0.x).hypot(c1.y - p0.y) + (c2.x - c1.x).hypot(c2.y - c1.y) + (p1.x - c2.x).hypot(p1.y - c2.y);
    let effective = tol.min(size * 1e-3).max(1e-4);
    if !d1.is_finite() || !d2.is_finite() || !size.is_finite() || !(0.75 * d1.max(d2) / effective).is_finite() {
        return Err(E::InvalidSample);
    }
    Ok(crate::flatten::pieces(seg, tol))
}
fn upper(s: &Subpath, tol: f64, cap: usize, control: &Interrupt<'_>) -> Result<usize, E> {
    let mut n = usize::from(!s.knots.is_empty());
    for i in 0..segments(s) {
        poll(control)?;
        n = n.checked_add(count(&segment(s, i)?, tol)?).filter(|n| *n <= cap).ok_or(E::LimitExceeded)?;
    }
    if n > cap {
        return Err(E::LimitExceeded);
    }
    Ok(n)
}
struct Memory {
    used: usize,
    max: usize,
}
impl Memory {
    fn reserve<T>(&mut self, n: usize, control: &Interrupt<'_>) -> Result<Vec<T>, E> {
        poll(control)?;
        let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        if self.used.checked_add(bytes).is_none_or(|v| v > self.max) {
            return Err(E::LimitExceeded);
        }
        let mut values = Vec::new();
        values.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
        let actual = values.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
        self.used = self.used.checked_add(actual).filter(|v| *v <= self.max).ok_or(E::LimitExceeded)?;
        Ok(values)
    }
}
fn push(out: &mut Polyline, p: (f64, f64), knot: bool) -> Result<(), E> {
    if !p.0.is_finite() || !p.1.is_finite() {
        return Err(E::InvalidSample);
    }
    if let Some(&last) = out.pts.last()
        && (last.0 - p.0).abs() < 1e-12
        && (last.1 - p.1).abs() < 1e-12
    {
        if knot && let Some(k) = out.knot.last_mut() {
            *k = true;
        }
        return Ok(());
    }
    if out.pts.len() == out.pts.capacity() || out.knot.len() == out.knot.capacity() {
        return Err(E::LimitExceeded);
    }
    out.pts.push(p);
    out.knot.push(knot);
    Ok(())
}
/// Flattens with the exact legacy line/piece/evaluation equations and point order.
/// Limits can only lower the existing preparation defaults. Borrowed retained path
/// capacities and actual output capacities are included; this is not a total RSS cap.
pub fn flatten_path_with_interrupt(p: &Path, tol: f64, limits: VectorPreparationLimits, control: &Interrupt<'_>) -> Result<Vec<Polyline>, E> {
    poll(control)?;
    if !tol.is_finite() {
        return Err(E::InvalidSample);
    }
    let defaults = VectorPreparationLimits::default();
    if limits.max_bytes > defaults.max_bytes
        || limits.max_edges > defaults.max_edges
        || limits.max_components > defaults.max_components
        || limits.max_work > defaults.max_work
    {
        return Err(E::LimitExceeded);
    }
    if p.subpaths.len() > limits.max_components {
        return Err(E::LimitExceeded);
    }
    let mut source = p.subpaths.capacity().checked_mul(std::mem::size_of::<Subpath>()).ok_or(E::LimitExceeded)?;
    let mut knots = 0usize;
    // Charge source inspection work before touching knot coordinates or curve math.
    for s in &p.subpaths {
        poll(control)?;
        source =
            source.checked_add(s.knots.capacity().checked_mul(std::mem::size_of::<photocraft_doc::Knot>()).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
        if source > limits.max_bytes {
            return Err(E::LimitExceeded);
        }
        knots = knots.checked_add(s.knots.len()).filter(|n| *n <= limits.max_edges).ok_or(E::LimitExceeded)?;
    }
    let base_work = u64::try_from(knots)
        .ok()
        .and_then(|n| n.checked_mul(256))
        .and_then(|n| n.checked_add(u64::try_from(p.subpaths.len()).ok()?.checked_mul(64)?))
        .ok_or(E::LimitExceeded)?;
    if base_work > limits.max_work {
        return Err(E::LimitExceeded);
    }
    let point_work_cap = usize::try_from((limits.max_work - base_work) / 64).unwrap_or(usize::MAX).min(limits.max_edges);
    let mut points = 0usize;
    for s in &p.subpaths {
        poll(control)?;
        for (i, k) in s.knots.iter().enumerate() {
            if i % 256 == 0 {
                poll(control)?;
            }
            for pt in [k.anchor, k.in_ctrl, k.out_ctrl] {
                if !pt.x.is_finite() || !pt.y.is_finite() {
                    return Err(E::InvalidSample);
                }
            }
        }
        let left = point_work_cap.checked_sub(points).ok_or(E::LimitExceeded)?;
        points = points.checked_add(upper(s, tol, left, control)?).ok_or(E::LimitExceeded)?;
    }
    let output = p
        .subpaths
        .len()
        .checked_mul(std::mem::size_of::<Polyline>())
        .and_then(|v| v.checked_add(points.checked_mul(std::mem::size_of::<(f64, f64)>() + std::mem::size_of::<bool>())?))
        .ok_or(E::LimitExceeded)?;
    let resident = source.checked_add(output).ok_or(E::LimitExceeded)?;
    if resident > limits.max_bytes || u64::try_from(resident).map_err(|_| E::LimitExceeded)? > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    let work = u64::try_from(knots)
        .ok()
        .and_then(|n| n.checked_mul(256))
        .and_then(|n| n.checked_add(u64::try_from(points).ok()?.checked_mul(64)?))
        .and_then(|n| n.checked_add(u64::try_from(p.subpaths.len()).ok()?.checked_mul(64)?))
        .ok_or(E::LimitExceeded)?;
    if work > limits.max_work {
        return Err(E::LimitExceeded);
    }
    let mut memory = Memory { used: source, max: limits.max_bytes };
    let mut lines = memory.reserve(p.subpaths.len(), control)?;
    for s in &p.subpaths {
        poll(control)?;
        let n = upper(s, tol, point_work_cap, control)?;
        let mut out = Polyline { pts: memory.reserve(n, control)?, knot: memory.reserve(n, control)?, closed: s.closed };
        if let Some(k) = s.knots.first() {
            push(&mut out, (k.anchor.x, k.anchor.y), true)?;
        }
        for i in 0..segments(s) {
            poll(control)?;
            let seg = segment(s, i)?;
            if crate::flatten::is_line(&seg) {
                let p = seg.get(3).ok_or(E::InvalidBounds)?;
                push(&mut out, (p.x, p.y), true)?;
                continue;
            }
            let n = crate::flatten::pieces(&seg, tol);
            for j in 1..=n {
                if j % 256 == 0 {
                    poll(control)?;
                }
                push(&mut out, crate::flatten::eval(&seg, j as f64 / n as f64), j == n)?;
            }
        }
        if s.closed && out.pts.len() > 1 {
            let a = out.pts.first().copied().ok_or(E::InvalidBounds)?;
            let b = out.pts.last().copied().ok_or(E::InvalidBounds)?;
            if (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12 {
                out.pts.pop();
                out.knot.pop();
                if let Some(k) = out.knot.first_mut() {
                    *k = true;
                }
            }
        }
        if lines.len() == lines.capacity() {
            return Err(E::LimitExceeded);
        }
        lines.push(out);
    }
    poll(control)?;
    Ok(lines)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_order_closed_open_curved_and_degenerate() {
        for path in [
            Path::default(),
            Path::new(vec![Subpath::default()]),
            Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]), Subpath::polyline(&[(1.0, 2.0), (1.0, 2.0), (5.0, 9.0)])]),
            crate::shapes::ellipse(0.0, 0.0, 37.0, 23.0),
        ] {
            for tol in [0.0, 0.001, 0.1, 1.0] {
                assert_eq!(
                    flatten_path_with_interrupt(&path, tol, VectorPreparationLimits::default(), &Interrupt::NONE).unwrap(),
                    crate::flatten_path(&path, tol)
                );
            }
        }
    }
    #[test]
    fn refusal_and_mid_piece_cancellation_leave_path() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let path = crate::shapes::ellipse(0.0, 0.0, 10000.0, 10000.0);
        let before = path.clone();
        assert_eq!(
            flatten_path_with_interrupt(&path, 0.1, VectorPreparationLimits { max_bytes: 0, ..Default::default() }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        );
        assert_eq!(
            flatten_path_with_interrupt(&path, 0.1, VectorPreparationLimits { max_edges: 2, ..Default::default() }, &Interrupt::NONE),
            Err(E::LimitExceeded)
        );
        for after in [0, 4, 16] {
            let polls = AtomicUsize::new(0);
            let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= after;
            assert_eq!(flatten_path_with_interrupt(&path, 0.001, VectorPreparationLimits::default(), &Interrupt::cancel_only(&cancel)), Err(E::Cancelled));
            assert_eq!(path, before);
        }
    }
}
