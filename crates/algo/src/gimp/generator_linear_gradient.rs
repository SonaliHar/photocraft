//! Original straight-channel linear gradient in an explicit caller working domain.
//! Pixel centers project onto the start/end line. This does not define GEGL
//! versions, gamma, dithering, seeds, defaults or imported-source admission.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    /// Caller-established CMS interpretation identity, not an inferred profile.
    pub profile_identity: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub struct Endpoint<'a> {
    /// Exact one-pixel F32/F64 encoding, already converted by caller CMS.
    pub native: &'a [u8],
    pub context: WorkingContext,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// No wrapping: use the finite projected parameter even outside [0,1].
    /// Alpha must still remain in its explicitly supported unit domain.
    Extrapolate,
    /// Clamp only the projected parameter to [0,1], not finite HDR colors.
    Clamp,
}
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub repeat: Repeat,
}
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub format: PixelFormat,
    pub roi: Rect,
    pub geometry: Geometry,
    pub start: Endpoint<'a>,
    pub end: Endpoint<'a>,
    pub output_context: WorkingContext,
    /// Explicit native default outside ROI, interpreted in output_context.
    pub output_default: &'a [u8],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
/// Conservative invocation-owned resource categories. Parameters are borrowed
/// caller storage; graph owners still retain and debit that storage explicitly.
/// Map/index overlap is deliberately charged twice, not an allocator/RSS claim.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub parameter_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub output_index_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
struct Prepared {
    dx: f64,
    dy: f64,
    norm: f64,
    start: [f64; 4],
    end: [f64; 4],
}

