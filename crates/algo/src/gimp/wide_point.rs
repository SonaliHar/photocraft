//! Standalone native affine point arithmetic, not a serialized GEGL evaluator.
//! The caller has already established CMS working interpretation. Context
//! equality binds that assertion; it does not validate ICC bytes or perform CMS.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_color::checked_arithmetic::{self as arithmetic, Destination, Domain, Operand, Policy, Storage, UnitOverflow};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    /// Exact caller-established working profile identity. No default is inferred.
    pub profile_identity: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityPolicy { FiniteOnly, PreserveRaw }

#[derive(Clone, Copy, Debug)]
pub struct Affine {
    /// Two unfused binary64 stages: value * scale, then + offset, with only
    /// final storage quantization. Each arithmetic intermediate must be finite.
    pub scale: f64,
    pub offset: f64,
    pub policy: Policy,
    /// Raw identity is permitted only for exact scale=1 and positive-zero offset.
    pub identity: IdentityPolicy,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits { pub resident_bytes: u64, pub work_units: u64 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError), Arithmetic(arithmetic::ArithmeticError), ContextMismatch,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "wide affine point: {self:?}") }
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error { fn from(error: RasterInputError) -> Self { Self::Raster(error) } }
impl From<arithmetic::ArithmeticError> for Error { fn from(error: arithmetic::ArithmeticError) -> Self { Self::Arithmetic(error) } }
type Result<T> = std::result::Result<T, Error>;

fn validate(format: PixelFormat, source: WorkingContext, expected: WorkingContext, affine: Affine) -> Result<()> {
    if source != expected { return Err(Error::ContextMismatch); }
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(format.sample, SampleType::U32 | SampleType::F64) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    if !affine.scale.is_finite() || !affine.offset.is_finite() { return Err(arithmetic::ArithmeticError::NonFinite.into()); }
    if affine.policy.domain == Domain::FloatHdr && affine.policy.unit_overflow != UnitOverflow::Reject {
        return Err(arithmetic::ArithmeticError::InvalidPolicy.into());
    }
    Ok(())
}

/// Allocation-free one-pixel transaction. Alpha is never decoded or changed;
/// source and destination must be exactly one native pixel. All refusal paths
/// leave destination unchanged. HDR/rounding are explicit arithmetic policies.
pub fn evaluate_pixel_into(format: PixelFormat, input: &[u8], output: &mut [u8], source: WorkingContext, expected: WorkingContext, affine: Affine) -> Result<()> {
    validate(format, source, expected, affine)?;
    let bpp = format.bytes_per_pixel();
    if input.len() != bpp || output.len() != bpp { return Err(RasterInputError::LengthMismatch.into()); }
    let mut staging = [0u8; 32];
    let encoded = staging.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    encoded.copy_from_slice(input);
    let identity = affine.scale.to_bits() == 1.0f64.to_bits() && affine.offset.to_bits() == 0.0f64.to_bits();
    let color_channels = match format.mode { ColorMode::Rgb => 3, ColorMode::Grayscale => 1, _ => return Err(RasterInputError::UnsupportedConversion.into()) };
    for channel in 0..color_channels {
        if identity && affine.identity == IdentityPolicy::PreserveRaw { continue; }
        let value = arithmetic::decode_finite(Operand { bytes: input, storage: Storage::Native(format.sample), index: channel }, affine.policy.domain)?;
        if identity { continue; }
        let product = value * affine.scale;
        if !product.is_finite() { return Err(arithmetic::ArithmeticError::NonFinite.into()); }
        arithmetic::encode_finite(product + affine.offset, Destination { bytes: encoded, storage: Storage::Native(format.sample), index: channel }, affine.policy)?;
    }
    output.copy_from_slice(encoded);
    Ok(())
}

fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() { return Ok(0); }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let x = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let y = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    Ok(x.checked_mul(y).ok_or(RasterInputError::LimitExceeded)?)
}

/// Checked scalar precharge includes repeated raw writer map traversal/cloning.
pub fn required_work(source: &Surface, roi: Rect) -> Result<u64> {
    let bytes = checked_region_byte_len(roi, source.format())? as u64;
    let tiles = tile_count(roi)?;
    let source_tiles = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    Ok(bytes.checked_mul(16).and_then(|n| n.checked_add(source_tiles.checked_mul(4)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(source_tiles.checked_add(tiles)?)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)?)
}

/// Private raster transaction. Outside ROI, default bytes, alpha and unmodified
/// native storage remain raw. On cancellation/refusal no result is published.
/// Enclosing graphs must additionally charge simultaneous aggregate owners.
pub fn try_apply(source: &Surface, roi: Rect, source_context: WorkingContext, expected_context: WorkingContext, affine: Affine, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    validate(source.format(), source_context, expected_context, affine)?;
    let work = required_work(source, roi)?;
    if work > limits.work_units { return Err(RasterInputError::LimitExceeded.into()); }
    let tiles = tile_count(roi)?;
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let payload = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let replacements = tiles.checked_mul(payload.checked_add(512).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if cap == 0 || replacements > cap { return Err(RasterInputError::LimitExceeded.into()); }
    let maps = (source.tile_count() as u64).checked_mul(512).ok_or(RasterInputError::LimitExceeded)?;
    let peak = source.resident_accounted_bytes()?.checked_add(maps).and_then(|n| n.checked_add(replacements))
        .and_then(|n| n.checked_add(payload.checked_mul(6)?)).ok_or(RasterInputError::LimitExceeded)?;
    if peak > cap { return Err(RasterInputError::LimitExceeded.into()); }
    source.try_validate_tile_origins_with(control)?;
    let mut result = source.clone();
    for tile in roi.tiles() {
        control.check().map_err(|_| RasterInputError::Cancelled)?;
        let part = tile.rect().intersect(&roi);
        let length = checked_region_byte_len(part, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(length, 0);
        source.try_read_interleaved_into_with(part, &mut bytes, control)?;
        for (index, pixel) in bytes.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) { control.check().map_err(|_| RasterInputError::Cancelled)?; }
            let mut original = [0u8; 32];
            let input = original.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
            input.copy_from_slice(pixel);
            evaluate_pixel_into(format, input, pixel, source_context, expected_context, affine)?;
        }
        result.try_write_interleaved_with(part, &bytes, control)?;
    }
    control.check().map_err(|_| RasterInputError::Cancelled)?;
    Ok(result)
}
