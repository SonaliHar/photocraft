//! Checked EDT-backed versions of the existing selection equations. This is
//! narrow f32 coverage math, not a native-wide/source-qualified transform.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
const NO_SEED: u32 = u32::MAX;
const EMPTY_DISTANCE: f32 = 1e10;
const BAND_ROWS: usize = 32;
const MAX_WORK: u64 = 1 << 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Expand,
    Contract,
    Border,
}
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
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub work_units: u64,
    pub peak_resident_bytes: u64,
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
pub fn requirements(width: usize, height: usize, operation: Operation, radius: f32) -> Result<Requirements, RasterInputError> {
    if width == 0 || height == 0 || u32::try_from(width).is_err() || u32::try_from(height).is_err() {
        return Err(RasterInputError::InvalidBounds);
    }
    if !radius.is_finite() || radius < 0.0 || (operation == Operation::Border && radius == 0.0) {
        return Err(RasterInputError::InvalidSample);
    }
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)? as u64;
    let input_bytes = mul(n, 4)?;
    let output_bytes = input_bytes;
    // Cumulative internal reservations, including both EDTs for Border and
    // Contract's inverted plane; no transient allocator/RSS refund is assumed.
    let planes = mul(n, if operation == Operation::Border { 32 } else { 24 })?;
    let band = mul(mul(width as u64, height.min(BAND_ROWS) as u64)?, 8)?;
    let envelopes = mul(width as u64, (std::mem::size_of::<usize>() as u64 + 8) * 2)?;
    let scratch_bytes = add(add(planes, band)?, add(envelopes, 65536)?)?;
    let peak_resident_bytes = add(add(input_bytes, scratch_bytes)?, output_bytes)?;
    let work_units = add(mul(n, if operation == Operation::Border { 128 } else { 96 })?, mul(width as u64, 128)?)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES || work_units > MAX_WORK {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(Requirements { input_bytes, scratch_bytes, output_bytes, work_units, peak_resident_bytes })
}
struct Ledger {
    bytes: u64,
    work: u64,
}
impl Ledger {
    fn tick(&mut self, ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
        poll(ctl)?;
        self.work = self.work.checked_sub(1).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn vector<T: Clone>(&mut self, n: usize, value: T, ctl: &Interrupt<'_>) -> Result<Vec<T>, RasterInputError> {
        poll(ctl)?;
        let requested = n.checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)? as u64;
        if requested > self.bytes {
            return Err(RasterInputError::LimitExceeded);
        }
        let mut result = Vec::new();
        result.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
        let actual = result.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)? as u64;
        self.bytes = self.bytes.checked_sub(actual).ok_or(RasterInputError::LimitExceeded)?;
        while result.len() < n {
            self.tick(ctl)?;
            let count = (n - result.len()).min(1024);
            self.work = self.work.checked_sub(count as u64).ok_or(RasterInputError::LimitExceeded)?;
            result.resize(result.len() + count, value.clone());
        }
        poll(ctl)?;
        Ok(result)
    }
}
/// Bounded companion of the existing binary distance transform, for callers
/// needing distances rather than Expand/Contract coverage. Same envelope/order.
pub fn edt_with_interrupt(inside:&[bool],width:usize,height:usize,limits:Limits,ctl:&Interrupt<'_>)->Result<Vec<f32>,RasterInputError>{
    poll(ctl)?;
    let r=requirements(width,height,Operation::Expand,0.0)?;
    if r.peak_resident_bytes>limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES)||r.work_units>limits.work_units.min(MAX_WORK){return Err(RasterInputError::LimitExceeded);}
    let mut ledger=Ledger{bytes:r.peak_resident_bytes,work:r.work_units};
    let result=edt(inside,width,height,&mut ledger,ctl)?;poll(ctl)?;Ok(result)
}