fn pixel_values(bytes: &[u8], format: PixelFormat) -> Result<[f64; 4], RasterInputError> {
    if bytes.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut values = [0.0; 4];
    for (channel, value) in values.iter_mut().take(format.channels()).enumerate() {
        *value = try_read_sample_f64(bytes, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || format.alpha && channel == format.mode.color_channels() && !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(values)
}
fn parameter(request: Request<'_>, prepared: &Prepared, x: f64, y: f64) -> Result<f64, RasterInputError> {
    let sx = x - request.geometry.start[0];
    let sy = y - request.geometry.start[1];
    let px = sx * prepared.dx;
    let py = sy * prepared.dy;
    let dot = px + py;
    let value = dot / prepared.norm;
    if [sx, sy, px, py, dot, value].iter().any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(match request.geometry.repeat {
        Repeat::Extrapolate => value,
        Repeat::Clamp => value.clamp(0.0, 1.0),
    })
}
fn interpolate(request: Request<'_>, prepared: &Prepared, t: f64) -> Result<[f64; 4], RasterInputError> {
    if t == 0.0 {
        return Ok(prepared.start);
    }
    if t == 1.0 {
        return Ok(prepared.end);
    }
    let mut values = [0.0; 4];
    for (channel, value) in values.iter_mut().take(request.format.channels()).enumerate() {
        let a = (1.0 - t) * prepared.start.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
        let b = t * prepared.end.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
        *value = a + b;
        if !a.is_finite()
            || !b.is_finite()
            || !value.is_finite()
            || request.format.sample == SampleType::F32 && !(*value as f32).is_finite()
            || request.format.alpha && channel == request.format.mode.color_channels() && !(0.0..=1.0).contains(value)
        {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(values)
}
fn prepare(request: Request<'_>) -> Result<Prepared, RasterInputError> {
    if !matches!(request.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(request.format.sample, SampleType::F32 | SampleType::F64)
        || request.start.context != request.output_context
        || request.end.context != request.output_context
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(request.roi, request.format)?;
    if request.roi.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    if request.geometry.start.iter().chain(&request.geometry.end).any(|v| !v.is_finite()) {
        return Err(RasterInputError::InvalidBounds);
    }
    let dx = request.geometry.end[0] - request.geometry.start[0];
    let dy = request.geometry.end[1] - request.geometry.start[1];
    let norm = dx * dx + dy * dy;
    if !dx.is_finite() || !dy.is_finite() || !norm.is_finite() || norm <= 0.0 {
        return Err(RasterInputError::InvalidBounds);
    }
    let prepared =
        Prepared { dx, dy, norm, start: pixel_values(request.start.native, request.format)?, end: pixel_values(request.end.native, request.format)? };
    pixel_values(request.output_default, request.format)?;
    // Constant-time finite/domain envelope at ROI pixel-center extrema before
    // allocations. Per-pixel checks still protect rounded intermediate results.
    let last_x = request.roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let last_y = request.roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    for (x, y) in [(request.roi.x0, request.roi.y0), (last_x, request.roi.y0), (request.roi.x0, last_y), (last_x, last_y)] {
        let t = parameter(request, &prepared, f64::from(x) + 0.5, f64::from(y) + 0.5)?;
        interpolate(request, &prepared, t)?;
    }
    Ok(prepared)
}
fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    if roi.is_empty() { return Err(RasterInputError::InvalidBounds); }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}
fn work(request: Request<'_>) -> Result<u64, RasterInputError> {
    let tiles = tile_count(request.roi)?;
    request
        .roi
        .size()
        .area()
        .checked_mul(request.format.channels() as u64)
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)
}
/// O(1) configuration/domain/work preflight, including growing map/writer visits.
pub fn required_work(request: Request<'_>) -> Result<u64, RasterInputError> {
    prepare(request)?;
    work(request)
}
pub(crate) fn generation_requirements(format: PixelFormat, roi: Rect, parameters: u64, work_units: u64) -> Result<Requirements, RasterInputError> {
    checked_region_byte_len(roi, format)?;
    let count = tile_count(roi)?;
    let bpp = format.bytes_per_pixel() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp)).ok_or(RasterInputError::LimitExceeded)?;
    let output_retained_bytes = count.checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(256 + bpp)).ok_or(RasterInputError::LimitExceeded)?;
    let output_index_bytes = count.checked_mul(768).and_then(|n| n.checked_add(1024)).ok_or(RasterInputError::LimitExceeded)?;
    let scratch_bytes = tile_bytes.checked_mul(8).and_then(|n| n.checked_add(4096)).ok_or(RasterInputError::LimitExceeded)?;
    let peak_resident_bytes = output_retained_bytes.checked_add(output_index_bytes).and_then(|n| n.checked_add(scratch_bytes))
        .and_then(|n| n.checked_add(parameters)).ok_or(RasterInputError::LimitExceeded)?;
    if peak_resident_bytes > MAX_CHECKED_RASTER_BYTES { return Err(RasterInputError::LimitExceeded); }
    Ok(Requirements { source_retained_bytes: 0, parameter_retained_bytes: parameters, output_retained_bytes, output_index_bytes, scratch_bytes, peak_resident_bytes, work_units })
}
/// Allocation-free exact configuration and full invocation resource preflight.
pub fn requirements(request: Request<'_>) -> Result<Requirements, RasterInputError> {
    prepare(request)?;
    let parameters = request.start.native.len().checked_add(request.end.native.len()).and_then(|n| n.checked_add(request.output_default.len()))
        .and_then(|n| n.checked_add(std::mem::size_of::<Request<'_>>())).ok_or(RasterInputError::LimitExceeded)?;
    generation_requirements(request.format, request.roi, parameters as u64, work(request)?)
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}

/// Authored ROI-only output. Straight channels (including alpha) use two f64
/// products and a sum, with one final native quantization. Exact t=0/1 copies
/// native endpoint words; no premultiplication, HDR color clamp or CMS guessing.
pub fn try_generate(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    poll(control)?;
    let prepared = prepare(request)?;
    let admitted = requirements(request)?;
    if admitted.work_units > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    if admitted.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(control)?;
    let mut output = Surface::try_with_default_bytes(request.format, request.output_default)?;
    for tile in request.roi.tiles() {
        poll(control)?;
        let area = tile.rect().intersect(&request.roi);
        let length = checked_region_byte_len(area, request.format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        let envelope = (length as u64).checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(RasterInputError::LimitExceeded)?;
        if bytes.capacity() as u64 > envelope || envelope > admitted.scratch_bytes { return Err(RasterInputError::LimitExceeded); }
        while bytes.len() < length {
            poll(control)?;
            bytes.resize(bytes.len().checked_add((length - bytes.len()).min(64 << 10)).ok_or(RasterInputError::LimitExceeded)?, 0);
        }
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                let t = parameter(request, &prepared, f64::from(x) + 0.5, f64::from(y) + 0.5)?;
                let end = cursor.checked_add(request.format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let pixel = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                if t == 0.0 || t == 1.0 {
                    pixel.copy_from_slice(if t == 0.0 { request.start.native } else { request.end.native });
                } else {
                    let values = interpolate(request, &prepared, t)?;
                    for (channel, value) in values.iter().take(request.format.channels()).enumerate() {
                        try_write_sample_f64(pixel, request.format.sample, channel, *value).map_err(|_| RasterInputError::InvalidSample)?;
                    }
                }
                cursor = end;
            }
        }
        output.try_write_interleaved_with(area, &bytes, control)?;
    }
    poll(control)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [7; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 64 << 20, work_units: 1 << 28 }
    }
    #[test]
    fn negative_roi_f64_exact_endpoints_hdr_and_default() {
        let start: Vec<u8> = [-0.0f64, 2.0, -1.0, 0.0].into_iter().flat_map(f64::to_ne_bytes).collect();
        let end: Vec<u8> = [1.0000000000000002f64, 4.0, 3.0, 1.0].into_iter().flat_map(f64::to_ne_bytes).collect();
        let default = [0u8; 32];
        let request = Request {
            format: PixelFormat::RGBA64F,
            roi: Rect::new(-1, 0, 2, 1),
            geometry: Geometry { start: [-0.5, 0.5], end: [1.5, 0.5], repeat: Repeat::Clamp },
            start: Endpoint { native: &start, context: context() },
            end: Endpoint { native: &end, context: context() },
            output_context: context(),
            output_default: &default,
        };
        let output = try_generate(request, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), start);
        assert_eq!(output.try_read_interleaved(Rect::new(1, 0, 2, 1)).unwrap(), end);
        assert_eq!(output.default_bytes(), default);
        let midpoint = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&midpoint, SampleType::F64, 1).unwrap(), 3.0);
        assert_eq!(try_read_sample_f64(&midpoint, SampleType::F64, 3).unwrap(), 0.5);
    }
    #[test]
    fn context_geometry_extrapolated_alpha_work_and_cancel_refuse() {
        let start: Vec<u8> = [0f32, 0., 0., 0.].into_iter().flat_map(f32::to_ne_bytes).collect();
        let end: Vec<u8> = [1f32, 1., 1., 1.].into_iter().flat_map(f32::to_ne_bytes).collect();
        let default = [0u8; 16];
        let request = Request {
            format: PixelFormat::RGBA32F,
            roi: Rect::new(0, 0, 2, 1),
            geometry: Geometry { start: [0.5, 0.5], end: [1.5, 0.5], repeat: Repeat::Clamp },
            start: Endpoint { native: &start, context: context() },
            end: Endpoint { native: &end, context: context() },
            output_context: context(),
            output_default: &default,
        };
        let mut changed = request;
        changed.end.context.profile_identity = [8; 32];
        assert!(matches!(try_generate(changed, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        changed = request;
        changed.geometry.end = changed.geometry.start;
        assert!(matches!(try_generate(changed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        changed = request;
        changed.geometry.repeat = Repeat::Extrapolate;
        changed.roi = Rect::new(-1, 0, 2, 1);
        assert!(matches!(try_generate(changed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        assert!(matches!(try_generate(request, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert!(matches!(try_generate(request, limits(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
    }
}
