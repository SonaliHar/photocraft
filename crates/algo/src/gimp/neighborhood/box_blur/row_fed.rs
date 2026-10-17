//! Row-fed working-sample execution. No CMS, source or operation authority.
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::{TILE_SIZE, TileCoord};

/// The original callback error is returned intact; no string/error-code parsing.
#[derive(Debug)]
pub enum BoxBlurRowError<E> {
    Kernel(RasterInputError),
    Source(E),
}
impl<E> From<RasterInputError> for BoxBlurRowError<E> {
    fn from(error: RasterInputError) -> Self {
        Self::Kernel(error)
    }
}

/// Positive-radius logical geometry, independent of full support residency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxBlurRowPlan {
    source: Rect,
    output: Rect,
    tile: Rect,
    support: Rect,
    source_read: Rect,
    radius: u32,
}

/// Actual-capacity prospective facts, not quota receipts. The caller separately
/// owns/pays one input row, profiles, native pins/cache and overlapping outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxBlurRowRequirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub row_pixels: usize,
    pub row_requests: u64,
    pub output_bytes: usize,
}

fn format() -> PixelFormat {
    PixelFormat::new(ColorMode::Rgb, SampleType::F64, true)
}
fn geometry(rect: Rect) -> Result<(), RasterInputError> {
    let bound = 1 << 29;
    if rect.is_empty() || [rect.x0, rect.y0, rect.x1, rect.y1].iter().any(|v| !(-bound..=bound).contains(v)) {
        return Err(RasterInputError::InvalidBounds);
    }
    let w = u64::try_from(i64::from(rect.x1) - i64::from(rect.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
    let h = u64::try_from(i64::from(rect.y1) - i64::from(rect.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
    if w > 262_144 || h > 262_144 || w.checked_mul(h).ok_or(RasterInputError::LimitExceeded)? > 1 << 30 {
        return Err(RasterInputError::LimitExceeded);
    }
    Ok(())
}
impl BoxBlurRowPlan {
    /// Compute all of global tile intersect full inflated O, never a cropped ROI.
    /// Zero remains the existing packed working-space identity, not this entry.
    pub fn new(source: Rect, tile: TileCoord, radius: u32, interrupt: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        if !(1..=1000).contains(&radius) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        geometry(source)?;
        let output = box_blur_extent(source, radius)?;
        geometry(output)?;
        let x0 = i64::from(tile.tx).checked_mul(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let y0 = i64::from(tile.ty).checked_mul(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let tile = Rect::new(
            i32::try_from(x0).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(y0).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(x0.checked_add(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(y0.checked_add(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?).map_err(|_| RasterInputError::InvalidBounds)?,
        )
        .intersect(&output);
        if tile.is_empty() {
            return Err(RasterInputError::InvalidBounds);
        }
        let support = box_blur_extent(tile, radius)?;
        let xmax = source.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let ymax = source.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let source_read = Rect::new(
            support.x0.clamp(source.x0, xmax),
            support.y0.clamp(source.y0, ymax),
            support.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.clamp(source.x0, xmax).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
            support.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?.clamp(source.y0, ymax).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
        );
        // Only these actual allocations have resident byte admission. The full
        // logical source_read/H can be large and is never allocated here.
        checked_region_byte_len(Rect::new(0, 0, source_read.width() as i32, 1), format())?;
        checked_region_byte_len(tile, format())?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(Self { source, output, tile, support, source_read, radius })
    }
    pub fn source_extent(self) -> Rect {
        self.source
    }
    pub fn output_extent(self) -> Rect {
        self.output
    }
    pub fn tile_output(self) -> Rect {
        self.tile
    }
    pub fn support(self) -> Rect {
        self.support
    }
    pub fn source_read(self) -> Rect {
        self.source_read
    }
    pub fn radius(self) -> u32 {
        self.radius
    }
    pub fn source_row_width(self) -> usize {
        self.source_read.width() as usize
    }
    pub fn row_requests(self) -> Result<u64, RasterInputError> {
        u64::from(self.radius)
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_add(u64::from(self.tile.height()).checked_sub(1)?.checked_mul(2)?))
            .ok_or(RasterInputError::LimitExceeded)
    }
    pub fn output_bytes(self) -> Result<usize, RasterInputError> {
        checked_region_byte_len(self.tile, format())
    }
    /// Includes 2x actual allocation capacities and fixed headers. Input row
    /// poisoning/validation is included; callback native/CMS/provider work is NOT.
    /// Caller prepays this work once and separately pays all callback work.
    pub fn requirements(self, interrupt: &Interrupt<'_>) -> Result<BoxBlurRowRequirements, RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let w = u64::from(self.tile.width());
        let h = u64::from(self.tile.height());
        let k = u64::from(self.radius).checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
        let e = self.row_requests()?;
        let l = self.source_row_width() as u64;
        let raw = self.output_bytes()?;
        let scratch = w.checked_mul(32 * 3).ok_or(RasterInputError::LimitExceeded)?;
        let peak_resident_bytes =
            scratch.checked_add(raw as u64).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(4096)).ok_or(RasterInputError::LimitExceeded)?;
        let horizontal =
            k.checked_add(w.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?).and_then(|n| n.checked_mul(64)).ok_or(RasterInputError::LimitExceeded)?;
        let each_row = horizontal
            .checked_add(l.checked_mul(64).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|n| n.checked_add(w.checked_mul(32)?))
            .ok_or(RasterInputError::LimitExceeded)?;
        let vertical = k
            .checked_add(h.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|n| n.checked_add(h.checked_sub(1)?.checked_mul(2)?))
            .and_then(|n| n.checked_mul(w))
            .and_then(|n| n.checked_mul(64))
            .ok_or(RasterInputError::LimitExceeded)?;
        let work_units = e
            .checked_mul(each_row)
            .and_then(|n| n.checked_add(vertical))
            .and_then(|n| n.checked_add(scratch))
            .and_then(|n| n.checked_add(raw as u64))
            .and_then(|n| n.checked_add(4096))
            .ok_or(RasterInputError::LimitExceeded)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(BoxBlurRowRequirements { peak_resident_bytes, work_units, row_pixels: self.source_row_width(), row_requests: e, output_bytes: raw })
    }
}

fn row<E>(
    plan: &BoxBlurRowPlan,
    j: usize,
    input: &mut [[f64; 4]],
    dest: &mut [f64],
    units: &mut usize,
    interrupt: &Interrupt<'_>,
    read: &mut impl FnMut(i32, i32, &mut [[f64; 4]]) -> Result<(), E>,
) -> Result<(), BoxBlurRowError<E>> {
    // Poison each private row before callback: partial successful fills cannot
    // silently reuse values from a preceding request. Work is prepaid above.
    for chunk in input.chunks_mut(64) {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        chunk.fill([f64::NAN; 4]);
    }
    let y = i64::from(plan.tile.y0).checked_add(j as i64).and_then(|n| n.checked_sub(i64::from(plan.radius))).ok_or(RasterInputError::InvalidBounds)?;
    let y = y.clamp(i64::from(plan.source.y0), i64::from(plan.source.y1) - 1);
    let y = i32::try_from(y).map_err(|_| RasterInputError::InvalidBounds)?;
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    read(plan.source_read.x0, y, input).map_err(BoxBlurRowError::Source)?;
    for chunk in input.chunks(64) {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        for p in chunk {
            if p.iter().any(|v| !v.is_finite()) || !p.last().is_some_and(|a| (0.0..=1.0).contains(a)) {
                return Err(RasterInputError::InvalidSample.into());
            }
        }
    }
    let mut pixels = |x: i32, yy: i32, out: &mut [f64]| -> Result<(), BoxBlurRowError<E>> {
        if yy != y || out.len() != 4 {
            return Err(RasterInputError::InvalidBounds.into());
        }
        let index = usize::try_from(i64::from(x) - i64::from(plan.source_read.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        out.copy_from_slice(input.get(index).ok_or(RasterInputError::InvalidBounds)?);
        Ok(())
    };
    let r = plan.radius as usize;
    horizontal_row(dest, plan.tile, plan.source, format(), 4, r, r * 2 + 1, j, units, interrupt, &mut pixels)
}

/// Compute complete canonical T with bounded recomputation. Callback receives
/// exact source x0, clamped world y, and caller-owned row of row_pixels entries.
/// It must synchronously fill straight linear finite RGB/unit-alpha RGBA; all
/// real native/provider/CMS work remains caller-controlled and separately paid.
/// Neither callback nor buffers are retained. Active radius0 uses the old packed
/// working-sample identity plus caller CMS, never this positive-radius entry.
pub fn try_box_blur_row_tile<E>(
    plan: &BoxBlurRowPlan,
    row_scratch: &mut [[f64; 4]],
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
    read_row: &mut impl FnMut(i32, i32, &mut [[f64; 4]]) -> Result<(), E>,
) -> Result<Vec<u8>, BoxBlurRowError<E>> {
    let required = plan.requirements(interrupt)?;
    if row_scratch.len() != required.row_pixels {
        return Err(RasterInputError::LengthMismatch.into());
    }
    if required.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || required.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded.into());
    }
    let w = plan.tile.width() as usize;
    let h = plan.tile.height() as usize;
    let count = w.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?;
    let k = (plan.radius as usize).checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let mut remove = allocate::<f64>(count)?;
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let mut add = allocate::<f64>(count)?;
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let mut sums = allocate::<f64>(count)?;
    let mut units = 0;
    for j in 0..k {
        row(plan, j, row_scratch, &mut remove, &mut units, interrupt, read_row)?;
        for (sum, sample) in sums.as_chunks_mut::<4>().0.iter_mut().zip(remove.as_chunks::<4>().0.iter()) {
            poll(&mut units, 4, interrupt)?;
            for (s, v) in sum.iter_mut().zip(sample) {
                *s += v;
            }
        }
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    let mut raw = allocate::<u8>(required.output_bytes)?;
    for y in 0..h {
        for x in 0..w {
            poll(&mut units, 16, interrupt)?;
            let at = x.checked_mul(4).ok_or(RasterInputError::LimitExceeded)?;
            let end = at.checked_add(4).ok_or(RasterInputError::LimitExceeded)?;
            let offset = y.checked_mul(w).and_then(|n| n.checked_add(x)).and_then(|n| n.checked_mul(32)).ok_or(RasterInputError::LimitExceeded)?;
            let stop = offset.checked_add(32).ok_or(RasterInputError::LimitExceeded)?;
            write_box_pixel(
                sums.get(at..end).ok_or(RasterInputError::LengthMismatch)?,
                format(),
                4,
                k,
                raw.get_mut(offset..stop).ok_or(RasterInputError::LengthMismatch)?,
            )?;
        }
        if y + 1 < h {
            row(plan, y, row_scratch, &mut remove, &mut units, interrupt, read_row)?;
            row(plan, y.checked_add(k).ok_or(RasterInputError::LimitExceeded)?, row_scratch, &mut add, &mut units, interrupt, read_row)?;
            for (sign, values) in [(-1., remove.as_slice()), (1., add.as_slice())] {
                for (sum, sample) in sums.as_chunks_mut::<4>().0.iter_mut().zip(values.as_chunks::<4>().0.iter()) {
                    poll(&mut units, 4, interrupt)?;
                    for (s, v) in sum.iter_mut().zip(sample) {
                        *s += sign * v;
                    }
                }
            }
        }
    }
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(raw)
}

#[cfg(test)]
#[path = "row_fed_tests.rs"]
mod tests;
