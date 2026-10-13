//! Caller-defined nearest geometry over complete encoded native pixels.
//! Coordinates refer to pixel centers. This primitive performs no sample math,
//! CMS, interpolation, GEGL/version admission or source-space interpretation.
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    backing::{BackingError, ReadControl},
    checked_region_byte_len,
    checked_view::{CheckedRasterView, RasterViewError},
    staged::{SOURCE_CELL_SIZE, StagedSurfaceBuilder, StagingBudget},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ties {
    Lower,
    Upper,
    ToEven,
}

/// Up to one CMYKA binary64 pixel. Only `length` bytes are meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativePixel {
    pub bytes: [u8; 40],
    pub length: u8,
}
impl NativePixel {
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > 40 {
            return Err(RasterInputError::LengthMismatch.into());
        }
        let mut pixel = Self { bytes: [0; 40], length: u8::try_from(bytes.len()).map_err(|_| limit())? };
        pixel.bytes.get_mut(..bytes.len()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(bytes);
        Ok(pixel)
    }
    fn slice(&self, length: usize) -> Result<&[u8]> {
        if usize::from(self.length) != length {
            return Err(RasterInputError::LengthMismatch.into());
        }
        self.bytes.get(..length).ok_or_else(|| RasterInputError::LengthMismatch.into())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abyss {
    Clamp,
    ConstantNative(NativePixel),
}

/// Output center `(x + 0.5, y + 0.5)` to source spatial coordinates. Each
/// multiply and add is a separate unfused binary64 operation. Singular maps
/// are valid direct maps; no inverse, determinant or division is computed.
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    pub xx: f64,
    pub xy: f64,
    pub yx: f64,
    pub yy: f64,
    pub tx: f64,
    pub ty: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub map: Affine,
    pub ties: Ties,
    pub abyss: Abyss,
    pub output_default: NativePixel,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub output_retained_bytes: u64,
    /// Builder descriptors, coverage and conservative native map capacity.
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    View(RasterViewError),
    Coordinate,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native nearest geometry: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
impl From<RasterViewError> for Error {
    fn from(error: RasterViewError) -> Self {
        if matches!(
            error,
            RasterViewError::Resident(RasterInputError::Cancelled)
                | RasterViewError::Control(BackingError::Cancelled)
                | RasterViewError::Backed(BackingError::Cancelled)
        ) {
            Self::Raster(RasterInputError::Cancelled)
        } else {
            Self::View(error)
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn limit() -> Error {
    RasterInputError::LimitExceeded.into()
}
fn bounds() -> Error {
    RasterInputError::InvalidBounds.into()
}
fn poll(control: &Interrupt<'_>) -> Result<()> {
    control.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn finite(value: f64) -> Result<f64> {
    if value.is_finite() { Ok(value) } else { Err(Error::Coordinate) }
}

fn nearest(coordinate: f64, ties: Ties) -> Result<i32> {
    let shifted = finite(coordinate - 0.5)?;
    let lower = shifted.floor();
    let fraction = finite(shifted - lower)?;
    let upper = fraction > 0.5
        || (fraction == 0.5
            && match ties {
                Ties::Lower => false,
                Ties::Upper => true,
                Ties::ToEven => lower.rem_euclid(2.0) != 0.0,
            });
    let selected = finite(if upper { lower + 1.0 } else { lower })?;
    if selected < f64::from(i32::MIN) || selected > f64::from(i32::MAX) {
        return Err(Error::Coordinate);
    }
    // Integral, finite and within i32, checked above; no saturating cast policy.
    Ok(selected as i32)
}
fn mapped(map: Affine, x: i32, y: i32, ties: Ties) -> Result<(i32, i32)> {
    let x = f64::from(x) + 0.5;
    let y = f64::from(y) + 0.5;
    let sx0 = finite(map.xx * x)?;
    let sx1 = finite(map.xy * y)?;
    let sx2 = finite(sx0 + sx1)?;
    let sx = finite(sx2 + map.tx)?;
    let sy0 = finite(map.yx * x)?;
    let sy1 = finite(map.yy * y)?;
    let sy2 = finite(sy0 + sy1)?;
    let sy = finite(sy2 + map.ty)?;
    Ok((nearest(sx, ties)?, nearest(sy, ties)?))
}
fn tile_count(roi: Rect) -> Result<u64> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or_else(bounds)?, roi.y1.checked_sub(1).ok_or_else(bounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(bounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or_else(bounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| bounds())?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| bounds())?;
    width.checked_mul(height).ok_or_else(limit)
}

/// Conservative work includes source descriptor/origin visits, per-pixel raw
/// pin/read/map/copy and staged tile initialization/publication. No color
/// samples are inspected in a separate full-input pass.
pub fn required_work(source: &Surface, roi: Rect) -> Result<u64> {
    required_work_metadata(source.format(), source.tile_count() as u64, roi)
}
fn required_work_metadata(format: PixelFormat, source_tiles: u64, roi: Rect) -> Result<u64> {
    if roi.x1 < roi.x0 || roi.y1 < roi.y0 {
        return Err(bounds());
    }
    let bytes = u64::try_from(checked_region_byte_len(roi, format)?).map_err(|_| limit())?;
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    let tiles = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or_else(limit)?;
    bytes
        .checked_mul(8)
        .and_then(|n| n.checked_add(pixels.checked_mul(64)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(8)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tile_bytes.checked_add(512)?)?))
        .ok_or_else(limit)
}

struct Budget {
    resident: u64,
    work: u64,
}
impl StagingBudget for Budget {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> std::result::Result<(), RasterInputError> {
        self.resident = self.resident.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> std::result::Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn source_resident(source: &Surface, cap: u64, control: &Interrupt<'_>) -> Result<u64> {
    let maps = u64::try_from(source.tile_count()).map_err(|_| limit())?.checked_mul(512).ok_or_else(limit)?;
    let mut bytes = maps.checked_add(source.default_bytes().len() as u64).ok_or_else(limit)?;
    if bytes > cap {
        return Err(limit());
    }
    for (index, (_, tile)) in source.tiles().enumerate() {
        if index.is_multiple_of(256) {
            poll(control)?;
        }
        bytes = bytes.checked_add(tile.bytes().len() as u64).ok_or_else(limit)?;
        if bytes > cap {
            return Err(limit());
        }
    }
    poll(control)?;
    Ok(bytes)
}

fn scratch(length: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    // Charge twice the cell scratch length in the peak; refuse allocator excess.
    if bytes.capacity() > length.checked_mul(2).ok_or_else(limit)? {
        return Err(limit());
    }
    while bytes.len() < length {
        poll(control)?;
        let end = bytes.len().checked_add(8192).ok_or_else(limit)?.min(length);
        bytes.resize(end, 0);
    }
    Ok(bytes)
}

/// Same preflight used by execution: geometry/default checks and cancellable
/// retained-tile metadata census only, never a source pixel/sample scan.
/// Enclosing graph owners must precharge this metadata work before calling it.
pub fn requirements_with_interrupt(source: &Surface, input_extent: Rect, roi: Rect, params: Params, control: &Interrupt<'_>) -> Result<Requirements> {
    poll(control)?;
    if input_extent.is_empty() || input_extent.x1 < input_extent.x0 || input_extent.y1 < input_extent.y0 || roi.x1 < roi.x0 || roi.y1 < roi.y0 {
        return Err(bounds());
    }
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    checked_region_byte_len(input_extent, format)?;
    checked_region_byte_len(roi, format)?;
    params.output_default.slice(bpp)?;
    if let Abyss::ConstantNative(pixel) = &params.abyss {
        pixel.slice(bpp)?;
    }
    for value in [params.map.xx, params.map.xy, params.map.yx, params.map.yy, params.map.tx, params.map.ty] {
        finite(value)?;
    }
    if !roi.is_empty() {
        let last_x = roi.x1.checked_sub(1).ok_or_else(bounds)?;
        let last_y = roi.y1.checked_sub(1).ok_or_else(bounds)?;
        for (x, y) in [(roi.x0, roi.y0), (last_x, roi.y0), (roi.x0, last_y), (last_x, last_y)] {
            mapped(params.map, x, y, params.ties)?;
        }
    }
    let work_units = required_work(source, roi)?;
    let tiles = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let source_retained_bytes = source_resident(source, MAX_CHECKED_RASTER_BYTES, control)?;
    source.try_validate_tile_origins_with(control)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(bpp as u64)).and_then(|n| n.checked_add(256)).ok_or_else(limit)?;
    let output_index_bytes = tiles.checked_mul(512).ok_or_else(limit)?;
    let scratch_bytes = if roi.is_empty() {
        0
    } else {
        (SOURCE_CELL_SIZE as u64)
            .checked_mul(SOURCE_CELL_SIZE as u64)
            .and_then(|n| n.checked_mul(bpp as u64))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(limit)?
    };
    let peak_resident_bytes = source_retained_bytes
        .checked_add(output_retained_bytes)
        .and_then(|n| n.checked_add(output_index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or_else(limit)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(limit());
    }
    poll(control)?;
    Ok(Requirements { source_retained_bytes, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}

/// Source-free preflight only when the independent native ROI planner proves
/// no source pixels are needed. Empty output may use either abyss policy;
/// nonempty output requires explicit native constant abyss, never an image default.
pub fn requirements_no_pixels_with_interrupt(
    format: PixelFormat,
    input_extent: Rect,
    roi: Rect,
    params: Params,
    control: &Interrupt<'_>,
) -> Result<Requirements> {
    use super::geometry_sampling_roi::{self as sampling, SourceMetadata, SourceRequirement};
    poll(control)?;
    let required = sampling::plan_nearest(SourceMetadata { format, extent: input_extent }, roi, params, sampling::Limits { work_units: 128 }, control)
        .map_err(|error| match error {
            sampling::Error::Raster(error) => Error::Raster(error),
            _ => Error::Coordinate,
        })?;
    if required != SourceRequirement::None || (!roi.is_empty() && !matches!(params.abyss, Abyss::ConstantNative(_))) {
        return Err(RasterInputError::UnsupportedConversion.into());
    }
    let bpp = format.bytes_per_pixel();
    params.output_default.slice(bpp)?;
    if let Abyss::ConstantNative(pixel) = params.abyss {
        pixel.slice(bpp)?;
    }
    let tiles = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(256 + bpp as u64)).ok_or_else(limit)?;
    let output_index_bytes = tiles.checked_mul(512).ok_or_else(limit)?;
    let scratch_bytes = if roi.is_empty() {
        0
    } else {
        (SOURCE_CELL_SIZE as u64)
            .checked_mul(SOURCE_CELL_SIZE as u64)
            .and_then(|n| n.checked_mul(bpp as u64))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(limit)?
    };
    let peak_resident_bytes = output_retained_bytes.checked_add(output_index_bytes).and_then(|n| n.checked_add(scratch_bytes)).ok_or_else(limit)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(limit());
    }
    let work_units = required_work_metadata(format, 0, roi)?.checked_add(128).ok_or_else(limit)?;
    poll(control)?;
    Ok(Requirements { source_retained_bytes: 0, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
/// Explicit no-source execution. Every actual mapped pixel is checked again;
/// encountering a source read refuses rather than supplying default/zero pixels.
pub fn try_map_no_pixels(format: PixelFormat, input_extent: Rect, roi: Rect, params: Params, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    let requirements = requirements_no_pixels_with_interrupt(format, input_extent, roi, params, control)?;
    if requirements.work_units > limits.work_units || requirements.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(limit());
    }
    try_map_prepared(None, format, input_extent, roi, params, requirements, limits, control)
}

/// Private staged output, with explicitly declared default outside `roi`.
/// `input_extent` bounds the source's logical pixel domain, including sparse
/// defaults. Clamp applies AFTER explicit nearest selection. Constant abyss
/// and output defaults are complete native pixels, never interpreted colors.
/// NaN, infinity, signed zero, alpha and HDR sample bits are copied unchanged.
/// On any cancellation/refusal, no output is published and source is untouched.
/// The shared builder reserves requested Vec capacities fallibly and prepays
/// bounded standard-library map/Arc publication. It cannot guarantee fallible
/// allocation for those standard-library internals or measure process RSS.
pub fn try_map(source: &Surface, input_extent: Rect, roi: Rect, params: Params, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    let work = required_work(source, roi)?;
    if work > limits.work_units {
        return Err(limit());
    }
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if (source.tile_count() as u64).checked_mul(512).ok_or_else(limit)? > cap {
        return Err(limit());
    }
    let requirements = requirements_with_interrupt(source, input_extent, roi, params, control)?;
    if requirements.peak_resident_bytes > cap {
        return Err(limit());
    }
    try_map_prepared(Some(source), source.format(), input_extent, roi, params, requirements, limits, control)
}

// Both available and unavailable lanes use identical coordinate/sample-copy
// math and staging. Absence is never represented by a fabricated Surface.
#[allow(clippy::too_many_arguments)]
fn try_map_prepared(
    source: Option<&Surface>,
    format: PixelFormat,
    input_extent: Rect,
    roi: Rect,
    params: Params,
    requirements: Requirements,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface> {
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let bpp = format.bytes_per_pixel();
    let output_default = params.output_default.slice(bpp)?;
    // Source retained once; builder reserves full output tiles, descriptor Vec,
    // coverage and prepaid BTreeMap/Arc publication. One cell scratch buffer's
    // actual capacity is limited to the separately reserved 2x allowance.
    let overhead = requirements.source_retained_bytes.checked_add(requirements.scratch_bytes).and_then(|n| n.checked_add(256)).ok_or_else(limit)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let view = source.map(|source| CheckedRasterView::resident_with_control(source, input_extent, &read)).transpose()?;
    if roi.is_empty() {
        poll(control)?;
        return Ok(Surface::try_with_default_bytes(format, output_default)?);
    }
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    let read_map_work =
        pixels.checked_mul(64).and_then(|n| n.checked_add((source.map_or(0, |source| source.tile_count()) as u64).checked_mul(8)?)).ok_or_else(limit)?;
    let mut budget = Budget { resident: cap.checked_sub(overhead).ok_or_else(limit)?, work: limits.work_units.checked_sub(read_map_work).ok_or_else(limit)? };
    let mut output = StagedSurfaceBuilder::new(format, output_default, roi, &mut budget, control)?;
    let mut cell_y = roi.y0;
    while cell_y < roi.y1 {
        let end_y = cell_y.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.y1);
        let mut cell_x = roi.x0;
        while cell_x < roi.x1 {
            poll(control)?;
            let end_x = cell_x.checked_add(SOURCE_CELL_SIZE).ok_or_else(bounds)?.min(roi.x1);
            let part = Rect::new(cell_x, cell_y, end_x, end_y);
            let length = checked_region_byte_len(part, format)?;
            budget.work(length as u64)?;
            let mut encoded = scratch(length, control)?;
            let width = usize::try_from(part.width()).map_err(|_| limit())?;
            for (index, pixel) in encoded.chunks_exact_mut(bpp).enumerate() {
                if index.is_multiple_of(256) {
                    poll(control)?;
                }
                let row = index.checked_div(width).ok_or_else(bounds)?;
                let column = index.checked_rem(width).ok_or_else(bounds)?;
                let x = i32::try_from(i64::from(part.x0) + i64::try_from(column).map_err(|_| bounds())?).map_err(|_| bounds())?;
                let y = i32::try_from(i64::from(part.y0) + i64::try_from(row).map_err(|_| bounds())?).map_err(|_| bounds())?;
                let (sx, sy) = mapped(params.map, x, y, params.ties)?;
                let inside = sx >= input_extent.x0 && sx < input_extent.x1 && sy >= input_extent.y0 && sy < input_extent.y1;
                match params.abyss {
                    Abyss::ConstantNative(constant) if !inside => pixel.copy_from_slice(constant.slice(bpp)?),
                    _ => {
                        let sx = sx.clamp(input_extent.x0, input_extent.x1.checked_sub(1).ok_or_else(bounds)?);
                        let sy = sy.clamp(input_extent.y0, input_extent.y1.checked_sub(1).ok_or_else(bounds)?);
                        let view = view.as_ref().ok_or(RasterInputError::UnsupportedConversion)?;
                        view.read_logical_pixel_native_into(sx, sy, pixel, &read)?;
                    }
                }
            }
            poll(control)?;
            output.push_region(part, &encoded, &mut budget, control)?;
            cell_x = end_x;
        }
        cell_y = end_y;
    }
    poll(control)?;
    let (surface, ()) = output.finalize(&mut budget, control)?.into_parts();
    poll(control)?;
    Ok(surface)
}

// Synthetic raw/geometry drafts only, intentionally unrun; no GIMP oracle or
// source/version/interpolation/profile qualification is claimed.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn identity() -> Affine {
        Affine { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0, tx: 0.0, ty: 0.0 }
    }
    fn params(format: PixelFormat) -> Params {
        Params { map: identity(), ties: Ties::Lower, abyss: Abyss::Clamp, output_default: NativePixel::from_slice(&vec![0; format.bytes_per_pixel()]).unwrap() }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    fn raw(surface: &Surface, x: i32, y: i32) -> Vec<u8> {
        surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()
    }
    #[test]
    fn explicit_no_pixels_constant_preserves_native_bits_and_refuses_any_required_read() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let constant =
            NativePixel::from_slice(&[0x8000000000000000u64, 0x7ff8000000000019].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        let extent = Rect::new(-2, -1, 2, 1);
        let roi = Rect::new(-257, -2, 257, 2);
        let params = Params {
            map: Affine { xx: 0., xy: 0., yx: 0., yy: 0., tx: 1000., ty: 1000. },
            ties: Ties::Lower,
            abyss: Abyss::ConstantNative(constant),
            output_default: NativePixel::from_slice(&[0; 16]).unwrap(),
        };
        let required = requirements_no_pixels_with_interrupt(format, extent, roi, params, &Interrupt::NONE).unwrap();
        assert_eq!(required.source_retained_bytes, 0);
        let output = try_map_no_pixels(
            format,
            extent,
            roi,
            params,
            Limits { resident_bytes: required.peak_resident_bytes, work_units: required.work_units },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 1, 1)).unwrap(), constant.slice(16).unwrap().repeat(2));
        assert_eq!(output.default_bytes(), &[0; 16]);
        assert!(try_map_no_pixels(format, extent, roi, Params { map: identity(), ..params }, limits(), &Interrupt::NONE).is_err());
        assert!(try_map_no_pixels(format, extent, roi, Params { abyss: Abyss::Clamp, ..params }, limits(), &Interrupt::NONE).is_err());
        assert!(try_map_no_pixels(format, extent, roi, params, Limits { work_units: 1, ..limits() }, &Interrupt::NONE).is_err());
        assert!(try_map_no_pixels(format, extent, roi, params, Limits { resident_bytes: 1, ..limits() }, &Interrupt::NONE).is_err());
        assert!(matches!(
            try_map_no_pixels(format, extent, roi, params, limits(), &Interrupt::cancel_only(&|| true)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
    }
    #[test]
    fn identity_copies_all_models_depths_complete_raw_pixels_across_negative_tile_edge() {
        for mode in [
            ColorMode::Bitmap,
            ColorMode::Grayscale,
            ColorMode::Indexed,
            ColorMode::Rgb,
            ColorMode::Cmyk,
            ColorMode::Lab,
            ColorMode::Multichannel,
            ColorMode::Duotone,
        ] {
            for depth in SampleType::ALL {
                let format = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(format);
                let extent = Rect::new(-257, -1, -255, 0);
                let bytes = (0..format.bytes_per_pixel() * 2).map(|n| (n * 37 + 13) as u8).collect::<Vec<_>>();
                source.try_write_interleaved(extent, &bytes).unwrap();
                let before = source.clone();
                let output = try_map(&source, extent, extent, params(format), limits(), &Interrupt::NONE).unwrap();
                assert_eq!(output.try_read_interleaved(extent).unwrap(), bytes);
                assert_eq!(source, before);
            }
        }
    }
    #[test]
    fn u32_adjacent_codes_and_f64_nonfinite_negative_zero_hdr_are_not_decoded() {
        for (depth, bytes) in [
            (SampleType::U32, [16_777_216u32, 16_777_217, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>()),
            (SampleType::F64, [0x8000000000000000u64, 0x7ff8000000000017, 1e100f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>()),
        ] {
            let format = PixelFormat::new(ColorMode::Grayscale, depth, false);
            let mut source = Surface::new(format);
            let extent = Rect::new(0, 0, 3, 1);
            source.try_write_interleaved(extent, &bytes).unwrap();
            let output = try_map(&source, extent, extent, params(format), limits(), &Interrupt::NONE).unwrap();
            assert_eq!(output.try_read_interleaved(extent).unwrap(), bytes);
        }
    }
    #[test]
    fn explicit_ties_apply_to_source_centers_including_negative_indices() {
        for (value, lower, upper, even) in [(0.0, -1, 0, 0), (1.0, 0, 1, 0), (-1.0, -2, -1, -2), (2.0, 1, 2, 2)] {
            assert_eq!(nearest(value, Ties::Lower).unwrap(), lower);
            assert_eq!(nearest(value, Ties::Upper).unwrap(), upper);
            assert_eq!(nearest(value, Ties::ToEven).unwrap(), even);
        }
        assert!(nearest(f64::INFINITY, Ties::Lower).is_err());
        assert!(nearest(f64::from(i32::MAX) + 2.0, Ties::Upper).is_err());
    }
    #[test]
    fn explicit_transpose_and_translation_preserve_u32_ordinals() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(-2, 0, 1, 2);
        let roi = Rect::new(10, 20, 12, 23);
        let bytes = [16_777_217u32, 16_777_218, 16_777_219, u32::MAX, 0, 7].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &bytes).unwrap();
        let mut p = params(format);
        p.map = Affine { xx: 0.0, xy: 1.0, yx: 1.0, yy: 0.0, tx: -22.0, ty: -10.0 };
        let output = try_map(&source, extent, roi, p, limits(), &Interrupt::NONE).unwrap();
        for y in roi.y0..roi.y1 {
            for x in roi.x0..roi.x1 {
                assert_eq!(raw(&output, x, y), raw(&source, y - 22, x - 10));
            }
        }
    }
    #[test]
    fn clamp_constant_and_outside_roi_default_are_distinct() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(-1, 0, 1, 1);
        source.try_write_interleaved(extent, &[11u16, 29].into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>()).unwrap();
        let mut p = params(format);
        p.output_default = NativePixel::from_slice(&91u16.to_ne_bytes()).unwrap();
        let roi = Rect::new(255, 0, 257, 1);
        p.map.tx = -260.0;
        let clamped = try_map(&source, extent, roi, p, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(raw(&clamped, 255, 0), 11u16.to_ne_bytes());
        assert_eq!(raw(&clamped, 254, 0), 91u16.to_ne_bytes());
        p.abyss = Abyss::ConstantNative(NativePixel::from_slice(&57u16.to_ne_bytes()).unwrap());
        let constant = try_map(&source, extent, roi, p, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(raw(&constant, 256, 0), 57u16.to_ne_bytes());
    }
    #[test]
    fn direct_degenerate_map_empty_roi_and_hostile_coordinate_limits() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(0, 0, 1, 1);
        source.try_write_interleaved(extent, &[23]).unwrap();
        let mut p = params(format);
        p.map = Affine { xx: 0.0, xy: 0.0, yx: 0.0, yy: 0.0, tx: 0.5, ty: 0.5 };
        let output = try_map(&source, extent, Rect::new(-2, -2, 2, 2), p, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(raw(&output, -1, -1), [23]);
        assert_eq!(try_map(&source, extent, Rect::new(0, 0, 0, 0), p, limits(), &Interrupt::NONE).unwrap().tile_count(), 0);
        p.map.xx = f64::MAX;
        assert!(try_map(&source, extent, Rect::new(2, 0, 3, 1), p, limits(), &Interrupt::NONE).is_err());
        for restricted in [Limits { resident_bytes: 1, ..limits() }, Limits { work_units: 1, ..limits() }] {
            assert!(matches!(
                try_map(&source, extent, Rect::new(0, 0, 1, 1), params(format), restricted, &Interrupt::NONE),
                Err(Error::Raster(RasterInputError::LimitExceeded))
            ));
        }
        let mut malformed = params(format);
        malformed.output_default.length = 2;
        assert!(try_map(&source, extent, extent, malformed, limits(), &Interrupt::NONE).is_err());
        assert!(try_map(&source, extent, Rect::new(2, 0, 1, 1), params(format), limits(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn late_cancellation_keeps_source_and_never_publishes_partial_output() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let source = Surface::new(format);
        let before = source.clone();
        let extent = Rect::new(0, 0, 512, 8);
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 20;
        let control = Interrupt::cancel_only(&cancel);
        assert!(matches!(try_map(&source, extent, extent, params(format), limits(), &control), Err(Error::Raster(RasterInputError::Cancelled))));
        assert_eq!(source, before);
    }
    #[test]
    fn shared_requirements_allow_exact_peak_and_keep_the_same_geometry_refusals() {
        let format = PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true);
        let source = Surface::new(format);
        let extent = Rect::new(-1, -1, 2, 2);
        let p = params(format);
        let requirements = requirements_with_interrupt(&source, extent, extent, p, &Interrupt::NONE).unwrap();
        assert_eq!(
            requirements.peak_resident_bytes,
            requirements.source_retained_bytes + requirements.output_retained_bytes + requirements.output_index_bytes + requirements.scratch_bytes
        );
        assert!(
            try_map(
                &source,
                extent,
                extent,
                p,
                Limits { resident_bytes: requirements.peak_resident_bytes, work_units: requirements.work_units },
                &Interrupt::NONE
            )
            .is_ok()
        );
        let mut invalid = p;
        invalid.map.tx = f64::INFINITY;
        assert!(matches!(requirements_with_interrupt(&source, extent, extent, invalid, &Interrupt::NONE), Err(Error::Coordinate)));
        assert!(matches!(try_map(&source, extent, extent, invalid, limits(), &Interrupt::NONE), Err(Error::Coordinate)));
    }
}
