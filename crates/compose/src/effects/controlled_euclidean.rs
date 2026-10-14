//! Exact effects f32 two-pass EDT, including nearest-coordinate tie policy.
use super::{INSIDE_EPS, Map};
use photocraft_raster::{Interrupt, RasterInputError as E};
const INF: f32 = 1e20;
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Copy>(n: usize, v: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() != n {
        return Err(E::LimitExceeded);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(4096).min(n), v);
    }
    Ok(out)
}
pub(super) fn scratch_bytes(w: usize, h: usize) -> Result<usize, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if n == 0 {
        return Ok(0);
    }
    if n > u32::MAX as usize {
        return Err(E::LimitExceeded);
    }
    let line = w.max(h);
    line.saturating_sub(1).checked_mul(line.saturating_sub(1)).ok_or(E::LimitExceeded)?;
    // seed + four full planes + returned coverage map; one reused serial scratch.
    n.checked_mul(21)
        .and_then(|n| line.checked_mul(12 + 2 * std::mem::size_of::<usize>()).and_then(|v| n.checked_add(v)))
        .and_then(|n| n.checked_add(4))
        .ok_or(E::LimitExceeded)
}
pub(super) fn work_units(w: usize, h: usize) -> Result<u64, E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    scratch_bytes(w, h)?;
    let work = (n as u64).checked_mul(256).ok_or(E::LimitExceeded)?;
    if work > 8_000_000_000 {
        return Err(E::LimitExceeded);
    }
    Ok(work)
}
struct Scratch {
    f: Vec<f32>,
    o: Vec<f32>,
    v: Vec<usize>,
    z: Vec<f32>,
    near: Vec<usize>,
}
impl Scratch {
    fn new(n: usize, c: &Interrupt<'_>) -> Result<Self, E> {
        Ok(Self {
            f: filled(n, 0.0, c)?,
            o: filled(n, 0.0, c)?,
            v: filled(n, 0, c)?,
            z: filled(n.checked_add(1).ok_or(E::LimitExceeded)?, 0.0, c)?,
            near: filled(n, 0, c)?,
        })
    }
}
fn dt1(t: &mut Scratch, n: usize, c: &Interrupt<'_>) -> Result<(), E> {
    poll(c)?;
    if n == 0 {
        return Ok(());
    }
    let mut k = 0usize;
    *t.v.get_mut(0).ok_or(E::LengthMismatch)? = 0;
    *t.z.get_mut(0).ok_or(E::LengthMismatch)? = f32::NEG_INFINITY;
    *t.z.get_mut(1).ok_or(E::LengthMismatch)? = f32::INFINITY;
    for q in 1..n {
        poll(c)?;
        loop {
            poll(c)?;
            let p = *t.v.get(k).ok_or(E::LengthMismatch)?;
            let qq = q.checked_mul(q).ok_or(E::LimitExceeded)?;
            let pp = p.checked_mul(p).ok_or(E::LimitExceeded)?;
            let s =
                ((*t.f.get(q).ok_or(E::LengthMismatch)? + qq as f32) - (*t.f.get(p).ok_or(E::LengthMismatch)? + pp as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
            let zk = *t.z.get(k).ok_or(E::LengthMismatch)?;
            if s <= zk && k > 0 {
                k -= 1;
                continue;
            }
            if s <= zk {
                *t.v.get_mut(0).ok_or(E::LengthMismatch)? = q;
                *t.z.get_mut(0).ok_or(E::LengthMismatch)? = f32::NEG_INFINITY;
                *t.z.get_mut(1).ok_or(E::LengthMismatch)? = f32::INFINITY;
                break;
            }
            k = k.checked_add(1).ok_or(E::LimitExceeded)?;
            *t.v.get_mut(k).ok_or(E::LengthMismatch)? = q;
            *t.z.get_mut(k).ok_or(E::LengthMismatch)? = s;
            *t.z.get_mut(k + 1).ok_or(E::LengthMismatch)? = f32::INFINITY;
            break;
        }
    }
    k = 0;
    for q in 0..n {
        poll(c)?;
        while *t.z.get(k + 1).ok_or(E::LengthMismatch)? < q as f32 {
            poll(c)?;
            k = k.checked_add(1).ok_or(E::LimitExceeded)?;
        }
        let p = *t.v.get(k).ok_or(E::LengthMismatch)?;
        let d = q as f32 - p as f32;
        *t.o.get_mut(q).ok_or(E::LengthMismatch)? = d * d + *t.f.get(p).ok_or(E::LengthMismatch)?;
        *t.near.get_mut(q).ok_or(E::LengthMismatch)? = p;
    }
    poll(c)
}
fn edt(inside: &[bool], w: usize, h: usize, c: &Interrupt<'_>) -> Result<(Vec<f32>, Vec<u32>), E> {
    poll(c)?;
    let total = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    let admitted = scratch_bytes(w, h)?;
    if admitted > 64 << 20 {
        return Err(E::LimitExceeded);
    }
    work_units(w, h)?;
    if inside.len() != total {
        return Err(E::LengthMismatch);
    }
    if total == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut gc = filled(total, 0.0f32, c)?;
    let mut nc = filled(total, 0u32, c)?;
    let mut t = Scratch::new(w.max(h), c)?;
    for x in 0..w {
        poll(c)?;
        for y in 0..h {
            if y.is_multiple_of(1024) {
                poll(c)?;
            }
            *t.f.get_mut(y).ok_or(E::LengthMismatch)? = if *inside.get(y * w + x).ok_or(E::LengthMismatch)? { 0.0 } else { INF };
        }
        dt1(&mut t, h, c)?;
        for y in 0..h {
            if y.is_multiple_of(1024) {
                poll(c)?;
            }
            *gc.get_mut(x * h + y).ok_or(E::LengthMismatch)? = *t.o.get(y).ok_or(E::LengthMismatch)?;
            *nc.get_mut(x * h + y).ok_or(E::LengthMismatch)? = u32::try_from(*t.near.get(y).ok_or(E::LengthMismatch)?).map_err(|_| E::LimitExceeded)?;
        }
    }
    let mut distance = filled(total, 0.0f32, c)?;
    let mut nearest = filled(total, 0u32, c)?;
    for y in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            *t.f.get_mut(x).ok_or(E::LengthMismatch)? = *gc.get(x * h + y).ok_or(E::LengthMismatch)?;
        }
        dt1(&mut t, w, c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            *distance.get_mut(y * w + x).ok_or(E::LengthMismatch)? = t.o.get(x).ok_or(E::LengthMismatch)?.sqrt();
            let px = *t.near.get(x).ok_or(E::LengthMismatch)?;
            let py = *nc.get(px * h + y).ok_or(E::LengthMismatch)? as usize;
            let index = py.checked_mul(w).and_then(|n| n.checked_add(px)).ok_or(E::LimitExceeded)?;
            *nearest.get_mut(y * w + x).ok_or(E::LengthMismatch)? = u32::try_from(index).map_err(|_| E::LimitExceeded)?;
        }
    }
    poll(c)?;
    Ok((distance, nearest))
}
pub(super) fn distance(shape: &Map, inner: bool, budget: usize, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    let peak = scratch_bytes(shape.w, shape.h)?;
    work_units(shape.w, shape.h)?;
    if peak > budget.min(64 << 20) {
        return Err(E::LimitExceeded);
    }
    let n = shape.w.checked_mul(shape.h).ok_or(E::LimitExceeded)?;
    if n != shape.v.len() {
        return Err(E::LengthMismatch);
    }
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut seeds = filled(n, false, c)?;
    for (i, (s, a)) in seeds.iter_mut().zip(&shape.v).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if !a.is_finite() {
            return Err(E::InvalidSample);
        }
        *s = if inner { *a <= INSIDE_EPS } else { *a > INSIDE_EPS };
    }
    let (d, near) = edt(&seeds, shape.w, shape.h, c)?;
    let mut out = filled(n, 0.0f32, c)?;
    for (i, o) in out.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let seed = *seeds.get(i).ok_or(E::LengthMismatch)?;
        let distance = *d.get(i).ok_or(E::LengthMismatch)?;
        *o = if seed {
            -0.5
        } else if inner {
            distance - 0.5
        } else {
            let at = *near.get(i).ok_or(E::LengthMismatch)? as usize;
            distance - 0.5 + (1.0 - shape.v.get(at).ok_or(E::LengthMismatch)?.min(1.0))
        };
    }
    poll(c)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_f32_distances_and_nearest_ties_match_effects() {
        for (w, h) in [(0, 0), (1, 1), (1, 37), (41, 1), (19, 17)] {
            for mode in [0, 1, 2] {
                let input = (0..w * h)
                    .map(|i| match mode {
                        0 => false,
                        1 => true,
                        _ => i % 11 == 0,
                    })
                    .collect::<Vec<_>>();
                let expected = super::super::edt_nearest(&input, w, h);
                let got = edt(&input, w, h, &Interrupt::NONE).unwrap();
                assert_eq!(got.0.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.0.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
                assert_eq!(got.1, expected.1);
            }
        }
    }
    #[test]
    fn precise_inside_outside_subpixel_correction_and_failure() {
        let mut shape = Map::new(29, 23, 0.0);
        for (i, a) in shape.v.iter_mut().enumerate() {
            *a = ((i * 73) % 101) as f32 / 100.0;
        }
        for inner in [false, true] {
            let expected = if inner { super::super::dist_inside(&shape) } else { super::super::dist_outside(&shape) };
            assert_eq!(
                distance(&shape, inner, 64 << 20, &Interrupt::NONE).unwrap().iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
        assert!(matches!(distance(&shape, false, 1, &Interrupt::NONE), Err(E::LimitExceeded)));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ticks = AtomicUsize::new(0);
        let cancel = || ticks.fetch_add(1, Ordering::Relaxed) > 50;
        assert!(matches!(distance(&shape, false, 64 << 20, &Interrupt::cancel_only(&cancel)), Err(E::Cancelled)));
    }
    #[test]
    fn squared_coordinate_and_field_caps_refuse_before_allocation() {
        assert!(scratch_bytes(usize::MAX, 2).is_err());
        assert!(scratch_bytes(100000, 100000).is_err());
        #[cfg(target_pointer_width = "32")]
        assert!(scratch_bytes(65537, 1).is_err());
    }
}
