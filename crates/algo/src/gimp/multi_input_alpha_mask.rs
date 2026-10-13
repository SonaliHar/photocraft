//! Original alpha-only matte arithmetic, without GEGL/profile/source admission.
//! Coordinates are already in one caller-established space. Colors are never
//! decoded: hidden values, NaN payloads and signed zero remain native bytes.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
}
#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub surface: &'a Surface,
    /// Complete authoritative extent, including source-owned default pixels.
    pub extent: Rect,
    pub context: WorkingContext,
    pub coordinate_space: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageInterpretation {
    /// Integer full scale is one; floating samples are finite unit scalars.
    /// No profile, transfer curve or alpha channel is applied to coverage.
    UnitSamples,
}
#[derive(Clone, Copy)]
pub struct Mask<'a> {
    pub surface: &'a Surface,
    pub extent: Rect,
    pub coordinate_space: [u8; 32],
    pub interpretation: CoverageInterpretation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    Multiply,
    Replace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutsideCoverage {
    Reject,
    /// Explicit exact unit rational outside the independent mask extent.
    Constant {
        numerator: u32,
        denominator: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Nonnegative integer quantization: nearest, exact half ties upward.
    NearestTiesUp,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub source: Source<'a>,
    pub mask: Mask<'a>,
    pub roi: Rect,
    pub output_context: WorkingContext,
    pub apply: Apply,
    pub outside: OutsideCoverage,
    pub rounding: Rounding,
}
/// Explicit optional pixel owner. None means the ROI reads no mask pixels,
/// never native default coverage; the exact outside rational remains authority.
#[derive(Clone, Copy)]
pub struct AvailableMask<'a> {
    pub surface: Option<&'a Surface>,
    pub format: PixelFormat,
    pub extent: Rect,
    pub coordinate_space: [u8; 32],
    pub interpretation: CoverageInterpretation,
}
#[derive(Clone, Copy)]
pub struct AvailableRequest<'a> {
    pub source: Source<'a>,
    pub mask: AvailableMask<'a>,
    pub roi: Rect,
    pub output_context: WorkingContext,
    pub apply: Apply,
    pub outside: OutsideCoverage,
    pub rounding: Rounding,
}
impl<'a> From<Request<'a>> for AvailableRequest<'a> {
    fn from(request: Request<'a>) -> Self {
        Self { source: request.source, mask: AvailableMask { surface: Some(request.mask.surface), format: request.mask.surface.format(),
            extent: request.mask.extent, coordinate_space: request.mask.coordinate_space, interpretation: request.mask.interpretation },
            roi: request.roi, output_context: request.output_context, apply: request.apply, outside: request.outside, rounding: request.rounding }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy)]
