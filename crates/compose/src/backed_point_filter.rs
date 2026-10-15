//! Explicit Threshold Alpha execution over an actual owned-backed native plane.
//!
//! This is a caller-supplied operation/storage closure, not a source-qualification
//! receipt, imported filter mapper, profile interpretation or default dispatcher.
//! No mask, opacity, outer operator, source epoch or scene policy is inferred.
//! The existing point operation itself bypasses color transfer for this alpha-only
//! computation. Every returned color sample retains its exact native source words.
use crate::backed_normal::NormalBandLimits;
use photocraft_algo::gimp::threshold_alpha::ThresholdAlpha;
use photocraft_color::{ColorMode, PixelFormat, SampleAccessError, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::BackedLayer;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    RasterInputError,
    backing::{
        BackingError, NativePlaneDescriptor, ReadControl,
        quota::{QuotaKind, QuotaLease},
    },
    checked_view::{CheckedTilePin, RasterViewError},
};

#[derive(Debug, PartialEq, Eq)]
pub enum BackedPointError {
    Unsupported(&'static str),
    Invalid(&'static str),
    Backing(BackingError),
    View(RasterViewError),
    Sample(SampleAccessError),
    Operation(RasterInputError),
}
impl std::fmt::Display for BackedPointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "backed Threshold Alpha: {self:?}")
    }
}
impl std::error::Error for BackedPointError {}

/// Borrows the actual current COW/source owner, with no clone of the layer, tile
/// map, Document, profile or store. The fixed prepared header is same-ledger paid.
#[derive(Debug)]
pub struct PreparedBackedThresholdAlpha<'a> {
    layer: &'a BackedLayer,
    operation: ThresholdAlpha,
    limits: NormalBandLimits,
    _lease: QuotaLease,
}

/// Private row-major native-endian output over exactly `rect`, in the original
/// runtime format. This is not a Surface/default plane or authoritative edit.
/// Payload is destroyed before its real capacity/header reservation is refunded.
#[derive(Debug)]
pub struct OwnedNativePointBand {
    rect: Rect,
    format: PixelFormat,
    bytes: Vec<u8>,
    lease: QuotaLease,
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
        self.lease.bytes()
    }
}

fn plus(a: usize, b: usize) -> Result<usize, BackedPointError> {
    a.checked_add(b).ok_or(BackedPointError::Invalid("native band capacity overflow"))
}
fn bytes_u64(n: usize) -> Result<u64, BackedPointError> {
    u64::try_from(n).map_err(|_| BackedPointError::Invalid("native band address space"))
}
fn tile_rect(coord: TileCoord) -> Result<Rect, BackedPointError> {
    let x = coord.tx.checked_mul(TILE_SIZE).ok_or(BackedPointError::Invalid("native tile coordinate"))?;
    let y = coord.ty.checked_mul(TILE_SIZE).ok_or(BackedPointError::Invalid("native tile coordinate"))?;
    Ok(Rect::new(
        x,
        y,
        x.checked_add(TILE_SIZE).ok_or(BackedPointError::Invalid("native tile coordinate"))?,
        y.checked_add(TILE_SIZE).ok_or(BackedPointError::Invalid("native tile coordinate"))?,
    ))
}

