//! Exact upstream sampled-tip pyramid with aggregate admission before allocation.
//! Arc publication remains standard-library managed; pixel/level Vec storage is
//! reserved fallibly and its actual capacity is charged for the complete pyramid.
use super::{StrokeError, StrokeLimits};
use crate::tile::{GrayTile, Mips};
use photocraft_raster::Interrupt;
use std::mem::size_of;
type Result<T> = std::result::Result<T, StrokeError>;
#[derive(Clone, Copy, Debug)]
pub struct SampledTipRequirements {
    pub retained_bytes: u64,
    pub staged_bytes: u64,
    pub work_units: u64,
    pub levels: usize,
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| StrokeError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(StrokeError::Limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(StrokeError::Limit)
}
fn limits(limits: StrokeLimits) -> Result<()> {
    let default = StrokeLimits::default();
    if limits.resident_bytes > default.resident_bytes || limits.work_units > default.work_units { Err(StrokeError::Limit) } else { Ok(()) }
}
impl Mips {
    /// No sample scan or allocation: exact pyramid geometry and conservative work.
    pub fn controlled_requirements(tile: &GrayTile, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<SampledTipRequirements> {
        poll(control)?;
        self::limits(limits)?;
        let mut w = usize::try_from(tile.width).map_err(|_| StrokeError::Limit)?;
        let mut h = usize::try_from(tile.height).map_err(|_| StrokeError::Limit)?;
        if w == 0 || h == 0 || w.checked_mul(h) != Some(tile.data.len()) {
            return Err(StrokeError::InvalidInput("sampled tip dimensions"));
        }
        let retained_bytes = add(mul(tile.data.len() as u64, size_of::<u16>() as u64)?, 32)?;
        if retained_bytes > limits.resident_bytes {
            return Err(StrokeError::Limit);
        }
        let mut pixels = 0u64;
        let mut work = 0u64;
        let mut levels = 0usize;
        loop {
            poll(control)?;
            let count = w.checked_mul(h).ok_or(StrokeError::Limit)?;
            pixels = add(pixels, count as u64)?;
            work = add(work, mul(count as u64, if levels == 0 { 2 } else { 12 })?)?;
            levels = levels.checked_add(1).filter(|n| *n <= 33).ok_or(StrokeError::Limit)?;
            if w <= 1 && h <= 1 {
                break;
            }
            w = w.div_ceil(2);
            h = h.div_ceil(2);
        }
        work = add(work, levels as u64)?;
        let staged_bytes = add(mul(pixels, size_of::<f32>() as u64)?, mul(levels as u64, size_of::<(usize, usize, Vec<f32>)>() as u64)?)?;
        if add(retained_bytes, staged_bytes)? > limits.resident_bytes || work > limits.work_units {
            return Err(StrokeError::Limit);
        }
        Ok(SampledTipRequirements { retained_bytes, staged_bytes, work_units: work, levels })
    }
    pub fn try_new_with_interrupt(tile: &GrayTile, limits: StrokeLimits, control: &Interrupt<'_>) -> Result<Self> {
        let req = Self::controlled_requirements(tile, limits, control)?;
        let mut remaining = limits.resident_bytes.checked_sub(req.retained_bytes).ok_or(StrokeError::Limit)?;
        let mut levels: Vec<(usize, usize, Vec<f32>)> = Vec::new();
        let descriptor_bytes = mul(req.levels as u64, size_of::<(usize, usize, Vec<f32>)>() as u64)?;
        if descriptor_bytes > remaining {
            return Err(StrokeError::Limit);
        }
        levels.try_reserve_exact(req.levels).map_err(|_| StrokeError::Allocation)?;
        remaining = remaining.checked_sub(mul(levels.capacity() as u64, size_of::<(usize, usize, Vec<f32>)>() as u64)?).ok_or(StrokeError::Limit)?;
        let reserve = |count: usize, remaining: &mut u64| -> Result<Vec<f32>> {
            poll(control)?;
            let bytes = mul(count as u64, size_of::<f32>() as u64)?;
            if bytes > *remaining {
                return Err(StrokeError::Limit);
            }
            let mut values = Vec::new();
            values.try_reserve_exact(count).map_err(|_| StrokeError::Allocation)?;
            *remaining = remaining.checked_sub(mul(values.capacity() as u64, size_of::<f32>() as u64)?).ok_or(StrokeError::Limit)?;
            Ok(values)
        };
        let mut first = reserve(tile.data.len(), &mut remaining)?;
        for chunk in tile.data.chunks(256) {
            poll(control)?;
            for value in chunk {
                first.push(f32::from(*value) / 65535.0);
            }
        }
        levels.push((tile.width as usize, tile.height as usize, first));
        while let Some((w, h, values)) = levels.last() {
            poll(control)?;
            if *w <= 1 && *h <= 1 {
                break;
            }
            let (nw, nh) = (w.div_ceil(2), h.div_ceil(2));
            let mut next = reserve(nw.checked_mul(nh).ok_or(StrokeError::Limit)?, &mut remaining)?;
            for y in 0..nh {
                poll(control)?;
                for x in 0..nw {
                    if x.is_multiple_of(256) {
                        poll(control)?;
                    }
                    let (mut sum, mut count) = (0.0, 0.0);
                    // Identical upstream dy/dx order, f32 sum and division.
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = x.checked_mul(2).and_then(|v| v.checked_add(dx)).ok_or(StrokeError::Limit)?;
                            let sy = y.checked_mul(2).and_then(|v| v.checked_add(dy)).ok_or(StrokeError::Limit)?;
                            if sx < *w && sy < *h {
                                let index = sy.checked_mul(*w).and_then(|v| v.checked_add(sx)).ok_or(StrokeError::Limit)?;
                                sum += *values.get(index).ok_or(StrokeError::InvalidInput("sampled mip index"))?;
                                count += 1.0;
                            }
                        }
                    }
                    next.push(sum / count);
                }
            }
            if levels.len() >= levels.capacity() {
                return Err(StrokeError::Limit);
            }
            levels.push((nw, nh, next));
        }
        poll(control)?;
        Ok(Self { levels })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_and_thin_mips_keep_upstream_reduction_words() {
        for (w, h) in [(1u32, 1u32), (1, 257), (257, 1), (5, 7), (256, 129)] {
            let tile = GrayTile { width: w, height: h, data: (0..w * h).map(|i| i.wrapping_mul(7919) as u16).collect::<Vec<_>>().into() };
            let baseline = Mips::new(&tile);
            let current = Mips::try_new_with_interrupt(&tile, StrokeLimits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(baseline.levels.len(), current.levels.len());
            for (a, b) in baseline.levels.iter().zip(&current.levels) {
                assert_eq!((a.0, a.1), (b.0, b.1));
                assert_eq!(a.2.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), b.2.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn aggregate_pyramid_and_late_cancel_refuse_without_result() {
        let tile = GrayTile { width: 257, height: 129, data: vec![12345; 257 * 129].into() };
        let req = Mips::controlled_requirements(&tile, StrokeLimits::default(), &Interrupt::NONE).unwrap();
        let small = StrokeLimits { resident_bytes: req.retained_bytes + req.staged_bytes - 1, ..StrokeLimits::default() };
        assert!(matches!(Mips::try_new_with_interrupt(&tile, small, &Interrupt::NONE), Err(StrokeError::Limit)));
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let stop = || {
            polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            polls.load(std::sync::atomic::Ordering::Relaxed) > 40
        };
        assert!(matches!(Mips::try_new_with_interrupt(&tile, StrokeLimits::default(), &Interrupt::cancel_only(&stop)), Err(StrokeError::Cancelled)));
    }
}
