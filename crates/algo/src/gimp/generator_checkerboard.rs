//! Standalone checkerboard candidate from the public named operation contract.
//! Retained own 3.0.8 observations cover 0:0, x=16/17, y=16 and zero offsets.
//! The caller supplies CMS-interpreted native RGB colors and explicit encoding;
//! no profile, color default, Babl pointer, filter composition or admission is inferred.
use photocraft_color::{ColorMode, PixelFormat, try_read_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

/// Named typed values, decoded by the caller using its independently bound schema.
pub enum Argument<'a> {
    Integer { name: &'a str, value: i32 },
    /// Already converted to the caller's explicit working interpretation/format.
    Color { name: &'a str, encoding: SampleEncoding, pixel: &'a [u8] },
}

pub struct Checkerboard {
    cell_width: i32,
    format: PixelFormat,
    encoding: SampleEncoding,
    color1: [u8; 32],
    color2: [u8; 32],
}

impl Checkerboard {
    pub fn from_named(operation: &str, version: &str, format: PixelFormat, encoding: SampleEncoding, arguments: &[Argument<'_>]) -> Result<Self, RasterInputError> {
        if operation != "gegl:checkerboard" || version != "0:0" || arguments.len() != 6
            || format != PixelFormat::new(ColorMode::Rgb, format.sample, true)
        {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let (mut x, mut y, mut x_offset, mut y_offset) = (None, None, None, None);
        let (mut color1, mut color2) = (None, None);
        for argument in arguments {
            match argument {
                Argument::Integer { name, value } => {
                    let destination = match *name {
                        "x" => &mut x, "y" => &mut y, "x-offset" => &mut x_offset, "y-offset" => &mut y_offset,
                        _ => return Err(RasterInputError::UnsupportedConversion),
                    };
                    if destination.replace(*value).is_some() { return Err(RasterInputError::UnsupportedConversion); }
                }
                Argument::Color { name, encoding: color_encoding, pixel } => {
                    if *color_encoding != encoding { return Err(RasterInputError::UnsupportedConversion); }
                    let destination = match *name { "color1" => &mut color1, "color2" => &mut color2, _ => return Err(RasterInputError::UnsupportedConversion) };
                    if destination.replace(*pixel).is_some() { return Err(RasterInputError::UnsupportedConversion); }
                }
            }
        }
        if !matches!(x, Some(16 | 17)) || y != Some(16) || x_offset != Some(0) || y_offset != Some(0) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let cell_width = x.ok_or(RasterInputError::UnsupportedConversion)?;
        let color1 = encoded_color(color1.ok_or(RasterInputError::UnsupportedConversion)?, format)?;
        let color2 = encoded_color(color2.ok_or(RasterInputError::UnsupportedConversion)?, format)?;
        Ok(Self { cell_width, format, encoding, color1, color2 })
    }

    pub fn format(&self) -> PixelFormat { self.format }
    pub fn encoding(&self) -> SampleEncoding { self.encoding }
    pub fn cell_width(&self) -> i32 { self.cell_width }
    pub fn cell_height(&self) -> i32 { 16 }
    pub fn offsets(&self) -> [i32; 2] { [0, 0] }
    pub fn color1_bytes(&self) -> &[u8] { self.color1.get(..self.format.bytes_per_pixel()).unwrap_or(&[]) }
    pub fn color2_bytes(&self) -> &[u8] { self.color2.get(..self.format.bytes_per_pixel()).unwrap_or(&[]) }
}

fn encoded_color(pixel: &[u8], format: PixelFormat) -> Result<[u8; 32], RasterInputError> {
    if pixel.len() != format.bytes_per_pixel() { return Err(RasterInputError::LengthMismatch); }
    for channel in 0..4 {
        let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) || (channel == 3 && value != 1.0) {
            return Err(RasterInputError::UnsupportedConversion);
        }
    }
    let mut encoded = [0u8; 32];
    encoded.get_mut(..pixel.len()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(pixel);
    Ok(encoded)
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Invocation-local peak; the enclosing graph must also reserve aggregate owners.
    pub resident_bytes: u64,
    pub work_units: u64,
}

/// Known output/native-map ownership and conservative overlapping writer scratch.
/// This is not an allocator/RSS measurement or enclosing source admission.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub parameter_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}

fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if control.cancelled() { Err(RasterInputError::Cancelled) } else { Ok(()) }
}

/// Generate only the supplied finite nonnegative ROI, anchored at document origin.
/// The first cell uses color1. Native color bytes are copied without sample math;
/// pixels outside ROI read the explicit transparent-zero default, not an infinite
/// generator. The caller owns clipping/graph extent and subsequent quantization.
/// On refusal/cancellation all private output is discarded.
pub fn requirements(configuration: &Checkerboard, roi: Rect) -> Result<Requirements, RasterInputError> {
    if roi.is_empty() || roi.x0 < 0 || roi.y0 < 0 { return Err(RasterInputError::InvalidBounds); }
    let format = configuration.format;
    let bpp = format.bytes_per_pixel();
    let bytes = checked_region_byte_len(roi, format)?;
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    let columns = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let rows = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let tiles = columns.checked_mul(rows).ok_or(RasterInputError::LimitExceeded)?;
    let tile_bytes = u64::try_from(TILE_SIZE).map_err(|_| RasterInputError::LimitExceeded)?.checked_pow(2)
        .and_then(|count| count.checked_mul(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output_index_bytes = tiles.checked_mul(512).ok_or(RasterInputError::LimitExceeded)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(bpp as u64)).and_then(|n| n.checked_add(std::mem::size_of::<Surface>() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let parameter_retained_bytes = std::mem::size_of::<Checkerboard>() as u64;
    let scratch_bytes = tile_bytes.checked_mul(4).and_then(|n| n.checked_add((bpp as u64).checked_mul(2)?)).ok_or(RasterInputError::LimitExceeded)?;
    let peak = output_retained_bytes.checked_add(output_index_bytes).and_then(|n| n.checked_add(scratch_bytes))
        .and_then(|n| n.checked_add(parameter_retained_bytes)).ok_or(RasterInputError::LimitExceeded)?;
    // Includes copying/initialization and the native writer's growing map walks.
    let work = (bytes as u64).checked_mul(4)
        .and_then(|count| count.checked_add(roi.size().area().checked_mul(16)?))
        .and_then(|count| count.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?))
        .and_then(|count| count.checked_add(tiles.checked_mul(tile_bytes)?.checked_mul(2)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    Ok(Requirements { parameter_retained_bytes, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes: peak, work_units: work })
}

/// Same admitted native generator, with requirements computed before allocation.
pub fn try_generate(configuration: &Checkerboard, roi: Rect, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    poll(control)?;
    let required = requirements(configuration, roi)?;
    let peak = required.peak_resident_bytes;
    let work = required.work_units;
    if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || work > limits.work_units { return Err(RasterInputError::LimitExceeded); }
    let format = configuration.format;
    let bpp = format.bytes_per_pixel();
    let zero = [0u8; 32];
    let mut output = Surface::try_with_default_bytes(format, zero.get(..bpp).ok_or(RasterInputError::LengthMismatch)?)?;
    for tile in roi.tiles() {
        poll(control)?;
        let rect = tile.rect().intersect(&roi);
        let length = checked_region_byte_len(rect, format)?;
        let width = usize::try_from(rect.width()).map_err(|_| RasterInputError::InvalidBounds)?;
        if width == 0 { return Err(RasterInputError::InvalidBounds); }
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if pixels.capacity() > length.checked_mul(2).ok_or(RasterInputError::LimitExceeded)? { return Err(RasterInputError::LimitExceeded); }
        // Bound zero initialization/polls independently of the later color fill.
        for start in (0..length).step_by(32 << 10) {
            poll(control)?;
            pixels.resize(start.checked_add((length - start).min(32 << 10)).ok_or(RasterInputError::LimitExceeded)?, 0);
        }
        for (index, pixel) in pixels.chunks_exact_mut(bpp).enumerate() {
            if index.is_multiple_of(256) { poll(control)?; }
            let x = i64::from(rect.x0) + i64::try_from(index % width).map_err(|_| RasterInputError::InvalidBounds)?;
            let y = i64::from(rect.y0) + i64::try_from(index / width).map_err(|_| RasterInputError::InvalidBounds)?;
            let first_color = (x / i64::from(configuration.cell_width) + y / 16) % 2 == 0;
            let color = if first_color { &configuration.color1 } else { &configuration.color2 };
            pixel.copy_from_slice(color.get(..bpp).ok_or(RasterInputError::LengthMismatch)?);
        }
        output.try_write_interleaved_with(rect, &pixels, control)?;
    }
    poll(control)?;
    Ok(output)
}
