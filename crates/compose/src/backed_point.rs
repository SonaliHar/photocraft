//! Shared typed point arithmetic over genuine already-working native backing.
//! No CMS, FilterNode/schema/outer/mask interpretation or source/runtime/default
//! enrollment occurs here. PointOperation does not encode a stock name: the two
//! Invert identities and configured GIMP versus GEGL operations remain distinct
//! in their enclosing caller. This bridge supports the existing Surface point
//! pixel contract, not every candidate kernel or PointStack precision policy.
use crate::backed_normal::NormalBandLimits;
use photocraft_algo::gimp::point::{PointOperation, PreparedPointOperation};
use photocraft_color::PixelFormat;
use photocraft_doc::{BackedLayer, SampleEncoding};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    RasterInputError,
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
    checked_view::{CheckedTilePin, RasterViewError},
};
/// Explicit caller-established operation working interpretation. Equality binds
/// its identity only; no ICC bytes, native stock transfer or CMS is inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointWorkingContext {
    pub format: PixelFormat,
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Backing(BackingError),
    View(RasterViewError),
    Operation(RasterInputError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backed native point: {self:?}")
    }
}
impl std::error::Error for Error {}
/// Cooperative bridge work only. Native store/cache/read work additionally
/// debits the SAME caller control at the actual checked-view calls, never a
/// reset local counter or a fabricated total provider-work bound.
pub const PREPARATION_WORK: u64 = 1024;
pub const PIXEL_WORK: u64 = 1024;
#[derive(Clone, Copy, Debug)]
pub struct BandRequirements {
    pub output_resident_bytes: u64,
    pub scratch_resident_bytes: u64,
    pub prepaid_work_units: u64,
    pub pixels: u64,
    pub native_bytes: usize,
}
// Deliberately sealed. Future prepared configured/native kernels require their
// own typed variant/constructor and exact domain, not an arbitrary callback.
#[derive(Debug)]
enum PreparedKernel {
    Point(PreparedPointOperation),
}
impl PreparedKernel {
    fn evaluate(&self, input: &[u8], format: PixelFormat, output: &mut [u8]) -> Result<(), RasterInputError> {
        match self {
            Self::Point(p) => p.try_eval_native_pixel(input, format, output),
        }
    }
}
#[derive(Debug)]
pub struct PreparedBackedPoint<'a> {
    layer: &'a BackedLayer,
    context: PointWorkingContext,
    operation: PointOperation,
    kernel: PreparedKernel,
    enabled: bool,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}
