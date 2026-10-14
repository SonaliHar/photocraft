//! Bounded companion to the ordinary style natural-spline/corner LUT.
//! Names and original DTOs are inert; this does not admit new source epochs.
use super::{Contour, Map};
use photocraft_raster::{Interrupt, RasterInputError as E};
const TABLE: usize = 4096;
const MAX_POINTS: usize = 4096;
const MAX_WORK: u64 = 8_000_000_000;
#[derive(Clone, Copy)]
pub(super) struct Requirements {
    pub peak_bytes: usize,
    pub work_units: u64,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(super) fn requirements(contour: &Contour, range: Option<f32>, force_linear: bool) -> Result<Requirements, E> {
    if range.is_some_and(|v| !v.is_finite()) {
        return Err(E::UnsupportedConversion);
    }
    let n = match contour {
        Contour::Linear => 0,
        Contour::Custom { points, .. } => points.len(),
    };
    if n > MAX_POINTS {
        return Err(E::LimitExceeded);
    }
    let table = matches!(contour, Contour::Custom { .. }) || force_linear || range.is_some_and(|v| v.clamp(0.01, 1.0) < 0.999);
    if !table {
        return Ok(Requirements { peak_bytes: 0, work_units: 0 });
    }
    // Tagged sorted points, five f64 solve arrays, and simultaneous base/run
    // or base/ranged tables. No input-sized stable-sort scratch is hidden.
    let peak_bytes = n.checked_mul(68).and_then(|n| n.checked_add(TABLE * 8 + 1024)).ok_or(E::LimitExceeded)?;
    let count = n as u64;
    let work_units = count
        .checked_mul(count)
        .and_then(|n| n.checked_mul(32))
        .and_then(|n| {
            count
                .checked_mul(2)
                .and_then(|v| v.checked_add(1))
                .and_then(|v| v.checked_mul(TABLE as u64))
                .and_then(|v| v.checked_mul(64))
                .and_then(|v| n.checked_add(v))
        })
        .and_then(|n| count.checked_mul(128).and_then(|v| n.checked_add(v)))
        .ok_or(E::LimitExceeded)?;
    if work_units > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(Requirements { peak_bytes, work_units })
}
pub(super) fn validate(contour: &Contour, c: &Interrupt<'_>) -> Result<(), E> {
    poll(c)?;
    requirements(contour, None, false)?;
    if let Contour::Custom { points, .. } = contour {
        for point in points {
            poll(c)?;
            if !point.input.is_finite() || !point.output.is_finite() {
                return Err(E::UnsupportedConversion);
            }
        }
    }
    poll(c)
}
fn filled<T: Copy>(n: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != n {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), value);
    }
    Ok(out)
}
#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
    corner: bool,
}
fn points(contour: &Contour, c: &Interrupt<'_>) -> Result<Vec<Point>, E> {
    let Contour::Custom { points, corners, .. } = contour else {
        return Ok(Vec::new());
    };
    let mut out = filled(points.len(), Point { x: 0.0, y: 0.0, corner: false }, c)?;
    for (i, (dst, p)) in out.iter_mut().zip(points).enumerate() {
        poll(c)?;
        *dst = Point { x: p.input, y: p.output, corner: corners.get(i).copied().unwrap_or(false) };
    }
    // Stable insertion sorting uses the same finite-input comparison as the old
    // stable sort, and polls each bounded move. Equal X keeps original order.
    for i in 1..out.len() {
        let p = *out.get(i).ok_or(E::LengthMismatch)?;
        let mut j = i;
        while j > 0 {
            poll(c)?;
            let previous = *out.get(j - 1).ok_or(E::LengthMismatch)?;
            if previous.x <= p.x {
                break;
            }
            *out.get_mut(j).ok_or(E::LengthMismatch)? = previous;
            j -= 1;
        }
        *out.get_mut(j).ok_or(E::LengthMismatch)? = p;
    }
    let mut kept = 0usize;
    for i in 0..out.len() {
        poll(c)?;
        let p = *out.get(i).ok_or(E::LengthMismatch)?;
        if kept > 0 && (p.x - out.get(kept - 1).ok_or(E::LengthMismatch)?.x).abs() < 1e-6 {
            continue;
        }
        *out.get_mut(kept).ok_or(E::LengthMismatch)? = p;
        kept = kept.checked_add(1).ok_or(E::LimitExceeded)?;
    }
    out.truncate(kept);
    Ok(out)
}
fn natural(pts: &[Point], c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let n = pts.len();
    let mut out = filled(TABLE, 0.0f32, c)?;
    if n < 2 {
        for (k, v) in out.iter_mut().enumerate() {
            poll(c)?;
            *v = k as f32 / (TABLE - 1) as f32;
        }
        return Ok(out);
    }
    let mut m2 = filled(n, 0.0f64, c)?;
    if n > 2 {
        let mut x = filled(n, 0.0f64, c)?;
        let mut y = filled(n, 0.0f64, c)?;
        let mut cc = filled(n, 0.0f64, c)?;
        let mut d = filled(n, 0.0f64, c)?;
        for ((x, y), p) in x.iter_mut().zip(&mut y).zip(pts) {
            poll(c)?;
            *x = f64::from(p.x);
            *y = f64::from(p.y);
        }
        for i in 1..n - 1 {
            poll(c)?;
            let (h0, h1) = (
                x.get(i).ok_or(E::LengthMismatch)? - x.get(i - 1).ok_or(E::LengthMismatch)?,
                x.get(i + 1).ok_or(E::LengthMismatch)? - x.get(i).ok_or(E::LengthMismatch)?,
            );
            let a = h0 / 6.0;
            let b = (h0 + h1) / 3.0;
            let cv = h1 / 6.0;
            let r = (y.get(i + 1).ok_or(E::LengthMismatch)? - y.get(i).ok_or(E::LengthMismatch)?) / h1
                - (y.get(i).ok_or(E::LengthMismatch)? - y.get(i - 1).ok_or(E::LengthMismatch)?) / h0;
            let denom = b - a * cc.get(i - 1).ok_or(E::LengthMismatch)?;
            let ci = cv / denom;
            let di = (r - a * d.get(i - 1).ok_or(E::LengthMismatch)?) / denom;
            if h0 <= 0.0 || h1 <= 0.0 || !ci.is_finite() || !di.is_finite() {
                return Err(E::UnsupportedConversion);
            }
            *cc.get_mut(i).ok_or(E::LengthMismatch)? = ci;
            *d.get_mut(i).ok_or(E::LengthMismatch)? = di;
        }
        for i in (1..n - 1).rev() {
            poll(c)?;
            let v = d.get(i).ok_or(E::LengthMismatch)? - cc.get(i).ok_or(E::LengthMismatch)? * m2.get(i + 1).ok_or(E::LengthMismatch)?;
            if !v.is_finite() {
                return Err(E::UnsupportedConversion);
            }
            *m2.get_mut(i).ok_or(E::LengthMismatch)? = v;
        }
    }
    let first = pts.first().ok_or(E::LengthMismatch)?;
    let last = pts.last().ok_or(E::LengthMismatch)?;
    for (k, v) in out.iter_mut().enumerate() {
        poll(c)?;
        let xv = k as f32 / (TABLE - 1) as f32;
        if xv <= first.x {
            *v = first.y.clamp(0.0, 1.0);
            continue;
        }
        if xv >= last.x {
            *v = last.y.clamp(0.0, 1.0);
            continue;
        }
        let mut i = n - 2;
        for (index, pair) in pts.windows(2).enumerate() {
            poll(c)?;
            if xv <= pair.get(1).ok_or(E::LengthMismatch)?.x {
                i = index;
                break;
            }
        }
        let p0 = pts.get(i).ok_or(E::LengthMismatch)?;
        let p1 = pts.get(i + 1).ok_or(E::LengthMismatch)?;
        let (x0, y0) = (f64::from(p0.x), f64::from(p0.y));
        let (x1, y1) = (f64::from(p1.x), f64::from(p1.y));
        let h = x1 - x0;
        let t = f64::from(xv);
        let a = (x1 - t) / h;
        let b = (t - x0) / h;
        let y =
            a * y0 + b * y1 + ((a * a * a - a) * m2.get(i).ok_or(E::LengthMismatch)? + (b * b * b - b) * m2.get(i + 1).ok_or(E::LengthMismatch)?) * h * h / 6.0;
        if !y.is_finite() || !(y as f32).is_finite() {
            return Err(E::UnsupportedConversion);
        }
        *v = (y as f32).clamp(0.0, 1.0);
    }
    Ok(out)
}
fn custom(contour: &Contour, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let pts = points(contour, c)?;
    let mut has_corner = false;
    for p in &pts {
        poll(c)?;
        has_corner |= p.corner;
    }
    if !has_corner || pts.len() < 2 {
        return natural(&pts, c);
    }
    let mut out: Option<Vec<f32>> = None;
    let mut start = 0usize;
    let last = pts.len() - 1;
    for i in 1..=last {
        poll(c)?;
        if !pts.get(i).ok_or(E::LengthMismatch)?.corner && i != last {
            continue;
        }
        let seg = pts.get(start..=i).ok_or(E::LengthMismatch)?;
        let from = seg.first().ok_or(E::LengthMismatch)?.x;
        let lut = natural(seg, c)?;
        if let Some(ref mut output) = out {
            for (k, (o, v)) in output.iter_mut().zip(lut).enumerate() {
                poll(c)?;
                if k as f32 / (TABLE - 1) as f32 >= from {
                    *o = v;
                }
            }
        } else {
            out = Some(lut);
        }
        start = i;
    }
    out.ok_or(E::LengthMismatch)
}
pub(super) fn prepare(contour: &Contour, range: Option<f32>, force_linear: bool, budget: usize, c: &Interrupt<'_>) -> Result<Option<Vec<f32>>, E> {
    poll(c)?;
    let req = requirements(contour, range, force_linear)?;
    if req.peak_bytes > budget.min(64 << 20) {
        return Err(E::LimitExceeded);
    }
    validate(contour, c)?;
    let base = match contour {
        Contour::Linear => None,
        Contour::Custom { .. } => Some(custom(contour, c)?),
    };
    if let Some(range) = range {
        let range = range.clamp(0.01, 1.0);
        if range < 0.999 {
            let mut ranged = filled(TABLE, 0.0f32, c)?;
            for (k, o) in ranged.iter_mut().enumerate() {
                poll(c)?;
                let v = (k as f32 / (TABLE - 1) as f32 / range).min(1.0);
                *o = match &base {
                    Some(lut) => sample(lut, v)?,
                    None => v,
                };
            }
            poll(c)?;
            return Ok(Some(ranged));
        }
    }
    if base.is_some() || !force_linear {
        poll(c)?;
        return Ok(base);
    }
    let mut linear = filled(TABLE, 0.0f32, c)?;
    for (k, v) in linear.iter_mut().enumerate() {
        poll(c)?;
        *v = k as f32 / (TABLE - 1) as f32;
    }
    poll(c)?;
    Ok(Some(linear))
}
pub(super) fn sample(lut: &[f32], v: f32) -> Result<f32, E> {
    if lut.len() != TABLE || !v.is_finite() {
        return Err(E::InvalidSample);
    }
    let x = v.clamp(0.0, 1.0) * (lut.len() - 1) as f32;
    let i = x.floor() as usize;
    let j = (i + 1).min(lut.len() - 1);
    let a = *lut.get(i).ok_or(E::LengthMismatch)?;
    let b = *lut.get(j).ok_or(E::LengthMismatch)?;
    let value = a + (b - a) * (x - i as f32);
    if !value.is_finite() {
        return Err(E::InvalidSample);
    }
    Ok(value)
}
pub(super) fn apply(map: &mut Map, lut: Option<&[f32]>, c: &Interrupt<'_>) -> Result<(), E> {
    let Some(lut) = lut else {
        return poll(c);
    };
    for v in &mut map.v {
        poll(c)?;
        *v = sample(lut, *v)?;
    }
    poll(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::adjust::CurvePoint;
    fn contour(corners: Vec<bool>) -> Contour {
        Contour::Custom {
            name: "inert".into(),
            points: vec![
                CurvePoint { input: 1.0, output: 0.2 },
                CurvePoint { input: 0.4, output: 0.9 },
                CurvePoint { input: 0.0, output: 0.1 },
                CurvePoint { input: 0.4, output: 0.7 },
                CurvePoint { input: 0.7, output: 0.05 },
            ],
            corners,
        }
    }
    #[test]
    fn exact_corner_runs_sort_dedup_range_and_forced_linear_words() {
        for c in [
            Contour::Linear,
            contour(vec![]),
            contour(vec![true; 5]),
            contour(vec![false, true, false]),
            Contour::Custom { name: String::new(), points: vec![], corners: vec![] },
        ] {
            for range in [None, Some(0.0), Some(0.5), Some(0.998), Some(0.999), Some(1.0)] {
                let expected = if let Some(range) = range { super::super::ranged_lut(&c, range) } else { super::super::contour_lut(&c) };
                let actual = prepare(&c, range, false, 64 << 20, &Interrupt::NONE).unwrap();
                assert_eq!(
                    actual.as_ref().map(|l| l.iter().map(|v| v.to_bits()).collect::<Vec<_>>()),
                    expected.as_ref().map(|l| l.iter().map(|v| v.to_bits()).collect::<Vec<_>>())
                );
            }
            let expected = super::super::bevel_contour_lut(&photocraft_doc::BevelContour { contour: c.clone(), range: 0.17, anti_alias: true });
            let actual = prepare(&c, None, true, 64 << 20, &Interrupt::NONE).unwrap().unwrap();
            assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        }
    }
    #[test]
    fn bounded_private_preparation_refuses_and_preserves_config() {
        let c = contour(vec![true; 5]);
        let before = c.clone();
        assert!(matches!(prepare(&c, None, false, 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ticks = AtomicUsize::new(0);
        let cancel = || ticks.fetch_add(1, Ordering::Relaxed) > 40;
        assert!(matches!(prepare(&c, None, false, 64 << 20, &Interrupt::cancel_only(&cancel)), Err(E::Cancelled)));
        assert_eq!(c, before);
        let mut bad = c.clone();
        if let Contour::Custom { points, .. } = &mut bad {
            points[0].input = f32::NAN;
        }
        assert!(matches!(prepare(&bad, None, false, 64 << 20, &Interrupt::NONE), Err(E::UnsupportedConversion)));
        if let Contour::Custom { points, .. } = &mut bad {
            points[0].input = 1.0;
            points.resize(MAX_POINTS + 1, CurvePoint { input: 0.0, output: 0.0 });
        }
        assert!(matches!(prepare(&bad, None, false, 64 << 20, &Interrupt::NONE), Err(E::LimitExceeded)));
    }
}
