//! Bounded working-sample Box execution; no CMS, operation or source grant.
use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::{TILE_SIZE, TileCoord};

/// Allocation-free checked S/O/T/H metadata. Logical image bytes are not a
/// resident allocation request. Packed samples are straight linear F64 RGBA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxBlurTilePlan {
    source: Rect,
    output: Rect,
    tile: Rect,
    support: Rect,
    source_read: Rect,
    radius: u32,
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
fn format() -> PixelFormat {
    PixelFormat::new(ColorMode::Rgb, SampleType::F64, true)
}
fn count(rect: Rect) -> Result<usize, RasterInputError> {
    checked_region_byte_len(rect, format())?.checked_div(32).ok_or(RasterInputError::LengthMismatch)
}
impl BoxBlurTilePlan {
    pub fn new(source: Rect, tile: TileCoord, radius: u32, interrupt: &Interrupt<'_>) -> Result<Self, RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        geometry(source)?;
        let output = box_blur_extent(source, radius)?;
        geometry(output)?;
        let x0 = i64::from(tile.tx).checked_mul(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let y0 = i64::from(tile.ty).checked_mul(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let x1 = x0.checked_add(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let y1 = y0.checked_add(i64::from(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        let tile = Rect::new(
            i32::try_from(x0).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(y0).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(x1).map_err(|_| RasterInputError::InvalidBounds)?,
            i32::try_from(y1).map_err(|_| RasterInputError::InvalidBounds)?,
        )
        .intersect(&output);
        if tile.is_empty() {
            return Err(RasterInputError::InvalidBounds);
        }
        let support = box_blur_extent(tile, radius)?;
        // H is a logical tap rectangle, not an allocation or native read.
        // It may cross the source coordinate guard because taps clamp back to
        // valid S (as in the old kernel). Only actual source_read is allocated.
        // Same clamped dependency rectangle as RoiPlan's Clamp source_read.
        // Full source uses logical geometry validation above, never fake U8 or
        // a full-source F64 resident allocation request (important on wasm).
        let xmax = source.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let ymax = source.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let lastx = support.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let lasty = support.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
        let source_read = Rect::new(
            support.x0.clamp(source.x0, xmax),
            support.y0.clamp(source.y0, ymax),
            lastx.clamp(source.x0, xmax).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
            lasty.clamp(source.y0, ymax).checked_add(1).ok_or(RasterInputError::InvalidBounds)?,
        );
        checked_region_byte_len(source_read, format())?;
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
    pub fn packed_input_pixels(self) -> Result<usize, RasterInputError> {
        count(self.source_read)
    }
    pub fn output_bytes(self) -> Result<usize, RasterInputError> {
        checked_region_byte_len(self.tile, format())
    }
    /// Prospective ACTUAL-capacity bound for lower scratch and returned raw Vec
    /// (2x logical allocations plus headers). The caller separately owns/pays
    /// packed input, profiles, native pins/cache and all overlapping outputs.
    /// Work includes packed validation and scratch/output initialization. This
    /// is a fact, never a quota receipt. Caller prepays it on its shared ledger.
    pub fn requirements(self, interrupt: &Interrupt<'_>) -> Result<NeighborhoodRequirements, RasterInputError> {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        let input_bytes = (self.packed_input_pixels()? as u64).checked_mul(32).ok_or(RasterInputError::LimitExceeded)?;
        let raw = self.output_bytes()? as u64;
        let w = u64::from(self.tile.width());
        let rows = u64::from(self.tile.height()).checked_add(u64::from(self.radius) * 2).ok_or(RasterInputError::LimitExceeded)?;
        let horizontal = if self.radius == 0 { 0 } else { w.checked_mul(rows).and_then(|n| n.checked_mul(32)).ok_or(RasterInputError::LimitExceeded)? };
        let window = u64::from(self.radius).checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
        let kernel = if self.radius == 0 {
            raw
        } else {
            rows.checked_mul(w.checked_mul(4).and_then(|n| n.checked_add(window)).ok_or(RasterInputError::LimitExceeded)?)
                .and_then(|n| n.checked_mul(4 * 16))
                .ok_or(RasterInputError::LimitExceeded)?
        };
        let peak_resident_bytes =
            horizontal.checked_add(raw).and_then(|n| n.checked_mul(2)).and_then(|n| n.checked_add(4096)).ok_or(RasterInputError::LimitExceeded)?;
        let work_units = kernel
            .checked_add(input_bytes)
            .and_then(|n| n.checked_add(horizontal))
            .and_then(|n| n.checked_add(raw))
            .and_then(|n| n.checked_add(128))
            .ok_or(RasterInputError::LimitExceeded)?;
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        Ok(NeighborhoodRequirements { peak_resident_bytes, work_units })
    }
}
/// Compute the COMPLETE canonical T; cropping belongs to the caller afterwards.
/// Borrows exactly source_read row-major straight linear F64 RGBA samples.
/// Returns native-endian F64 RGBA bytes for T, actual Vec capacity<=2*len.
/// No callback or reference is retained. Finite RGB/unit alpha is mandatory,
/// including radius0, whose raw words are copied without premultiply/roundtrip.
pub fn try_box_blur_packed_tile(
    plan: &BoxBlurTilePlan,
    packed: &[[f64; 4]],
    limits: GaussianLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Vec<u8>, RasterInputError> {
    let required = plan.requirements(interrupt)?;
    if packed.len() != plan.packed_input_pixels()? {
        return Err(RasterInputError::LengthMismatch);
    }
    if required.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || required.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    for chunk in packed.chunks(64) {
        interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
        for p in chunk {
            if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(p.get(3).ok_or(RasterInputError::LengthMismatch)?) {
                return Err(RasterInputError::InvalidSample);
            }
        }
    }
    let read = &mut |x: i32, y: i32, out: &mut [f64]| {
        if !plan.source_read.contains(x, y) || out.len() != 4 {
            return Err(RasterInputError::InvalidBounds);
        }
        let row = usize::try_from(i64::from(y) - i64::from(plan.source_read.y0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let col = usize::try_from(i64::from(x) - i64::from(plan.source_read.x0)).map_err(|_| RasterInputError::InvalidBounds)?;
        let index = row.checked_mul(plan.source_read.width() as usize).and_then(|n| n.checked_add(col)).ok_or(RasterInputError::LimitExceeded)?;
        out.copy_from_slice(packed.get(index).ok_or(RasterInputError::LengthMismatch)?);
        Ok(())
    };
    let mut units = 0;
    let raw = if plan.radius == 0 {
        let mut raw = allocate::<u8>(plan.output_bytes()?)?;
        for (pixel, bytes) in packed.iter().zip(raw.chunks_exact_mut(32)) {
            poll(&mut units, 32, interrupt)?;
            for (v, dest) in pixel.iter().zip(bytes.chunks_exact_mut(8)) {
                dest.copy_from_slice(&v.to_ne_bytes());
            }
        }
        raw
    } else {
        let r = plan.radius as usize;
        let window = r.checked_mul(2).and_then(|n| n.checked_add(1)).ok_or(RasterInputError::LimitExceeded)?;
        tile_core(plan.tile, plan.source, format(), 4, r, window, &mut units, interrupt, read)?
    };
    interrupt.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(raw)
}
#[cfg(test)]
#[path = "packed_tests.rs"]
mod tests;