/// Tight row-major native-endian output for exactly rect; no Surface/default,
/// outside-ROI words, mutable source access or write-back authority is supplied.
#[derive(Debug)]
pub struct OwnedNativePointBand {
    rect: Rect,
    format: PixelFormat,
    bytes: Vec<u8>,
    _lease: QuotaLease,
}
impl OwnedNativePointBand {
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn resident_bytes(&self) -> u64 {
        self._lease.bytes()
    }
}
fn length() -> Error {
    Error::Invalid("native point length or coordinate overflow")
}
fn add(a: usize, b: usize) -> Result<usize, Error> {
    a.checked_add(b).ok_or_else(length)
}
fn n(a: usize) -> Result<u64, Error> {
    u64::try_from(a).map_err(|_| length())
}
fn tile_rect(c: TileCoord) -> Result<Rect, Error> {
    let x = c.tx.checked_mul(TILE_SIZE).ok_or_else(length)?;
    let y = c.ty.checked_mul(TILE_SIZE).ok_or_else(length)?;
    Ok(Rect::new(x, y, x.checked_add(TILE_SIZE).ok_or_else(length)?, y.checked_add(TILE_SIZE).ok_or_else(length)?))
}
impl<'a> PreparedBackedPoint<'a> {
    /// Source must ALREADY be in the explicit operation-working context. In
    /// particular, a nonlinear source is not converted to Linear for Exposure,
    /// nor is live integer-Gray temperature replaced with the wide equation.
    pub fn new(
        layer: &'a BackedLayer,
        expected: NativePlaneDescriptor,
        context: PointWorkingContext,
        operation: PointOperation,
        enabled: bool,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, Error> {
        control.check(1).map_err(Error::Backing)?;
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(Error::Invalid("lower-only nonzero band limits"));
        }
        if expected != layer.descriptor() || context.format != expected.format() || context.profile_identity == [0; 32] {
            return Err(Error::Invalid("actual descriptor or explicit working context mismatch"));
        }
        let lease = layer.quota().reserve(QuotaKind::Resident, n(add(std::mem::size_of::<Self>(), 128)?)?).map_err(|_| Error::Backing(BackingError::Quota))?;
        control.check(PREPARATION_WORK).map_err(Error::Backing)?;
        let prepared = operation.prepare().map_err(Error::Operation)?;
        prepared.validate_native_format(context.format).map_err(Error::Operation)?;
        let result = Self { layer, context, operation, kernel: PreparedKernel::Point(prepared), enabled, limits, _lease: lease };
        control.check(0).map_err(Error::Backing)?;
        Ok(result)
    }
    pub fn context(&self) -> PointWorkingContext {
        self.context
    }
    pub fn operation(&self) -> PointOperation {
        self.operation
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn full_extent(&self) -> Rect {
        self.layer.operative_bounds()
    }
    /// O(1), allocation-free prospective envelope; provider/cache work is metered
    /// separately by actual reads. Full extent remains the authoritative source,
    /// while rect is only the contained requested output band.
    pub fn requirements(&self, rect: Rect, control: &ReadControl<'_>) -> Result<BandRequirements, Error> {
        control.check(1).map_err(Error::Backing)?;
        if rect.is_empty() || rect.intersect(&self.full_extent()) != rect || rect.height() > self.limits.max_rows {
            return Err(Error::Invalid("band containment or row limit"));
        }
        let pixels = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or_else(length)?;
        if pixels > self.limits.max_pixels {
            return Err(Error::Invalid("band pixel limit"));
        }
        let bytes = usize::try_from(pixels).ok().and_then(|p| p.checked_mul(self.context.format.bytes_per_pixel())).ok_or_else(length)?;
        let output = n(add(add(bytes, 64)?, add(std::mem::size_of::<OwnedNativePointBand>(), 128)?)?)?;
        let scratch =
            n(add(add(std::mem::size_of::<[[u8; 40]; 3]>(), std::mem::size_of::<[[f64; 4]; 2]>())?, add(std::mem::size_of::<CheckedTilePin>(), 128)?)?)?;
        let tiles_x = (i64::from(rect.x1) - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(rect.x0).div_euclid(i64::from(TILE_SIZE)) + 1;
        let tiles_y = (i64::from(rect.y1) - 1).div_euclid(i64::from(TILE_SIZE)) - i64::from(rect.y0).div_euclid(i64::from(TILE_SIZE)) + 1;
        let tiles = u64::try_from(tiles_x).ok().and_then(|x| x.checked_mul(u64::try_from(tiles_y).ok()?)).ok_or_else(length)?;
        let work = pixels
            .checked_mul(PIXEL_WORK)
            .and_then(|w| w.checked_mul(2))
            .and_then(|w| w.checked_add(n(bytes).ok()?.checked_mul(8)?))
            .and_then(|w| w.checked_add(tiles.checked_mul(8)?))
            .ok_or_else(length)?;
        Ok(BandRequirements { output_resident_bytes: output, scratch_resident_bytes: scratch, prepaid_work_units: work, pixels, native_bytes: bytes })
    }
    /// Preflight/evaluate every selected native pixel before private output
    /// allocation, then render with one source pin at a time. Disabled execution
    /// copies raw pixels without invoking the equation; descriptor/format/domain
    /// shape is still validated. Exact operation identities retain raw identity.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNativePointBand, Error> {
        let req = self.requirements(rect, control)?;
        // Both real envelopes and both passes' bridge work precede source traversal.
        let lease = self.layer.quota().reserve(QuotaKind::Resident, req.output_resident_bytes).map_err(|_| Error::Backing(BackingError::Quota))?;
        let _scratch = self.layer.quota().reserve(QuotaKind::Resident, req.scratch_resident_bytes).map_err(|_| Error::Backing(BackingError::Quota))?;
        control.check(req.prepaid_work_units).map_err(Error::Backing)?;
        self.walk(rect, None, control)?;
        let mut result = OwnedNativePointBand { rect, format: self.context.format, bytes: Vec::new(), _lease: lease };
        control.check(0).map_err(Error::Backing)?;
        result.bytes.try_reserve_exact(req.native_bytes).map_err(|_| Error::Backing(BackingError::Allocation))?;
        if result.bytes.capacity() > add(req.native_bytes, 64)? {
            return Err(Error::Backing(BackingError::Allocation));
        }
        result
            ._lease
            .reconcile_capacity(n(add(result.bytes.capacity(), add(std::mem::size_of::<OwnedNativePointBand>(), 128)?)?)?)
            .map_err(|_| Error::Backing(BackingError::Quota))?;
        while result.bytes.len() < req.native_bytes {
            control.check(0).map_err(Error::Backing)?;
            let end = add(result.bytes.len(), (req.native_bytes - result.bytes.len()).min(64 << 10))?;
            result.bytes.resize(end, 0);
        }
        self.walk(rect, Some(&mut result.bytes), control)?;
        control.check(0).map_err(Error::Backing)?;
        Ok(result)
    }
    fn walk(&self, rect: Rect, mut output: Option<&mut [u8]>, control: &ReadControl<'_>) -> Result<(), Error> {
        let view = self.layer.checked_view();
        let bpp = self.context.format.bytes_per_pixel();
        for coord in rect.tiles() {
            control.check(0).map_err(Error::Backing)?;
            let overlap = tile_rect(coord)?.intersect(&rect);
            let pin = view.pin_tile(coord, control).map_err(Error::View)?;
            for y in overlap.y0..overlap.y1 {
                for x in overlap.x0..overlap.x1 {
                    control.check(0).map_err(Error::Backing)?;
                    let mut raw = [0u8; 40];
                    let source = raw.get_mut(..bpp).ok_or_else(length)?;
                    pin.read_pixel_native_into(x, y, source, control).map_err(Error::View)?;
                    let mut evaluated = [0u8; 40];
                    let pixel = evaluated.get_mut(..bpp).ok_or_else(length)?;
                    if self.enabled {
                        self.kernel.evaluate(source, self.context.format, pixel).map_err(Error::Operation)?;
                    } else {
                        pixel.copy_from_slice(source);
                    }
                    if let Some(bytes) = output.as_deref_mut() {
                        let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| length())?;
                        let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| length())?;
                        let start =
                            row.checked_mul(rect.width() as usize).and_then(|v| v.checked_add(col)).and_then(|v| v.checked_mul(bpp)).ok_or_else(length)?;
                        bytes.get_mut(start..add(start, bpp)?).ok_or_else(length)?.copy_from_slice(pixel);
                    }
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "backed_point_tests.rs"]
mod tests;
