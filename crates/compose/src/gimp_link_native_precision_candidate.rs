//! Explicit PNG16/current-native preparation candidate. No import dispatch join.
//! Observation scope: owned nonlinear RGBA PNG16, GIMP3.2.6, matching caller CMS
//! context. F64 STORAGE here is explicitly promoted F32, never arbitrary F64 input.
use crate::gimp_composite::{CompositionError, Result};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64, try_write_sample_f64};
use photocraft_doc::{Rect, SampleEncoding, xcf_items::TransformState};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingContext {
    pub encoding: SampleEncoding,
    /// Explicit caller-established current CMS identity; no profile name inference.
    pub profile_identity: [u8; 32],
}
#[derive(Clone, Copy, Debug)]
pub enum PreparationPolicy {
    PreserveU16Words,
    NormalizeU16ToF32,
    NormalizeU16ToF32ThenPromoteF64,
}
#[derive(Clone, Copy, Debug)]
pub enum FinalQuantization {
    Float32ThenU16Nearest,
    Float32,
    Float32ThenPromoteF64,
}
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Caller-authorized, decoded, same-current-context RGBA16 native words.
    /// Original encoded resource/cache bytes are never mutated or read via paths.
    pub raw_source: &'a [u8],
    pub source_format: PixelFormat,
    pub current_format: PixelFormat,
    pub source_extent: Rect,
    pub source_context: WorkingContext,
    pub current_context: WorkingContext,
    pub policy: PreparationPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_retained_bytes: u64,
    pub prepared_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
