//! Exact existing five-by-five chamfer and anti-aliased spread companions.
//! Private ordinary-style preparation; no source-render admission is implied.
use super::{CHAMFER_INF, INSIDE_EPS, Map};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
const MAX_WORK: u64 = 8_000_000_000;
#[derive(Clone, Copy, Debug)]
pub(super) struct Requirements {
    pub peak_bytes: usize,
    pub work_units: u64,
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
/// Conservative peak includes independent start/distance/nearest/result owners;
/// start actually becomes distance in place, so no upstream charge is refunded.
pub(super) fn requirements(source: &Map, r: f32, budget: usize) -> Result<Requirements, E> {
    if !r.is_finite() {
        return Err(E::UnsupportedConversion);
    }
    let n = source.w.checked_mul(source.h).ok_or(E::LimitExceeded)?;
    if source.v.len() != n {
        return Err(E::LengthMismatch);
    }
    let input = source.v.capacity().checked_mul(4).ok_or(E::LimitExceeded)?;
    let owners = if r > 0.0 { 16usize.checked_add(std::mem::size_of::<usize>()).ok_or(E::LimitExceeded)? } else { 4 };
    let peak = input.checked_add(n.checked_mul(owners).ok_or(E::LimitExceeded)?).ok_or(E::LimitExceeded)?;
    let work = (n as u64).checked_mul(if r > 0.0 { 192 } else { 4 }).ok_or(E::LimitExceeded)?;
    if peak > budget.min(64 << 20) || peak as u64 > MAX_CHECKED_RASTER_BYTES || work > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    Ok(Requirements { peak_bytes: peak, work_units: work })
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
fn relax(x: i64, y: i64, sign: i64, extent: (i64, i64), d: &mut [f32], near: &mut [usize], c: &Interrupt<'_>) -> Result<(), E> {
    let (w, h) = extent;
    const S2: f32 = std::f32::consts::SQRT_2;
    const S5: f32 = 2.236_068;
    const FWD: [(i64, i64, f32); 8] = [(-1, 0, 1.0), (-1, -1, S2), (0, -1, 1.0), (1, -1, S2), (-1, -2, S5), (1, -2, S5), (-2, -1, S5), (2, -1, S5)];
    let at = |x: i64, y: i64| -> Result<usize, E> {
        usize::try_from(y)
            .ok()
            .and_then(|y| usize::try_from(w).ok().and_then(|w| y.checked_mul(w)))
            .and_then(|n| usize::try_from(x).ok().and_then(|x| n.checked_add(x)))
            .ok_or(E::LimitExceeded)
    };
    let i = at(x, y)?;
    let mut best = *d.get(i).ok_or(E::LengthMismatch)?;
    let mut bn = *near.get(i).ok_or(E::LengthMismatch)?;
    for (ox, oy, wt) in FWD {
        poll(c)?;
        let nx = x.checked_add(ox * sign).ok_or(E::InvalidBounds)?;
        let ny = y.checked_add(oy * sign).ok_or(E::InvalidBounds)?;
        if nx < 0 || ny < 0 || nx >= w || ny >= h {
            continue;
        }
        let j = at(nx, ny)?;
        let candidate = *d.get(j).ok_or(E::LengthMismatch)? + wt;
        // Strict comparison and original neighbor sequence preserve nearest ties.
        if candidate < best {
            best = candidate;
            bn = *near.get(j).ok_or(E::LengthMismatch)?;
        }
    }
    *d.get_mut(i).ok_or(E::LengthMismatch)? = best;
    *near.get_mut(i).ok_or(E::LengthMismatch)? = bn;
    Ok(())
}
fn chamfer(mut distance: Vec<f32>, w: usize, h: usize, c: &Interrupt<'_>) -> Result<(Vec<f32>, Vec<usize>), E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if distance.len() != n {
        return Err(E::LengthMismatch);
    }
    let mut nearest = filled(n, 0usize, c)?;
    for (i, v) in nearest.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = i;
    }
    let wi = i64::try_from(w).map_err(|_| E::InvalidBounds)?;
    let hi = i64::try_from(h).map_err(|_| E::InvalidBounds)?;
    for y in 0..hi {
        poll(c)?;
        for x in 0..wi {
            relax(x, y, 1, (wi, hi), &mut distance, &mut nearest, c)?;
        }
    }
    for y in (0..hi).rev() {
        poll(c)?;
        for x in (0..wi).rev() {
            relax(x, y, -1, (wi, hi), &mut distance, &mut nearest, c)?;
        }
    }
    poll(c)?;
    Ok((distance, nearest))
}
/// Same `dilate` coverage and sentinel equations, after requirements admission.
pub(super) fn dilate(source: &Map, r: f32, budget: usize, c: &Interrupt<'_>) -> Result<Map, E> {
    poll(c)?;
    let req = requirements(source, r, budget)?;
    if req.peak_bytes > budget.min(64 << 20) || req.work_units > MAX_WORK {
        return Err(E::LimitExceeded);
    }
    let n = source.v.len();
    for values in source.v.chunks(4096) {
        poll(c)?;
        if values.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    if r <= 0.0 || n == 0 {
        let mut out = filled(n, 0.0f32, c)?;
        for (a, b) in out.chunks_mut(4096).zip(source.v.chunks(4096)) {
            poll(c)?;
            a.copy_from_slice(b);
        }
        return Ok(Map { w: source.w, h: source.h, v: out });
    }
    let mut start = filled(n, 0.0f32, c)?;
    for (i, (v, a)) in start.iter_mut().zip(&source.v).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = if *a > INSIDE_EPS { 1.0 - a.min(1.0) } else { CHAMFER_INF };
    }
    let (distance, nearest) = chamfer(start, source.w, source.h, c)?;
    drop(nearest);
    let mut outside = filled(n, 0.0f32, c)?;
    for (i, ((o, d), a)) in outside.iter_mut().zip(&distance).zip(&source.v).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = if *a > INSIDE_EPS { -0.5 } else { d - 0.5 };
    }
    let mut out = filled(n, 0.0f32, c)?;
    for (i, ((o, a), d)) in out.iter_mut().zip(&source.v).zip(outside).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = if d < 0.0 { (*a + r).min(1.0) } else { a.max((r + 0.5 - d).clamp(0.0, 1.0)) };
        if !o.is_finite() {
            return Err(E::InvalidSample);
        }
    }
    poll(c)?;
    Ok(Map { w: source.w, h: source.h, v: out })
}
/// Exact stroke chamfer field. Local coverage is computed from immutable
/// neighbors in place of materializing an otherwise unobserved full map.
pub(super) fn field(shape: &Map, inner: bool, budget: usize, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    requirements(shape, 1.0, budget)?;
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    if (n as u64).checked_mul(if inner { 320 } else { 192 }).is_none_or(|n| n > MAX_WORK) {
        return Err(E::LimitExceeded);
    }
    let mut start = filled(n, 0.0f32, c)?;
    let get = |x: i64, y: i64| -> Result<f32, E> {
        if x < 0 || y < 0 || x >= shape.w as i64 || y >= shape.h as i64 {
            return Ok(0.0);
        }
        shape.v.get(y as usize * shape.w + x as usize).copied().ok_or(E::LengthMismatch)
    };
    for y in 0..shape.h {
        poll(c)?;
        for x in 0..shape.w {
            poll(c)?;
            let i = y * shape.w + x;
            let a = *shape.v.get(i).ok_or(E::LengthMismatch)?;
            if !a.is_finite() {
                return Err(E::InvalidSample);
            }
            let seed = if inner {
                if a <= INSIDE_EPS {
                    0.0
                } else {
                    let mut maximum = a;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            poll(c)?;
                            let value = get(x as i64 + dx, y as i64 + dy)?;
                            if !value.is_finite() {
                                return Err(E::InvalidSample);
                            }
                            maximum = maximum.max(value);
                        }
                    }
                    let cov = (a / maximum).min(1.0);
                    if cov < 1.0 - INSIDE_EPS { cov } else { CHAMFER_INF }
                }
            } else if a > INSIDE_EPS {
                1.0 - a.min(1.0)
            } else {
                CHAMFER_INF
            };
            *start.get_mut(i).ok_or(E::LengthMismatch)? = seed;
        }
    }
    let (d, nearest) = chamfer(start, shape.w, shape.h, c)?;
    drop(nearest);
    let mut out = filled(n, 0.0f32, c)?;
    for (i, ((o, d), a)) in out.iter_mut().zip(d).zip(&shape.v).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = if if inner { *a <= INSIDE_EPS } else { *a > INSIDE_EPS } { -0.5 } else { d - 0.5 };
    }
    poll(c)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_chamfer_order_ties_and_degenerate_shapes() {
        for (w, h) in [(0, 0), (1, 1), (1, 17), (23, 1), (21, 19)] {
            let n = w * h;
            let seeds = (0..n).map(|i| if i % 7 == 0 { 0.25 } else { CHAMFER_INF }).collect::<Vec<_>>();
            let expected = super::super::chamfer_from(seeds.clone(), w, h);
            let got = chamfer(seeds, w, h, &Interrupt::NONE).unwrap();
            assert_eq!(got.0.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.0.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            assert_eq!(got.1, expected.1);
        }
    }
    #[test]
    fn exact_fractional_spread_and_raw_identity() {
        for (w, h) in [(0, 0), (1, 1), (1, 37), (31, 1), (39, 29)] {
            let mut shape = Map::new(w, h, 0.0);
            for (i, v) in shape.v.iter_mut().enumerate() {
                *v = ((i * 37) % 101) as f32 / 100.0;
            }
            for radius in [-2.0, 0.0, 0.25, 1.0, 3.75, 29.0] {
                let got = dilate(&shape, radius, 64 << 20, &Interrupt::NONE).unwrap();
                let expected = super::super::dilate(&shape, radius);
                assert_eq!(got.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.v.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn private_refusal_and_inner_pass_cancellation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let source = Map::new(64, 64, 0.3);
        let before = source.v.clone();
        assert!(matches!(dilate(&source, 4.0, 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        let ticks = AtomicUsize::new(0);
        let cancelled = || ticks.fetch_add(1, Ordering::Relaxed) >= 40;
        assert!(matches!(dilate(&source, 4.0, 64 << 20, &Interrupt::cancel_only(&cancelled)), Err(E::Cancelled)));
        assert_eq!(source.v, before);
    }
}
