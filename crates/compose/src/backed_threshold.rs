//! Explicit nonlinear unit-RGB Histogram Threshold over an owned-backed plane.
//! The caller supplies the existing typed operation and declares nonlinear input.
//! No profile conversion, original-source qualification, filter/mask/outer policy,
//! runtime storage class or default renderer enrollment is established here.
//! Every source alpha byte is copied raw; enabled RGB output is exact native0/1.
use crate::backed_normal::NormalBandLimits;
use photocraft_algo::gimp::threshold::Threshold;
use photocraft_color::{ColorMode, PixelFormat, SampleAccessError, SampleType, try_read_sample_f64, try_write_sample_f64};
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

/// Per enabled pixel, in addition to native read/copy/encode bytes.
pub const SAMPLE_WORK: u64 = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum BackedThresholdError {
    Unsupported(&'static str),
    Invalid(&'static str),
    Backing(BackingError),
    View(RasterViewError),
    Sample(SampleAccessError),
    Operation(RasterInputError),
}
impl std::fmt::Display for BackedThresholdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backed Histogram Threshold: {self:?}")
    }
}
impl std::error::Error for BackedThresholdError {}

/// Borrows the actual current COW/source owner, with no clone of the layer, tile
/// map, Document, profile or store. The fixed prepared header is same-ledger paid.
#[derive(Debug)]
pub struct PreparedBackedThreshold<'a> {
    layer: &'a BackedLayer,
    operation: Threshold,
    enabled: bool,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}

/// Private row-major native-endian output over exactly `rect`, in the original
/// runtime format. This is not a Surface/default plane or authoritative edit.
/// Payload is destroyed before its real capacity/header reservation is refunded.
#[derive(Debug)]
pub struct OwnedNativeThresholdBand {
    rect: Rect,
    format: PixelFormat,
    bytes: Vec<u8>,
    lease: QuotaLease,
}
impl OwnedNativeThresholdBand {
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
        self.lease.bytes()
    }
}

fn plus(a: usize, b: usize) -> Result<usize, BackedThresholdError> {
    a.checked_add(b).ok_or(BackedThresholdError::Invalid("native band capacity overflow"))
}
fn bytes_u64(n: usize) -> Result<u64, BackedThresholdError> {
    u64::try_from(n).map_err(|_| BackedThresholdError::Invalid("native band address space"))
}
fn tile_rect(coord: TileCoord) -> Result<Rect, BackedThresholdError> {
    let x = coord.tx.checked_mul(TILE_SIZE).ok_or(BackedThresholdError::Invalid("native tile coordinate"))?;
    let y = coord.ty.checked_mul(TILE_SIZE).ok_or(BackedThresholdError::Invalid("native tile coordinate"))?;
    Ok(Rect::new(
        x,
        y,
        x.checked_add(TILE_SIZE).ok_or(BackedThresholdError::Invalid("native tile coordinate"))?,
        y.checked_add(TILE_SIZE).ok_or(BackedThresholdError::Invalid("native tile coordinate"))?,
    ))
}

