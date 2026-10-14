//! Exact legacy color guided-filter companion with bounded cooperative preparation.
use super::{MattingLimits, RgbImage, box_mean_with_interrupt};
use photocraft_raster::{Interrupt, RasterInputError as E};
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn value(v: &[f32], i: usize) -> Result<f32, E> {
    v.get(i).copied().ok_or(E::LengthMismatch)
}
fn allocated(n: usize, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if v.capacity() > n {
        return Err(E::LimitExceeded);
    }
    while v.len() < n {
        poll(c)?;
        v.resize(v.len().saturating_add(4096).min(n), 0.0);
    }
    Ok(v)
}
fn mapped(n: usize, c: &Interrupt<'_>, mut f: impl FnMut(usize) -> Result<f32, E>) -> Result<Vec<f32>, E> {
    let mut v = allocated(n, c)?;
    for (i, o) in v.iter_mut().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *o = f(i)?;
        if !o.is_finite() {
            return Err(E::InvalidSample);
        }
    }
    poll(c)?;
    Ok(v)
}
pub fn guided_filter_color_with_interrupt(guide: &RgbImage, p: &[f32], r: usize, eps: f32, limits: MattingLimits, ctl: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    let (w, h) = (guide.w, guide.h);
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if limits.resident_bytes > photocraft_raster::MAX_CHECKED_RASTER_BYTES
        || limits.work_units > 8_000_000_000
        || n == 0
        || w > 300000
        || h > 300000
        || r > 300000
        || guide.px.len() != n
        || p.len() != n
        || !eps.is_finite()
        || eps <= 0.0
    {
        return Err(E::InvalidBounds);
    }
    if (n as u64).checked_mul(192).and_then(|n| n.checked_add(65536)).is_none_or(|n| n > limits.resident_bytes)
        || (n as u64).checked_mul(1024).is_none_or(|n| n > limits.work_units)
    {
        return Err(E::LimitExceeded);
    }
    for c in guide.px.chunks(1024) {
        poll(ctl)?;
        if c.iter().flatten().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    for c in p.chunks(4096) {
        poll(ctl)?;
        if c.iter().any(|v| !v.is_finite()) {
            return Err(E::InvalidSample);
        }
    }
    let ch = |c: usize| -> Result<Vec<f32>, E> { mapped(n, ctl, |i| guide.px.get(i).and_then(|q| q.get(c)).copied().ok_or(E::LengthMismatch)) };
    let chans = [ch(0)?, ch(1)?, ch(2)?];
    let mean = |v: &[f32]| box_mean_with_interrupt(v, w, h, r, limits, ctl);
    let product = |a: &[f32], b: &[f32]| -> Result<Vec<f32>, E> {
        let v = mapped(n, ctl, |i| Ok(value(a, i)? * value(b, i)?))?;
        mean(&v)
    };
    let m = [mean(chans.get(0).ok_or(E::LengthMismatch)?)?, mean(chans.get(1).ok_or(E::LengthMismatch)?)?, mean(chans.get(2).ok_or(E::LengthMismatch)?)?];
    let mp = mean(p)?;
    let mip = [
        product(chans.get(0).ok_or(E::LengthMismatch)?, p)?,
        product(chans.get(1).ok_or(E::LengthMismatch)?, p)?,
        product(chans.get(2).ok_or(E::LengthMismatch)?, p)?,
    ];
    let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let mut mii: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::new());
    for (v, (a, b)) in mii.iter_mut().zip(pairs) {
        *v = product(chans.get(a).ok_or(E::LengthMismatch)?, chans.get(b).ok_or(E::LengthMismatch)?)?;
    }
    let mut av = [allocated(n, ctl)?, allocated(n, ctl)?, allocated(n, ctl)?];
    let mut bv = allocated(n, ctl)?;
    for i in 0..n {
        poll(ctl)?;
        let mu = [
            value(m.get(0).ok_or(E::LengthMismatch)?, i)? as f64,
            value(m.get(1).ok_or(E::LengthMismatch)?, i)? as f64,
            value(m.get(2).ok_or(E::LengthMismatch)?, i)? as f64,
        ];
        let s = |k: usize, a: usize, b: usize| -> Result<f64, E> {
            Ok(value(mii.get(k).ok_or(E::LengthMismatch)?, i)? as f64 - mu.get(a).ok_or(E::LengthMismatch)? * mu.get(b).ok_or(E::LengthMismatch)?)
        };
        let e = eps as f64;
        let (rr, rg, rb, gg, gb, bb) = (s(0, 0, 0)? + e, s(1, 0, 1)?, s(2, 0, 2)?, s(3, 1, 1)? + e, s(4, 1, 2)?, s(5, 2, 2)? + e);
        let cov = [
            value(mip.get(0).ok_or(E::LengthMismatch)?, i)? as f64 - *mu.get(0).ok_or(E::LengthMismatch)? * value(&mp, i)? as f64,
            value(mip.get(1).ok_or(E::LengthMismatch)?, i)? as f64 - *mu.get(1).ok_or(E::LengthMismatch)? * value(&mp, i)? as f64,
            value(mip.get(2).ok_or(E::LengthMismatch)?, i)? as f64 - *mu.get(2).ok_or(E::LengthMismatch)? * value(&mp, i)? as f64,
        ];
        // Inverse of the symmetric 3×3 matrix.
        let i00 = gg * bb - gb * gb;
        let i01 = gb * rb - rg * bb;
        let i02 = rg * gb - gg * rb;
        let i11 = rr * bb - rb * rb;
        let i12 = rb * rg - rr * gb;
        let i22 = rr * gg - rg * rg;
        let det = rr * i00 + rg * i01 + rb * i02;
        let (a0, a1, a2) = if det.abs() < 1e-30 {
            (0.0, 0.0, 0.0)
        } else {
            (
                (i00 * *cov.get(0).ok_or(E::LengthMismatch)? + i01 * *cov.get(1).ok_or(E::LengthMismatch)? + i02 * *cov.get(2).ok_or(E::LengthMismatch)?) / det,
                (i01 * *cov.get(0).ok_or(E::LengthMismatch)? + i11 * *cov.get(1).ok_or(E::LengthMismatch)? + i12 * *cov.get(2).ok_or(E::LengthMismatch)?) / det,
                (i02 * *cov.get(0).ok_or(E::LengthMismatch)? + i12 * *cov.get(1).ok_or(E::LengthMismatch)? + i22 * *cov.get(2).ok_or(E::LengthMismatch)?) / det,
            )
        };
        *av.get_mut(0).and_then(|v| v.get_mut(i)).ok_or(E::LengthMismatch)? = a0 as f32;
        *av.get_mut(1).and_then(|v| v.get_mut(i)).ok_or(E::LengthMismatch)? = a1 as f32;
        *av.get_mut(2).and_then(|v| v.get_mut(i)).ok_or(E::LengthMismatch)? = a2 as f32;
        *bv.get_mut(i).ok_or(E::LengthMismatch)? = (value(&mp, i)? as f64
            - a0 * *mu.get(0).ok_or(E::LengthMismatch)?
            - a1 * *mu.get(1).ok_or(E::LengthMismatch)?
            - a2 * *mu.get(2).ok_or(E::LengthMismatch)?) as f32;
    }
    let ma = [mean(av.get(0).ok_or(E::LengthMismatch)?)?, mean(av.get(1).ok_or(E::LengthMismatch)?)?, mean(av.get(2).ok_or(E::LengthMismatch)?)?];
    let mb = mean(&bv)?;
    mapped(n, ctl, |i| {
        Ok(value(ma.get(0).ok_or(E::LengthMismatch)?, i)? * value(chans.get(0).ok_or(E::LengthMismatch)?, i)?
            + value(ma.get(1).ok_or(E::LengthMismatch)?, i)? * value(chans.get(1).ok_or(E::LengthMismatch)?, i)?
            + value(ma.get(2).ok_or(E::LengthMismatch)?, i)? * value(chans.get(2).ok_or(E::LengthMismatch)?, i)?
            + value(&mb, i)?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_guided_color_words_match_current_filter() {
        let img = RgbImage::from_fn(17, 11, |x, y| [(x % 7) as f32 / 7.0, (y % 5) as f32 / 5.0, ((x + y) % 13) as f32 / 13.0]);
        let p = img.px.iter().map(|v| v[0] * 0.3 + v[1] * 0.7).collect::<Vec<_>>();
        for r in [0, 2, 7] {
            let got = guided_filter_color_with_interrupt(&img, &p, r, 1e-2, MattingLimits::default(), &Interrupt::NONE).unwrap();
            let expected = crate::matting::guided_filter_color(&img, &p, r, 1e-2);
            assert_eq!(got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
        }
    }
}
