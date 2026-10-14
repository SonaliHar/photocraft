//! Bounded form of the existing Grow/Similar RGBA min/max and flood equations.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
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
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)
}
pub fn requirements(width: usize, height: usize, contiguous: bool) -> Result<Requirements, RasterInputError> {
    if width == 0 || height == 0 || width > 300000 || height > 300000 {
        return Err(RasterInputError::InvalidBounds);
    }
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)? as u64;
    let input_bytes = n.checked_mul(20).ok_or(RasterInputError::LimitExceeded)?;
    let output_bytes = n;
    let scratch_bytes = if contiguous { n.checked_mul(std::mem::size_of::<usize>() as u64).ok_or(RasterInputError::LimitExceeded)? } else { 0 };
    let peak_resident_bytes = input_bytes
        .checked_add(output_bytes)
        .and_then(|n| n.checked_add(scratch_bytes))
        .and_then(|n| n.checked_add(4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    let work_units = n.checked_mul(if contiguous { 96 } else { 48 }).ok_or(RasterInputError::LimitExceeded)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES || work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(Requirements { input_bytes, output_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
fn reserve<T>(v: &mut Vec<T>, n: usize) -> Result<(), RasterInputError> {
    v.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
    if v.capacity() > n.saturating_add(16) {
        return Err(RasterInputError::AllocationFailed);
    }
    Ok(())
}
pub fn grow_with_interrupt(
    px: &[[f32; 4]],
    mask: &[f32],
    width: usize,
    height: usize,
    tolerance: f32,
    contiguous: bool,
    limits: Limits,
    ctl: &Interrupt<'_>,
) -> Result<Vec<u8>, RasterInputError> {
    poll(ctl)?;
    let req = requirements(width, height, contiguous)?;
    if req.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || req.work_units > limits.work_units.min(MAX_WORK) {
        return Err(RasterInputError::LimitExceeded);
    }
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)?;
    if px.len() != n || mask.len() != n {
        return Err(RasterInputError::LengthMismatch);
    }
    if !tolerance.is_finite() {
        return Err(RasterInputError::InvalidSample);
    }
    let tol = tolerance / 255.;
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for (i, (q, v)) in px.iter().zip(mask).enumerate() {
        if i.is_multiple_of(1024) {
            poll(ctl)?;
        }
        if !v.is_finite() || q.iter().any(|v| !v.is_finite()) {
            return Err(RasterInputError::InvalidSample);
        }
        if *v >= 0.5 {
            for ((lo, hi), value) in lo.iter_mut().zip(hi.iter_mut()).zip(q) {
                *lo = lo.min(*value);
                *hi = hi.max(*value);
            }
        }
    }
    if lo.first().zip(hi.first()).is_none_or(|(lo, hi)| lo > hi) {
        return Err(RasterInputError::InvalidSample);
    }
    let inside = |q: &[f32; 4]| q.iter().zip(lo.iter().zip(hi.iter())).all(|(q, (lo, hi))| *q >= *lo - tol - 1e-6 && *q <= *hi + tol + 1e-6);
    let mut out = Vec::new();
    reserve(&mut out, n)?;
    let mut stack = Vec::new();
    if contiguous {
        reserve(&mut stack, n)?;
    }
    let mut enqueued = 0usize;
    for (i, v) in mask.iter().enumerate() {
        if i.is_multiple_of(1024) {
            poll(ctl)?;
        }
        let seed = *v >= 0.5;
        out.push(if seed { 255 } else { 0 });
        if contiguous && seed {
            stack.push(i);
            enqueued += 1;
        }
    }
    if contiguous {
        while let Some(i) = stack.pop() {
            poll(ctl)?;
            if i >= n {
                return Err(RasterInputError::LengthMismatch);
            }
            let (x, y) = (i % width, i / width);
            let neighbors = [(x > 0).then(|| i - 1), (x + 1 < width).then(|| i + 1), (y > 0).then(|| i - width), (y + 1 < height).then(|| i + width)];
            for j in neighbors.into_iter().flatten() {
                poll(ctl)?;
                if *out.get(j).ok_or(RasterInputError::LengthMismatch)? == 0 && inside(px.get(j).ok_or(RasterInputError::LengthMismatch)?) {
                    *out.get_mut(j).ok_or(RasterInputError::LengthMismatch)? = 255;
                    enqueued = enqueued.checked_add(1).filter(|n| *n <= px.len()).ok_or(RasterInputError::LimitExceeded)?;
                    stack.push(j);
                }
            }
        }
    } else {
        for (i, (out, q)) in out.iter_mut().zip(px).enumerate() {
            if i.is_multiple_of(1024) {
                poll(ctl)?;
            }
            if inside(q) {
                *out = 255;
            }
        }
    }
    poll(ctl)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seed_threshold_tolerance_and_contiguous_disconnected_regions_match_equations() {
        let px = [[0.2; 4], [0.2; 4], [0.9; 4], [0.2; 4]];
        let mask = [0.5, 0.49, 0., 0.];
        assert_eq!(grow_with_interrupt(&px, &mask, 4, 1, 0., true, Limits::default(), &Interrupt::NONE).unwrap(), [255, 255, 0, 0]);
        assert_eq!(grow_with_interrupt(&px, &mask, 4, 1, 0., false, Limits::default(), &Interrupt::NONE).unwrap(), [255, 255, 0, 255]);
        assert_eq!(grow_with_interrupt(&px, &mask, 4, 1, 255., true, Limits::default(), &Interrupt::NONE).unwrap(), [255; 4]);
    }
    #[test]
    fn malformed_planes_and_late_cancellation_return_no_partial_mask() {
        assert!(grow_with_interrupt(&[[0.; 4]], &[], 1, 1, 32., true, Limits::default(), &Interrupt::NONE).is_err());
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 20;
        let px = vec![[0.2; 4]; 4096];
        let mut mask = vec![0.; 4096];
        mask[0] = 1.;
        assert_eq!(grow_with_interrupt(&px, &mask, 64, 64, 32., true, Limits::default(), &Interrupt::cancel_only(&stop)), Err(RasterInputError::Cancelled));
    }
}
