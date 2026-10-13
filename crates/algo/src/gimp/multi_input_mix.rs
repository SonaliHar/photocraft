//! Original straight-channel interpolation, not a GEGL/compositing equivalent.
//! In the caller's explicit matching working domain, each channel (including
//! straight alpha) is `(1-right_weight)*left + right_weight*right`. Hidden colors
//! are mixed as straight values; no premultiplication or alpha normalization occurs.
use photocraft_color::{ColorMode, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::SampleEncoding;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    /// Caller-established profile interpretation identity, never a profile guess.
    pub profile_identity: [u8; 32],
}

#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub source: &'a Surface,
    /// Full authoritative source bounds, including explicitly owned default pixels.
    pub extent: Rect,
    pub context: WorkingContext,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Abyss {
    /// Both authoritative extents must contain the entire requested ROI.
    Reject,
}

/// Required work, including growing output-map/native-writer visits. No allocation.
pub fn required_work(left: Input<'_>, right: Input<'_>, roi: Rect) -> Result<u64, RasterInputError> {
    validate_domain(left, right, roi)?;
    let tiles = tile_count(roi)?;
    let source_tiles = (left.source.tile_count() as u64).checked_add(right.source.tile_count() as u64).ok_or(RasterInputError::LimitExceeded)?;
    roi.size()
        .area()
        .checked_mul(left.source.channels() as u64)
        .and_then(|n| n.checked_mul(32))
        .and_then(|n| n.checked_add(tiles.checked_mul(source_tiles.checked_add(tiles)?)?.checked_mul(256)?))
        .and_then(|n| n.checked_add(source_tiles.checked_mul(256)?))
        .ok_or(RasterInputError::LimitExceeded)
}

fn validate_domain(left: Input<'_>, right: Input<'_>, roi: Rect) -> Result<(), RasterInputError> {
    let format = left.source.format();
    if left.context != right.context
        || format != right.source.format()
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_region_byte_len(left.extent, format)?;
    checked_region_byte_len(right.extent, format)?;
    checked_region_byte_len(roi, format)?;
    if roi.is_empty() || left.extent.is_empty() || right.extent.is_empty() || !left.extent.contains_rect(&roi) || !right.extent.contains_rect(&roi) {
        return Err(RasterInputError::InvalidBounds);
    }
    validate_pixel(left.source.default_bytes(), format)?;
    validate_pixel(right.source.default_bytes(), format)?;
    Ok(())
}

