//! Caller-declared orthogonal repeat/mirror over complete native pixel bytes.
//! No GIMP tile/fractal/version/default extent, interpolation or CMS admission.
use super::geometry_nearest::{self, Limits, NativePixel};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    backing::{BackingError, ReadControl},
    checked_region_byte_len,
    checked_view::{CheckedRasterView, RasterViewError},
    staged::{SOURCE_CELL_SIZE, StagedSurfaceBuilder, StagingBudget},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Repeat,
    /// Endpoint-repeating mirror: length 3 produces 0,1,2,2,1,0,0,1,...
    /// Its period is twice the input length, including for length 1.
    Mirror,
}
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Output coordinate where the first input pixel occurs, independently per
    /// axis. The origin is not inferred from source/output bounds or metadata.
    pub origin: [i32; 2],
    pub horizontal: Axis,
    pub vertical: Axis,
    /// Complete raw output default outside the declared output ROI.
    pub output_default: NativePixel,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    View(RasterViewError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native repeat geometry: {self:?}")
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
impl From<geometry_nearest::Error> for Error {
    fn from(error: geometry_nearest::Error) -> Self {
        match error {
            geometry_nearest::Error::Raster(error) => Self::Raster(error),
            geometry_nearest::Error::View(error) => Self::from(error),
            geometry_nearest::Error::Coordinate => Self::Raster(RasterInputError::InvalidBounds),
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
fn valid_rect(rect: Rect, nonempty: bool) -> Result<()> {
    if rect.x1 < rect.x0 || rect.y1 < rect.y0 || (nonempty && rect.is_empty()) {
        return Err(bounds());
    }
    Ok(())
}

fn coordinate(output: i32, origin: i32, input_min: i32, length: u32, axis: Axis) -> Result<i32> {
    if length == 0 {
        return Err(bounds());
    }
    let length = i64::from(length);
    let period = match axis {
        Axis::Repeat => length,
        Axis::Mirror => length.checked_mul(2).ok_or_else(bounds)?,
    };
    let offset = i64::from(output).checked_sub(i64::from(origin)).ok_or_else(bounds)?;
    // Positive checked period, so Euclidean modulo cannot divide by zero or
    // encounter signed MIN / -1; negative output offsets retain exact phase.
    let phase = offset.checked_rem_euclid(period).ok_or_else(bounds)?;
    let ordinal = match axis {
        Axis::Repeat => phase,
        Axis::Mirror if phase >= length => period.checked_sub(1).and_then(|n| n.checked_sub(phase)).ok_or_else(bounds)?,
        Axis::Mirror => phase,
    };
    if ordinal < 0 || ordinal >= length {
        return Err(bounds());
    }
    i32::try_from(i64::from(input_min).checked_add(ordinal).ok_or_else(bounds)?).map_err(|_| bounds())
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
fn resident(source: &Surface, cap: u64, control: &Interrupt<'_>) -> Result<u64> {
    let mut bytes = (source.tile_count() as u64).checked_mul(512).and_then(|n| n.checked_add(source.default_bytes().len() as u64)).ok_or_else(limit)?;
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
fn scratch(length: usize, control: &Interrupt<'_>) -> Result<Vec<u8>> {
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
    if bytes.capacity() > length.checked_mul(2).ok_or_else(limit)? {
        return Err(limit());
    }
    while bytes.len() < length {
        poll(control)?;
        bytes.resize(bytes.len().checked_add(8192).ok_or_else(limit)?.min(length), 0);
    }
    Ok(bytes)
}
/// Same conservative native read/map/staging schedule as the nearest primitive;
/// integer modulo/reflection replaces affine arithmetic without sample decode.
pub fn required_work(source: &Surface, roi: Rect) -> Result<u64> {
    Ok(geometry_nearest::required_work(source, roi)?)
}

/// Same allocation-free geometry/default and retained metadata preflight used
/// by execution. No source pixels are decoded or scanned; enclosing graph
/// callers must charge these cancellable metadata walks before invoking it.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
pub fn requirements_with_interrupt(source: &Surface, input_extent: Rect, roi: Rect, params: Params, control: &Interrupt<'_>) -> Result<Requirements> {
    requirements_with_cap(source, input_extent, roi, params, MAX_CHECKED_RASTER_BYTES, control)
}
fn requirements_with_cap(source: &Surface, input_extent: Rect, roi: Rect, params: Params, cap: u64, control: &Interrupt<'_>) -> Result<Requirements> {
    poll(control)?;
    valid_rect(input_extent, true)?;
    valid_rect(roi, false)?;
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    checked_region_byte_len(input_extent, format)?;
    checked_region_byte_len(roi, format)?;
    if usize::from(params.output_default.length) != bpp {
        return Err(RasterInputError::LengthMismatch.into());
    }
    params.output_default.bytes.get(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    // Validate both periods and destination coordinates before allocating output.
    coordinate(roi.x0, params.origin[0], input_extent.x0, input_extent.width(), params.horizontal)?;
    coordinate(roi.y0, params.origin[1], input_extent.y0, input_extent.height(), params.vertical)?;
    let work = required_work(source, roi)?;
    let source_bytes = resident(source, cap, control)?;
    let tiles = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or_else(limit)?;
    let outputs = tiles.checked_mul(tile_bytes.checked_add(512).ok_or_else(limit)?).ok_or_else(limit)?;
    let scratch_bytes = if roi.is_empty() {
        0
    } else {
        (SOURCE_CELL_SIZE as u64)
            .checked_mul(SOURCE_CELL_SIZE as u64)
            .and_then(|n| n.checked_mul(bpp as u64))
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(limit)?
    };
    let overhead = source_bytes.checked_add(scratch_bytes).and_then(|n| n.checked_add(256)).ok_or_else(limit)?;
    let peak_resident_bytes = overhead.checked_add(outputs).and_then(|n| n.checked_add(bpp as u64)).ok_or_else(limit)?;
    if peak_resident_bytes > cap {
        return Err(limit());
    }
    source.try_validate_tile_origins_with(control)?;
    poll(control)?;
    Ok(Requirements {
        source_retained_bytes: source_bytes,
        output_retained_bytes: tiles.checked_mul(tile_bytes).and_then(|n| n.checked_add(256 + bpp as u64)).ok_or_else(limit)?,
        output_index_bytes: tiles.checked_mul(512).ok_or_else(limit)?,
        scratch_bytes,
        peak_resident_bytes,
        work_units: work,
    })
}

/// Private tiled output in exactly the source format. The input logical extent
/// defines the nonzero periods, including sparse defaults. All sample bytes,
/// including U32/F64 precision, nonfinite payloads, HDR and raw alpha, are copied
/// without arithmetic. Outside ROI reads the caller's complete native default.
/// Source remains unchanged on cancellation/refusal; partial output is dropped.
/// Requested Vec capacities are fallibly reserved by the shared staged builder;
/// bounded standard map/Arc publication is prepaid, not full allocator/RSS control.
pub fn try_repeat(source: &Surface, input_extent: Rect, roi: Rect, params: Params, limits: Limits, control: &Interrupt<'_>) -> Result<Surface> {
    poll(control)?;
    if required_work(source, roi)? > limits.work_units {
        return Err(limit());
    }
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    let footprint = requirements_with_cap(source, input_extent, roi, params, cap, control)?;
    if footprint.work_units > limits.work_units || footprint.peak_resident_bytes > cap {
        return Err(limit());
    }
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let default = params.output_default.bytes.get(..bpp).ok_or(RasterInputError::LengthMismatch)?;
    let overhead = footprint.source_retained_bytes.checked_add(footprint.scratch_bytes).and_then(|n| n.checked_add(256 + bpp as u64)).ok_or_else(limit)?;
    let read = ReadControl { interrupt: control, work: &|_| Ok(()) };
    let view = CheckedRasterView::resident_with_control(source, input_extent, &read)?;
    if roi.is_empty() {
        poll(control)?;
        return Ok(Surface::try_with_default_bytes(format, default)?);
    }
    let pixels = u64::from(roi.width()).checked_mul(u64::from(roi.height())).ok_or_else(limit)?;
    let read_map_work = pixels.checked_mul(64).and_then(|n| n.checked_add((source.tile_count() as u64).checked_mul(8)?)).ok_or_else(limit)?;
    let mut budget = Budget { resident: cap.checked_sub(overhead).ok_or_else(limit)?, work: limits.work_units.checked_sub(read_map_work).ok_or_else(limit)? };
    let mut output = StagedSurfaceBuilder::new(format, default, roi, &mut budget, control)?;
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
                let sx = coordinate(x, params.origin[0], input_extent.x0, input_extent.width(), params.horizontal)?;
                let sy = coordinate(y, params.origin[1], input_extent.y0, input_extent.height(), params.vertical)?;
                view.read_logical_pixel_native_into(sx, sy, pixel, &read)?;
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

// Unrun synthetic period/raw-sample drafts; no stock operation qualification.
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn params(format: PixelFormat) -> Params {
        Params {
            origin: [0, 0],
            horizontal: Axis::Repeat,
            vertical: Axis::Repeat,
            output_default: NativePixel::from_slice(&vec![0; format.bytes_per_pixel()]).unwrap(),
        }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 40 }
    }
    fn raw(surface: &Surface, x: i32, y: i32) -> Vec<u8> {
        surface.try_read_interleaved(Rect::new(x, y, x + 1, y + 1)).unwrap()
    }
    #[test]
    fn negative_phases_and_endpoint_mirror_seams_are_explicit() {
        let repeat = (-7..8).map(|x| coordinate(x, 0, -2, 3, Axis::Repeat).unwrap()).collect::<Vec<_>>();
        assert_eq!(repeat, [0, -2, -1, 0, -2, -1, 0, -2, -1, 0, -2, -1, 0, -2, -1]);
        let mirror = (-7..8).map(|x| coordinate(x, 0, 0, 3, Axis::Mirror).unwrap()).collect::<Vec<_>>();
        assert_eq!(mirror, [0, 0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0, 0, 1]);
        for x in -7..8 {
            assert_eq!(coordinate(x, 0, -19, 1, Axis::Mirror).unwrap(), -19);
        }
    }
    #[test]
    fn raw_repeat_all_formats_and_depths_with_negative_input_and_output_origin() {
        let extent = Rect::new(-4, -2, -2, -1);
        let roi = Rect::new(255, -1, 258, 1);
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
                let bytes = (0..format.bytes_per_pixel() * 2).map(|n| (n * 29 + 11) as u8).collect::<Vec<_>>();
                source.try_write_interleaved(extent, &bytes).unwrap();
                let before = source.clone();
                let mut p = params(format);
                p.origin = [256, 0];
                let output = try_repeat(&source, extent, roi, p, limits(), &Interrupt::NONE).unwrap();
                assert_eq!(raw(&output, 255, -1), raw(&source, -3, -2));
                assert_eq!(raw(&output, 256, 0), raw(&source, -4, -2));
                assert_eq!(raw(&output, 257, 0), raw(&source, -3, -2));
                assert_eq!(source, before);
            }
        }
    }
    #[test]
    fn two_axis_mirror_preserves_u32_precision_and_complete_alpha() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U32, true);
        let mut source = Surface::new(format);
        let extent = Rect::new(-1, -1, 1, 1);
        let bytes = [16_777_217u32, 1, 16_777_218, 2, u32::MAX, 3, 0, 4].into_iter().flat_map(u32::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &bytes).unwrap();
        let mut p = params(format);
        p.horizontal = Axis::Mirror;
        p.vertical = Axis::Mirror;
        let output = try_repeat(&source, extent, Rect::new(-2, -2, 4, 4), p, limits(), &Interrupt::NONE).unwrap();
        let axis = [0, -1, -1, 0, 0, -1];
        for (row, y) in (-2..4).enumerate() {
            for (column, x) in (-2..4).enumerate() {
                assert_eq!(raw(&output, x, y), raw(&source, axis[column], axis[row]));
            }
        }
    }
    #[test]
    fn f64_nan_hdr_signed_zero_and_outside_roi_default_copy_exactly() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let mut source = Surface::new(format);
        let extent = Rect::new(0, 0, 3, 1);
        let bytes = [0x8000000000000000u64, 0x7ff8000000000017, 1e100f64.to_bits()].into_iter().flat_map(u64::to_ne_bytes).collect::<Vec<_>>();
        source.try_write_interleaved(extent, &bytes).unwrap();
        let mut p = params(format);
        p.output_default = NativePixel::from_slice(&0x7ff0000000000000u64.to_ne_bytes()).unwrap();
        let output = try_repeat(&source, extent, Rect::new(-3, 0, 3, 1), p, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-3, 0, 0, 1)).unwrap(), bytes);
        assert_eq!(raw(&output, 3, 0), 0x7ff0000000000000u64.to_ne_bytes());
    }
    #[test]
    fn extreme_origin_zero_period_invalid_bounds_limits_and_empty_roi() {
        assert_eq!(coordinate(i32::MAX, i32::MIN, -1, 3, Axis::Repeat).unwrap(), -1);
        assert_eq!(coordinate(i32::MIN, i32::MAX, 0, 3, Axis::Repeat).unwrap(), 0);
        assert!(coordinate(0, 0, 0, 0, Axis::Mirror).is_err());
        assert!(coordinate(1, 0, i32::MAX, 2, Axis::Repeat).is_err());
        let source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        let extent = Rect::new(0, 0, 1, 1);
        let p = params(source.format());
        assert_eq!(try_repeat(&source, extent, Rect::new(0, 0, 0, 0), p, limits(), &Interrupt::NONE).unwrap().tile_count(), 0);
        assert!(try_repeat(&source, Rect::new(1, 0, 0, 1), extent, p, limits(), &Interrupt::NONE).is_err());
        for restricted in [Limits { resident_bytes: 1, ..limits() }, Limits { work_units: 1, ..limits() }] {
            assert!(matches!(try_repeat(&source, extent, extent, p, restricted, &Interrupt::NONE), Err(Error::Raster(RasterInputError::LimitExceeded))));
        }
    }
    #[test]
    fn cancellation_keeps_input_and_drops_private_output() {
        let source = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F64, true));
        let before = source.clone();
        let extent = Rect::new(-3, -2, 4, 2);
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 40;
        assert!(matches!(
            try_repeat(&source, extent, Rect::new(-257, -2, 257, 2), params(source.format()), limits(), &Interrupt::cancel_only(&cancel)),
            Err(Error::Raster(RasterInputError::Cancelled))
        ));
        assert_eq!(source, before);
    }
}