impl<'a> PreparedBackedThresholdAlpha<'a> {
    /// Accept only the existing native U16/F32/F64 RGB/Gray-with-alpha storage
    /// slice. The expected descriptor must equal the actual current plane. The
    /// caller's threshold is validated by the unchanged lower operation; this
    /// does not bind it to an XCF argument, profile, epoch or physical mask.
    /// Untouched pixels/defaults are not scanned or granted sample authority.
    pub fn new(
        layer: &'a BackedLayer,
        expected: NativePlaneDescriptor,
        operation: ThresholdAlpha,
        limits: NormalBandLimits,
        control: &ReadControl<'_>,
    ) -> Result<Self, BackedPointError> {
        control.check(1).map_err(BackedPointError::Backing)?;
        let maximum = NormalBandLimits::default();
        if limits.max_rows == 0 || limits.max_rows > maximum.max_rows || limits.max_pixels == 0 || limits.max_pixels > maximum.max_pixels {
            return Err(BackedPointError::Invalid("native band limits"));
        }
        if layer.descriptor() != expected {
            return Err(BackedPointError::Invalid("operative native descriptor mismatch"));
        }
        let format = expected.format();
        if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !format.alpha
            || !matches!(format.sample, SampleType::U16 | SampleType::F32 | SampleType::F64)
        {
            return Err(BackedPointError::Unsupported("native RGB/Gray alpha U16/F32/F64 only"));
        }
        operation.validate().map_err(BackedPointError::Operation)?;
        let header = plus(std::mem::size_of::<Self>(), 128)?;
        let lease = layer.quota().reserve(QuotaKind::Resident, bytes_u64(header)?).map_err(|_| BackedPointError::Backing(BackingError::Quota))?;
        let prepared = Self { layer, operation, limits, _lease: lease };
        control.check(0).map_err(BackedPointError::Backing)?;
        Ok(prepared)
    }

