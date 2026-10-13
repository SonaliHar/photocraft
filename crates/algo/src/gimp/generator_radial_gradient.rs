//! Caller-defined elliptic radial gradient, not a serialized GEGL operation.
//! Straight finite HDR channels; no gamma, premultiplication or dithering.
pub use super::generator_linear_gradient::{Endpoint, Limits, Repeat, Requirements, WorkingContext};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct Geometry { pub center: [f64; 2], pub radii: [f64; 2], pub repeat: Repeat }
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub format: PixelFormat,
    pub roi: Rect,
    pub geometry: Geometry,
    pub start: Endpoint<'a>,
    pub end: Endpoint<'a>,
    pub output_context: WorkingContext,
    pub output_default: &'a [u8],
}
struct Prepared { start: [f64; 4], end: [f64; 4] }
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> { control.check().map_err(|_| RasterInputError::Cancelled) }
fn values(bytes: &[u8], format: PixelFormat) -> Result<[f64; 4], RasterInputError> {
    if bytes.len() != format.bytes_per_pixel() { return Err(RasterInputError::LengthMismatch); }
    let mut result = [0.0; 4];
    for (channel, value) in result.iter_mut().take(format.channels()).enumerate() {
        *value = try_read_sample_f64(bytes, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || format.alpha && channel == format.mode.color_channels() && !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(result)
}
fn parameter(geometry: Geometry, x: i32, y: i32) -> Result<f64, RasterInputError> {
    let dx = (f64::from(x) + 0.5) - geometry.center[0];
    let dy = (f64::from(y) + 0.5) - geometry.center[1];
    let nx = dx / geometry.radii[0];
    let ny = dy / geometry.radii[1];
    let xx = nx * nx;
    let yy = ny * ny;
    let sum = xx + yy;
    let distance = sum.sqrt();
    if [dx, dy, nx, ny, xx, yy, sum, distance].iter().any(|value| !value.is_finite()) { return Err(RasterInputError::InvalidSample); }
    Ok(match geometry.repeat { Repeat::Clamp => distance.clamp(0.0, 1.0), Repeat::Extrapolate => distance })
}
fn interpolate(request: Request<'_>, prepared: &Prepared, t: f64) -> Result<[f64; 4], RasterInputError> {
    if t == 0.0 { return Ok(prepared.start); }
    if t == 1.0 { return Ok(prepared.end); }
    let mut result = [0.0; 4];
    for (channel, value) in result.iter_mut().take(request.format.channels()).enumerate() {
        let first = (1.0 - t) * prepared.start.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
        let second = t * prepared.end.get(channel).copied().ok_or(RasterInputError::LengthMismatch)?;
        *value = first + second;
        if !first.is_finite() || !second.is_finite() || !value.is_finite()
            || request.format.sample == SampleType::F32 && !(*value as f32).is_finite()
            || request.format.alpha && channel == request.format.mode.color_channels() && !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(result)
}
fn tiles(roi: Rect) -> Result<u64, RasterInputError> {
    if roi.is_empty() { return Err(RasterInputError::InvalidBounds); }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last = TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    for coordinate in [first, last] {
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|n| n.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
    }
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}
fn prepare(request: Request<'_>) -> Result<Prepared, RasterInputError> {
    if !matches!(request.format.mode, ColorMode::Rgb | ColorMode::Grayscale) || !matches!(request.format.sample, SampleType::F32 | SampleType::F64)
        || request.start.context != request.output_context || request.end.context != request.output_context {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(request.roi, request.format)?;
    tiles(request.roi)?;
    if request.geometry.center.iter().any(|value| !value.is_finite()) || request.geometry.radii.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err(RasterInputError::InvalidBounds);
    }
    let prepared = Prepared { start: values(request.start.native, request.format)?, end: values(request.end.native, request.format)? };
    values(request.output_default, request.format)?;
    // Maximum elliptic distance over the rectangle is at a corner. Start and
    // corner interpolation also bound extrapolated affine alpha before allocation.
    // Per-pixel checks retain protection against rounded intermediate overflow.
    let last_x = request.roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    let last_y = request.roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?;
    for (x, y) in [(request.roi.x0, request.roi.y0), (last_x, request.roi.y0), (request.roi.x0, last_y), (last_x, last_y)] {
        interpolate(request, &prepared, parameter(request.geometry, x, y)?)?;
    }
    Ok(prepared)
}
fn work(request: Request<'_>) -> Result<u64, RasterInputError> {
    let bytes = checked_region_byte_len(request.roi, request.format)? as u64;
    let count = tiles(request.roi)?;
    bytes.checked_mul(128).and_then(|n| n.checked_add(count.checked_mul(count)?.checked_mul(256)?)).ok_or(RasterInputError::LimitExceeded)
}
/// O(1) geometry, endpoint, corner arithmetic and growing-map work admission.
pub fn required_work(request: Request<'_>) -> Result<u64, RasterInputError> { prepare(request)?; work(request) }
/// Allocation-free exact configuration and resource preflight, shared with execution.
pub fn requirements(request: Request<'_>) -> Result<Requirements, RasterInputError> {
    prepare(request)?;
    let parameters = request.start.native.len().checked_add(request.end.native.len()).and_then(|n| n.checked_add(request.output_default.len()))
        .and_then(|n| n.checked_add(std::mem::size_of::<Request<'_>>())).ok_or(RasterInputError::LimitExceeded)?;
    super::generator_linear_gradient::generation_requirements(request.format, request.roi, parameters as u64, work(request)?)
}

/// Pixel centers are (x+0.5,y+0.5). Parameter = sqrt(nx*nx + ny*ny), where
/// nx=(centerX-callerCenterX)/radiusX and likewise Y. These are separate unfused
/// f64 operations, not hypot or an inferred library schedule. Straight channels
/// use two products and a sum; native quantization happens only at final encode.
/// Exact t=0/1 copies raw endpoint words, preserving signed zero and F64 low bits.
/// Clamp limits only the parameter; Extrapolate never clamps HDR colors or alpha.
/// Finite HDR colors and unit alpha are mandatory, including the raw default.
/// Only ROI is generated; outside reads the exact caller default. Partial output
/// is private and discarded on any refusal/cancellation.
pub fn try_generate(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    poll(control)?;
    let prepared = prepare(request)?;
    let admitted = requirements(request)?;
    if admitted.work_units > limits.work_units { return Err(RasterInputError::LimitExceeded); }
    let bpp = request.format.bytes_per_pixel();
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let scratch_envelope = tile_bytes.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or(RasterInputError::LimitExceeded)?;
    if admitted.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) { return Err(RasterInputError::LimitExceeded); }
    poll(control)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(tile_bytes).map_err(|_| RasterInputError::LimitExceeded)?).map_err(|_| RasterInputError::AllocationFailed)?;
    if bytes.capacity() as u64 > scratch_envelope { return Err(RasterInputError::LimitExceeded); }
    let mut output = Surface::try_with_default_bytes(request.format, request.output_default)?;
    for coordinate in request.roi.tiles() {
        poll(control)?;
        let area = coordinate.rect().intersect(&request.roi);
        let length = checked_region_byte_len(area, request.format)?;
        bytes.truncate(length);
        while bytes.len() < length {
            poll(control)?;
            bytes.resize(bytes.len().checked_add((length - bytes.len()).min(64 << 10)).ok_or(RasterInputError::LimitExceeded)?, 0);
        }
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                let t = parameter(request.geometry, x, y)?;
                let end = cursor.checked_add(bpp).ok_or(RasterInputError::LimitExceeded)?;
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
    use photocraft_doc::SampleEncoding;
    fn context() -> WorkingContext { WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [5; 32] } }
    fn limits() -> Limits { Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 } }
    fn request<'a>(start: &'a [u8], end: &'a [u8], default: &'a [u8], format: PixelFormat) -> Request<'a> {
        Request { format, roi: Rect { x0: 0, y0: 0, x1: 2, y1: 2 }, geometry: Geometry { center: [0.5, 0.5], radii: [1.0, 1.0], repeat: Repeat::Clamp }, start: Endpoint { native: start, context: context() }, end: Endpoint { native: end, context: context() }, output_context: context(), output_default: default }
    }

    #[test]
    fn isotropic_and_elliptic_distances_have_independent_exact_axis_controls() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let start = (-2.0f64).to_ne_bytes();
        let end = 4.0f64.to_ne_bytes();
        let default = (-0.0f64).to_ne_bytes();
        let mut input = request(&start, &end, &default, format);
        let output = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(input.roi).unwrap(), vec![-2.0, 4.0, 4.0, 4.0]);
        input.geometry.radii = [1.0, 2.0];
        let output = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(input.roi).unwrap(), vec![-2.0, 4.0, 1.0, 4.0]);
        assert_eq!(output.default_bytes(), default);
    }

    #[test]
    fn negative_roi_center_and_radius_copy_exact_raw_endpoints() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false);
        let start = (-0.0f64).to_ne_bytes();
        let end = (1.0f64.to_bits() + 1).to_ne_bytes();
        let default = (-2.0f64).to_ne_bytes();
        let mut input = request(&start, &end, &default, format);
        input.roi = Rect { x0: -1, y0: -1, x1: 1, y1: 0 };
        input.geometry.center = [-0.5, -0.5];
        let output = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        let mut expected = start.to_vec(); expected.extend_from_slice(&end);
        assert_eq!(output.try_read_interleaved(input.roi).unwrap(), expected);
        assert_eq!(output.try_read_interleaved(Rect { x0: 0, y0: 0, x1: 1, y1: 1 }).unwrap(), default);
    }

    #[test]
    fn straight_alpha_and_extrapolation_refusal_are_explicit() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let mut start = Vec::new();
        let mut end = Vec::new();
        for value in [-2.0f64, 0.25] { start.extend_from_slice(&value.to_ne_bytes()); }
        for value in [4.0f64, 0.75] { end.extend_from_slice(&value.to_ne_bytes()); }
        let default = [0u8; 16];
        let mut input = request(&start, &end, &default, format);
        input.geometry.radii = [2.0, 2.0];
        input.roi = Rect { x0: 0, y0: 0, x1: 3, y1: 1 };
        let output = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_region_f64(input.roi).unwrap(), vec![-2.0, 0.25, 1.0, 0.5, 4.0, 0.75]);
        input.geometry.radii = [1.0, 1.0];
        input.geometry.repeat = Repeat::Extrapolate;
        assert!(matches!(try_generate(input, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        input.geometry.repeat = Repeat::Clamp;
        assert!(try_generate(input, limits(), &Interrupt::NONE).is_ok());
    }

    #[test]
    fn invalid_radii_overflow_context_limits_and_cancellation_refuse() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let start = 0.0f32.to_ne_bytes();
        let end = 1.0f32.to_ne_bytes();
        let mut input = request(&start, &end, &start, format);
        for radii in [[0.0, 1.0], [-1.0, 1.0], [f64::NAN, 1.0], [f64::MIN_POSITIVE, f64::MIN_POSITIVE]] {
            input.geometry.radii = radii;
            assert!(try_generate(input, limits(), &Interrupt::NONE).is_err());
        }
        input.geometry.radii = [1.0, 1.0];
        let mut bad_context = input;
        bad_context.end.context.profile_identity = [6; 32];
        assert!(matches!(try_generate(bad_context, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        assert!(matches!(try_generate(input, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4;
        assert!(matches!(try_generate(input, limits(), &Interrupt::cancel_only(&cancel)), Err(RasterInputError::Cancelled)));
        let first = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        let second = try_generate(input, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(first.try_read_interleaved(input.roi).unwrap(), second.try_read_interleaved(input.roi).unwrap());
    }
}