impl<'a> PreparedBackedThreshold<'a> {
    /// The explicit encoding must be nonlinear and the expected native descriptor
    /// must match the actual RGB-with-alpha plane. This caller declaration is not
    /// a profile/working-space witness or an XCF argument/source binding. Only
    /// requested operative samples are scanned; defaults/padding stay inert.
    pub fn new(
        layer: &'a BackedLayer,
        expected: NativePlaneDescriptor,
        encoding: SampleEncoding,
        operation: Threshold,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedThresholdError> {
        Self::new_with_enabled(layer, expected, encoding, operation, true, limits, control)
    }
    /// Explicit disabled bypass. The typed operation remains validated, and all
    /// selected native samples must still be finite unit. No RGB/alpha word is
    /// re-encoded in this branch; disabling supplies no source authority.
    pub fn new_with_enabled(
        layer: &'a BackedLayer,
        expected: NativePlaneDescriptor,
        encoding: SampleEncoding,
        operation: Threshold,
        enabled: bool,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedThresholdError> {
        control.check(16).map_err(BackedThresholdError::Backing)?;
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(BackedThresholdError::Invalid("native band limits"));
        }
        if layer.descriptor() != expected {
            return Err(BackedThresholdError::Invalid("operative native descriptor mismatch"));
        }
        let format = expected.format();
        if encoding != SampleEncoding::NonlinearProfile
            || format.mode != ColorMode::Rgb
            || !format.alpha
            || !matches!(format.sample, SampleType::U16 | SampleType::F32 | SampleType::F64)
        {
            return Err(BackedThresholdError::Unsupported("nonlinear unit RGB alpha U16/F32/F64 only"));
        }
        operation.validate().map_err(BackedThresholdError::Operation)?;
        let header = plus(std::mem::size_of::<Self>(), 128)?;
        let lease = layer.quota().reserve(QuotaKind::Resident, bytes_u64(header)?).map_err(|_| BackedThresholdError::Backing(BackingError::Quota))?;
        let prepared = Self { layer, operation, enabled, limits, _lease: lease };
        control.check(0).map_err(BackedThresholdError::Backing)?;
        Ok(prepared)
    }

    /// Produce one bounded contained band with the unchanged nonlinear histogram
    /// math (Float32 histogram ingress, inclusive native threshold bounds). All
    /// selected native components must be finite unit. Enabled RGB is native0/1;
    /// the original alpha sample, including signed zero/F64 low bits, stays raw.
    ///
    /// Caller retains all earlier returned band leases and supplies its actual
    /// cumulative ReadControl. Each call uses the source's same persistent quota;
    /// no local replacement work budget, allowance reset or tile-pin-all exists.
    /// Cache residency can change on errors; private output is never published.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNativeThresholdBand, BackedThresholdError> {
        control.check(1).map_err(BackedThresholdError::Backing)?;
        let bounds = self.layer.operative_bounds();
        if rect.is_empty() || rect.intersect(&bounds) != rect || rect.height() > self.limits.max_rows {
            return Err(BackedThresholdError::Invalid("native band outside operative bounds or row limit"));
        }
        let pixels = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(BackedThresholdError::Invalid("native band pixel overflow"))?;
        if pixels > self.limits.max_pixels {
            return Err(BackedThresholdError::Invalid("native band pixel limit"));
        }
        let format = self.layer.format();
        let channels = format.channels();
        let color_channels = format.mode.color_channels();
        let bpp = format.bytes_per_pixel();
        let length = usize::try_from(pixels).ok().and_then(|n| n.checked_mul(bpp)).ok_or(BackedThresholdError::Invalid("native band byte overflow"))?;
        let header = plus(std::mem::size_of::<OwnedNativeThresholdBand>(), 128)?;
        let capacity_limit = plus(length, 64)?;
        let envelope = plus(capacity_limit, header)?;
        let lease = self.layer.quota().reserve(QuotaKind::Resident, bytes_u64(envelope)?).map_err(|_| BackedThresholdError::Backing(BackingError::Quota))?;
        let mut output = OwnedNativeThresholdBand { rect, format, bytes: Vec::new(), lease };
        // Fixed native and numeric pixel arrays plus one tile pin. Real tile
        // payload/cache reservations remain inside the original backing owner.
        let scratch = plus(plus(std::mem::size_of::<[u8; 40]>(), std::mem::size_of::<[[f64; 4]; 2]>())?, plus(std::mem::size_of::<CheckedTilePin>(), 128)?)?;
        let _scratch = self.layer.quota().reserve(QuotaKind::Resident, bytes_u64(scratch)?).map_err(|_| BackedThresholdError::Backing(BackingError::Quota))?;
        control.check(1).map_err(BackedThresholdError::Backing)?;
        output.bytes.try_reserve_exact(length).map_err(|_| BackedThresholdError::Backing(BackingError::Allocation))?;
        if output.bytes.capacity() > capacity_limit {
            return Err(BackedThresholdError::Backing(BackingError::Allocation));
        }
        output.lease.reconcile_capacity(bytes_u64(plus(output.bytes.capacity(), header)?)?).map_err(|_| BackedThresholdError::Backing(BackingError::Quota))?;
        while output.bytes.len() < length {
            let end = plus(output.bytes.len(), (length - output.bytes.len()).min(65_536))?;
            control.check(bytes_u64(end - output.bytes.len())?).map_err(BackedThresholdError::Backing)?;
            output.bytes.resize(end, 0);
        }
        let view = self.layer.checked_view();
        for coord in rect.tiles() {
            control.check(1).map_err(BackedThresholdError::Backing)?;
            let overlap = tile_rect(coord)?.intersect(&rect);
            let pin = view.pin_tile(coord, control).map_err(BackedThresholdError::View)?;
            for y in overlap.y0..overlap.y1 {
                for x in overlap.x0..overlap.x1 {
                    let mut raw = [0u8; 40];
                    let native = raw.get_mut(..bpp).ok_or(BackedThresholdError::Invalid("native pixel width"))?;
                    pin.read_pixel_native_into(x, y, native, control).map_err(BackedThresholdError::View)?;
                    let mut input = [0f64; 4];
                    control.check(bytes_u64(channels)?).map_err(BackedThresholdError::Backing)?;
                    for (index, sample) in input.get_mut(..channels).ok_or(BackedThresholdError::Invalid("native channel count"))?.iter_mut().enumerate() {
                        *sample = try_read_sample_f64(native, format.sample, index).map_err(BackedThresholdError::Sample)?;
                    }
                    let mut evaluated = [0f64; 4];
                    if self.enabled {
                        // Fixed bound for the existing <=4-channel unit scan,
                        // histogram selection/weighting, rounding and RGB fill.
                        // This is logical work accounting, not a CPU-cycle claim.
                        control.check(SAMPLE_WORK).map_err(BackedThresholdError::Backing)?;
                        self.operation
                            .try_eval_pixel(
                                input.get(..channels).ok_or(BackedThresholdError::Invalid("native channel count"))?,
                                color_channels,
                                evaluated.get_mut(..channels).ok_or(BackedThresholdError::Invalid("native channel count"))?,
                            )
                            .map_err(BackedThresholdError::Operation)?;
                    } else {
                        control.check(bytes_u64(channels)?).map_err(BackedThresholdError::Backing)?;
                        if input
                            .get(..channels)
                            .ok_or(BackedThresholdError::Invalid("native channel count"))?
                            .iter()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        {
                            return Err(BackedThresholdError::Operation(RasterInputError::UnsupportedConversion));
                        }
                    }
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackedThresholdError::Invalid("native band row"))?;
                    let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackedThresholdError::Invalid("native band column"))?;
                    let start = row
                        .checked_mul(rect.width() as usize)
                        .and_then(|n| n.checked_add(col))
                        .and_then(|n| n.checked_mul(bpp))
                        .ok_or(BackedThresholdError::Invalid("native band pixel offset"))?;
                    let target = output.bytes.get_mut(start..plus(start, bpp)?).ok_or(BackedThresholdError::Invalid("native band pixel length"))?;
                    let writes = if self.enabled {
                        color_channels.checked_mul(format.sample.bytes()).ok_or(BackedThresholdError::Invalid("native RGB write length"))?
                    } else {
                        0
                    };
                    control.check(bytes_u64(plus(bpp, writes)?)?).map_err(BackedThresholdError::Backing)?;
                    // Preserve alpha as original bytes, never through f32/f64
                    // decoding/re-encoding, even when it selected the histogram.
                    target.copy_from_slice(native);
                    if self.enabled {
                        for index in 0..color_channels {
                            try_write_sample_f64(target, format.sample, index, *evaluated.get(index).ok_or(BackedThresholdError::Invalid("native RGB index"))?)
                                .map_err(BackedThresholdError::Sample)?;
                        }
                    }
                }
            }
        }
        control.check(0).map_err(BackedThresholdError::Backing)?;
        Ok(output)
    }
}

#[cfg(test)]
#[path = "backed_threshold_tests.rs"]
mod tests;
