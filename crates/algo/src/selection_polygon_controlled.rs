//! Bounded, cancellable form of selection::polygon's upstream even-odd coverage.
//! Arithmetic/sub-row schedule and 64-row band restarts remain unchanged.
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
const MAX_POINTS: usize = 4096;
const MAX_WORK: u64 = 1 << 30;
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: MAX_WORK }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub output_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
    pub peak_resident_bytes: u64,
}
#[derive(Clone, Copy)]
struct Edge {
    top: f32,
    bottom: f32,
    a: (f32, f32),
    b: (f32, f32),
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
pub fn requirements_with_interrupt(points: &[(f32, f32)], area: Rect, anti_alias: bool, ctl: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    poll(ctl)?;
    if points.len() < 3
        || points.len() > MAX_POINTS
        || area.x1 < area.x0
        || area.y1 < area.y0
        || [area.x0, area.y0, area.x1, area.y1].iter().any(|v| !(-(1 << 29)..=1 << 29).contains(v))
    {
        return Err(RasterInputError::InvalidBounds);
    }
    for (i, (x, y)) in points.iter().enumerate() {
        if i.is_multiple_of(64) {
            poll(ctl)?;
        }
        if !x.is_finite() || !y.is_finite() || x.abs() > (1 << 29) as f32 || y.abs() > (1 << 29) as f32 {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let width = u64::from(area.width());
    let height = u64::from(area.height());
    let output_bytes = mul(mul(width, height)?, 4)?;
    let scratch_bytes = add(mul(points.len() as u64, (2 * std::mem::size_of::<Edge>() + std::mem::size_of::<usize>() + 4) as u64)?, 4096)?;
    let sub = if anti_alias { 4 } else { 1 };
    let log = (usize::BITS - points.len().leading_zeros()) as u64;
    let edge_work = mul(mul(height, sub)?, mul(points.len() as u64, add(log, 16)?)?)?;
    let work_units = add(add(edge_work, mul(mul(width, height)?, sub + 4)?)?, mul(points.len() as u64, add(log, 4)?)?)?;
    let peak_resident_bytes = add(output_bytes, scratch_bytes)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES || work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(ctl)?;
    Ok(Requirements { output_bytes, scratch_bytes, work_units, peak_resident_bytes })
}
fn reserve<T>(v: &mut Vec<T>, n: usize) -> Result<(), RasterInputError> {
    v.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
    if v.capacity() > n.saturating_add(16) {
        return Err(RasterInputError::AllocationFailed);
    }
    Ok(())
}
// Stable merge sort with explicit scratch, including polls within large merges.
fn sort_edges(edges: &mut Vec<Edge>, scratch: &mut Vec<Edge>, ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    let n = edges.len();
    let mut width = 1usize;
    while width < n {
        scratch.clear();
        for start in (0..n).step_by(width * 2) {
            let mid = start.saturating_add(width).min(n);
            let end = mid.saturating_add(width).min(n);
            let (mut left, mut right) = (start, mid);
            while left < mid || right < end {
                if scratch.len().is_multiple_of(64) {
                    poll(ctl)?;
                }
                let take_left = right == end
                    || (left < mid
                        && edges
                            .get(left)
                            .ok_or(RasterInputError::LengthMismatch)?
                            .top
                            .total_cmp(&edges.get(right).ok_or(RasterInputError::LengthMismatch)?.top)
                            .is_le());
                let at = if take_left {
                    let at = left;
                    left += 1;
                    at
                } else {
                    let at = right;
                    right += 1;
                    at
                };
                scratch.push(*edges.get(at).ok_or(RasterInputError::LengthMismatch)?);
            }
        }
        std::mem::swap(edges, scratch);
        width = width.checked_mul(2).ok_or(RasterInputError::LimitExceeded)?;
    }
    poll(ctl)
}
pub fn polygon_with_interrupt(points: &[(f32, f32)], area: Rect, anti_alias: bool, limits: Limits, ctl: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    let req = requirements_with_interrupt(points, area, anti_alias, ctl)?;
    if req.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > limits.work_units.min(MAX_WORK) {
        return Err(RasterInputError::LimitExceeded);
    }
    let width = area.width() as usize;
    let height = area.height() as usize;
    let pixels = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)?;
    let mut output = Vec::new();
    reserve(&mut output, pixels)?;
    while output.len() < pixels {
        poll(ctl)?;
        output.resize(output.len().saturating_add(4096).min(pixels), 0.0);
    }
    if pixels == 0 {
        poll(ctl)?;
        return Ok(output);
    }
    let mut edges = Vec::new();
    let mut scratch = Vec::new();
    let mut active = Vec::<usize>::new();
    let mut xs = Vec::<f32>::new();
    reserve(&mut edges, points.len())?;
    reserve(&mut scratch, points.len())?;
    reserve(&mut active, points.len())?;
    reserve(&mut xs, points.len())?;
    for i in 0..points.len() {
        if i.is_multiple_of(64) {
            poll(ctl)?;
        }
        let a = *points.get(i).ok_or(RasterInputError::LengthMismatch)?;
        let b = *points.get(if i + 1 == points.len() { 0 } else { i + 1 }).ok_or(RasterInputError::LengthMismatch)?;
        if a.1 < b.1 {
            edges.push(Edge { top: a.1, bottom: b.1, a, b });
        } else if b.1 < a.1 {
            edges.push(Edge { top: b.1, bottom: a.1, a, b });
        }
    }
    sort_edges(&mut edges, &mut scratch, ctl)?;
    let sub = if anti_alias { 4usize } else { 1 };
    let sample_y = |py: usize, s: usize| area.y0 as f32 + py as f32 + (s as f32 + 0.5) / sub as f32;
    for py0 in (0..height).step_by(64) {
        poll(ctl)?;
        let rows = (height - py0).min(64);
        let (first, last) = (sample_y(py0, 0), sample_y(py0 + rows - 1, sub - 1));
        let mut reach = 0usize;
        while reach < edges.len() && edges.get(reach).ok_or(RasterInputError::LengthMismatch)?.top <= last {
            if reach.is_multiple_of(64) {
                poll(ctl)?;
            }
            reach += 1;
        }
        let mut pending = 0usize;
        active.clear();
        for r in 0..rows {
            for s in 0..sub {
                poll(ctl)?;
                let y = sample_y(py0 + r, s);
                while pending < reach {
                    if pending.is_multiple_of(64) {
                        poll(ctl)?;
                    }
                    let e = edges.get(pending).ok_or(RasterInputError::LengthMismatch)?;
                    if e.bottom <= first {
                        pending += 1;
                        continue;
                    }
                    if e.top > y {
                        break;
                    }
                    active.push(pending);
                    pending += 1;
                    if pending.is_multiple_of(64) {
                        poll(ctl)?;
                    }
                }
                let mut retained = 0;
                for i in 0..active.len() {
                    if i.is_multiple_of(64) {
                        poll(ctl)?;
                    }
                    let at = *active.get(i).ok_or(RasterInputError::LengthMismatch)?;
                    if edges.get(at).ok_or(RasterInputError::LengthMismatch)?.bottom > y {
                        *active.get_mut(retained).ok_or(RasterInputError::LengthMismatch)? = at;
                        retained += 1;
                    }
                }
                active.truncate(retained);
                xs.clear();
                for (i, at) in active.iter().enumerate() {
                    if i.is_multiple_of(64) {
                        poll(ctl)?;
                    }
                    let Edge { a, b, .. } = *edges.get(*at).ok_or(RasterInputError::LengthMismatch)?;
                    let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
                    if !x.is_finite() {
                        return Err(RasterInputError::InvalidSample);
                    }
                    xs.push(x);
                }
                // At most 4096 finite scalar crossings, in-place/no allocation. Identical
                // total-ordered crossing words have interchangeable positions.
                xs.sort_unstable_by(|a, b| a.total_cmp(b));
                poll(ctl)?;
                let row_start = (py0 + r).checked_mul(width).ok_or(RasterInputError::LimitExceeded)?;
                let row = output.get_mut(row_start..row_start + width).ok_or(RasterInputError::LengthMismatch)?;
                for span in xs.as_chunks::<2>().0 {
                    let (x0, x1) = (
                        (span.first().ok_or(RasterInputError::LengthMismatch)? - area.x0 as f32).max(0.0),
                        (span.get(1).ok_or(RasterInputError::LengthMismatch)? - area.x0 as f32).min(width as f32),
                    );
                    if x1 <= x0 {
                        continue;
                    }
                    let (c0, c1) = (x0.floor() as usize, (x1.ceil() as usize).min(width));
                    for (cx, v) in row.iter_mut().enumerate().take(c1).skip(c0) {
                        if cx.is_multiple_of(1024) {
                            poll(ctl)?;
                        }
                        let cov = if anti_alias {
                            ((cx + 1) as f32).min(x1) - (cx as f32).max(x0)
                        } else {
                            let c = cx as f32 + 0.5;
                            if c >= x0 && c < x1 { 1.0 } else { 0.0 }
                        };
                        *v += cov.max(0.0) / sub as f32;
                    }
                }
            }
        }
        let start = py0 * width;
        let end = (py0 + rows) * width;
        for (i, v) in output.get_mut(start..end).ok_or(RasterInputError::LengthMismatch)?.iter_mut().enumerate() {
            if i.is_multiple_of(1024) {
                poll(ctl)?;
            }
            *v = v.min(1.0);
        }
    }
    poll(ctl)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_upstream_aa_and_band_math_match_for_finite_polygons() {
        for points in
            [vec![(-2.5, -3.0), (17.25, 2.5), (8.5, 135.5)], vec![(1., 1.), (16., 120.), (1., 120.), (16., 1.)], vec![(2., 2.), (2., 2.), (7., 8.), (13., 2.)]]
        {
            for aa in [false, true] {
                let area = Rect::new(-4, -5, 21, 140);
                let expected = crate::selection::polygon(&points, area, aa);
                let actual = polygon_with_interrupt(&points, area, aa, Limits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn cancellation_and_hostile_budget_fail_before_returning_partial_output() {
        let points = [(0., 0.), (20., 0.), (0., 20.)];
        let area = Rect::new(0, 0, 20, 20);
        assert_eq!(
            polygon_with_interrupt(&points, area, true, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE),
            Err(RasterInputError::LimitExceeded)
        );
        assert_eq!(polygon_with_interrupt(&points, area, true, Limits::default(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled));
        assert!(requirements_with_interrupt(&[(f32::NAN, 0.), (1., 0.), (1., 1.)], area, true, &Interrupt::NONE).is_err());
    }
}

#[cfg(test)]
mod late_cancel_tests {
    use super::*;
    #[test]
    fn stop_inside_band_work_does_not_return_partial_plane() {
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 100;
        let points = [(0., 0.), (200., 0.), (200., 200.), (0., 200.)];
        assert_eq!(
            polygon_with_interrupt(&points, Rect::new(0, 0, 200, 200), true, Limits::default(), &Interrupt::cancel_only(&stop)),
            Err(RasterInputError::Cancelled)
        );
        assert!(count.load(std::sync::atomic::Ordering::Relaxed) > 100);
    }
}
