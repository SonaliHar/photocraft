//! Checked serial consumer of the existing Smooth box passes. The nonzero
//! window, f64 running sums, intermediate f32 casts and >=0.5 curve are retained.
//! This API accepts explicit f32 coverage; no native-wide/source math is inferred.
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError};
const STRIP: usize = 64;
const MAX_BOX_R: usize = 1 << 24;
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
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub work_units: u64,
    pub peak_resident_bytes: u64,
}
fn poll(ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    ctl.check().map_err(|_| RasterInputError::Cancelled)
}
pub fn requirements(width: usize, height: usize, radius: f32) -> Result<Requirements, RasterInputError> {
    if width == 0 || height == 0 || u32::try_from(width).is_err() || u32::try_from(height).is_err() {
        return Err(RasterInputError::InvalidBounds);
    }
    if !radius.is_finite() || radius < 0.0 {
        return Err(RasterInputError::InvalidSample);
    }
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)? as u64;
    let input_bytes = n.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?;
    let output_bytes = input_bytes;
    let scratch_bytes = n.checked_mul(12).and_then(|n| n.checked_add(131072)).ok_or(RasterInputError::LimitExceeded)?;
    let peak_resident_bytes = input_bytes.checked_add(scratch_bytes).and_then(|n| n.checked_add(output_bytes)).ok_or(RasterInputError::LimitExceeded)?;
    let work_units =
        n.checked_mul(128).and_then(|n| n.checked_add((width as u64).checked_add(height as u64)?.checked_mul(128)?)).ok_or(RasterInputError::LimitExceeded)?;
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
        let mut output = Vec::new();
        output.try_reserve_exact(n).map_err(|_| RasterInputError::AllocationFailed)?;
        let actual = output.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(RasterInputError::LimitExceeded)? as u64;
        self.bytes = self.bytes.checked_sub(actual).ok_or(RasterInputError::LimitExceeded)?;
        while output.len() < n {
            self.tick(ctl)?;
            let count = (n - output.len()).min(1024);
            self.work = self.work.checked_sub(count as u64).ok_or(RasterInputError::LimitExceeded)?;
            output.resize(output.len() + count, value.clone());
        }
        poll(ctl)?;
        Ok(output)
    }
}
#[derive(Clone, Copy)]
struct Win {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}
impl Win {
    fn w(self) -> usize {
        self.x1 - self.x0
    }
    fn h(self) -> usize {
        self.y1 - self.y0
    }
}
fn box_line(source: &[f32], output: &mut [f32], radius: usize, ledger: &mut Ledger, ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    let n = source.len();
    if n == 0 || output.len() != n {
        return Err(RasterInputError::LengthMismatch);
    }
    if radius == 0 {
        for (value, &input) in output.iter_mut().zip(source) {
            ledger.tick(ctl)?;
            *value = input;
        }
        return poll(ctl);
    }
    let first = *source.first().ok_or(RasterInputError::LengthMismatch)?;
    let last = *source.last().ok_or(RasterInputError::LengthMismatch)?;
    let inside = radius.min(n - 1);
    // f64::Sum's ordered fold and the original left-associated window expression.
    let mut sum = -0.0f64;
    for &value in source.get(..=inside).ok_or(RasterInputError::LengthMismatch)? {
        ledger.tick(ctl)?;
        sum += f64::from(value);
    }
    let mut acc = radius as f64 * f64::from(first) + sum + (radius - inside) as f64 * f64::from(last);
    let norm = 1.0 / radius.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)? as f64;
    let ri = radius as i64;
    for (x, value) in output.iter_mut().enumerate() {
        ledger.tick(ctl)?;
        *value = (acc * norm) as f32;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        let x = x as i64;
        let a = usize::try_from((x + ri + 1).clamp(0, n as i64 - 1)).map_err(|_| RasterInputError::InvalidBounds)?;
        let b = usize::try_from((x - ri).clamp(0, n as i64 - 1)).map_err(|_| RasterInputError::InvalidBounds)?;
        acc += f64::from(*source.get(a).ok_or(RasterInputError::LengthMismatch)?) - f64::from(*source.get(b).ok_or(RasterInputError::LengthMismatch)?);
    }
    poll(ctl)
}
struct Rows<'a> {
    tmp: &'a [f32],
    gw: usize,
    bh: usize,
    off: usize,
    gh: usize,
}
fn add_row(acc: &mut [f64], rows: &Rows<'_>, y: usize, c0: usize, k: f64, ledger: &mut Ledger, ctl: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if k == 0.0 {
        return Ok(());
    }
    let Some(row) = y.checked_sub(rows.off).filter(|row| *row < rows.bh) else { return Ok(()) };
    let start = row.checked_mul(rows.gw).and_then(|n| n.checked_add(c0)).ok_or(RasterInputError::LimitExceeded)?;
    let input = rows.tmp.get(start..start.checked_add(acc.len()).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
    for (value, &sample) in acc.iter_mut().zip(input) {
        ledger.tick(ctl)?;
        *value += k * f64::from(sample);
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(())
}
pub fn apply(mask: &[f32], width: usize, height: usize, radius: f32, limits: Limits, ctl: &Interrupt<'_>) -> Result<Vec<f32>, RasterInputError> {
    poll(ctl)?;
    let required = requirements(width, height, radius)?;
    let n = width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)?;
    if mask.len() != n {
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
    let r = (radius.max(0.0).round() as usize).min(MAX_BOX_R);
    let mut bounds = None::<Win>;
    for (y, row) in mask.chunks_exact(width).enumerate() {
        poll(ctl)?;
        let mut first = None;
        let mut last = None;
        for (x, &value) in row.iter().enumerate() {
            ledger.tick(ctl)?;
            if !value.is_finite() {
                return Err(RasterInputError::InvalidSample);
            }
            if value != 0.0 {
                first.get_or_insert(x);
                last = Some(x);
            }
        }
        if let (Some(a), Some(z)) = (first, last) {
            bounds = Some(match bounds {
                None => Win { x0: a, x1: z + 1, y0: y, y1: y + 1 },
                Some(b) => Win { x0: b.x0.min(a), x1: b.x1.max(z + 1), y0: b.y0, y1: y + 1 },
            });
        }
    }
    let mut output = ledger.vector(n, 0.0f32, ctl)?;
    let Some(b) = bounds else { return Ok(output) };
    let grow = r.saturating_add(1);
    let win = Win {
        x0: b.x0.saturating_sub(grow),
        x1: b.x1.saturating_add(grow).min(width),
        y0: b.y0.saturating_sub(grow),
        y1: b.y1.saturating_add(grow).min(height),
    };
    let (gw, gh) = (win.w(), win.h());
    let mut tmp = ledger.vector(b.h().checked_mul(gw).ok_or(RasterInputError::LimitExceeded)?, 0.0f32, ctl)?;
    for y in 0..b.h() {
        poll(ctl)?;
        let start = (b.y0 + y).checked_mul(width).and_then(|n| n.checked_add(win.x0)).ok_or(RasterInputError::LimitExceeded)?;
        let input = mask.get(start..start.checked_add(gw).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?;
        let target = y.checked_mul(gw).ok_or(RasterInputError::LimitExceeded)?;
        box_line(
            input,
            tmp.get_mut(target..target.checked_add(gw).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?,
            r,
            &mut ledger,
            ctl,
        )?;
    }
    let rows = Rows { tmp: &tmp, gw, bh: b.h(), off: b.y0 - win.y0, gh };
    let sw_max = STRIP.min(gw);
    let mut strip = ledger.vector(gh.checked_mul(sw_max).ok_or(RasterInputError::LimitExceeded)?, 0.0f32, ctl)?;
    let mut accumulator = ledger.vector(sw_max, 0.0f64, ctl)?;
    for c0 in (0..gw).step_by(STRIP) {
        poll(ctl)?;
        let sw = STRIP.min(gw - c0);
        let acc = accumulator.get_mut(..sw).ok_or(RasterInputError::LengthMismatch)?;
        for value in acc.iter_mut() {
            ledger.tick(ctl)?;
            *value = 0.0;
        }
        let inside = r.min(rows.gh - 1);
        add_row(acc, &rows, 0, c0, r as f64, &mut ledger, ctl)?;
        for j in 0..=inside {
            poll(ctl)?;
            add_row(acc, &rows, j, c0, 1.0, &mut ledger, ctl)?;
        }
        add_row(acc, &rows, rows.gh - 1, c0, (r - inside) as f64, &mut ledger, ctl)?;
        let norm = 1.0 / r.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)? as f64;
        for yy in 0..gh {
            poll(ctl)?;
            let start = yy.checked_mul(sw).ok_or(RasterInputError::LimitExceeded)?;
            for (d, &a) in strip
                .get_mut(start..start.checked_add(sw).ok_or(RasterInputError::LimitExceeded)?)
                .ok_or(RasterInputError::LengthMismatch)?
                .iter_mut()
                .zip(acc.iter())
            {
                ledger.tick(ctl)?;
                *d = (a * norm) as f32;
                if !d.is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
            }
            add_row(
                acc,
                &rows,
                yy.checked_add(r).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?.min(gh - 1),
                c0,
                1.0,
                &mut ledger,
                ctl,
            )?;
            add_row(acc, &rows, yy.saturating_sub(r), c0, -1.0, &mut ledger, ctl)?;
        }
        // Same strip/window coordinates and threshold as assemble(curve).
        for yy in 0..gh {
            poll(ctl)?;
            let source = yy.checked_mul(sw).ok_or(RasterInputError::LimitExceeded)?;
            let target = (win.y0 + yy).checked_mul(width).and_then(|n| n.checked_add(win.x0 + c0)).ok_or(RasterInputError::LimitExceeded)?;
            for (d, &value) in output
                .get_mut(target..target.checked_add(sw).ok_or(RasterInputError::LimitExceeded)?)
                .ok_or(RasterInputError::LengthMismatch)?
                .iter_mut()
                .zip(strip.get(source..source.checked_add(sw).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LengthMismatch)?)
            {
                ledger.tick(ctl)?;
                *d = if value >= 0.5 { 1.0 } else { 0.0 };
            }
        }
    }
    poll(ctl)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_smooth_retains_upstream_box_and_threshold_order() {
        let mut input = vec![0.0f32; 67 * 35];
        for y in 3..31 {
            for x in 0..64 {
                input[y * 67 + x] = if (x + y) % 7 == 0 { 0.5 } else { 1.0 };
            }
        }
        for radius in [0.0, 1.0, 1.5, 4.0, 12.0] {
            let old = crate::selection::smooth(&input, 67, 35, radius);
            let new = apply(&input, 67, 35, radius, Limits::default(), &Interrupt::NONE).unwrap();
            assert_eq!(new, old);
        }
    }
    #[test]
    fn zeros_skinny_edges_cancel_and_limits_are_explicit() {
        for input in [vec![0.0; 65], vec![1.0; 65], (0..65).map(|i| if i % 2 == 0 { 0.5 } else { 0.0 }).collect()] {
            assert_eq!(apply(&input, 1, 65, 4.0, Limits::default(), &Interrupt::NONE).unwrap(), crate::selection::smooth(&input, 1, 65, 4.0));
        }
        assert_eq!(apply(&[1.0], 1, 1, 1.0, Limits::default(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled));
        assert!(requirements(usize::MAX, 2, 1.0).is_err());
    }
}