pub struct PreparedNativeSource {
    bytes: Vec<u8>,
    format: PixelFormat,
    extent: Rect,
    context: WorkingContext,
}
impl PreparedNativeSource {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn extent(&self) -> Rect {
        self.extent
    }
    pub fn context(&self) -> WorkingContext {
        self.context
    }
}
fn unsupported() -> CompositionError {
    CompositionError::Unsupported("unqualified linked native precision candidate")
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn validate_limits(limits: Limits) -> Result<()> {
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 8_000_000_000 {
        return Err(limit());
    }
    Ok(())
}
fn target_sample(policy: PreparationPolicy) -> SampleType {
    match policy {
        PreparationPolicy::PreserveU16Words => SampleType::U16,
        PreparationPolicy::NormalizeU16ToF32 => SampleType::F32,
        PreparationPolicy::NormalizeU16ToF32ThenPromoteF64 => SampleType::F64,
    }
}
pub fn requirements(request: Request<'_>) -> Result<Requirements> {
    let expected = PixelFormat::new(ColorMode::Rgb, target_sample(request.policy), true);
    if request.source_format != PixelFormat::new(ColorMode::Rgb, SampleType::U16, true)
        || request.current_format != expected
        || request.source_context != request.current_context
        || request.current_context.encoding != SampleEncoding::NonlinearProfile
        || request.current_context.profile_identity == [0; 32]
    {
        return Err(unsupported());
    }
    let r = request.source_extent;
    if r.is_empty() || r.width() > 8192 || r.height() > 8192 || r.size().area() > 4_000_000 {
        return Err(unsupported());
    }
    let source = checked_region_byte_len(r, request.source_format)?;
    if source != request.raw_source.len() {
        return Err(RasterInputError::LengthMismatch.into());
    }
    let encoded = checked_region_byte_len(r, request.current_format)? as u64;
    let prepared_bytes = encoded.checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(limit)?;
    let peak_resident_bytes = (source as u64).checked_add(prepared_bytes).and_then(|n| n.checked_add(4096)).ok_or_else(limit)?;
    let work_units = r.size().area().checked_mul(256).and_then(|n| n.checked_add(prepared_bytes.checked_mul(4)?)).ok_or_else(limit)?;
    Ok(Requirements { source_retained_bytes: source as u64, prepared_bytes, peak_resident_bytes, work_units })
}
pub fn prepare(request: Request<'_>, limits: Limits, c: &Interrupt<'_>) -> Result<PreparedNativeSource> {
    poll(c)?;
    validate_limits(limits)?;
    let req = requirements(request)?;
    if req.peak_resident_bytes > limits.resident_bytes || req.work_units > limits.work_units {
        return Err(limit());
    }
    let size = checked_region_byte_len(request.source_extent, request.current_format)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| RasterInputError::AllocationFailed)?;
    if bytes.capacity() as u64 > req.prepared_bytes {
        return Err(limit());
    }
    while bytes.len() < size {
        poll(c)?;
        let end = bytes.len().checked_add((size - bytes.len()).min(64 << 10)).ok_or_else(limit)?;
        bytes.resize(end, 0);
    }
    if matches!(request.policy, PreparationPolicy::PreserveU16Words) {
        for (input, output) in request.raw_source.chunks(64 << 10).zip(bytes.chunks_mut(64 << 10)) {
            poll(c)?;
            output.copy_from_slice(input);
        }
    } else {
        for (i, (input, output)) in request.raw_source.chunks_exact(8).zip(bytes.chunks_exact_mut(request.current_format.bytes_per_pixel())).enumerate() {
            if i.is_multiple_of(256) {
                poll(c)?;
            }
            for ch in 0..4 {
                let value = try_read_sample_f64(input, SampleType::U16, ch).map_err(|_| RasterInputError::InvalidSample)? as f32;
                try_write_sample_f64(output, request.current_format.sample, ch, f64::from(value)).map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
    }
    poll(c)?;
    Ok(PreparedNativeSource { bytes, format: request.current_format, extent: request.source_extent, context: request.current_context })
}
/// Existing nearest transform actually consumes the prepared current-native words.
/// Combined preparation+sampler admission happens BEFORE preparation allocation.
/// Source qualifier/byte authority remains caller-owned; this is not import admission.
#[allow(clippy::too_many_arguments)]
pub fn prepare_nearest(request: Request<'_>, state: &TransformState, bounds: Rect, limits: Limits, c: &Interrupt<'_>) -> Result<Surface> {
    poll(c)?;
    validate_limits(limits)?;
    let prep = requirements(request)?;
    if state.pre_offset != [request.source_extent.x0, request.source_extent.y0] {
        return Err(unsupported());
    }
    crate::gimp_link_nearest::qualify_geometry(request.source_extent.width(), request.source_extent.height(), state, bounds)?;
    let transformed_bytes = checked_region_byte_len(bounds, request.current_format)?;
    let prepared_length = checked_region_byte_len(request.source_extent, request.current_format)?;
    if transformed_bytes > 64 << 20 || prepared_length > 64 << 20 {
        return Err(unsupported());
    }
    let first = TileCoord::containing(bounds.x0, bounds.y0);
    let last = TileCoord::containing(bounds.x1.checked_sub(1).ok_or_else(limit)?, bounds.y1.checked_sub(1).ok_or_else(limit)?);
    let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|n| n.checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok()?))
        .ok_or_else(limit)?;
    let tile_bytes =
        (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(request.current_format.bytes_per_pixel() as u64)).ok_or_else(limit)?;
    let native = tiles
        .checked_mul(tile_bytes.checked_add(1024).ok_or_else(limit)?)
        .and_then(|n| n.checked_add(tile_bytes.checked_mul(3)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(32)?))
        .and_then(|n| n.checked_add(8192))
        .ok_or_else(limit)?;
    let transform_work = bounds
        .size()
        .area()
        .checked_mul(128)
        .and_then(|n| n.checked_add(native.checked_mul(6)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(512)?))
        .and_then(|n| n.checked_add(prep.prepared_bytes))
        .ok_or_else(limit)?;
    if prep.peak_resident_bytes.checked_add(native).is_none_or(|n| n > limits.resident_bytes)
        || prep.work_units.checked_add(transform_work).is_none_or(|n| n > limits.work_units)
    {
        return Err(limit());
    }
    let source = prepare(request, limits, c)?;
    // The sampler charges its actual prepared slice length. Retain the original
    // borrowed resource and prospective Vec-capacity slack alongside that charge.
    let slack = (source.bytes.capacity() as u64).checked_sub(source.bytes.len() as u64).ok_or_else(limit)?;
    let held = prep.source_retained_bytes.checked_add(slack).and_then(|n| n.checked_add(4096)).ok_or_else(limit)?;
    let output = crate::gimp_link_nearest::transform_with_limits(
        &source.bytes,
        source.extent.width(),
        source.extent.height(),
        source.format,
        state,
        bounds,
        crate::gimp_link_nearest::Limits {
            resident_bytes: limits.resident_bytes.checked_sub(held).ok_or_else(limit)?,
            work_units: limits.work_units.checked_sub(prep.work_units).ok_or_else(limit)?,
        },
        c,
    )?;
    poll(c)?;
    Ok(output)
}
/// Allocation-free explicit final policy for a separately evaluated cubic pixel.
/// This is not a cubic evaluator or a claim about its internal arithmetic precision.
pub fn write_cubic_result(values: [f64; 4], format: PixelFormat, policy: FinalQuantization, output: &mut [u8]) -> Result<()> {
    let sample = match policy {
        FinalQuantization::Float32ThenU16Nearest => SampleType::U16,
        FinalQuantization::Float32 => SampleType::F32,
        FinalQuantization::Float32ThenPromoteF64 => SampleType::F64,
    };
    if format != PixelFormat::new(ColorMode::Rgb, sample, true) || output.len() != format.bytes_per_pixel() {
        return Err(unsupported());
    }
    let mut staged = [0u8; 32];
    let dst = staged.get_mut(..output.len()).ok_or_else(unsupported)?;
    for (ch, value) in values.into_iter().enumerate() {
        if !value.is_finite() || !(value as f32).is_finite() || ch == 3 && !(0.0..=1.0).contains(&value) {
            return Err(RasterInputError::InvalidSample.into());
        }
        let rounded = f64::from(value as f32);
        if sample == SampleType::U16 {
            let word = (rounded.clamp(0.0, 1.0) * 65535.0 + 0.5).floor() as u16;
            let at = ch.checked_mul(2).ok_or_else(limit)?;
            dst.get_mut(at..at + 2).ok_or_else(unsupported)?.copy_from_slice(&word.to_ne_bytes());
        } else {
            try_write_sample_f64(dst, sample, ch, rounded).map_err(|_| RasterInputError::InvalidSample)?;
        }
    }
    output.copy_from_slice(dst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::transform::Homography;
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] }
    }
    // Unrun: exact source ownership, lowbits, explicit promotion and real nearest consumer.
    #[test]
    fn preparation_is_explicit_and_nearest_preserves_prepared_native_words() {
        let words = [8193u16, 32769, 60001, 65535, 12003, 48007, 24011, 16385];
        let original = words.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
        let extent = Rect::new(-2, 1, 0, 2);
        let limits = Limits { resident_bytes: 64 << 20, work_units: 100_000_000 };
        for (policy, sample) in [
            (PreparationPolicy::PreserveU16Words, SampleType::U16),
            (PreparationPolicy::NormalizeU16ToF32, SampleType::F32),
            (PreparationPolicy::NormalizeU16ToF32ThenPromoteF64, SampleType::F64),
        ] {
            let request = Request {
                raw_source: &original,
                source_format: PixelFormat::new(ColorMode::Rgb, SampleType::U16, true),
                current_format: PixelFormat::new(ColorMode::Rgb, sample, true),
                source_extent: extent,
                source_context: context(),
                current_context: context(),
                policy,
            };
            let prepared = prepare(request, limits, &Interrupt::NONE).unwrap();
            let state = TransformState { matrix_f64_bits: Homography::IDENTITY.0.map(f64::to_bits), interpolation: 0, pre_offset: [-2, 1] };
            let transformed = prepare_nearest(request, &state, extent, limits, &Interrupt::NONE).unwrap();
            assert_eq!(transformed.try_read_interleaved(extent).unwrap(), prepared.bytes());
            for (pixel, source) in prepared.bytes().chunks_exact(request.current_format.bytes_per_pixel()).zip(words.chunks_exact(4)) {
                for (ch, word) in source.iter().enumerate() {
                    let expected = if sample == SampleType::U16 { f64::from(*word) / 65535. } else { f64::from((f64::from(*word) / 65535.) as f32) };
                    assert_eq!(try_read_sample_f64(pixel, sample, ch).unwrap(), expected);
                }
            }
        }
        assert_eq!(original, words.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>());
    }
    #[test]
    fn wrong_runtime_policy_context_and_cancel_do_not_publish() {
        let input = [0u8; 8];
        let extent = Rect::new(0, 0, 1, 1);
        let limits = Limits { resident_bytes: 64 << 20, work_units: 100_000_000 };
        let request = Request {
            raw_source: &input,
            source_format: PixelFormat::new(ColorMode::Rgb, SampleType::U16, true),
            current_format: PixelFormat::new(ColorMode::Rgb, SampleType::F64, true),
            source_extent: extent,
            source_context: context(),
            current_context: context(),
            policy: PreparationPolicy::NormalizeU16ToF32,
        };
        assert!(prepare(request, limits, &Interrupt::NONE).is_err());
        let mut changed = request;
        changed.policy = PreparationPolicy::NormalizeU16ToF32ThenPromoteF64;
        changed.current_context.profile_identity = [8; 32];
        assert!(prepare(changed, limits, &Interrupt::NONE).is_err());
        changed.current_context = context();
        assert!(prepare(changed, limits, &Interrupt::cancel_only(&|| true)).is_err());
        let mut output = [0x55u8; 32];
        let before = output;
        assert!(write_cubic_result([f64::NAN, 0., 0., 1.], changed.current_format, FinalQuantization::Float32ThenPromoteF64, &mut output).is_err());
        assert_eq!(output, before);
    }
}
