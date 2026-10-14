//! Component-wise global range stretching from the public GEGL operation contract.
//! https://gegl.org/operations/gegl-stretch-contrast.html
//!
//! The caller owns CMS conversion and supplies linear working-space RGBA Float32.
//! This standalone kernel does not qualify an imported filter's color envelope.
//! Only the observed 0:0, keep-colors=false, perceptual=false, opaque unit-range
//! domain is implemented. Constant channels, HDR and transparency remain refused.
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_write_sample_f64};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};

/// A typed named Boolean: serialized numeric/type decoding belongs to the caller.
#[derive(Clone, Copy, Debug)]
pub struct BoolArgument<'a> {
    pub name: &'a str,
    pub value: bool,
}

/// Validated exact configuration, with no opaque positional parameter aliases.
#[derive(Clone, Copy, Debug)]
pub struct StretchContrast {
    _validated: (),
}

impl StretchContrast {
    pub fn from_named_booleans(operation: &str, version: &str, arguments: &[BoolArgument<'_>]) -> Result<Self, RasterInputError> {
        if operation != "gegl:stretch-contrast" || version != "0:0" || arguments.len() != 2 {
            return Err(RasterInputError::UnsupportedConversion);
        }
        let mut keep_colors = None;
        let mut perceptual = None;
        for argument in arguments {
            let destination = match argument.name {
                "keep-colors" => &mut keep_colors,
                "perceptual" => &mut perceptual,
                _ => return Err(RasterInputError::UnsupportedConversion),
            };
            if destination.replace(argument.value).is_some() {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
        if keep_colors != Some(false) || perceptual != Some(false) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        Ok(Self { _validated: () })
    }
}

/// Invocation-local limits; the enclosing graph owns aggregate reservations.
#[derive(Clone, Copy, Debug)]
pub struct GlobalLimits {
    pub resident_bytes: u64,
    pub work_units: u64,
}

impl Default for GlobalLimits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: if cfg!(target_arch = "wasm32") { 1 << 36 } else { 1 << 40 } }
    }
}

/// Allocation-free conservative work requirement shared with graph scheduling.
/// Counts both pixel passes and repeated growing-map visits by native tile writes.
pub(crate) fn required_work(source: &Surface, input_extent: Rect, output: Rect) -> Result<u64, RasterInputError> {
    if source.format() != PixelFormat::new(ColorMode::Rgb, SampleType::F32, true) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if input_extent.is_empty() || output.is_empty() || !input_extent.contains_rect(&output) {
        return Err(RasterInputError::InvalidBounds);
    }
    checked_region_byte_len(input_extent, source.format())?;
    checked_region_byte_len(output, source.format())?;
    let source_tiles = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    let output_tiles = tile_count(output)?;
    let repeated_map_work = source_tiles
        .checked_add(output_tiles)
        .and_then(|tiles| output_tiles.checked_mul(tiles))
        .and_then(|visits| visits.checked_mul(256))
        .ok_or(RasterInputError::LimitExceeded)?;
    input_extent
        .size()
        .area()
        .checked_add(output.size().area())
        .and_then(|pixels| pixels.checked_mul(64))
        .and_then(|work| work.checked_add(source_tiles.checked_mul(256)?))
        .and_then(|work| work.checked_add(repeated_map_work))
        .ok_or(RasterInputError::LimitExceeded)
}

/// Reduce over the entire input extent, then write only the contained ROI.
/// `input_extent` must be the caller's complete authoritative drawable extent,
/// including default pixels; deriving it from occupied/nonzero tiles is invalid.
/// Native alpha and all bytes outside the ROI remain unchanged. Cancellation or
/// refusal discards the private output and leaves the source untouched.
pub fn try_stretch_contrast(
    source: &Surface,
    output: Rect,
    input_extent: Rect,
    _configuration: StretchContrast,
    limits: GlobalLimits,
    interrupt: &Interrupt<'_>,
) -> Result<Surface, RasterInputError> {
    poll(interrupt)?;
    if source.format() != PixelFormat::new(ColorMode::Rgb, SampleType::F32, true) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    if input_extent.is_empty() || output.is_empty() || !input_extent.contains_rect(&output) {
        return Err(RasterInputError::InvalidBounds);
    }
    checked_region_byte_len(input_extent, source.format())?;
    checked_region_byte_len(output, source.format())?;
    let output_tiles = tile_count(output)?;
    let work = required_work(source, input_extent, output)?;
    if work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    let tile_bytes = u64::try_from(TILE_SIZE).map_err(|_| RasterInputError::LimitExceeded)?.pow(2) * 16;
    let peak = source
        .resident_accounted_bytes()?
        .checked_mul(2)
        .and_then(|v| v.checked_add(output_tiles.checked_mul(tile_bytes + 256)?))
        .and_then(|v| v.checked_add(tile_bytes * 3 + 4096))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
        return Err(RasterInputError::LimitExceeded);
    }
    let prepared = PreparedStretch::prepare_internal(source, input_extent, _configuration, limits, interrupt)?;
    prepared.apply_with_limits(output, limits, interrupt)
}

