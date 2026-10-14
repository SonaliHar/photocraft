//! Controlled companions to current 8-connected foreground/4-connected hole cleanup.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as E};
#[derive(Clone, Copy, Debug)]
pub struct CleanupLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for CleanupLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 8_000_000_000 }
    }
}
fn poll(c: &Interrupt<'_>) -> Result<(), E> {
    c.check().map_err(|_| E::Cancelled)
}
fn filled<T: Clone>(n: usize, value: T, c: &Interrupt<'_>) -> Result<Vec<T>, E> {
    poll(c)?;
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
fn components(mask: &[bool], w: usize, h: usize, eight: bool, c: &Interrupt<'_>) -> Result<(Vec<u32>, Vec<usize>), E> {
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if mask.len() != n {
        return Err(E::LengthMismatch);
    }
    let mut labels = filled(n, 0u32, c)?;
    let mut sizes = filled(n.checked_add(1).ok_or(E::LimitExceeded)?, 0usize, c)?;
    let mut stack = Vec::new();
    stack.try_reserve_exact(n).map_err(|_| E::AllocationFailed)?;
    if stack.capacity() > n.saturating_add(16) {
        return Err(E::AllocationFailed);
    }
    let mut next = 1usize;
    for seed in 0..n {
        if seed.is_multiple_of(1024) {
            poll(c)?;
        }
        if !*mask.get(seed).ok_or(E::LengthMismatch)? || *labels.get(seed).ok_or(E::LengthMismatch)? != 0 {
            continue;
        }
        let label = u32::try_from(next).map_err(|_| E::LimitExceeded)?;
        next = next.checked_add(1).ok_or(E::LimitExceeded)?;
        *labels.get_mut(seed).ok_or(E::LengthMismatch)? = label;
        stack.push(seed);
        while let Some(i) = stack.pop() {
            poll(c)?;
            let size = sizes.get_mut(label as usize).ok_or(E::LengthMismatch)?;
            *size = size.checked_add(1).ok_or(E::LimitExceeded)?;
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| -> Result<(), E> {
                poll(c)?;
                if *mask.get(j).ok_or(E::LengthMismatch)? && *labels.get(j).ok_or(E::LengthMismatch)? == 0 {
                    *labels.get_mut(j).ok_or(E::LengthMismatch)? = label;
                    stack.push(j);
                }
                Ok(())
            };
            if eight {
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64 {
                            visit(ny as usize * w + nx as usize)?;
                        }
                    }
                }
            } else {
                for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                    visit(j)?;
                }
            }
        }
    }
    sizes.truncate(next);
    poll(c)?;
    Ok((labels, sizes))
}
pub fn clean_mask_with_interrupt(mask: &[bool], w: usize, h: usize, frac: f32, hole_frac: f32, l: CleanupLimits, c: &Interrupt<'_>) -> Result<Vec<bool>, E> {
    poll(c)?;
    let n = w.checked_mul(h).ok_or(E::LimitExceeded)?;
    if w == 0 || h == 0 || w > 300000 || h > 300000 || n >= u32::MAX as usize {
        return Err(E::InvalidBounds);
    }
    if mask.len() != n {
        return Err(E::LengthMismatch);
    }
    if !frac.is_finite() || !hole_frac.is_finite() || frac < 0. || hole_frac < 0. {
        return Err(E::InvalidSample);
    }
    let bytes = (n as u64).checked_mul(80).and_then(|n| n.checked_add(4096)).ok_or(E::LimitExceeded)?;
    let work = (n as u64).checked_mul(192).ok_or(E::LimitExceeded)?;
    if bytes > l.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > l.work_units.min(8_000_000_000) {
        return Err(E::LimitExceeded);
    }
    let (labels, sizes) = components(mask, w, h, true, c)?;
    let mut largest = 0usize;
    for (i, size) in sizes.iter().skip(1).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        largest = largest.max(*size);
    }
    let mut out = filled(n, false, c)?;
    if largest == 0 {
        return Ok(out);
    }
    let min = (largest as f32 * frac).ceil() as usize;
    let mut area = 0usize;
    for (i, label) in labels.iter().enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        let keep = *label != 0 && *sizes.get(*label as usize).ok_or(E::LengthMismatch)? >= min;
        *out.get_mut(i).ok_or(E::LengthMismatch)? = keep;
        area += usize::from(keep);
    }
    let max_hole = (area as f32 * hole_frac) as usize;
    let mut inv = filled(n, false, c)?;
    for (i, (v, o)) in inv.iter_mut().zip(&out).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        *v = !*o;
    }
    let (holes, hole_sizes) = components(&inv, w, h, false, c)?;
    let mut touches = filled(hole_sizes.len(), false, c)?;
    for x in 0..w {
        if x.is_multiple_of(1024) {
            poll(c)?;
        }
        for at in [x, (h - 1) * w + x] {
            let label = *holes.get(at).ok_or(E::LengthMismatch)?;
            *touches.get_mut(label as usize).ok_or(E::LengthMismatch)? = true;
        }
    }
    for y in 0..h {
        if y.is_multiple_of(1024) {
            poll(c)?;
        }
        for at in [y * w, y * w + w - 1] {
            let label = *holes.get(at).ok_or(E::LengthMismatch)?;
            *touches.get_mut(label as usize).ok_or(E::LengthMismatch)? = true;
        }
    }
    for (i, (o, label)) in out.iter_mut().zip(&holes).enumerate() {
        if i.is_multiple_of(1024) {
            poll(c)?;
        }
        if *label != 0 && !*touches.get(*label as usize).ok_or(E::LengthMismatch)? && *hole_sizes.get(*label as usize).ok_or(E::LengthMismatch)? <= max_hole {
            *o = true;
        }
    }
    poll(c)?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_diagonal_component_and_hole_cleanup_match() {
        let mut mask = vec![false; 20 * 17];
        for y in 1..16 {
            for x in 1..19 {
                mask[y * 20 + x] = true;
            }
        }
        mask[5 * 20 + 5] = false;
        mask[6 * 20 + 6] = false;
        mask[0] = true;
        let expected = super::super::clean_mask(&mask, 20, 17, 0.002, 0.01);
        assert_eq!(clean_mask_with_interrupt(&mask, 20, 17, 0.002, 0.01, CleanupLimits::default(), &Interrupt::NONE).unwrap(), expected);
        assert_eq!(clean_mask_with_interrupt(&mask, 20, 17, 0.002, 0.01, CleanupLimits::default(), &Interrupt::cancel_only(&|| true)), Err(E::Cancelled));
    }
}