/// Private ROI-only output with the caller's explicit native outside-ROI default.
/// The endpoints copy exact native input words. Intermediate weights perform
/// explicit f64 products/sum, then one native F32/F64 quantization. All nonfinite
/// results refuse; no clipping of finite HDR colors is applied.
#[allow(clippy::too_many_arguments)]
pub fn try_mix(
    left: Input<'_>,
    right: Input<'_>,
    roi: Rect,
    right_weight: f64,
    output_default: &[u8],
    abyss: Abyss,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    poll(control)?;
    validate_domain(left, right, roi)?;
    if !right_weight.is_finite() || !(0.0..=1.0).contains(&right_weight) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    match abyss {
        Abyss::Reject => {}
    }
    let format = left.source.format();
    validate_pixel(output_default, format)?;
    let work = required_work(left, right, roi)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(format.bytes_per_pixel() as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let peak = left
        .source
        .resident_accounted_bytes()?
        .checked_add(right.source.resident_accounted_bytes()?)
        .and_then(|n| n.checked_add(tile_count(roi).ok()?.checked_mul(tile_bytes.checked_add(512)?)?))
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(4)?))
        .and_then(|n| n.checked_add(4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    left.source.try_validate_tile_origins_with(control)?;
    right.source.try_validate_tile_origins_with(control)?;
    let mut output = Surface::try_with_default_bytes(format, output_default)?;
    for tile in roi.tiles() {
        poll(control)?;
        let part = tile.rect().intersect(&roi);
        let length = checked_region_byte_len(part, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| RasterInputError::AllocationFailed)?;
        bytes.resize(length, 0);
        let mut offset = 0usize;
        for y in part.y0..part.y1 {
            poll(control)?;
            for x in part.x0..part.x1 {
                if x.abs_diff(part.x0).is_multiple_of(256) {
                    poll(control)?;
                }
                let end = offset.checked_add(format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
                let pixel = bytes.get_mut(offset..end).ok_or(RasterInputError::LengthMismatch)?;
                let mut a = [0.0; 4];
                let mut b = [0.0; 4];
                let channels = format.channels();
                left.source.try_read_pixel_f64(x, y, a.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?)?;
                right.source.try_read_pixel_f64(x, y, b.get_mut(..channels).ok_or(RasterInputError::LengthMismatch)?)?;
                for (channel, (a, b)) in a.iter().zip(b).take(channels).enumerate() {
                    let alpha = format.alpha && channel == format.mode.color_channels();
                    if !a.is_finite() || !b.is_finite() || alpha && (!(0.0..=1.0).contains(a) || !(0.0..=1.0).contains(&b)) {
                        return Err(RasterInputError::InvalidSample);
                    }
                }
                if right_weight == 0.0 || right_weight == 1.0 {
                    let selected = if right_weight == 0.0 { left.source } else { right.source };
                    let pixel_bounds =
                        Rect::new(x, y, x.checked_add(1).ok_or(RasterInputError::InvalidBounds)?, y.checked_add(1).ok_or(RasterInputError::InvalidBounds)?);
                    selected.try_read_interleaved_into_with(pixel_bounds, pixel, control)?;
                    validate_pixel(pixel, format)?;
                } else {
                    for (channel, (a, b)) in a.iter().zip(b).take(channels).enumerate() {
                        let left_part = (1.0 - right_weight) * a;
                        let right_part = right_weight * b;
                        let value = left_part + right_part;
                        if !left_part.is_finite()
                            || !right_part.is_finite()
                            || !value.is_finite()
                            || format.sample == SampleType::F32 && !(value as f32).is_finite()
                        {
                            return Err(RasterInputError::InvalidSample);
                        }
                        try_write_sample_f64(pixel, format.sample, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                    }
                }
                offset = end;
            }
        }
        output.try_write_interleaved_with(part, &bytes, control)?;
    }
    poll(control)?;
    Ok(output)
}

fn validate_pixel(pixel: &[u8], format: photocraft_color::PixelFormat) -> Result<(), RasterInputError> {
    if pixel.len() != format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    for channel in 0..format.channels() {
        let value = try_read_sample_f64(pixel, format.sample, channel).map_err(|_| RasterInputError::InvalidSample)?;
        if !value.is_finite() || format.alpha && channel == format.mode.color_channels() && !(0.0..=1.0).contains(&value) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    Ok(())
}

fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    if roi.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}

/// Explicit own mathematical graph schema, never a stock-operation alias.
pub struct GraphBinding<'a> {
    pub node: photocraft_doc::item_identity::GraphNodeId,
    pub identity: &'a photocraft_doc::xcf_filters::OperationIdentity,
    pub version_origin: photocraft_doc::xcf_filters::GraphVersionOrigin,
    pub context: WorkingContext,
    pub left_extent: Rect,
    pub right_extent: Rect,
    pub right_weight: f64,
    pub output_default: &'a [u8],
}

pub fn accepts_port(direction: super::graph_plan::PortDirection, port: &str) -> bool {
    match direction {
        super::graph_plan::PortDirection::Input => matches!(port, "left" | "right"),
        super::graph_plan::PortDirection::Output => port == "output",
    }
}

pub fn evaluate_graph(
    binding: GraphBinding<'_>,
    request: super::graph_execute::NodeRequest<'_>,
    budget: &mut super::graph_execute::NodeBudget<'_, '_>,
) -> Result<super::graph_execute::NodeOutput, super::graph_execute::GraphExecuteError> {
    use super::graph_execute::GraphExecuteError as Error;
    use photocraft_doc::xcf_filters::{ExactNumber, FilterValue};
    budget.poll()?;
    if binding.node.0 == 0
        || binding.node != request.node.id
        || binding.identity.name != "own:straight-channel-mix"
        || binding.identity.stored_version != "1"
        || binding.identity.schema_epoch.is_empty()
        || binding.identity.schema_epoch.len() > 256
        || binding.identity != &request.node.operation
        || binding.version_origin != request.node.version_origin
    {
        return Err(Error::Operation("mix requires exact caller-bound own straight-channel schema"));
    }
    if binding.context.encoding != request.domain.encoding || request.domain.profile_fingerprint != Some(binding.context.profile_identity) {
        return Err(Error::Domain);
    }
    if binding.left_extent != request.roi || binding.right_extent != request.roi {
        return Err(Error::Operation("mix graph bridge requires both complete authoritative inputs to match fixed ROI"));
    }
    if request.node.arguments.len() != 1
        || !request.node.arguments.first().is_some_and(|argument| {
            argument.name == "right-weight"
                && matches!(argument.value, FilterValue::Number(ExactNumber::Float64Bits(bits)) if bits == binding.right_weight.to_bits())
        })
        || !binding.right_weight.is_finite()
        || !(0.0..=1.0).contains(&binding.right_weight)
    {
        return Err(Error::Operation("mix requires exact finite unit Float64 right-weight"));
    }
    if request.inputs.len() != 2 {
        return Err(Error::Operation("mix requires exactly left and right inputs"));
    }
    let (mut left, mut right) = (None, None);
    for input in request.inputs {
        if input.output_port != "output" {
            return Err(Error::Operation("mix input requires exact upstream output port"));
        }
        let destination = match input.input_port {
            "left" => &mut left,
            "right" => &mut right,
            _ => return Err(Error::Operation("unknown mix input port")),
        };
        if destination.replace(input.surface).is_some() {
            return Err(Error::Operation("duplicate mix input port"));
        }
    }
    let left = Input { source: left.ok_or(Error::MissingInput)?, extent: binding.left_extent, context: binding.context };
    let right = Input { source: right.ok_or(Error::MissingInput)?, extent: binding.right_extent, context: binding.context };
    if left.source.format() != request.domain.format {
        return Err(Error::Domain);
    }
    validate_domain(left, right, request.roi).map_err(Error::Raster)?;
    validate_pixel(binding.output_default, request.domain.format).map_err(Error::Raster)?;
    let tiles = tile_count(request.roi).map_err(Error::Raster)?;
    let bpp = request.domain.format.bytes_per_pixel() as u64;
    let tile_bytes = (TILE_SIZE as u64).checked_pow(2).and_then(|n| n.checked_mul(bpp)).ok_or(Error::ResidentLimit)?;
    let source_tiles = (left.source.tile_count() as u64).checked_add(right.source.tile_count() as u64).ok_or(Error::ResidentLimit)?;
    let sources = source_tiles.checked_mul(tile_bytes + 256).and_then(|n| n.checked_add((256 + bpp) * 2)).ok_or(Error::ResidentLimit)?;
    let output = tiles.checked_mul(tile_bytes + 256).and_then(|n| n.checked_add(256 + bpp)).ok_or(Error::ResidentLimit)?;
    let index = tiles.checked_mul(512).and_then(|n| n.checked_add(1024)).ok_or(Error::IndexLimit)?;
    let scratch = sources.checked_add(tile_bytes.checked_mul(4).ok_or(Error::ResidentLimit)?).and_then(|n| n.checked_add(4096)).ok_or(Error::ResidentLimit)?;
    let work = required_work(left, right, request.roi).map_err(Error::Raster)?;
    budget.reserve_index(index)?;
    budget.reserve_output(output)?;
    budget.reserve_scratch(scratch)?;
    budget.charge_work(work)?;
    let resident = output.checked_add(index).and_then(|n| n.checked_add(scratch)).ok_or(Error::ResidentLimit)?.min(MAX_CHECKED_RASTER_BYTES);
    let output = {
        let cancelled = || budget.poll().is_err();
        try_mix(
            left,
            right,
            request.roi,
            binding.right_weight,
            binding.output_default,
            Abyss::Reject,
            Limits { resident_bytes: resident, work_units: work },
            &Interrupt::cancel_only(&cancelled),
        )
        .map_err(Error::Raster)?
    };
    budget.finish_output(output, request.roi, request.domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    fn native(format: PixelFormat, values: &[f64]) -> Vec<u8> {
        let mut pixel = vec![0; format.bytes_per_pixel()];
        assert_eq!(values.len(), format.channels());
        for (channel, value) in values.iter().enumerate() {
            try_write_sample_f64(&mut pixel, format.sample, channel, *value).unwrap();
        }
        pixel
    }
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [42; 32] }
    }
    fn limits() -> Limits {
        Limits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1 << 30 }
    }

    #[test]
    fn straight_zero_alpha_hidden_colors_mix_without_premultiplication_at_both_depths() {
        let roi = Rect::new(0, 0, 1, 1);
        for sample in [SampleType::F32, SampleType::F64] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat::new(mode, sample, true);
                let left_values: &[f64] = if mode == ColorMode::Rgb { &[1.0, 0.0, 1.0, 0.0] } else { &[1.0, 0.0] };
                let right_values: &[f64] = if mode == ColorMode::Rgb { &[0.0, 1.0, 0.0, 0.0] } else { &[0.0, 0.0] };
                let expected: &[f64] = if mode == ColorMode::Rgb { &[0.5, 0.5, 0.5, 0.0] } else { &[0.5, 0.0] };
                let left_raw = native(format, left_values);
                let right_raw = native(format, right_values);
                let left_surface = Surface::try_with_default_bytes(format, &left_raw).unwrap();
                let right_surface = Surface::try_with_default_bytes(format, &right_raw).unwrap();
                let left = Input { source: &left_surface, extent: roi, context: context() };
                let right = Input { source: &right_surface, extent: roi, context: context() };
                let output = try_mix(left, right, roi, 0.5, &right_raw, Abyss::Reject, limits(), &Interrupt::NONE).unwrap();
                assert_eq!(output.try_read_interleaved_with(roi, &Interrupt::NONE).unwrap(), native(format, expected));
                assert_eq!(output.default_bytes(), right_raw);
                assert_eq!(left_surface.default_bytes(), left_raw);
                assert_eq!(right_surface.default_bytes(), right_raw);
            }
        }
    }

    #[test]
    fn exact_endpoints_preserve_negative_zero_and_inputs_remain_immutable() {
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F64, true);
        let left_raw = native(format, &[-0.0, 0.25, 2.0, -0.0]);
        let right_raw = native(format, &[0.0, 0.75, -2.0, 1.0]);
        let left_surface = Surface::try_with_default_bytes(format, &left_raw).unwrap();
        let right_surface = Surface::try_with_default_bytes(format, &right_raw).unwrap();
        let roi = Rect::new(-1, -1, 1, 1);
        let left = Input { source: &left_surface, extent: roi, context: context() };
        let right = Input { source: &right_surface, extent: roi, context: context() };
        for (weight, expected) in [(0.0, &left_raw), (1.0, &right_raw)] {
            let output = try_mix(left, right, roi, weight, &right_raw, Abyss::Reject, limits(), &Interrupt::NONE).unwrap();
            assert_eq!(output.try_read_interleaved_with(Rect::new(0, 0, 1, 1), &Interrupt::NONE).unwrap(), *expected);
        }
        assert_eq!(left_surface.default_bytes(), left_raw);
        assert_eq!(right_surface.default_bytes(), right_raw);
        assert_eq!(left_surface.tile_count(), 0);
        assert_eq!(right_surface.tile_count(), 0);
    }

    #[test]
    fn mismatched_context_bounds_limits_and_cancellation_refuse_without_input_writes() {
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let raw = native(format, &[0.5]);
        let surface = Surface::try_with_default_bytes(format, &raw).unwrap();
        let roi = Rect::new(0, 0, 2, 1);
        let input = Input { source: &surface, extent: roi, context: context() };
        let wrong = Input { context: WorkingContext { profile_identity: [43; 32], ..context() }, ..input };
        assert!(matches!(try_mix(input, wrong, roi, 0.5, &raw, Abyss::Reject, limits(), &Interrupt::NONE), Err(RasterInputError::UnsupportedConversion)));
        assert!(matches!(
            try_mix(input, input, Rect::new(0, 0, 3, 1), 0.5, &raw, Abyss::Reject, limits(), &Interrupt::NONE),
            Err(RasterInputError::InvalidBounds)
        ));
        for limited in [Limits { resident_bytes: 1, ..limits() }, Limits { work_units: 1, ..limits() }] {
            assert!(matches!(try_mix(input, input, roi, 0.5, &raw, Abyss::Reject, limited, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        }
        assert!(matches!(try_mix(input, input, roi, 0.5, &raw, Abyss::Reject, limits(), &Interrupt::cancel_only(&|| true)), Err(RasterInputError::Cancelled)));
        assert_eq!(surface.default_bytes(), raw);
        assert_eq!(surface.tile_count(), 0);
    }
}
