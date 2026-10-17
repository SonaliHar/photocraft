//! Original bounded affine combination in an explicit caller working domain.
//! Source order defines separate f64 multiply/add stages; no coefficient
//! normalization, profiles, premultiplication, GEGL schema or source admission.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
use std::{collections::HashSet, sync::Arc};

pub const MAX_INPUTS: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    pub profile_identity: [u8; 32],
    pub coordinate_space: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub enum Outside<'a> {
    Reject,
    Constant(&'a [u8]),
}
#[derive(Clone, Copy, Debug)]
pub enum Channel {
    /// Zero-based native channel index; alpha is a unit scalar when selected.
    Native(u8),
    Constant(f64),
}
#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub source: &'a Surface,
    pub extent: Rect,
    pub context: WorkingContext,
    pub outside: Outside<'a>,
    pub coefficient: f64,
    /// One selector per native output channel; inactive slots are still checked.
    pub channels: [Channel; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alpha {
    /// First input's alpha bytes are opaque and copied exactly, never decoded
    /// unless a caller explicitly selects alpha as an arithmetic operand.
    PreserveFirstRaw,
    /// Alpha participates in the affine sum and must finish finite in [0,1].
    Compute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    Evaluate,
    /// Require one input, coefficient exactly 1, active identity selectors and
    /// positive-zero biases; copy its native words after domain validation.
    PreserveExact,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub inputs: &'a [Input<'a>],
    pub roi: Rect,
    pub output_context: WorkingContext,
    pub bias: [f64; 4],
    pub alpha: Alpha,
    pub identity: Identity,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn output_channels(format: PixelFormat, alpha: Alpha) -> usize {
    if alpha == Alpha::PreserveFirstRaw { format.mode.color_channels() } else { format.channels() }
}
fn alpha_needed(input: Input<'_>, format: PixelFormat, alpha: Alpha) -> bool {
    format.alpha
        && (alpha == Alpha::Compute
            || input
                .channels
                .iter()
                .take(output_channels(format, alpha))
                .any(|selector| matches!(selector,Channel::Native(index) if usize::from(*index)==format.mode.color_channels())))
}
fn validate_pixel(pixel: &[u8], format: PixelFormat, needs_alpha: bool) -> Result<(), RasterInputError> {
    if pixel.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    for channel in 0..format.mode.color_channels() {
        let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
    }
    if needs_alpha {
        let value = try_read_sample_f64(pixel, format.sample, format.mode.color_channels()).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(())
}
fn validate(request: Request<'_>) -> Result<PixelFormat, RasterInputError> {
    if request.inputs.is_empty() || request.inputs.len() > MAX_INPUTS {
        return Err(RasterInputError::LimitExceeded);
    }
    let first = request.inputs.first().ok_or(RasterInputError::LengthMismatch)?;
    let format = first.source.format();
    if !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
        || request.alpha == Alpha::PreserveFirstRaw && !format.alpha
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(request.roi, format)?;
    if request.roi.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    if request.bias.iter().any(|value| !value.is_finite()) {
        return Err(RasterInputError::InvalidSample);
    }
    for input in request.inputs {
        if input.source.format() != format || input.context != request.output_context {
            return Err(RasterInputError::UnsupportedConversion);
        }
        if !input.coefficient.is_finite() {
            return Err(RasterInputError::InvalidSample);
        }
        for channel in input.channels {
            match channel {
                Channel::Native(index) if usize::from(index) >= format.channels() => return Err(RasterInputError::LengthMismatch),
                Channel::Constant(value) if !value.is_finite() => return Err(RasterInputError::InvalidSample),
                _ => {}
            }
        }
        checked_region_byte_len(input.extent, format)?;
        let needs_alpha = alpha_needed(*input, format, request.alpha);
        validate_pixel(input.source.default_bytes(), format, needs_alpha)?;
        match input.outside {
            Outside::Reject if !input.extent.contains_rect(&request.roi) => return Err(RasterInputError::InvalidBounds),
            Outside::Constant(pixel) => validate_pixel(pixel, format, needs_alpha)?,
            _ => {}
        }
    }
    if request.identity == Identity::PreserveExact {
        if request.inputs.len() != 1 || first.coefficient.to_bits() != 1.0f64.to_bits() {
            return Err(RasterInputError::UnsupportedConversion);
        }
        for (index, (selector, bias)) in first.channels.iter().zip(request.bias).take(output_channels(format, request.alpha)).enumerate() {
            if !matches!(selector,Channel::Native(channel) if usize::from(*channel)==index) || bias.to_bits() != 0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
    }
    Ok(format)
}
fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}
fn source_tiles(request: Request<'_>) -> Result<u64, RasterInputError> {
    request.inputs.iter().try_fold(0u64, |count, input| count.checked_add(input.source.tile_count() as u64).ok_or(RasterInputError::LimitExceeded))
}
/// O(1) in each input's metadata, conservatively charging all possible pixel
/// reads even where zero coefficients/constant selectors avoid a read.
pub fn required_work(request: Request<'_>) -> Result<u64, RasterInputError> {
    let format = validate(request)?;
    let tiles = tile_count(request.roi)?;
    let sources = source_tiles(request)?;
    let first = request.inputs.first().ok_or(RasterInputError::LengthMismatch)?.source.tile_count() as u64;
    request
        .roi
        .size()
        .area()
        .checked_mul(format.bytes_per_pixel() as u64)
        .and_then(|n| n.checked_mul(request.inputs.len() as u64))
        .and_then(|n| n.checked_mul(128))
        .and_then(|n| n.checked_add(tiles.checked_mul(first.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(sources.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)
}
/// Allocation-free conservative footprint. Shared tile payloads are counted
/// once per declared input here; evaluation still deduplicates actual Arc owners.
/// Index constants bound known structures, not allocator overhead or process RSS.
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub work_units: u64,
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub index_bytes: u64,
    pub scratch_bytes: u64,
    pub resident_bytes: u64,
    pub roi_output_bytes: u64,
    pub roi_tiles: u64,
}
pub fn requirements(request: Request<'_>, control: &Interrupt<'_>) -> Result<Requirements, RasterInputError> {
    poll(control)?;
    let format = validate(request)?;
    let work_units = required_work(request)?;
    let sources = source_tiles(request)?;
    let first = request.inputs.first().ok_or(RasterInputError::LengthMismatch)?.source.tile_count() as u64;
    let roi_tiles = tile_count(request.roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output_bytes = roi_tiles.checked_mul(tile_bytes).ok_or(RasterInputError::LimitExceeded)?;
    // Includes conservative capacity for the evaluator's pointer-dedup index,
    // alongside source/output sparse maps; actual capacity is checked there too.
    let index_bytes = sources
        .checked_mul(1280)
        .and_then(|n| n.checked_add(first.checked_mul(512)?))
        .and_then(|n| n.checked_add(roi_tiles.checked_mul(512)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    let scratch_bytes = tile_bytes.checked_mul(6).and_then(|n| n.checked_add(8192)).ok_or(RasterInputError::LimitExceeded)?;
    let source_bytes = request.inputs.iter().try_fold(0u64, |bytes, input| {
        poll(control)?;
        bytes
            .checked_add((input.source.tile_count() as u64).checked_mul(tile_bytes).ok_or(RasterInputError::LimitExceeded)?)
            .and_then(|n| n.checked_add(input.source.default_bytes().len() as u64))
            .ok_or(RasterInputError::LimitExceeded)
    })?;
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
fn retained_peak(request: Request<'_>, format: PixelFormat, cap: u64, control: &Interrupt<'_>) -> Result<u64, RasterInputError> {
    let sources = source_tiles(request)?;
    let first = request.inputs.first().ok_or(RasterInputError::LengthMismatch)?.source.tile_count() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let fixed = sources
        .checked_mul(256)
        .and_then(|n| n.checked_add(first.checked_mul(512)?))
        .and_then(|n| n.checked_add(tile_count(request.roi).ok()?.checked_mul(tile_bytes.checked_add(512)?)?))
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(8192))
        .ok_or(RasterInputError::LimitExceeded)?;
    let preliminary = fixed.checked_add(sources.checked_mul(256).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    if preliminary > cap {
        return Err(RasterInputError::LimitExceeded);
    }
    let mut payloads = HashSet::new();
    payloads.try_reserve(usize::try_from(sources).map_err(|_| RasterInputError::LimitExceeded)?).map_err(|_| RasterInputError::AllocationFailed)?;
    let mut peak =
        fixed.checked_add((payloads.capacity() as u64).checked_mul(128).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    if peak > cap {
        return Err(RasterInputError::LimitExceeded);
    }
    for input in request.inputs {
        poll(control)?;
        input.source.try_validate_tile_origins_with(control)?;
        peak = peak.checked_add(input.source.default_bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
        for (_, tile) in input.source.tiles() {
            poll(control)?;
            if tile.bytes().len() as u64 != tile_bytes {
                return Err(RasterInputError::LengthMismatch);
            }
            if payloads.insert(Arc::as_ptr(tile) as usize) {
                peak = peak.checked_add(tile.bytes().len() as u64).ok_or(RasterInputError::LimitExceeded)?;
            }
            if peak > cap {
                return Err(RasterInputError::LimitExceeded);
            }
        }
    }
    poll(control)?;
    Ok(peak)
}
fn read(input: Input<'_>, x: i32, y: i32, pixel: &mut [u8]) -> Result<(), RasterInputError> {
    if input.extent.contains(x, y) {
        input.source.try_read_pixel_bytes(x, y, pixel)
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

/// Cancellable private tiled COW output preserving first-input defaults and
/// all raw words outside ROI. Zero coefficients skip actual pixel reads, except
/// input zero must supply preserved raw alpha. Metadata/constants/defaults are
/// still validated for every input. Selected/nonzero operands must be finite.
pub fn try_apply(request: Request<'_>, limits: Limits, control: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
    poll(control)?;
    let format = validate(request)?;
    if required_work(request)? > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    retained_peak(request, format, limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES), control)?;
    let first = request.inputs.first().ok_or(RasterInputError::LengthMismatch)?;
    poll(control)?;
    let mut output = first.source.clone();
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
                poll(control)?;
                let mut native = [[0u8; 32]; MAX_INPUTS];
                for (index, input) in request.inputs.iter().enumerate() {
                    poll(control)?;
                    let preserved = index == 0 && request.alpha == Alpha::PreserveFirstRaw;
                    let arithmetic = input.coefficient != 0.0
                        && input.channels.iter().take(output_channels(format, request.alpha)).any(|channel| matches!(channel, Channel::Native(_)));
                    if preserved || arithmetic || request.identity == Identity::PreserveExact {
                        let pixel =
                            native.get_mut(index).and_then(|pixel| pixel.get_mut(..format.bytes_per_pixel())).ok_or(RasterInputError::LengthMismatch)?;
                        read(*input, x, y, pixel)?;
                        if input.coefficient != 0.0 {
                            validate_pixel(pixel, format, alpha_needed(*input, format, request.alpha))?;
                        }
                    }
                }
                let end = cursor.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let pixel = bytes.get_mut(cursor..end).ok_or(RasterInputError::LengthMismatch)?;
                if request.identity == Identity::PreserveExact {
                    pixel.copy_from_slice(native.first().and_then(|pixel| pixel.get(..format.bytes_per_pixel())).ok_or(RasterInputError::LengthMismatch)?);
                } else {
                    for channel in 0..output_channels(format, request.alpha) {
                        let mut sum = *request.bias.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                        for (index, input) in request.inputs.iter().enumerate() {
                            poll(control)?;
                            if input.coefficient == 0.0 {
                                continue;
                            }
                            let selector = *input.channels.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                            let value = match selector {
                                Channel::Constant(value) => value,
                                Channel::Native(source) => {
                                    let pixel =
                                        native.get(index).and_then(|pixel| pixel.get(..format.bytes_per_pixel())).ok_or(RasterInputError::LengthMismatch)?;
                                    try_read_sample_f64(pixel, format.sample, usize::from(source)).map_err(|_| RasterInputError::InvalidSample)?
                                }
                            };
                            let product = value * input.coefficient;
                            sum += product;
                            if !value.is_finite() || !product.is_finite() || !sum.is_finite() {
                                return Err(RasterInputError::InvalidSample);
                            }
                        }
                        if format.sample == SampleType::F32 && !(sum as f32).is_finite()
                            || format.alpha && channel == format.mode.color_channels() && !(0.0..=1.0).contains(&sum)
                        {
                            return Err(RasterInputError::InvalidSample);
                        }
                        try_write_sample_f64(pixel, format.sample, channel, sum).map_err(|_| RasterInputError::InvalidSample)?;
                    }
                    if request.alpha == Alpha::PreserveFirstRaw {
                        let start = format.mode.color_channels().checked_mul(format.sample.bytes()).ok_or(RasterInputError::LengthMismatch)?;
                        let alpha = native.first().and_then(|pixel| pixel.get(start..format.bytes_per_pixel())).ok_or(RasterInputError::LengthMismatch)?;
                        pixel.get_mut(start..format.bytes_per_pixel()).ok_or(RasterInputError::LengthMismatch)?.copy_from_slice(alpha);
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
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [3; 32], coordinate_space: [7; 32] }
    }
    fn input(source: &Surface, coefficient: f64) -> Input<'_> {
        Input {
            source,
            extent: Rect::new(-1, 0, 1, 1),
            context: context(),
            outside: Outside::Reject,
            coefficient,
            channels: [Channel::Native(0), Channel::Native(1), Channel::Constant(0.), Channel::Constant(0.)],
        }
    }
    fn request<'a>(inputs: &'a [Input<'a>]) -> Request<'a> {
        Request { inputs, roi: Rect::new(-1, 0, 1, 1), output_context: context(), bias: [0.; 4], alpha: Alpha::PreserveFirstRaw, identity: Identity::Evaluate }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: 64 << 20, work_units: 1 << 28 }
    }
    fn gray(color: f64, alpha: f64) -> Surface {
        Surface::try_with_default_bytes(
            PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::F64, alpha: true },
            &[color, alpha].into_iter().flat_map(f64::to_ne_bytes).collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn exact_identity_preserves_signedzero_and_opaque_rawalpha() {
        let source = gray(-0.0, f64::from_bits(0x7ff8000000000019));
        let before = source.clone();
        let inputs = [input(&source, 1.)];
        let mut request = request(&inputs);
        request.identity = Identity::PreserveExact;
        let output = try_apply(request, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(output.try_read_interleaved(request.roi).unwrap(), source.default_bytes().repeat(2));
        assert_eq!(output.default_bytes(), source.default_bytes());
        assert_eq!(output.try_read_interleaved(Rect::new(-2, -1, 2, 2)).unwrap(), source.default_bytes().repeat(12));
        // Materialized default-valued tiles may differ; the identity promises
        // exact native words, including opaque alpha, and immutable input.
        assert_eq!(source, before);
    }
    #[test]
    fn f64_hdr_sourceorder_and_zero_weight_avoid_ignored_nan_pixels() {
        let a = gray(2., 0.25);
        let mut b = gray(0., 0.);
        let nan: Vec<u8> = [f64::NAN, f64::NAN].into_iter().flat_map(f64::to_ne_bytes).collect();
        b.try_write_interleaved_with(Rect::new(0, 0, 1, 1), &nan, &Interrupt::NONE).unwrap();
        let inputs = [input(&a, 2.), input(&b, 0.)];
        let mut request = request(&inputs);
        request.bias[0] = 1.;
        let output = try_apply(request, limits(), &Interrupt::NONE).unwrap();
        let pixel = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 0).unwrap(), 5.);
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 1).unwrap(), 0.25);
        let b = gray(4., 0.75);
        let inputs = [input(&a, -1.), input(&b, 2.)];
        let output = try_apply(self::request(&inputs), limits(), &Interrupt::NONE).unwrap();
        let pixel = output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 0).unwrap(), 6.);
    }
    #[test]
    fn input_bound_native_u32_context_alpha_and_limits_refuse_atomically() {
        let source = gray(1., 0.5);
        let original = source.clone();
        let input = input(&source, 1.);
        let too_many = [input; 17];
        assert!(matches!(try_apply(request(&too_many), limits(), &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        let inputs = [input];
        let mut changed = request(&inputs);
        changed.alpha = Alpha::Compute;
        changed.bias[1] = 1.;
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        changed = request(&inputs);
        changed.output_context.profile_identity = [8; 32];
        assert!(matches!(try_apply(changed, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        assert!(matches!(try_apply(request(&inputs), Limits { resident_bytes: 1, work_units: 1 }, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert!(matches!(try_apply(request(&inputs), limits(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
        assert_eq!(source, original);
        let huge = gray(f64::MAX, 0.5);
        let inputs = [self::input(&huge, 2.0)];
        assert!(matches!(try_apply(request(&inputs), limits(), &Interrupt::NONE), Err(RasterInputError::InvalidSample)));
        let source = Surface::try_with_default_bytes(PixelFormat::RGBA32, &[0u8; 16]).unwrap();
        let inputs = [Input { source: &source, ..input }];
        assert!(matches!(try_apply(request(&inputs), limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
    }
    #[test]
    fn outside_constant_and_constant_channel_are_explicit() {
        let source = gray(2., 0.75);
        let constant: Vec<u8> = [3f64, 0.125].into_iter().flat_map(f64::to_ne_bytes).collect();
        let mut input = input(&source, 2.);
        input.extent = Rect::new(0, 0, 1, 1);
        input.outside = Outside::Constant(&constant);
        let inputs = [input];
        let output = try_apply(request(&inputs), limits(), &Interrupt::NONE).unwrap();
        let pixel = output.try_read_interleaved(Rect::new(-1, 0, 0, 1)).unwrap();
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 0).unwrap(), 6.);
        assert_eq!(try_read_sample_f64(&pixel, SampleType::F64, 1).unwrap(), 0.125);
        let inputs = [Input { channels: [Channel::Constant(4.), Channel::Constant(0.), Channel::Constant(0.), Channel::Constant(0.)], ..input }];
        let output = try_apply(request(&inputs), limits(), &Interrupt::NONE).unwrap();
        assert_eq!(try_read_sample_f64(&output.try_read_interleaved(Rect::new(0, 0, 1, 1)).unwrap(), SampleType::F64, 0).unwrap(), 8.);
    }
}
