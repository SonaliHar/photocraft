//! Controlled native coverage arithmetic, separate from the legacy GRAY8 helper.
//! All inputs are caller-owned. This charges their actual boxed payload lengths
//! and known conservative sparse-map/Arc/header bounds per owner (shared payloads
//! may be counted twice); allocator overhead/RSS is not measured. No source or
//! renderer qualification follows. Surface here is the concrete L0 owned raster;
//! no accounted Document, lazy provider or asserted decoded plane is accepted by
//! this API. Enclosing callers must retain their own document/provider authority.
//! Output clones A outside ROI and its default.
//! Float coverage is interpreted only where selected: finite unit samples are
//! validated in an allocation-free first pass, before any output/private buffer.
//! Unselected native words remain uninterpreted, including nonfinite defaults
//! when no selected pixel uses them. No hidden f32 arithmetic is used.
use crate::gimp::multi_input_coverage as coverage_math;
use photocraft_color::checked_arithmetic::{Domain, Policy, Rounding, UnitOverflow};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, Tile, checked_region_byte_len};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Maximum,
    Minimum,
    SubtractClamped,
    SubtractCoverage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoveragePolicy {
    FiniteUnit,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub a: &'a Surface,
    pub b: &'a Surface,
    pub roi: Rect,
    pub operation: Operation,
    pub coverage: CoveragePolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 } }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Access(photocraft_color::SampleAccessError),
    Coverage(coverage_math::Error),
    FormatMismatch,
    NonFiniteCoverage,
    OutsideUnitCoverage,
}
impl From<photocraft_color::SampleAccessError> for Error {
    fn from(value: photocraft_color::SampleAccessError) -> Self {
        Self::Access(value)
    }
}
impl From<RasterInputError> for Error {
    fn from(v: RasterInputError) -> Self {
        Self::Raster(v)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native mask combine: {self:?}")
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn limit() -> Error {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn poll(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn validate(r: Request<'_>, limits: Limits) -> Result<usize> {
    let standard = Limits::default();
    if limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
        return Err(limit());
    }
    let f = r.a.format();
    if f != r.b.format() || f.mode != ColorMode::Grayscale || f.alpha || !matches!(f.sample, SampleType::U32 | SampleType::F64) {
        return Err(Error::FormatMismatch);
    }
    match r.coverage {
        CoveragePolicy::FiniteUnit => {}
    }
    const COORD_LIMIT: i32 = 1 << 29;
    if r.roi.x1 < r.roi.x0 || r.roi.y1 < r.roi.y0 || [r.roi.x0, r.roi.x1, r.roi.y0, r.roi.y1].iter().any(|v| !(-COORD_LIMIT..=COORD_LIMIT).contains(v)) {
        return Err(RasterInputError::InvalidBounds.into());
    }
    if r.roi.width() > 262144 || r.roi.height() > 262144 {
        return Err(limit());
    }
    Ok(checked_region_byte_len(r.roi, f)?)
}
fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
fn retained(surface: &Surface, payload: u64, ctl: &Interrupt<'_>) -> Result<u64> {
    let bpp = surface.format().bytes_per_pixel();
    if surface.default_bytes().len() != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    let mut total = add(std::mem::size_of::<Surface>() as u64, bpp as u64)?;
    for (coord, tile) in surface.tiles() {
        poll(ctl)?;
        coord.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coord.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != payload {
            return Err(RasterInputError::LengthMismatch.into());
        }
        total = add(total, add(tile.bytes().len() as u64, add(256, std::mem::size_of::<Tile>() as u64)?)?)?;
    }
    Ok(total)
}
/// Metadata traversal only. The prospective envelope is admitted before walking
/// actual tile metadata; selected numeric coverage is admitted by try_combine.
pub fn requirements(r: Request<'_>, limits: Limits, ctl: &Interrupt<'_>) -> Result<Requirements> {
    poll(ctl)?;
    let length = validate(r, limits)? as u64;
    let bpp = r.a.format().bytes_per_pixel() as u64;
    let count = tile_count(r.roi)?;
    let ac = r.a.tile_count() as u64;
    let bc = r.b.tile_count() as u64;
    let payload = mul(65536, bpp)?;
    let header = add(256, std::mem::size_of::<Tile>() as u64)?;
    let source_bound = add(mul(add(ac, bc)?, add(payload, header)?)?, mul(add(std::mem::size_of::<Surface>() as u64, bpp)?, 2)?)?;
    let output_retained_bytes = add(mul(add(ac, count)?, add(payload, header)?)?, add(std::mem::size_of::<Surface>() as u64, bpp)?)?;
    // Input maps above are held; writer stages a further output map plus changed
    // tile Arc entries. Bounds are separate from exact boxed payload measurements.
    let index_bytes = add(mul(add(ac, count)?, 1024)?, 512)?;
    // Two Vecs each <=2*length plus simultaneous writer native Vec/boxed tile.
    let scratch_bytes = add(mul(length, 4)?, add(mul(payload, 2)?, 4096)?)?;
    let extra = add(output_retained_bytes, add(index_bytes, scratch_bytes)?)?;
    let work_units = add(add(mul(length, 32)?, mul(add(ac, bc)?, 256)?)?, add(mul(mul(count, add(ac, count)?)?, 512)?, mul(mul(count, payload)?, 2)?)?)?;
    if add(source_bound, extra)? > limits.resident_bytes || work_units > limits.work_units {
        return Err(limit());
    }
    let source_retained_bytes = add(retained(r.a, payload, ctl)?, retained(r.b, payload, ctl)?)?;
    let peak_resident_bytes = add(source_retained_bytes, extra)?;
    if peak_resident_bytes > limits.resident_bytes {
        return Err(limit());
    }
    poll(ctl)?;
    Ok(Requirements { source_retained_bytes, output_retained_bytes, index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
fn coverage(bytes: &[u8], sample: SampleType) -> Result<f64> {
    let v = try_read_sample_f64(bytes, sample, 0)?;
    if !v.is_finite() {
        return Err(Error::NonFiniteCoverage);
    }
    if !(0.0..=1.0).contains(&v) {
        return Err(Error::OutsideUnitCoverage);
    }
    Ok(v)
}
fn evaluate(left: &[u8], right: &[u8], out: &mut [u8], format: PixelFormat, op: Operation) -> Result<()> {
    if left.len() != out.len() || right.len() != out.len() {
        return Err(RasterInputError::LengthMismatch.into());
    }
    let zero = match format.sample {
        SampleType::U32 => u32::from_ne_bytes(right.try_into().map_err(|_| RasterInputError::LengthMismatch)?) == 0,
        SampleType::F64 => coverage(right, format.sample)? == 0.0,
        _ => return Err(Error::FormatMismatch),
    };
    if zero && matches!(op, Operation::SubtractClamped | Operation::SubtractCoverage) {
        out.copy_from_slice(left);
        return Ok(());
    }
    let operation = match op {
        Operation::Maximum => coverage_math::Operation::UnionMax,
        Operation::Minimum => coverage_math::Operation::IntersectionMin,
        Operation::SubtractClamped => coverage_math::Operation::SubtractSaturating,
        Operation::SubtractCoverage => coverage_math::Operation::Multiply,
    };
    let mut complement = [0u8; 8];
    let operand = if op == Operation::SubtractCoverage {
        match format.sample {
            SampleType::U32 => {
                let code = u32::from_ne_bytes(right.try_into().map_err(|_| RasterInputError::LengthMismatch)?);
                complement.get_mut(..4).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(&(u32::MAX - code).to_ne_bytes());
            }
            SampleType::F64 => complement.copy_from_slice(&(1.0 - coverage(right, format.sample)?).to_ne_bytes()),
            _ => return Err(Error::FormatMismatch),
        }
        complement.get(..right.len()).ok_or(RasterInputError::LengthMismatch)?
    } else {
        right
    };
    // Reuse the existing checked native code/rational/rounding and exact-winning
    // word primitives. Only the explicitly named coverage complement is prepared.
    coverage_math::evaluate_sample_into(
        format,
        left,
        operand,
        out,
        coverage_math::Params { operation, policy: Policy { domain: Domain::Unit, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Nearest } },
    )
    .map_err(Error::Coverage)
}
/// Min/Max preserve the exact chosen native sample; logical ties choose A.
/// Both subtraction operators preserve raw A when B is logical zero. U32
/// coverage multiplication rounds the normalized rational to nearest code.
/// Source A's default/outside ROI remains raw. No partial output is returned.
pub fn try_combine(r: Request<'_>, limits: Limits, ctl: &Interrupt<'_>) -> Result<Surface> {
    let _needs = requirements(r, limits, ctl)?;
    let format = r.a.format();
    let bpp = format.bytes_per_pixel();
    // Allocation-free numeric first pass includes actual defaults only when used
    // by selected coordinates. Unselected nonfinite/HDR words are not interpreted.
    if format.sample == SampleType::F64 {
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        for y in r.roi.y0..r.roi.y1 {
            poll(ctl)?;
            for (i, x) in (r.roi.x0..r.roi.x1).enumerate() {
                if i.is_multiple_of(256) {
                    poll(ctl)?;
                }
                r.a.try_read_pixel_bytes(x, y, &mut a)?;
                r.b.try_read_pixel_bytes(x, y, &mut b)?;
                coverage(&a, format.sample)?;
                coverage(&b, format.sample)?;
            }
        }
    }
    poll(ctl)?;
    if r.roi.is_empty() {
        return Ok(r.a.clone());
    }
    let length = checked_region_byte_len(r.roi, format)?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    for v in [&mut a, &mut b] {
        v.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if v.capacity() as u64 > mul(length as u64, 2)? {
            return Err(limit());
        }
        v.resize(length, 0);
    }
    r.a.try_read_interleaved_into_with(r.roi, &mut a, ctl)?;
    r.b.try_read_interleaved_into_with(r.roi, &mut b, ctl)?;
    for (i, (left, right)) in a.chunks_exact_mut(bpp).zip(b.chunks_exact(bpp)).enumerate() {
        if i.is_multiple_of(256) {
            poll(ctl)?;
        }
        let mut raw = [0u8; 8];
        let original = raw.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
        original.copy_from_slice(left);
        evaluate(original, right, left, format, r.operation)?;
    }
    poll(ctl)?;
    let mut output = r.a.clone();
    // Existing writer polls per tile; its native fill/map clone is a bounded lower
    // provider boundary priced above, not independently interruptible per byte.
    output.try_write_interleaved_with(r.roi, &a, ctl)?;
    poll(ctl)?;
    Ok(output)
}
#[cfg(test)]
#[path = "native_mask_combine_tests.rs"]
mod tests;