fn row_distance(
    costs: &[u32],
    output: &mut [f32],
    sites: &mut [usize],
    starts: &mut [f64],
    ledger: &mut Ledger,
    ctl: &Interrupt<'_>,
) -> Result<(), RasterInputError> {
    if costs.len() != output.len() || sites.len() != costs.len() || starts.len() != costs.len() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut count = 0usize;
    for (q, &distance) in costs.iter().enumerate() {
        ledger.tick(ctl)?;
        if distance == NO_SEED {
            continue;
        }
        let fq = f64::from(distance).powi(2);
        let mut start = f64::NEG_INFINITY;
        while count > 0 {
            ledger.tick(ctl)?;
            let p = *sites.get(count - 1).ok_or(RasterInputError::LengthMismatch)?;
            let dp = *costs.get(p).ok_or(RasterInputError::LengthMismatch)?;
            let delta = q.checked_sub(p).filter(|n| *n != 0).ok_or(RasterInputError::InvalidBounds)?;
            start = (fq - f64::from(dp).powi(2)) / (2.0 * delta as f64) + (q as f64 + p as f64) * 0.5;
            if start > *starts.get(count - 1).ok_or(RasterInputError::LengthMismatch)? {
                break;
            }
            count -= 1;
        }
        if count == 0 {
            start = f64::NEG_INFINITY
        }
        *sites.get_mut(count).ok_or(RasterInputError::LengthMismatch)? = q;
        *starts.get_mut(count).ok_or(RasterInputError::LengthMismatch)? = start;
        count = count.checked_add(1).ok_or(RasterInputError::LimitExceeded)?;
    }
    if count == 0 {
        for value in output {
            ledger.tick(ctl)?;
            *value = EMPTY_DISTANCE;
        }
        return poll(ctl);
    }
    let mut k = 0usize;
    for (x, value) in output.iter_mut().enumerate() {
        ledger.tick(ctl)?;
        while k.checked_add(1).is_some_and(|next| next < count) && *starts.get(k + 1).ok_or(RasterInputError::LengthMismatch)? < x as f64 {
            ledger.tick(ctl)?;
            k += 1;
        }
        let site = *sites.get(k).ok_or(RasterInputError::LengthMismatch)?;
        let vertical = *costs.get(site).ok_or(RasterInputError::LengthMismatch)?;
        let dx = x as f64 - site as f64;
        *value = (dx * dx + f64::from(vertical).powi(2)).sqrt() as f32;
    }
    poll(ctl)
}
fn edt(inside: &[bool], width: usize, height: usize, ledger: &mut Ledger, ctl: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)?;
    if n != inside.len() || n == 0 {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut any = false;
    let mut all = true;
    for &seed in inside {
        ledger.tick(ctl)?;
        any |= seed;
        all &= seed;
    }
    if !any {
        return ledger.vector(n, EMPTY_DISTANCE, ctl);
    }
    if all {
        return ledger.vector(n, 0.0, ctl);
    }
    let mut vertical = ledger.vector(n, NO_SEED, ctl)?;
    for x in 0..width {
        poll(ctl)?;
        let base = x.checked_mul(height).ok_or(RasterInputError::LimitExceeded)?;
        let column = vertical.get_mut(base..base.checked_add(height).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
        let mut nearest = None;
        for (y, distance) in column.iter_mut().enumerate() {
            ledger.tick(ctl)?;
            let index = y.checked_mul(width).and_then(|i| i.checked_add(x)).ok_or(RasterInputError::LimitExceeded)?;
            if *inside.get(index).ok_or(RasterInputError::LengthMismatch)? {
                nearest = Some(y)
            }
            if let Some(seed) = nearest {
                *distance = u32::try_from(y - seed).map_err(|_| RasterInputError::LimitExceeded)?;
            }
        }
        nearest = None;
        for (y, distance) in column.iter_mut().enumerate().rev() {
            ledger.tick(ctl)?;
            if *distance == 0 {
                nearest = Some(y)
            }
            if let Some(seed) = nearest {
                *distance = (*distance).min(u32::try_from(seed - y).map_err(|_| RasterInputError::LimitExceeded)?);
            }
        }
    }
    let mut output = ledger.vector(n, 0.0f32, ctl)?;
    let band_rows = height.min(BAND_ROWS);
    let band_len = width.checked_mul(band_rows).ok_or(RasterInputError::LimitExceeded)?;
    let mut costs = ledger.vector(band_len, NO_SEED, ctl)?;
    let mut sites = ledger.vector(width, 0usize, ctl)?;
    let mut starts = ledger.vector(width, 0.0f64, ctl)?;
    for y0 in (0..height).step_by(BAND_ROWS) {
        poll(ctl)?;
        let rows = (height - y0).min(BAND_ROWS);
        let length = width.checked_mul(rows).ok_or(RasterInputError::LimitExceeded)?;
        for x in 0..width {
            poll(ctl)?;
            let base = x.checked_mul(height).and_then(|n| n.checked_add(y0)).ok_or(RasterInputError::LimitExceeded)?;
            let column = vertical.get(base..base.checked_add(rows).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
            for (y, &distance) in column.iter().enumerate() {
                ledger.tick(ctl)?;
                let index = y.checked_mul(width).and_then(|n| n.checked_add(x)).ok_or(RasterInputError::LimitExceeded)?;
                *costs.get_mut(index).ok_or(RasterInputError::LengthMismatch)? = distance;
            }
        }
        for row in 0..rows {
            poll(ctl)?;
            let start = row.checked_mul(width).ok_or(RasterInputError::LimitExceeded)?;
            let values = costs
                .get(start..start.checked_add(width).ok_or(RasterInputError::LimitExceeded)?)
                .filter(|_| start < length)
                .ok_or(RasterInputError::LengthMismatch)?;
            let out = (y0 + row).checked_mul(width).ok_or(RasterInputError::LimitExceeded)?;
            row_distance(
                values,
                output.get_mut(out..out.checked_add(width).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?,
                &mut sites,
                &mut starts,
                ledger,
                ctl,
            )?;
        }
    }
    poll(ctl)?;
    Ok(output)
}
fn expanded(mask: &[f32], width: usize, height: usize, radius: f32, ledger: &mut Ledger, ctl: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    let mut inside = ledger.vector(mask.len(), false, ctl)?;
    for (seed, &value) in inside.iter_mut().zip(mask) {
        ledger.tick(ctl)?;
        *seed = value >= 0.5;
    }
    let distance = edt(&inside, width, height, ledger, ctl)?;
    let mut output = ledger.vector(mask.len(), 0.0f32, ctl)?;
    for (value, (&old, &distance)) in output.iter_mut().zip(mask.iter().zip(&distance)) {
        ledger.tick(ctl)?;
        *value = old.max((radius + 1.0 - distance).clamp(0.0, 1.0));
    }
    poll(ctl)?;
    Ok(output)
}
pub fn apply(
    mask: &[f32],
    width: usize,
    height: usize,
    operation: Operation,
    radius: f32,
    limits: Limits,
    ctl: &Interrupt<'_>,
) -> Result<Vec<f32>, RasterInputError> {
    poll(ctl)?;
    let required = requirements(width, height, operation, radius)?;
    if mask.len() != width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)? {
        return Err(RasterInputError::LengthMismatch);
    }
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
        || limits.work_units > MAX_WORK
        || required.peak_resident_bytes > limits.resident_bytes
        || required.work_units > limits.work_units
    {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut ledger = Ledger { bytes: limits.resident_bytes.checked_sub(required.input_bytes).ok_or(RasterInputError::LimitExceeded)?, work: limits.work_units };
    for value in mask {
        ledger.tick(ctl)?;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let output = match operation {
        Operation::Expand => expanded(mask, width, height, radius, &mut ledger, ctl)?,
        Operation::Contract => {
            let mut inverted = ledger.vector(mask.len(), 0.0f32, ctl)?;
            for (value, &old) in inverted.iter_mut().zip(mask) {
                ledger.tick(ctl)?;
                *value = 1.0 - old;
            }
            let mut output = expanded(&inverted, width, height, radius, &mut ledger, ctl)?;
            for value in &mut output {
                ledger.tick(ctl)?;
                *value = 1.0 - *value;
            }
            output
        }
        Operation::Border => {
            let mut inside = ledger.vector(mask.len(), false, ctl)?;
            let mut outside = ledger.vector(mask.len(), false, ctl)?;
            for ((a, b), &value) in inside.iter_mut().zip(&mut outside).zip(mask) {
                ledger.tick(ctl)?;
                *a = value >= 0.5;
                *b = !*a;
            }
            let to_inside = edt(&inside, width, height, &mut ledger, ctl)?;
            let to_outside = edt(&outside, width, height, &mut ledger, ctl)?;
            let mut output = ledger.vector(mask.len(), 0.0f32, ctl)?;
            for (value, (&selected, (&di, &dout))) in output.iter_mut().zip(inside.iter().zip(to_inside.iter().zip(&to_outside))) {
                ledger.tick(ctl)?;
                *value = (1.0 - ((if selected { dout } else { di }) - 1.0) / radius).clamp(0.0, 1.0);
            }
            output
        }
    };
    for value in &output {
        ledger.tick(ctl)?;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    poll(ctl)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_equations_match_the_existing_edt_consumer() {
        let input = [0.0, 0.25, 0.5, 1.0, 0.0, 0.75, 1.0, 0.125, 0.0, 0.0, 0.75, 0.0];
        for operation in [Operation::Expand, Operation::Contract, Operation::Border] {
            for radius in [1.0, 1.5, 3.0] {
                let old = match operation {
                    Operation::Expand => crate::selection::expand(&input, 4, 3, radius),
                    Operation::Contract => crate::selection::contract(&input, 4, 3, radius),
                    Operation::Border => crate::selection::border(&input, 4, 3, radius),
                };
                let new = apply(&input, 4, 3, operation, radius, Limits::default(), &Interrupt::NONE).unwrap();
                assert_eq!(new.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), old.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
    #[test]
    fn lowered_limits_and_late_cancellation_never_change_borrowed_input() {
        let mask = vec![0.5f32; 257 * 33];
        let before = mask.clone();
        let required = requirements(257, 33, Operation::Border, 3.0).unwrap();
        assert_eq!(
            apply(
                &mask,
                257,
                33,
                Operation::Border,
                3.0,
                Limits { resident_bytes: required.peak_resident_bytes - 1, work_units: required.work_units },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 1000;
        assert_eq!(apply(&mask, 257, 33, Operation::Border, 3.0, Limits::default(), &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled));
        assert_eq!(mask, before);
    }
    #[test]
    fn all_none_seed_sentinels_skinny_dimensions_and_cancel_are_bounded() {
        for input in [vec![0.0; 65], vec![1.0; 65]] {
            for operation in [Operation::Expand, Operation::Contract, Operation::Border] {
                let old = match operation {
                    Operation::Expand => crate::selection::expand(&input, 1, 65, 2.0),
                    Operation::Contract => crate::selection::contract(&input, 1, 65, 2.0),
                    Operation::Border => crate::selection::border(&input, 1, 65, 2.0),
                };
                assert_eq!(apply(&input, 1, 65, operation, 2.0, Limits::default(), &Interrupt::NONE).unwrap(), old);
            }
        }
        assert_eq!(apply(&[0.5], 1, 1, Operation::Expand, 1.0, Limits::default(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled));
        assert!(requirements(usize::MAX, 2, Operation::Border, 1.0).is_err());
    }
}

#[cfg(test)]
mod edt_companion_tests{
 use super::*;
 #[test]fn raw_distance_companion_matches_existing_seeds_and_cancel(){let mask=(0..35*23).map(|i|i%31==0).collect::<Vec<_>>();assert_eq!(edt_with_interrupt(&mask,35,23,Limits::default(),&Interrupt::NONE).unwrap(),crate::selection::edt(&mask,35,23));assert!(matches!(edt_with_interrupt(&mask,35,23,Limits::default(),&Interrupt::cancel_only(&||true)),Err(RasterInputError::Cancelled)));}
}
