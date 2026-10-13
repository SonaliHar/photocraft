//! Original straight-alpha source-over in a caller-established working domain.
//! No profile/TRC conversion, opacity parameter, premultiplication assumption,
//! GEGL version, GIMP blend-space mapping or imported-source admission is added.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
    /// Exact caller-established common coordinate space; no offset is inferred.
    pub coordinate_space: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub enum Outside<'a> {
    Reject,
    Constant(&'a [u8]),
}
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub surface: &'a Surface,
    pub extent: Rect,
    pub context: WorkingContext,
    pub outside: Outside<'a>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroAlpha {
    /// When both inputs have zero alpha, copy the complete background pixel.
    PreserveBackgroundRaw,
    /// Return native positive-zero colors and alpha for that pixel.
    ZeroColors,
    Reject,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    NearestTiesUp,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub foreground: Input<'a>,
    pub background: Input<'a>,
    pub roi: Rect,
    pub output_context: WorkingContext,
    pub zero_alpha: ZeroAlpha,
    pub rounding: Rounding,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
/// Allocation-free standard kernel footprint. Source payload sharing is charged
/// conservatively per live owner, matching the kernel's existing lower ceiling.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub work_units: u64,
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub resident_bytes: u64,
    /// Persistent ROI-only graph surface, including tile/default/map witnesses.
    pub roi_output_bytes: u64,
    pub roi_tiles: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn maximum(sample: SampleType) -> Option<u32> {
    match sample {
        SampleType::U8 => Some(u32::from(u8::MAX)),
        SampleType::U16 => Some(u32::from(u16::MAX)),
        SampleType::U32 => Some(u32::MAX),
        _ => None,
    }
}
fn range(sample: SampleType, channel: usize) -> Result<std::ops::Range<usize>, RasterInputError> {
    let start = channel.checked_mul(sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
    Ok(start..start.checked_add(sample.bytes()).ok_or(RasterInputError::LengthMismatch)?)
}
fn integer(bytes: &[u8], sample: SampleType, channel: usize) -> Result<u32, RasterInputError> {
    let bytes = bytes.get(range(sample, channel)?).ok_or(RasterInputError::LengthMismatch)?;
    Ok(match sample {
        SampleType::U8 => u32::from(*bytes.first().ok_or(RasterInputError::LengthMismatch)?),
        SampleType::U16 => u32::from(u16::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        SampleType::U32 => u32::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?),
        _ => return Err(RasterInputError::UnsupportedConversion),
    })
}
fn store_integer(bytes: &mut [u8], sample: SampleType, channel: usize, value: u32) -> Result<(), RasterInputError> {
    let bytes = bytes.get_mut(range(sample, channel)?).ok_or(RasterInputError::LengthMismatch)?;
    match sample {
        SampleType::U8 => bytes.copy_from_slice(&[u8::try_from(value).map_err(|_| RasterInputError::InvalidSample)?]),
        SampleType::U16 => bytes.copy_from_slice(&u16::try_from(value).map_err(|_| RasterInputError::InvalidSample)?.to_ne_bytes()),
        SampleType::U32 => bytes.copy_from_slice(&value.to_ne_bytes()),
        _ => return Err(RasterInputError::UnsupportedConversion),
    }
    Ok(())
}
fn floats(pixel: &[u8], format: PixelFormat) -> Result<[f64; 4], RasterInputError> {
    if pixel.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    let mut output = [0.0; 4];
    for (channel, value) in output.iter_mut().take(format.channels()).enumerate() {
        *value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || channel == format.mode.color_channels() && !(0.0..=1.0).contains(value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(output)
}
fn validate_pixel(pixel: &[u8], format: PixelFormat) -> Result<(), RasterInputError> {
    if pixel.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    if maximum(format.sample).is_none() {
        floats(pixel, format)?;
    }
    Ok(())
}
fn validate(request: Request<'_>) -> Result<(), RasterInputError> {
    let format = request.background.surface.format();
    if format != request.foreground.surface.format()
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !format.alpha
        || request.foreground.context != request.output_context
        || request.background.context != request.output_context
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(request.roi, format)?;
    if request.roi.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    for input in [request.foreground, request.background] {
        checked_region_byte_len(input.extent, format)?;
        match input.outside {
            Outside::Reject if !input.extent.contains_rect(&request.roi) => return Err(RasterInputError::InvalidBounds),
            Outside::Constant(pixel) => validate_pixel(pixel, format)?,
            _ => {}
        }
        validate_pixel(input.surface.default_bytes(), format)?;
    }
    match request.rounding {
        Rounding::NearestTiesUp => {}
    }
    Ok(())
}
fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}
/// O(1) sample and repeated growing-map/writer work census before cloning.
pub fn required_work(request: Request<'_>) -> Result<u64, RasterInputError> {
    validate(request)?;
    let foreground = request.foreground.surface.tile_count() as u64;
    let background = request.background.surface.tile_count() as u64;
    let tiles = tile_count(request.roi)?;
    request
        .roi
        .size()
        .area()
        .checked_mul(request.background.surface.format().bytes_per_pixel() as u64)
        .and_then(|n| n.checked_mul(128))
        .and_then(|n| n.checked_add(tiles.checked_mul(background.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(foreground.checked_add(background)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)
}

pub fn requirements(request: Request<'_>, control: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    poll(control)?;
    let work_units = required_work(request)?;
    let format = request.background.surface.format();
    let roi_tiles = tile_count(request.roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output_bytes = roi_tiles.checked_mul(tile_bytes).ok_or(RasterInputError::LimitExceeded)?;
    let index_bytes = (request.background.surface.tile_count() as u64)
        .checked_mul(512)
        .and_then(|n| n.checked_add(roi_tiles.checked_mul(512)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let scratch_bytes = tile_bytes.checked_mul(6).and_then(|n| n.checked_add(4096)).ok_or(RasterInputError::LimitExceeded)?;
    let mut source_bytes = 0u64;
    for input in [request.foreground, request.background] {
        poll(control)?;
        let map_bytes = (input.surface.tile_count() as u64).checked_mul(256).ok_or(RasterInputError::LimitExceeded)?;
        source_bytes = source_bytes
            .checked_add(map_bytes)
            .and_then(|n| n.checked_add(input.surface.default_bytes().len() as u64))
            .ok_or(RasterInputError::LimitExceeded)?;
        // Bound hostile map walks before visiting retained entries. Standard
        // native tiles have exactly this payload size, even when shared.
        let minimum =
            (input.surface.tile_count() as u64).checked_mul(tile_bytes).and_then(|n| n.checked_add(source_bytes)).ok_or(RasterInputError::LimitExceeded)?;
        if minimum > MAX_CHECKED_RASTER_BYTES {
            return Err(RasterInputError::LimitExceeded);
        }
        input.surface.try_validate_tile_origins_with(control)?;
        for (_, tile) in input.surface.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch);
            }
            source_bytes = source_bytes.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
        }
    }
    let resident_bytes = source_bytes
        .checked_add(output_bytes)
        .and_then(|n| n.checked_add(index_bytes))
        .and_then(|n| n.checked_add(scratch_bytes))
        .ok_or(RasterInputError::LimitExceeded)?;
    let roi_output_bytes = roi_tiles
        .checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?)
        .and_then(|n| n.checked_add(256 + format.bytes_per_pixel() as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    poll(control)?;
    Ok(Requirements { work_units, source_bytes, output_bytes, index_bytes, scratch_bytes, resident_bytes, roi_output_bytes, roi_tiles })
}
fn rounded(numerator: u128, denominator: u128, maximum: u32) -> Result<u32, RasterInputError> {
    if denominator == 0 {
        return Err(RasterInputError::InvalidSample);
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let upward = remainder.checked_mul(2).ok_or(RasterInputError::LimitExceeded)? >= denominator;
    let value = quotient.checked_add(u128::from(upward)).ok_or(RasterInputError::LimitExceeded)?;
    let value = u32::try_from(value).map_err(|_| RasterInputError::InvalidSample)?;
    if value > maximum {
        return Err(RasterInputError::InvalidSample);
    }
    Ok(value)
}
fn zero(output: &mut [u8], background: &[u8], policy: ZeroAlpha) -> Result<(), RasterInputError> {
    match policy {
        ZeroAlpha::PreserveBackgroundRaw => output.copy_from_slice(background),
        ZeroAlpha::ZeroColors => output.fill(0),
        ZeroAlpha::Reject => return Err(RasterInputError::InvalidSample),
    }
    Ok(())
}
fn evaluate(format: PixelFormat, foreground: &[u8], background: &[u8], output: &mut [u8], policy: ZeroAlpha) -> Result<(), RasterInputError> {
    validate_pixel(foreground, format)?;
    validate_pixel(background, format)?;
    if output.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    let alpha_channel = format.mode.color_channels();
    let mut staging = [0u8; 32];
    let staging = staging.get_mut(..output.len()).ok_or(RasterInputError::LengthMismatch)?;
    if let Some(maximum) = maximum(format.sample) {
        let a = u128::from(integer(foreground, format.sample, alpha_channel)?);
        let d = u128::from(integer(background, format.sample, alpha_channel)?);
        let full = u128::from(maximum);
        let remaining = full.checked_sub(a).ok_or(RasterInputError::InvalidSample)?;
        let source_weight = a.checked_mul(full).ok_or(RasterInputError::LimitExceeded)?;
        let destination_weight = d.checked_mul(remaining).ok_or(RasterInputError::LimitExceeded)?;
        let alpha = source_weight.checked_add(destination_weight).ok_or(RasterInputError::LimitExceeded)?;
        if alpha == 0 {
            zero(staging, background, policy)?;
        } else {
            for channel in 0..alpha_channel {
                let source = u128::from(integer(foreground, format.sample, channel)?).checked_mul(source_weight).ok_or(RasterInputError::LimitExceeded)?;
                let destination =
                    u128::from(integer(background, format.sample, channel)?).checked_mul(destination_weight).ok_or(RasterInputError::LimitExceeded)?;
                let color = rounded(source.checked_add(destination).ok_or(RasterInputError::LimitExceeded)?, alpha, maximum)?;
                store_integer(staging, format.sample, channel, color)?;
            }
            store_integer(staging, format.sample, alpha_channel, rounded(alpha, full, maximum)?)?;
        }
    } else {
        let foreground_values = floats(foreground, format)?;
        let background_values = floats(background, format)?;
        let a = *foreground_values.get(alpha_channel).ok_or(RasterInputError::LengthMismatch)?;
        let d = *background_values.get(alpha_channel).ok_or(RasterInputError::LengthMismatch)?;
        let remaining = 1.0 - a;
        let destination_weight = d * remaining;
        let alpha = a + destination_weight;
        if !remaining.is_finite() || !destination_weight.is_finite() || !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(RasterInputError::InvalidSample);
        }
        if alpha == 0.0 {
            zero(staging, background, policy)?;
        } else if a == 0.0 {
            staging.copy_from_slice(background);
        } else if a == 1.0 {
            staging.copy_from_slice(foreground);
        } else {
            for (channel, (foreground, background)) in foreground_values.iter().zip(background_values).take(alpha_channel).enumerate() {
                let source = *foreground * a;
                let destination = background * destination_weight;
                let sum = source + destination;
                let color = sum / alpha;
                if [source, destination, sum, color].iter().any(|v| !v.is_finite()) || format.sample == SampleType::F32 && !(color as f32).is_finite() {
                    return Err(RasterInputError::InvalidSample);
                }
                try_write_sample_f64(staging, format.sample, channel, color).map_err(|_| RasterInputError::InvalidSample)?;
            }
            try_write_sample_f64(staging, format.sample, alpha_channel, alpha).map_err(|_| RasterInputError::InvalidSample)?;
        }
    }
    output.copy_from_slice(staging);
    Ok(())
}
fn read(input: Input<'_>, x: i32, y: i32, pixel: &mut [u8]) -> Result<(), RasterInputError> {
    if input.extent.contains(x, y) {
        input.surface.try_read_pixel_bytes(x, y, pixel)
    } else {
        match input.outside {
            Outside::Reject => Err(RasterInputError::InvalidBounds),
            Outside::Constant(value) => {
                if value.len() != pixel.len() {
                    return Err(RasterInputError::LengthMismatch);
                }
                pixel.copy_from_slice(value);
                Ok(())
            }
        }
    }
}

/// Private bounded tiled COW output. Background defaults and every native word
/// outside ROI remain untouched. Integer alpha numerator is as*M+ad*(M-as);
/// each straight color numerator is cs*as*M+cd*ad*(M-as). Divide/round once.
/// Floats use separate f64 products, sum and division with finite HDR guards.
/// Floating opaque/transparent source identities copy native foreground/background
/// words after validating both pixels; zero-alpha policy takes precedence.
pub fn try_apply(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    poll(control)?;
    if required_work(request)? > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let format = request.background.surface.format();
    let footprint = requirements(request, control)?;
    if footprint.resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    request.foreground.surface.try_validate_tile_origins_with(control)?;
    request.background.surface.try_validate_tile_origins_with(control)?;
    poll(control)?;
    // Retained standard map allocation is bounded by the census, not an RSS claim.
    let mut output = request.background.surface.clone();
    for tile in request.roi.tiles() {
        poll(control)?;
        let area = tile.rect().intersect(&request.roi);
        let length = checked_region_byte_len(area, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(length, 0);
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                let mut foreground = [0u8; 32];
                let mut background = [0u8; 32];
                let foreground = foreground.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                let background = background.get_mut(..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                read(request.foreground, x, y, foreground)?;
                read(request.background, x, y, background)?;
                let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                evaluate(format, foreground, background, bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?, request.zero_alpha)?;
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
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [3; 32], coordinate_space: [9; 32] }
    }
    fn request<'a>(foreground: &'a Surface, background: &'a Surface) -> Request<'a> {
        Request {
            foreground: Input { surface: foreground, extent: Rect::new(-1, 0, 1, 1), context: context(), outside: Outside::Reject },
            background: Input { surface: background, extent: Rect::new(-1, 0, 1, 1), context: context(), outside: Outside::Reject },
            roi: Rect::new(-1, 0, 1, 1),
            output_context: context(),
            zero_alpha: ZeroAlpha::PreserveBackgroundRaw,
            rounding: Rounding::NearestTiesUp,
        }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 64 << 20, work_units: 1 << 28 }
    }
    #[test]
    fn u32_color_lowbits_and_exact_half_ties_are_retained() {
        let foreground: Vec<u8> = [0xfffffff8u32, 1, 17, 0x80000000].into_iter().flat_map(u32::to_ne_bytes).collect();
        let background: Vec<u8> = [0xfffffff6u32, 3, 29, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect();
        let f = Surface::try_with_default_bytes(PixelFormat::RGBA32, &foreground).unwrap();
        let b = Surface::try_with_default_bytes(PixelFormat::RGBA32, &background).unwrap();
        let output = try_apply(request(&f, &b), limits(), &Interrupt::NONE).unwrap();
        let raw = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(u32::from_ne_bytes(raw[..4].try_into().unwrap()), 0xfffffff7);
        assert_eq!(&raw[12..], &u32::MAX.to_ne_bytes());
        assert_eq!(output.default_bytes(), background);
        assert_eq!(output.try_read_interleaved(Rect::new(2, 0, 3, 1)).unwrap(), background);
        let mut output = [0; 4];
        evaluate(PixelFormat::RGBA8, &[0, 0, 0, 30], &[1, 1, 1, 34], &mut output, ZeroAlpha::Reject).unwrap();
        assert_eq!(output, [1, 1, 1, 60]);
    }
    #[test]
    fn f64_hdr_unfused_math_and_zeroalpha_policies() {
        let foreground: Vec<u8> = [2f64, -1., 4., 0.5].into_iter().flat_map(f64::to_ne_bytes).collect();
        let background: Vec<u8> = [-2f64, 3., 0.25, 0.25].into_iter().flat_map(f64::to_ne_bytes).collect();
        let mut output = [0u8; 32];
        evaluate(PixelFormat::RGBA64F, &foreground, &background, &mut output, ZeroAlpha::Reject).unwrap();
        assert_eq!(try_read_sample_f64(&output, SampleType::F64, 0).unwrap(), 1.2);
        assert_eq!(try_read_sample_f64(&output, SampleType::F64, 2).unwrap(), 3.25);
        assert_eq!(try_read_sample_f64(&output, SampleType::F64, 3).unwrap(), 0.625);
        let foreground: Vec<u8> = [1f64, 2., 3., 0.].into_iter().flat_map(f64::to_ne_bytes).collect();
        let background: Vec<u8> = [-0f64, -4., 5., -0.].into_iter().flat_map(f64::to_ne_bytes).collect();
        evaluate(PixelFormat::RGBA64F, &foreground, &background, &mut output, ZeroAlpha::PreserveBackgroundRaw).unwrap();
        assert_eq!(output.as_slice(), background);
        evaluate(PixelFormat::RGBA64F, &foreground, &background, &mut output, ZeroAlpha::ZeroColors).unwrap();
        assert_eq!(output, [0; 32]);
        output.fill(17);
        assert!(evaluate(PixelFormat::RGBA64F, &foreground, &background, &mut output, ZeroAlpha::Reject).is_err());
        assert_eq!(output, [17; 32]);
    }
    #[test]
    fn explicit_constants_context_limits_cancel_and_invalidalpha_are_atomic() {
        let foreground = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[255, 0, 0, 128]).unwrap();
        let background = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[0, 0, 255, 255]).unwrap();
        let before = background.clone();
        let mut base = request(&foreground, &background);
        base.foreground.extent = Rect::new(0, 0, 1, 1);
        assert!(matches!(try_apply(base, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        base.foreground.outside = Outside::Constant(&[0, 0, 0, 0]);
        let output = try_apply(base, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap(), [0, 0, 255, 255]);
        let mut changed = base;
        changed.foreground.context.encoding = SampleEncoding::Perceptual;
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        assert!(matches!(try_apply(base, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert!(matches!(try_apply(base, limits(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
        assert_eq!(background, before);
        let mut output = [7; 16];
        let invalid: Vec<u8> = [0f32, 0., 0., 2.].into_iter().flat_map(f32::to_ne_bytes).collect();
        assert!(evaluate(PixelFormat::RGBA32F, &invalid, &invalid, &mut output, ZeroAlpha::Reject).is_err());
        assert_eq!(output, [7; 16]);
    }
    #[test]
    fn opaque_gray_source_is_exact_at_every_native_depth() {
        for sample in SampleType::ALL {
            let format = PixelFormat { mode: ColorMode::Grayscale, sample, alpha: true };
            let mut foreground = vec![0; format.bytes_per_pixel()];
            let mut background = foreground.clone();
            let color = if matches!(sample, SampleType::F32 | SampleType::F64) { 1.0000000000000002 } else { 1.0 / 3.0 };
            try_write_sample_f64(&mut foreground, sample, 0, color).unwrap();
            try_write_sample_f64(&mut foreground, sample, 1, 1.0).unwrap();
            try_write_sample_f64(&mut background, sample, 0, 0.5).unwrap();
            try_write_sample_f64(&mut background, sample, 1, 0.5).unwrap();
            let f = Surface::try_with_default_bytes(format, &foreground).unwrap();
            let b = Surface::try_with_default_bytes(format, &background).unwrap();
            let output = try_apply(request(&f, &b), limits(), &Interrupt::NONE).unwrap();
            assert_eq!(output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), foreground);
            assert_eq!(output.default_bytes(), background);
        }
    }
}
