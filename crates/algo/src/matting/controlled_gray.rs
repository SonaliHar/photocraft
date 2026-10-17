//! Checked grayscale matting companions; original prefix and f32 equation order.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
#[derive(Clone, Copy, Debug)]
pub struct MattingLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for MattingLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
pub(crate) fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
pub(crate) fn filled<T: Clone>(n: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
    let bytes = n.checked_mul(std::mem::size_of::<T>()).ok_or(E::LimitExceeded)?;
    if bytes as u64 > MAX_CHECKED_RASTER_BYTES {
        return Err(E::LimitExceeded);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if out.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    while out.len() < n {
        poll(c)?;
        out.resize(out.len().saturating_add(1024).min(n), value.clone());
    }
    Ok(out)
}
fn shape(w: usize, h: usize, r: usize) -> Result<usize, E> {
    if w == 0 || h == 0 || w > 300000 || h > 300000 || r > 300000 {
        return Err(E::InvalidBounds);
    }
    w.checked_mul(h).ok_or(E::LimitExceeded)
}
fn admit(w: usize, h: usize, r: usize, guided: bool, l: MattingLimits) -> Result<usize, E> {
    let n = shape(w, h, r)?;
    let bytes = (n as u64)
        .checked_mul(if guided { 192 } else { 24 })
        .and_then(|v| v.checked_add((w.max(h) as u64).checked_mul(64)?))
        .and_then(|v| v.checked_add(8192))
        .ok_or(E::LimitExceeded)?;
    let work = (n as u64).checked_mul(if guided { 192 } else { 24 }).and_then(|v| v.checked_add((w.max(h) as u64).checked_mul(16)?)).ok_or(E::LimitExceeded)?;
    if bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > l.work_units.min(8_000_000_000) {
        return Err(E::LimitExceeded);
    }
    Ok(n)
}
fn validate(src: &[f32], n: usize, c: &Interrupt<'_>) -> Result<(), E> {
    if src.len() != n {
        return Err(E::LengthMismatch);
    }
    for (i, v) in src.iter().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if !v.is_finite() {
            return Err(E::InvalidSample);
        }
    }
    poll(c)
}
pub fn box_mean_with_interrupt(src: &[f32], w: usize, h: usize, r: usize, l: MattingLimits, c: &Interrupt<'_>) -> Result<Vec<f32>, E> {
    poll(c)?;
    let n = admit(w, h, r, false, l)?;
    validate(src, n, c)?;
    let mut tmp = filled(n, 0f32, c)?;
    let mut pre = filled(w.max(h).checked_add(1).ok_or(E::LimitExceeded)?, 0f64, c)?;
    for y in 0..h {
        poll(c)?;
        let row = src.get(y * w..(y + 1) * w).ok_or(E::LengthMismatch)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let value = *pre.get(x).ok_or(E::LengthMismatch)? + f64::from(*row.get(x).ok_or(E::LengthMismatch)?);
            *pre.get_mut(x + 1).ok_or(E::LengthMismatch)? = value;
        }
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let (lo, hi) = (x.saturating_sub(r), (x + r + 1).min(w));
            let value = ((*pre.get(hi).ok_or(E::LengthMismatch)? - *pre.get(lo).ok_or(E::LengthMismatch)?) / (hi - lo) as f64) as f32;
            *tmp.get_mut(y * w + x).ok_or(E::LengthMismatch)? = value;
        }
    }
    let mut out = filled(n, 0f32, c)?;
    let mut col = filled(h.checked_add(1).and_then(|n| n.checked_mul(w)).ok_or(E::LimitExceeded)?, 0f64, c)?;
    for y in 0..h {
        poll(c)?;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let value = *col.get(y * w + x).ok_or(E::LengthMismatch)? + f64::from(*tmp.get(y * w + x).ok_or(E::LengthMismatch)?);
            *col.get_mut((y + 1) * w + x).ok_or(E::LengthMismatch)? = value;
        }
    }
    for y in 0..h {
        poll(c)?;
        let (lo, hi) = (y.saturating_sub(r), (y + r + 1).min(h));
        let count = (hi - lo) as f64;
        for x in 0..w {
            if x.is_multiple_of(1024) {
                poll(c)?;
            }
            let value = ((*col.get(hi * w + x).ok_or(E::LengthMismatch)? - *col.get(lo * w + x).ok_or(E::LengthMismatch)?) / count) as f32;
            if !value.is_finite() {
                return Err(E::InvalidSample);
            }
            *out.get_mut(y * w + x).ok_or(E::LengthMismatch)? = value;
        }
    }
    poll(c)?;
    Ok(out)
}
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn guided_filter_gray_with_interrupt(
    guide: &[f32],
    p: &[f32],
    w: usize,
    h: usize,
    r: usize,
    eps: f32,
    l: MattingLimits,
    c: &Interrupt<'_>,
) -> Result<Vec<f32>, E> {
    poll(c)?;
    let n = admit(w, h, r, true, l)?;
    if !eps.is_finite() || eps <= 0. {
        return Err(E::InvalidSample);
    }
    validate(guide, n, c)?;
    validate(p, n, c)?;
    let mi = box_mean_with_interrupt(guide, w, h, r, l, c)?;
    let mp = box_mean_with_interrupt(p, w, h, r, l, c)?;
    let mut ip = filled(n, 0f32, c)?;
    let mut ii = filled(n, 0f32, c)?;
    for i in 0..n {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let a = *guide.get(i).ok_or(E::LengthMismatch)?;
        let b = *p.get(i).ok_or(E::LengthMismatch)?;
        *ip.get_mut(i).ok_or(E::LengthMismatch)? = a * b;
        *ii.get_mut(i).ok_or(E::LengthMismatch)? = a * a;
    }
    let mip = box_mean_with_interrupt(&ip, w, h, r, l, c)?;
    let mii = box_mean_with_interrupt(&ii, w, h, r, l, c)?;
    let mut a = filled(n, 0f32, c)?;
    let mut b = filled(n, 0f32, c)?;
    for i in 0..n {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let mi = *mi.get(i).ok_or(E::LengthMismatch)?;
        let mp = *mp.get(i).ok_or(E::LengthMismatch)?;
        let var = *mii.get(i).ok_or(E::LengthMismatch)? - mi * mi;
        let cov = *mip.get(i).ok_or(E::LengthMismatch)? - mi * mp;
        let slope = cov / (var + eps);
        let intercept = mp - slope * mi;
        if !slope.is_finite() || !intercept.is_finite() {
            return Err(E::InvalidSample);
        }
        *a.get_mut(i).ok_or(E::LengthMismatch)? = slope;
        *b.get_mut(i).ok_or(E::LengthMismatch)? = intercept;
    }
    let ma = box_mean_with_interrupt(&a, w, h, r, l, c)?;
    let mb = box_mean_with_interrupt(&b, w, h, r, l, c)?;
    let mut output = filled(n, 0f32, c)?;
    for i in 0..n {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let value = *ma.get(i).ok_or(E::LengthMismatch)? * *guide.get(i).ok_or(E::LengthMismatch)? + *mb.get(i).ok_or(E::LengthMismatch)?;
        if !value.is_finite() {
            return Err(E::InvalidSample);
        }
        *output.get_mut(i).ok_or(E::LengthMismatch)? = value;
    }
    poll(c)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_box_and_guided_words_match() {
        let src = (0..117).map(|i| (i % 13) as f32 / 13.).collect::<Vec<_>>();
        let mask = src.iter().map(|v| v * 0.75).collect::<Vec<_>>();
        for r in [0, 1, 7] {
            assert_eq!(box_mean_with_interrupt(&src, 13, 9, r, MattingLimits::default(), &Interrupt::NONE).unwrap(), crate::matting::box_mean(&src, 13, 9, r));
            assert_eq!(
                guided_filter_gray_with_interrupt(&src, &mask, 13, 9, r, 0.004, MattingLimits::default(), &Interrupt::NONE).unwrap(),
                crate::matting::guided_filter_gray(&src, &mask, 13, 9, r, 0.004)
            );
        }
    }
}