    /// Produce one bounded contained band with the original alpha Float32 ingress
    /// and strict Double comparison. All components must be finite unit values,
    /// as required by the unchanged lower kernel. Color bytes are copied raw,
    /// including Float64 precision and signed zero; only alpha is re-encoded.
    ///
    /// Caller retains all earlier returned band leases and supplies its actual
    /// cumulative ReadControl. Each call uses the source's same persistent quota;
    /// no local replacement work budget, allowance reset or tile-pin-all exists.
    /// Cache residency can change on errors; private output is never published.
    pub fn render_band(&self, rect: Rect, control: &ReadControl<'_>) -> Result<OwnedNativePointBand, BackedPointError> {
        control.check(1).map_err(BackedPointError::Backing)?;
        let bounds = self.layer.operative_bounds();
        if rect.is_empty() || rect.intersect(&bounds) != rect || rect.height() > self.limits.max_rows {
            return Err(BackedPointError::Invalid("native band outside operative bounds or row limit"));
        }
        let pixels = u64::from(rect.width()).checked_mul(u64::from(rect.height())).ok_or(BackedPointError::Invalid("native band pixel overflow"))?;
        if pixels > self.limits.max_pixels {
            return Err(BackedPointError::Invalid("native band pixel limit"));
        }
        let format = self.layer.format();
        let channels = format.channels();
        let color_channels = format.mode.color_channels();
        let bpp = format.bytes_per_pixel();
        let length = usize::try_from(pixels).ok().and_then(|n| n.checked_mul(bpp)).ok_or(BackedPointError::Invalid("native band byte overflow"))?;
        let header = plus(std::mem::size_of::<OwnedNativePointBand>(), 128)?;
        let capacity_limit = plus(length, 64)?;
        let envelope = plus(capacity_limit, header)?;
        let lease = self.layer.quota().reserve(QuotaKind::Resident, bytes_u64(envelope)?).map_err(|_| BackedPointError::Backing(BackingError::Quota))?;
        let mut output = OwnedNativePointBand { rect, format, bytes: Vec::new(), lease };
        // Fixed native and numeric pixel arrays plus one tile pin. Real tile
        // payload/cache reservations remain inside the original backing owner.
        let scratch = plus(plus(std::mem::size_of::<[u8; 40]>(), std::mem::size_of::<[[f64; 4]; 2]>())?, plus(std::mem::size_of::<CheckedTilePin>(), 128)?)?;
        let _scratch = self.layer.quota().reserve(QuotaKind::Resident, bytes_u64(scratch)?).map_err(|_| BackedPointError::Backing(BackingError::Quota))?;
        control.check(1).map_err(BackedPointError::Backing)?;
        output.bytes.try_reserve_exact(length).map_err(|_| BackedPointError::Backing(BackingError::Allocation))?;
        if output.bytes.capacity() > capacity_limit {
            return Err(BackedPointError::Backing(BackingError::Allocation));
        }
        output.lease.reconcile_capacity(bytes_u64(plus(output.bytes.capacity(), header)?)?).map_err(|_| BackedPointError::Backing(BackingError::Quota))?;
        while output.bytes.len() < length {
            let end = plus(output.bytes.len(), (length - output.bytes.len()).min(65_536))?;
            control.check(bytes_u64(end - output.bytes.len())?).map_err(BackedPointError::Backing)?;
            output.bytes.resize(end, 0);
        }
        let view = self.layer.checked_view();
        for coord in rect.tiles() {
            control.check(1).map_err(BackedPointError::Backing)?;
            let overlap = tile_rect(coord)?.intersect(&rect);
            let pin = view.pin_tile(coord, control).map_err(BackedPointError::View)?;
            for y in overlap.y0..overlap.y1 {
                for x in overlap.x0..overlap.x1 {
                    let mut raw = [0u8; 40];
                    let native = raw.get_mut(..bpp).ok_or(BackedPointError::Invalid("native pixel width"))?;
                    pin.read_pixel_native_into(x, y, native, control).map_err(BackedPointError::View)?;
                    let mut input = [0f64; 4];
                    control.check(bytes_u64(channels)?).map_err(BackedPointError::Backing)?;
                    for (index, sample) in input.get_mut(..channels).ok_or(BackedPointError::Invalid("native channel count"))?.iter_mut().enumerate() {
                        *sample = try_read_sample_f64(native, format.sample, index).map_err(BackedPointError::Sample)?;
                    }
                    // Original kernel validates the input components, copies
                    // them, and performs its alpha conversion/comparison. Price
                    // those bounded visits, distinct from native byte copies.
                    control
                        .check(bytes_u64(channels.checked_mul(2).and_then(|n| n.checked_add(2)).ok_or(BackedPointError::Invalid("pixel work overflow"))?)?)
                        .map_err(BackedPointError::Backing)?;
                    let mut evaluated = [0f64; 4];
                    self.operation
                        .try_eval_pixel(
                            input.get(..channels).ok_or(BackedPointError::Invalid("native channel count"))?,
                            color_channels,
                            evaluated.get_mut(..channels).ok_or(BackedPointError::Invalid("native channel count"))?,
                        )
                        .map_err(BackedPointError::Operation)?;
                    let row = usize::try_from(i64::from(y) - i64::from(rect.y0)).map_err(|_| BackedPointError::Invalid("native band row"))?;
                    let col = usize::try_from(i64::from(x) - i64::from(rect.x0)).map_err(|_| BackedPointError::Invalid("native band column"))?;
                    let start = row
                        .checked_mul(rect.width() as usize)
                        .and_then(|n| n.checked_add(col))
                        .and_then(|n| n.checked_mul(bpp))
                        .ok_or(BackedPointError::Invalid("native band pixel offset"))?;
                    let target = output.bytes.get_mut(start..plus(start, bpp)?).ok_or(BackedPointError::Invalid("native band pixel length"))?;
                    control.check(bytes_u64(plus(bpp, format.sample.bytes())?)?).map_err(BackedPointError::Backing)?;
                    target.copy_from_slice(native);
                    try_write_sample_f64(
                        target,
                        format.sample,
                        color_channels,
                        *evaluated.get(color_channels).ok_or(BackedPointError::Invalid("native alpha index"))?,
                    )
                    .map_err(BackedPointError::Sample)?;
                }
            }
        }
        control.check(0).map_err(BackedPointError::Backing)?;
        Ok(output)
    }
}

#[cfg(test)]
#[path = "backed_point_filter_tests.rs"]
mod tests;