/// Conservative known native payload/map/header and producer tile capacities.
/// Borrowed whole-owner/provider backing beyond these surfaces is caller-held.
#[derive(Clone, Copy, Debug)]
pub struct StretchRequirements {
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub work_units: u64,
}
pub struct PreparedStretch<'source> {
    source: &'source Surface,
    extent: Rect,
    minima: [f64; 3],
    spans: [f64; 3],
    limits: GlobalLimits,
    remaining_work: std::sync::atomic::AtomicU64,
    preparation: StretchRequirements,
}
fn checked_add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn checked_mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn source_charge(source: &Surface) -> Result<u64, RasterInputError> {
    if source.format() != PixelFormat::RGBA32F || source.default_bytes().len() != 16 {
        return Err(RasterInputError::UnsupportedConversion);
    }
    checked_add(checked_mul(source.tile_count() as u64, 65536 * 16 + 256)?, 16)
}
/// O(1) prospective admission; source words are validated only after reservation.
pub fn preparation_requirements(source: &Surface, extent: Rect) -> Result<StretchRequirements, RasterInputError> {
    if extent.is_empty() {
        return Err(RasterInputError::InvalidBounds);
    }
    checked_region_byte_len(extent, source.format())?;
    tile_count(extent)?;
    let source_bytes = source_charge(source)?;
    let header = std::mem::size_of::<PreparedStretch<'_>>() as u64;
    let work = checked_add(checked_mul(extent.size().area(), 64)?, checked_mul(source.tile_count() as u64, 256)?)?;
    Ok(StretchRequirements {
        retained_bytes: checked_add(source_bytes, header)?,
        peak_resident_bytes: checked_add(source_bytes, 65536 * 16 * 2 + 4096)?,
        index_bytes: checked_add(checked_mul(source.tile_count() as u64, 256)?, header)?,
        work_units: work,
    })
}
impl<'source> PreparedStretch<'source> {
    pub fn source(&self) -> &'source Surface {
        self.source
    }
    pub fn extent(&self) -> Rect {
        self.extent
    }
    pub fn requirements(&self) -> StretchRequirements {
        self.preparation
    }
    pub fn remaining_work(&self) -> u64 {
        self.remaining_work.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// One complete ordered global reduction. All later ROIs borrow this same
    /// immutable source. Caller limits may only lower the standard ceilings.
    pub fn prepare(
        source: &'source Surface,
        extent: Rect,
        config: StretchContrast,
        limits: GlobalLimits,
        interrupt: &Interrupt<'_>,
    ) -> Result<Self, RasterInputError> {
        let standard = GlobalLimits::default();
        if limits.resident_bytes == 0 || limits.resident_bytes > standard.resident_bytes || limits.work_units > standard.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        Self::prepare_internal(source, extent, config, limits, interrupt)
    }
    fn prepare_internal(
        source: &'source Surface,
        input_extent: Rect,
        _config: StretchContrast,
        limits: GlobalLimits,
        interrupt: &Interrupt<'_>,
    ) -> Result<Self, RasterInputError> {
        poll(interrupt)?;
        let preparation = preparation_requirements(source, input_extent)?;
        let remaining = limits.work_units.checked_sub(preparation.work_units).ok_or(RasterInputError::LimitExceeded)?;
        if preparation.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) {
            return Err(RasterInputError::LimitExceeded);
        }
        source.try_validate_tile_origins_with(interrupt)?;
        for (_, tile) in source.tiles() {
            poll(interrupt)?;
            if tile.bytes().len() != 65536 * 16 {
                return Err(RasterInputError::LengthMismatch);
            }
        }
        let scratch_size = 65536 * 16;
        let mut scratch = Vec::new();
        scratch.try_reserve_exact(scratch_size).map_err(|_| RasterInputError::AllocationFailed)?;
        if scratch.capacity() as u64 > 65536 * 16 * 2 {
            return Err(RasterInputError::LimitExceeded);
        }
        scratch.resize(scratch_size, 0);
        let mut minima = [f64::INFINITY; 3];
        let mut maxima = [f64::NEG_INFINITY; 3];
        for tile in input_extent.tiles() {
            poll(interrupt)?;
            let part = tile.rect().intersect(&input_extent);
            let length = checked_region_byte_len(part, source.format())?;
            let bytes = scratch.get_mut(..length).ok_or(RasterInputError::LengthMismatch)?;
            source.try_read_interleaved_into_with(part, bytes, interrupt)?;
            for (index, pixel) in bytes.as_chunks::<16>().0.iter().enumerate() {
                if index.is_multiple_of(4096) {
                    poll(interrupt)?;
                }
                validate_pixel(pixel)?;
                for ((sample, minimum), maximum) in pixel.as_chunks::<4>().0.iter().take(3).zip(&mut minima).zip(&mut maxima) {
                    let value = f64::from(f32::from_ne_bytes(*sample));
                    *minimum = minimum.min(value);
                    *maximum = maximum.max(value);
                }
            }
        }
        let mut spans = [0.0; 3];
        for ((span, maximum), minimum) in spans.iter_mut().zip(maxima).zip(minima) {
            *span = maximum - minimum;
            if !span.is_finite() || *span <= 0.0 {
                return Err(RasterInputError::UnsupportedConversion);
            }
        }
        poll(interrupt)?;
        Ok(Self { source, extent: input_extent, minima, spans, limits, remaining_work: std::sync::atomic::AtomicU64::new(remaining), preparation })
    }
    /// Reads and writes only the contained ROI; no repeated extrema pass.
    pub fn application_requirements(&self, output: Rect) -> Result<StretchRequirements, RasterInputError> {
        if output.is_empty() || !self.extent.contains_rect(&output) {
            return Err(RasterInputError::InvalidBounds);
        }
        checked_region_byte_len(output, self.source.format())?;
        let output_tiles = tile_count(output)?;
        let source_tiles = self.source.tile_count() as u64;
        let source_bytes = source_charge(self.source)?;
        let peak = checked_add(checked_add(checked_mul(source_bytes, 2)?, checked_mul(output_tiles, 65536 * 16 + 256)?)?, 65536 * 16 * 3 + 4096)?;
        let work =
            checked_add(checked_mul(output.size().area(), 64)?, checked_mul(checked_mul(output_tiles, checked_add(source_tiles, output_tiles)?)?, 256)?)?;
        Ok(StretchRequirements {
            retained_bytes: checked_add(source_bytes, checked_mul(output_tiles, 65536 * 16 + 256)?)?,
            peak_resident_bytes: peak,
            index_bytes: checked_mul(checked_add(source_tiles, output_tiles)?, 512)?,
            work_units: work,
        })
    }
    pub fn apply_with_limits(&self, output: Rect, limits: GlobalLimits, interrupt: &Interrupt<'_>) -> Result<Surface, RasterInputError> {
        poll(interrupt)?;
        if limits.resident_bytes > self.limits.resident_bytes || limits.work_units > self.limits.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        let needs = self.application_requirements(output)?;
        if needs.peak_resident_bytes > limits.resident_bytes.min(MAX_CHECKED_RASTER_BYTES) || needs.work_units > limits.work_units {
            return Err(RasterInputError::LimitExceeded);
        }
        self.remaining_work
            .try_update(std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed, |left| left.checked_sub(needs.work_units))
            .map_err(|_| RasterInputError::LimitExceeded)?;
        // No reset/refund on cancellation or failure; concurrent ROI debits cannot overdraw.
        let source = self.source;
        let minima = self.minima;
        let spans = self.spans;
        let scratch_size = 65536 * 16;
        let mut scratch = Vec::new();
        scratch.try_reserve_exact(scratch_size).map_err(|_| RasterInputError::AllocationFailed)?;
        if scratch.capacity() as u64 > 65536 * 16 * 2 {
            return Err(RasterInputError::LimitExceeded);
        }
        scratch.resize(scratch_size, 0);
        let mut result = source.clone();
        for tile in output.tiles() {
            poll(interrupt)?;
            let part = tile.rect().intersect(&output);
            let length = checked_region_byte_len(part, source.format())?;
            let bytes = scratch.get_mut(..length).ok_or(RasterInputError::LengthMismatch)?;
            source.try_read_interleaved_into_with(part, bytes, interrupt)?;
            for (index, pixel) in bytes.as_chunks_mut::<16>().0.iter_mut().enumerate() {
                if index.is_multiple_of(4096) {
                    poll(interrupt)?;
                }
                // All input samples were checked in the immutable first pass.
                for (channel, (minimum, span)) in minima.iter().zip(spans).enumerate() {
                    let sample = pixel.as_chunks::<4>().0.get(channel).ok_or(RasterInputError::LengthMismatch)?;
                    let value = (f64::from(f32::from_ne_bytes(*sample)) - minimum) / span;
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(RasterInputError::InvalidSample);
                    }
                    try_write_sample_f64(pixel, SampleType::F32, channel, value).map_err(|_| RasterInputError::InvalidSample)?;
                }
            }
            result.try_write_interleaved_with(part, bytes, interrupt)?;
        }
        poll(interrupt)?;
        Ok(result)
    }
}