enum Coverage {
    Ratio(u32, u32),
    Float(f64),
}
impl Coverage {
    fn unit(self) -> Result<f64, RasterInputError> {
        match self {
            Self::Ratio(n, d) if d != 0 && n <= d => Ok(f64::from(n) / f64::from(d)),
            Self::Float(value) if value.is_finite() && (0.0..=1.0).contains(&value) => Ok(value),
            _ => Err(RasterInputError::InvalidSample),
        }
    }
    fn is_one(self) -> bool {
        match self {
            Self::Ratio(n, d) => d != 0 && n == d,
            Self::Float(value) => value == 1.0,
        }
    }
}
fn integer_max(sample: SampleType) -> Option<u32> {
    match sample {
        SampleType::U8 => Some(u32::from(u8::MAX)),
        SampleType::U16 => Some(u32::from(u16::MAX)),
        SampleType::U32 => Some(u32::MAX),
        _ => None,
    }
}
fn integer(bytes: &[u8], sample: SampleType) -> Result<u32, RasterInputError> {
    Ok(match sample {
        SampleType::U8 => u32::from(*bytes.first().ok_or(RasterInputError::LengthMismatch)?),
        SampleType::U16 => u32::from(u16::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?)),
        SampleType::U32 => u32::from_ne_bytes(bytes.try_into().map_err(|_| RasterInputError::LengthMismatch)?),
        _ => return Err(RasterInputError::UnsupportedConversion),
    })
}
fn coverage(bytes: &[u8], sample: SampleType) -> Result<Coverage, RasterInputError> {
    let result = if let Some(maximum) = integer_max(sample) {
        Coverage::Ratio(integer(bytes, sample)?, maximum)
    } else {
        Coverage::Float(try_read_sample_f64(bytes, sample, 0).map_err(|_| RasterInputError::InvalidSample)?)
    };
    result.unit()?;
    Ok(result)
}
fn alpha_range(format: PixelFormat) -> Result<std::ops::Range<usize>, RasterInputError> {
    let start = format.mode.color_channels().checked_mul(format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
    let end = start.checked_add(format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
    Ok(start..end)
}
fn validate(request: AvailableRequest<'_>) -> Result<(), RasterInputError> {
    let format = request.source.surface.format();
    let mask = request.mask.format;
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !format.alpha
        || mask.mode != ColorMode::Grayscale
        || mask.alpha
        || request.source.context != request.output_context
        || request.source.coordinate_space != request.mask.coordinate_space
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if request.mask.surface.is_some_and(|surface| surface.format() != mask) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if request.mask.surface.is_none() && (!request.roi.intersect(&request.mask.extent).is_empty()
        || !matches!(request.outside, OutsideCoverage::Constant { .. })) {
        return Err(RasterInputError::InvalidBounds);
    }
    checked_region_byte_len(request.source.extent, format)?;
    checked_region_byte_len(request.mask.extent, mask)?;
    checked_region_byte_len(request.roi, format)?;
    if request.source.extent.is_empty() || request.roi.is_empty() || !request.source.extent.contains_rect(&request.roi) {
        return Err(RasterInputError::InvalidBounds);
    }
    match request.outside {
        OutsideCoverage::Reject if !request.mask.extent.contains_rect(&request.roi) => return Err(RasterInputError::InvalidBounds),
        OutsideCoverage::Constant { numerator, denominator } => {
            Coverage::Ratio(numerator, denominator).unit()?;
        }
        _ => {}
    }
    match request.mask.interpretation {
        CoverageInterpretation::UnitSamples => {}
    }
    match request.rounding {
        Rounding::NearestTiesUp => {}
    }
    coverage(request.source.surface.default_bytes().get(alpha_range(format)?).ok_or(RasterInputError::LengthMismatch)?, format.sample)?;
    if let Some(surface) = request.mask.surface { coverage(surface.default_bytes(), mask.sample)?; }
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
/// Checked tile/map/sample work before scanning or cloning retained storage.
pub fn required_work(request: Request<'_>) -> Result<u64, RasterInputError> { required_work_available(request.into()) }
pub fn required_work_available(request: AvailableRequest<'_>) -> Result<u64, RasterInputError> {
    validate(request)?;
    let tiles = tile_count(request.roi)?;
    let source = request.source.surface.tile_count() as u64;
    let mask = request.mask.surface.map_or(0, |surface| surface.tile_count() as u64);
    let bytes = (request.source.surface.format().bytes_per_pixel() as u64)
        .checked_add(request.mask.surface.map_or(0, |surface| surface.format().bytes_per_pixel() as u64))
        .ok_or(RasterInputError::LimitExceeded)?;
    request
        .roi
        .size()
        .area()
        .checked_mul(bytes)
        .and_then(|n| n.checked_mul(64))
        .and_then(|n| n.checked_add(tiles.checked_mul(source.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(source.checked_add(mask)?.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)
}
/// Allocation-free preflight shared by the direct kernel and graph callers.
/// Retained Arc tile payload is charged once by identity; coordinate maps/default
/// owners and all private output/scratch owners remain charged separately.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub work_units: u64,
    pub peak_bytes: u64,
}
pub fn requirements(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    requirements_available(request.into(), limits, control)
}
pub fn requirements_available(request: AvailableRequest<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    poll(control)?;
    validate(request)?;
    let source = request.source.surface;
    let mask = request.mask.surface;
    let source_count = source.tile_count() as u64;
    let mask_count = mask.map_or(0, |surface| surface.tile_count() as u64);
    let count = source_count.checked_add(mask_count).ok_or(RasterInputError::LimitExceeded)?;
    let comparisons = count.checked_mul(count).and_then(|v| v.checked_mul(32)).ok_or(RasterInputError::LimitExceeded)?;
    let work = required_work_available(request)?.checked_add(comparisons).ok_or(RasterInputError::LimitExceeded)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let source_payload =
        (TILE_SIZE as u64).checked_pow(2).and_then(|v| v.checked_mul(source.format().bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output_bytes =
        tile_count(request.roi)?.checked_mul(source_payload.checked_add(512).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let index_bytes = count.checked_mul(256).and_then(|v| v.checked_add(source_count.checked_mul(512)?)).ok_or(RasterInputError::LimitExceeded)?;
    let scratch_bytes = source_payload.checked_mul(6).and_then(|v| v.checked_add(4096)).ok_or(RasterInputError::LimitExceeded)?;
    let extra = output_bytes.checked_add(index_bytes).and_then(|v| v.checked_add(scratch_bytes)).ok_or(RasterInputError::LimitExceeded)?;
    let cap = limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES);
    if extra > cap {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut retained = (source.default_bytes().len() as u64).checked_add(mask.map_or(0, |surface| surface.default_bytes().len() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    for (index, (coordinate, tile)) in source.tiles().enumerate() {
        poll(control)?;
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != source_payload {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut shared = false;
        for (_, previous) in source.tiles().take(index) {
            poll(control)?;
            if std::sync::Arc::ptr_eq(tile, previous) {
                shared = true;
                break;
            }
        }
        if !shared {
            retained = retained.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
        }
        if retained.checked_add(extra).is_none_or(|v| v > cap) {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    let mask_payload =
        (TILE_SIZE as u64).checked_pow(2).and_then(|v| v.checked_mul(request.mask.format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    for (index, (coordinate, tile)) in mask.into_iter().flat_map(Surface::tiles).enumerate() {
        poll(control)?;
        coordinate.tx.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        coordinate.ty.checked_mul(TILE_SIZE).and_then(|v| v.checked_add(TILE_SIZE)).ok_or(RasterInputError::InvalidBounds)?;
        if tile.bytes().len() as u64 != mask_payload {
            return Err(RasterInputError::LengthMismatch);
        }
        let mut shared = false;
        for (_, previous) in source.tiles().chain(mask.into_iter().flat_map(Surface::tiles).take(index)) {
            poll(control)?;
            if std::sync::Arc::ptr_eq(tile, previous) {
                shared = true;
                break;
            }
        }
        if !shared {
            retained = retained.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
        }
        if retained.checked_add(extra).is_none_or(|v| v > cap) {
            return Err(RasterInputError::LimitExceeded);
        }
    }
    let peak_bytes = retained.checked_add(extra).filter(|v| *v <= cap).ok_or(RasterInputError::LimitExceeded)?;
    poll(control)?;
    Ok(Requirements { retained_bytes: retained, output_bytes, index_bytes, scratch_bytes, work_units: work, peak_bytes })
}

fn store_integer(bytes: &mut [u8], sample: SampleType, value: u32) -> Result<(), RasterInputError> {
    match sample {
        SampleType::U8 => {
            let value = u8::try_from(value).map_err(|_| RasterInputError::InvalidSample)?;
            bytes.get_mut(..1).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(&[value]);
        }
        SampleType::U16 => bytes
            .get_mut(..2)
            .ok_or(RasterInputError::LengthMismatch)?
            .copy_from_slice(&u16::try_from(value).map_err(|_| RasterInputError::InvalidSample)?.to_ne_bytes()),
        SampleType::U32 => bytes.get_mut(..4).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(&value.to_ne_bytes()),
        _ => return Err(RasterInputError::UnsupportedConversion),
    }
    Ok(())
}
fn change_alpha(alpha: &mut [u8], sample: SampleType, mask: Coverage, apply: Apply, matching_native: Option<&[u8]>) -> Result<(), RasterInputError> {
    let original = coverage(alpha, sample)?;
    mask.unit()?;
    if apply == Apply::Multiply && mask.is_one() {
        return Ok(());
    }
    if apply == Apply::Replace
        && let Some(native) = matching_native
    {
        alpha.copy_from_slice(native);
        return Ok(());
    }
    if let Some(maximum) = integer_max(sample) {
        let units = if apply == Apply::Multiply { integer(alpha, sample)? } else { maximum };
        let result = match mask {
            Coverage::Ratio(numerator, denominator) => {
                if denominator == 0 {
                    return Err(RasterInputError::InvalidSample);
                }
                let product = u64::from(units).checked_mul(u64::from(numerator)).ok_or(RasterInputError::LimitExceeded)?;
                let divisor = u64::from(denominator);
                let quotient = product / divisor;
                let remainder = product % divisor;
                let round = remainder.checked_mul(2).ok_or(RasterInputError::LimitExceeded)? >= divisor;
                u32::try_from(quotient.checked_add(u64::from(round)).ok_or(RasterInputError::LimitExceeded)?).map_err(|_| RasterInputError::InvalidSample)?
            }
            Coverage::Float(value) => {
                let value = f64::from(units) * value;
                if !value.is_finite() || value < 0.0 || value > f64::from(maximum) {
                    return Err(RasterInputError::InvalidSample);
                }
                u32::try_from(value.round() as u64).map_err(|_| RasterInputError::InvalidSample)?
            }
        };
        if result > maximum {
            return Err(RasterInputError::InvalidSample);
        }
        store_integer(alpha, sample, result)
    } else {
        let weight = mask.unit()?;
        let value = if apply == Apply::Multiply { original.unit()? * weight } else { weight };
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(RasterInputError::InvalidSample);
        }
        try_write_sample_f64(alpha, sample, 0, value).map_err(|_| RasterInputError::InvalidSample)
    }
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}

/// Private bounded COW output; source defaults and all samples outside ROI are
/// unchanged. Only alpha bytes inside ROI are rewritten. Integer/integer and
/// rational-outside cases use checked u64 products/division with exact ties-up;
/// float-involved cases use explicit f64 arithmetic and one native quantization.
pub fn try_apply(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    try_apply_available(request.into(), limits, control)
}
pub fn try_apply_available(request: AvailableRequest<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    let _requirements = requirements_available(request, limits, control)?;
    let format = request.source.surface.format();
    let mask_format = request.mask.format;
    poll(control)?;
    // Surface's bounded standard BTreeMap/COW ownership is reused. Its private
    // allocator overhead remains a library seam, not an RSS guarantee.
    let mut output = request.source.surface.clone();
    for tile in request.roi.tiles() {
        poll(control)?;
        let area = tile.rect().intersect(&request.roi);
        let length = checked_region_byte_len(area, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        if bytes.capacity() != length {
            return Err(RasterInputError::AllocationFailed);
        }
        while bytes.len() < length {
            poll(control)?;
            let end = bytes.len().saturating_add(8192).min(length);
            bytes.resize(end, 0);
        }
        request.source.surface.try_read_interleaved_into_with(area, &mut bytes, control)?;
        let mut cursor = 0usize;
        for y in area.y0..area.y1 {
            poll(control)?;
            for x in area.x0..area.x1 {
                poll(control)?;
                let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let pixel = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                let alpha = pixel.get_mut(alpha_range(format)?).ok_or(RasterInputError::LengthMismatch)?;
                let mut native = [0u8; 8];
                let (coverage, matching_native) = if request.mask.extent.contains(x, y) {
                    let native = native.get_mut(..mask_format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?;
                    request.mask.surface.ok_or(RasterInputError::InvalidBounds)?.try_read_pixel_bytes(x, y, native)?;
                    (coverage(native, mask_format.sample)?, if mask_format.sample == format.sample { Some(native as &[u8]) } else { None })
                } else {
                    match request.outside {
                        OutsideCoverage::Reject => return Err(RasterInputError::InvalidBounds),
                        OutsideCoverage::Constant { numerator, denominator } => (Coverage::Ratio(numerator, denominator), None),
                    }
                };
                change_alpha(alpha, format.sample, coverage, request.apply, matching_native)?;
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
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [5; 32] }
    }
    fn request<'a>(source: &'a Surface, mask: &'a Surface) -> Request<'a> {
        Request {
            source: Source { surface: source, extent: Rect::new(-1, 0, 1, 1), context: context(), coordinate_space: [9; 32] },
            mask: Mask { surface: mask, extent: Rect::new(0, 0, 1, 1), coordinate_space: [9; 32], interpretation: CoverageInterpretation::UnitSamples },
            roi: Rect::new(-1, 0, 1, 1),
            output_context: context(),
            apply: Apply::Multiply,
            outside: OutsideCoverage::Constant { numerator: 1, denominator: 2 },
            rounding: Rounding::NearestTiesUp,
        }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 64 << 20, work_units: 1 << 28 }
    }
    #[test]
    fn absent_mask_uses_exact_rational_across_all_native_depth_pairs() {
        let samples = [SampleType::U8, SampleType::U16, SampleType::U32, SampleType::F32, SampleType::F64];
        for source_sample in samples {
            let format = PixelFormat::new(ColorMode::Rgb, source_sample, true);
            let mut raw = vec![0u8; format.bytes_per_pixel()];
            let range = alpha_range(format).unwrap();
            if integer_max(source_sample).is_some() { store_integer(&mut raw[range.clone()], source_sample, 3).unwrap(); }
            else { try_write_sample_f64(&mut raw, source_sample, 3, 0.75).unwrap(); }
            // Arbitrary raw colors are never interpreted, including float NaNs.
            raw[..range.start].fill(0xff);
            let source = Surface::try_with_default_bytes(format, &raw).unwrap();
            let before = source.clone();
            for mask_sample in samples {
                let roi = Rect::new(-1, 0, 0, 1);
                let request = AvailableRequest { source: Source { surface: &source, extent: roi, context: context(), coordinate_space: [9; 32] },
                    mask: AvailableMask { surface: None, format: PixelFormat::new(ColorMode::Grayscale, mask_sample, false),
                        extent: Rect::new(1, 0, 2, 1), coordinate_space: [9; 32], interpretation: CoverageInterpretation::UnitSamples },
                    roi, output_context: context(), apply: Apply::Multiply, outside: OutsideCoverage::Constant { numerator: 1, denominator: 2 },
                    rounding: Rounding::NearestTiesUp };
                let req = requirements_available(request, limits(), &Interrupt::NONE).unwrap();
                let output = try_apply_available(request, limits(), &Interrupt::NONE).unwrap();
                let pixel = output.try_read_interleaved(roi).unwrap();
                assert_eq!(&pixel[..range.start], &raw[..range.start]);
                if integer_max(source_sample).is_some() { assert_eq!(integer(&pixel[range.clone()], source_sample).unwrap(), 2); }
                else { assert_eq!(try_read_sample_f64(&pixel, source_sample, 3).unwrap(), 0.375); }
                assert_eq!(output.default_bytes(), raw);
                assert_eq!(source, before);
                assert_eq!(req.retained_bytes, raw.len() as u64);
                let mut overlapping = request; overlapping.mask.extent = roi;
                assert!(requirements_available(overlapping, limits(), &Interrupt::NONE).is_err());
                let mut rejecting = request; rejecting.outside = OutsideCoverage::Reject;
                assert!(requirements_available(rejecting, limits(), &Interrupt::NONE).is_err());
            }
        }
    }
    #[test]
    fn u32_lowbits_use_exact_ratio_and_colors_outside_stay_native() {
        let values = [1u32, 17, u32::MAX - 1, 0x80000001];
        let raw: Vec<u8> = values.into_iter().flat_map(u32::to_ne_bytes).collect();
        let mut source = Surface::try_with_default_bytes(PixelFormat::RGBA32, &raw).unwrap();
        source.try_write_interleaved_with(Rect::new(-1, 0, 1, 1), &raw.repeat(2), &Interrupt::NONE).unwrap();
        let mask_format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U32, alpha: false };
        let mask = Surface::try_with_default_bytes(mask_format, &0x80000000u32.to_ne_bytes()).unwrap();
        let before = source.clone();
        let output = try_apply(request(&source, &mask), limits(), &Interrupt::NONE).unwrap();
        let actual = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(&actual[..12], &raw[..12]);
        assert_eq!(u32::from_ne_bytes(actual[12..16].try_into().unwrap()), 1073741825);
        assert_eq!(output.default_bytes(), raw);
        assert_eq!(source, before);
        assert_eq!(output.try_read_interleaved(Rect::new(2, 0, 3, 1)).unwrap(), raw);
    }
    #[test]
    fn f64_raw_nan_colors_and_negative_zero_mask_replace_are_preserved() {
        let raw: Vec<u8> = [f64::from_bits(0x7ff8000000000017), -0.0, -3.5, 0.75].into_iter().flat_map(f64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
        let format = PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: false };
        let mask = Surface::try_with_default_bytes(format, &(-0.0f64).to_ne_bytes()).unwrap();
        let mut request = request(&source, &mask);
        request.apply = Apply::Replace;
        let output = try_apply(request, limits(), &Interrupt::NONE).unwrap();
        let actual = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(&actual[..24], &raw[..24]);
        assert_eq!(&actual[24..], &(-0.0f64).to_ne_bytes());
        let outside = output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&outside, SampleType::F64, 3).unwrap(), 0.5);
    }
    #[test]
    fn outside_context_alpha_limits_cancel_refuse_without_source_change() {
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 255]).unwrap();
        let mask = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[128]).unwrap();
        let original = source.clone();
        let base = request(&source, &mask);
        let mut changed = base;
        changed.outside = OutsideCoverage::Reject;
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidBounds)));
        changed = base;
        changed.mask.coordinate_space = [4; 32];
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        changed = base;
        changed.outside = OutsideCoverage::Constant { numerator: 1, denominator: 0 };
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        assert!(matches!(try_apply(base, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert!(matches!(try_apply(base, limits(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
        assert_eq!(source, original);
    }
    #[test]
    fn every_native_depth_pair_preserves_gray_color_and_replaces_only_alpha() {
        for source_depth in SampleType::ALL {
            for mask_depth in SampleType::ALL {
                let format = PixelFormat { mode: ColorMode::Grayscale, sample: source_depth, alpha: true };
                let mut raw = vec![0; format.bytes_per_pixel()];
                try_write_sample_f64(&mut raw, source_depth, 0, 0.125).unwrap();
                try_write_sample_f64(&mut raw, source_depth, 1, 0.75).unwrap();
                let source = Surface::try_with_default_bytes(format, &raw).unwrap();
                let mut mask_raw = vec![0; mask_depth.bytes()];
                try_write_sample_f64(&mut mask_raw, mask_depth, 0, 0.5).unwrap();
                let mask = Surface::try_with_default_bytes(PixelFormat { mode: ColorMode::Grayscale, sample: mask_depth, alpha: false }, &mask_raw).unwrap();
                let mut request = request(&source, &mask);
                request.apply = Apply::Replace;
                let result = try_apply(request, limits(), &Interrupt::NONE).unwrap();
                let pixel = result.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
                assert_eq!(&pixel[..source_depth.bytes()], &raw[..source_depth.bytes()]);
                let actual = try_read_sample_f64(&pixel, source_depth, 1).unwrap();
                let expected = try_read_sample_f64(&mask_raw, mask_depth, 0).unwrap();
                let tolerance = integer_max(source_depth).map_or(1e-7, |maximum| 1.0 / f64::from(maximum));
                assert!((actual - expected).abs() <= tolerance);
                assert_eq!(source.default_bytes(), raw);
            }
        }
    }
    #[test]
    fn nonfinite_or_out_of_unit_alpha_and_coverage_refuse() {
        let raw: Vec<u8> = [0f64, 0., 0., f64::NAN].into_iter().flat_map(f64::to_ne_bytes).collect();
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA64F, &raw).unwrap();
        let mask = Surface::try_with_default_bytes(PixelFormat::GRAY8, &[255]).unwrap();
        assert!(matches!(try_apply(request(&source, &mask), limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA8, &[1, 2, 3, 255]).unwrap();
        let mask =
            Surface::try_with_default_bytes(PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F32, alpha: false }, &2f32.to_ne_bytes()).unwrap();
        assert!(matches!(try_apply(request(&source, &mask), limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
    }
    #[test]
    fn requirements_charge_shared_input_tile_payload_once_without_allocation() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
        let mut source = Surface::try_with_default_bytes(format, &[1, 255]).unwrap();
        source.try_write_interleaved(Rect::new(-1, 0, 1, 1), &[1, 255, 1, 255]).unwrap();
        let mut mask = Surface::try_with_default_bytes(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false), &0u16.to_ne_bytes()).unwrap();
        mask.put_tiles(source.tiles().map(|(coordinate, tile)| (*coordinate, tile.clone())));
        let request = request(&source, &mask);
        let needed = requirements(request, limits(), &Interrupt::NONE).unwrap();
        let payload = (TILE_SIZE as u64) * (TILE_SIZE as u64) * 2;
        assert_eq!(needed.retained_bytes, payload * 2 + 4);
        assert_eq!(needed.peak_bytes, needed.retained_bytes + needed.output_bytes + needed.index_bytes + needed.scratch_bytes);
        assert!(needed.work_units >= required_work(request).unwrap());
        assert!(matches!(requirements(request, Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        let cancelled = || true;
        assert!(matches!(requirements(request, limits(), &Interrupt::cancel_only(&cancelled)), Err(RasterInputError::Cancelled)));
    }
}