fn validate_pixel(pixel: &[u8; 16]) -> Result<(), RasterInputError> {
    for (channel, sample) in pixel.as_chunks::<4>().0.iter().enumerate() {
        let value = f32::from_ne_bytes(*sample);
        if !value.is_finite() || !(0.0..=1.0).contains(&value) || channel == 3 && value != 1.0 {
            return Err(RasterInputError::UnsupportedConversion);
        }
    }
    Ok(())
}

fn tile_count(bounds: Rect) -> Result<u64, RasterInputError> {
    let first = TileCoord::containing(bounds.x0, bounds.y0);
    let last = TileCoord::containing(
        bounds.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
        bounds.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?,
    );
    let width = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let height = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    width.checked_mul(height).ok_or(RasterInputError::LimitExceeded)
}

fn poll(interrupt: &Interrupt<'_>) -> Result<(), RasterInputError> {
    if interrupt.cancelled() { Err(RasterInputError::Cancelled) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration() -> StretchContrast {
        StretchContrast::from_named_booleans(
            "gegl:stretch-contrast",
            "0:0",
            &[BoolArgument { name: "perceptual", value: false }, BoolArgument { name: "keep-colors", value: false }],
        )
        .unwrap()
    }

    fn source(values: &[[f32; 4]]) -> Surface {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        let bytes: Vec<_> = values.iter().flatten().flat_map(|value| value.to_ne_bytes()).collect();
        surface.try_write_interleaved_with(Rect::new(0, 0, values.len() as i32, 1), &bytes, &Interrupt::NONE).unwrap();
        surface
    }

    #[test]
    fn split_preparation_two_rois_equal_legacy_and_cumulative_quota_cannot_reset() {
        let input = source(&[[0., 0.125, 0.25, 1.], [0.25, 0.5, 0.5, 1.], [0.5, 0.875, 0.75, 1.]]);
        let extent = Rect::new(0, 0, 3, 1);
        let roi = Rect::new(1, 0, 2, 1);
        let before = input.clone();
        let prep = preparation_requirements(&input, extent).unwrap();
        let repeated = tile_count(roi).unwrap() * (input.tile_count() as u64 + tile_count(roi).unwrap()) * 256;
        let apply_work = roi.size().area() * 64 + repeated;
        let limits = GlobalLimits { resident_bytes: GlobalLimits::default().resident_bytes, work_units: prep.work_units + apply_work };
        let prepared = PreparedStretch::prepare(&input, extent, configuration(), limits, &Interrupt::NONE).unwrap();
        let result = prepared.apply_with_limits(roi, limits, &Interrupt::NONE).unwrap();
        let legacy = try_stretch_contrast(&input, roi, extent, configuration(), limits, &Interrupt::NONE).unwrap();
        assert_eq!(result, legacy);
        assert_eq!(result.try_read_interleaved(roi).unwrap(), [0.5f32, 0.5, 0.5, 1.].into_iter().flat_map(f32::to_ne_bytes).collect::<Vec<_>>());
        assert_eq!(prepared.remaining_work(), 0);
        assert!(matches!(prepared.apply_with_limits(roi, limits, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        assert_eq!(input, before);
    }
    #[test]
    fn late_roi_cancellation_keeps_source_and_consumes_prepaid_work() {
        let input = source(&[[0., 0., 0., 1.], [1., 1., 1., 1.]]);
        let before = input.clone();
        let extent = Rect::new(0, 0, 2, 1);
        let limits = GlobalLimits::default();
        let prepared = PreparedStretch::prepare(&input, extent, configuration(), limits, &Interrupt::NONE).unwrap();
        let initial = prepared.remaining_work();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 1;
        assert!(matches!(prepared.apply_with_limits(extent, limits, &Interrupt::cancel_only(&cancelled)), Err(RasterInputError::Cancelled)));
        assert!(prepared.remaining_work() < initial);
        assert_eq!(input, before);
    }

    #[test]
    fn global_statistics_are_not_reduced_to_the_roi_and_native_outside_bytes_stay_exact() {
        let input = source(&[[0.0, 0.125, 0.25, 1.0], [0.25, 0.5, 0.5, 1.0], [0.5, 0.875, 0.75, 1.0]]);
        let extent = Rect::new(0, 0, 3, 1);
        let original = input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        let result = try_stretch_contrast(&input, Rect::new(1, 0, 2, 1), extent, configuration(), GlobalLimits::default(), &Interrupt::NONE).unwrap();
        let actual = result.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        assert_eq!(&actual[..16], &original[..16]);
        assert_eq!(&actual[32..], &original[32..]);
        assert_eq!(actual[16..32].as_chunks::<4>().0.iter().map(|v| f32::from_ne_bytes(*v)).collect::<Vec<_>>(), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap(), original);
    }

    #[test]
    fn unsupported_configuration_and_scope_are_refused() {
        for (operation, version, arguments) in [
            ("gegl:stretch-contrast", "0:1", vec![BoolArgument { name: "keep-colors", value: false }, BoolArgument { name: "perceptual", value: false }]),
            ("gegl:stretch-contrast", "0:0", vec![BoolArgument { name: "keep-colors", value: true }, BoolArgument { name: "perceptual", value: false }]),
            ("gegl:stretch-contrast", "0:0", vec![BoolArgument { name: "keep-colors", value: false }, BoolArgument { name: "keep-colors", value: false }]),
            ("opaque-triplet", "0:0", vec![]),
        ] {
            assert!(StretchContrast::from_named_booleans(operation, version, &arguments).is_err());
        }
        for value in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            let input = source(&[[value, 0.25, 0.25, 1.0], [0.75, 0.75, 0.75, 1.0]]);
            assert!(
                try_stretch_contrast(&input, Rect::new(0, 0, 2, 1), Rect::new(0, 0, 2, 1), configuration(), GlobalLimits::default(), &Interrupt::NONE).is_err()
            );
        }
        for values in [[[0.25, 0.25, 0.25, 1.0]; 2], [[0.25, 0.25, 0.25, 0.5], [0.75, 0.75, 0.75, 1.0]]] {
            let input = source(&values);
            assert!(
                try_stretch_contrast(&input, Rect::new(0, 0, 2, 1), Rect::new(0, 0, 2, 1), configuration(), GlobalLimits::default(), &Interrupt::NONE).is_err()
            );
        }
    }

    #[test]
    fn cancellation_and_preflight_limits_leave_the_source_unchanged() {
        let input = source(&[[0.25, 0.25, 0.25, 1.0], [0.75, 0.75, 0.75, 1.0]]);
        let extent = Rect::new(0, 0, 2, 1);
        let original = input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        for limits in [GlobalLimits { resident_bytes: 1, work_units: u64::MAX }, GlobalLimits { resident_bytes: u64::MAX, work_units: 1 }] {
            assert!(matches!(try_stretch_contrast(&input, extent, extent, configuration(), limits, &Interrupt::NONE), Err(RasterInputError::LimitExceeded)));
        }
        assert!(matches!(
            try_stretch_contrast(&input, extent, extent, configuration(), GlobalLimits::default(), &Interrupt::cancel_only(&|| true)),
            Err(RasterInputError::Cancelled)
        ));
        assert_eq!(input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap(), original);
    }

    #[test]
    fn thin_multi_tile_roi_refuses_a_budget_that_omits_repeated_map_visits() {
        let values: Vec<_> = (0..4097)
            .map(|index| {
                let value = index as f32 / 4096.0;
                [value, value, value, 1.0]
            })
            .collect();
        let input = source(&values);
        let extent = Rect::new(0, 0, 4097, 1);
        let original = input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        let pixel_only_work = extent.size().area() * 128 + input.tile_count() as u64 * 256;
        assert!(required_work(&input, extent, extent).unwrap() > pixel_only_work);
        assert!(matches!(
            try_stretch_contrast(
                &input,
                extent,
                extent,
                configuration(),
                GlobalLimits { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: pixel_only_work },
                &Interrupt::NONE
            ),
            Err(RasterInputError::LimitExceeded)
        ));
        assert_eq!(input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap(), original);
    }

    #[test]
    fn cancellation_during_reduction_or_private_writes_never_changes_input() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let values: Vec<_> = (0..513).map(|i| [i as f32 / 512.0, i as f32 / 1024.0, 0.25 + i as f32 / 1024.0, 1.0]).collect();
        let input = source(&values);
        let extent = Rect::new(0, 0, 513, 1);
        let original = input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        let complete_polls = AtomicUsize::new(0);
        let observe = || {
            complete_polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        try_stretch_contrast(&input, extent, extent, configuration(), GlobalLimits::default(), &Interrupt::cancel_only(&observe)).unwrap();
        let mut successes = 0;
        let mut refusals = 0;
        for threshold in 0..=complete_polls.load(Ordering::Relaxed) + 1 {
            let polls = AtomicUsize::new(0);
            let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= threshold;
            match try_stretch_contrast(&input, extent, extent, configuration(), GlobalLimits::default(), &Interrupt::cancel_only(&cancel)) {
                Ok(_) => successes += 1,
                Err(RasterInputError::Cancelled) => refusals += 1,
                Err(error) => panic!("unexpected cancellation control error: {error}"),
            }
            assert_eq!(input.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap(), original);
        }
        assert!(successes > 0 && refusals > 0);
    }

    /// The higher-layer CMS observer must prepare these explicit linear RGBAF32
    /// buffers from the original drawable and the independently authored edited
    /// PNG using its checked source/current context. No builtin profile is assumed.
    /// This comparison alone cannot qualify the import envelope or other domains.
    #[test]
    #[ignore = "requires retained local oracle and higher-layer CMS-prepared linear input/reference"]
    fn retained_false_false_edit_with_explicit_caller_cms_buffers() {
        use sha2::{Digest, Sha256};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/operations/gegl-stretch-contrast");
        let probe: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("probe.json")).unwrap()).unwrap();
        assert_eq!(probe["operation"], "gegl:stretch-contrast");
        assert_eq!(probe["edit_parameter"], serde_json::json!(["keep-colors", false]));
        let case = probe["cases"].as_array().unwrap().iter().find(|case| case["id"] == "operation-gegl-stretch-contrast").unwrap();
        assert_eq!(case["state_edited"]["size"], serde_json::json!([32, 32]));
        let filter = &case["state_edited"]["layers"][0]["filters"][0];
        assert_eq!(filter["operation"], "gegl:stretch-contrast");
        assert_eq!(filter["visible"], true);
        assert_eq!(filter["config"], serde_json::json!({"keep-colors": false, "perceptual": false}));
        for (file, digest) in
            [("input", "input_sha256"), ("reference_edit", "reference_edit_sha256"), ("reference_edit_render", "reference_edit_render_sha256")]
        {
            let bytes = std::fs::read(root.join(case[file].as_str().unwrap())).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), case[digest].as_str().unwrap());
        }
        let input = std::fs::read(std::env::var_os("PHOTOCRAFT_STRETCH_LINEAR_INPUT").expect("caller CMS-prepared native-endian RGBAF32 input path")).unwrap();
        let expected =
            std::fs::read(std::env::var_os("PHOTOCRAFT_STRETCH_LINEAR_EXPECTED").expect("same-context linear conversion of retained edited PNG path")).unwrap();
        let extent = Rect::new(0, 0, 32, 32);
        let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        assert_eq!(input.len(), checked_region_byte_len(extent, format).unwrap());
        assert_eq!(expected.len(), input.len());
        let mut surface = Surface::new(format);
        surface.try_write_interleaved_with(extent, &input, &Interrupt::NONE).unwrap();
        let result = try_stretch_contrast(&surface, extent, extent, configuration(), GlobalLimits::default(), &Interrupt::NONE).unwrap();
        let actual = result.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap();
        for (observed, reference) in actual.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0) {
            let observed = f32::from_ne_bytes(*observed);
            let reference = f32::from_ne_bytes(*reference);
            assert!(reference.is_finite() && (0.0..=1.0).contains(&reference));
            assert!((observed - reference).abs() <= 2.0 / 255.0, "linear sample {observed} versus {reference}");
        }
        assert_eq!(surface.try_read_interleaved_with(extent, &Interrupt::NONE).unwrap(), input);
    }
}
