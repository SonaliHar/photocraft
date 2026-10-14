//! Explicit caller-bound wider cubic candidate. Never imported-link dispatch.
//! Existing mapping/BC taps/association are reused. Intermediate and final
//! precision are declared policies, not inferred from requested projection formats.
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_link_native_precision_candidate::{self as precision, FinalQuantization, PreparedNativeSource, WorkingContext};
use photocraft_cms::{ColorSpace, Curve, Profile, control as cms};
use photocraft_color::{ColorMode, PixelFormat, SampleType, try_read_sample_f64};
use photocraft_doc::{Rect, xcf_items::TransformState};
use photocraft_geom::{TILE_SIZE, TileCoord};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len};
#[derive(Clone, Copy, Debug)]
pub enum InputUnitPrecision {
    NativeNormalizedF64,
    RoundUnitToF32BeforeTransfer,
}
#[derive(Clone, Copy, Debug)]
pub enum IntermediatePrecision {
    F64UnfusedExistingTapOrder,
}
/// Caller binds a resolved current definition to its already-established context.
/// This does not open resources, parse profiles, or confer source/edit authority.
#[derive(Clone, Copy)]
pub struct BoundProfile<'a> {
    pub definition: &'a Profile,
    pub context: WorkingContext,
}
#[derive(Clone, Copy)]
pub struct Request<'a> {
    pub preparation: precision::Request<'a>,
    pub profile: BoundProfile<'a>,
    pub transform: &'a TransformState,
    pub bounds: Rect,
    pub input_precision: InputUnitPrecision,
    pub intermediate: IntermediatePrecision,
    pub final_quantization: FinalQuantization,
}
pub use precision::Limits;
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub source_preparation_bytes: u64,
    pub profile_retained_bytes: u64,
    pub output_retained_bytes: u64,
    pub scratch_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    /// Separate prospective CMS validation and direct-curve scan quota.
    pub validation_work_units: u64,
}
fn unsupported() -> CompositionError {
    CompositionError::Unsupported("unqualified wider linked cubic candidate domain")
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn cms_error(e: cms::Error) -> CompositionError {
    match e {
        cms::Error::Cancelled => RasterInputError::Cancelled.into(),
        cms::Error::Limit(_) => limit(),
        cms::Error::Allocation => RasterInputError::AllocationFailed.into(),
        cms::Error::Cms(e) => e.into(),
    }
}
fn cms_limits(limits: Limits) -> Result<cms::Limits> {
    let max = cms::Limits::default();
    Ok(cms::Limits {
        resident_bytes: usize::try_from(limits.resident_bytes.min(max.resident_bytes as u64)).map_err(|_| limit())?,
        work_units: limits.work_units.min(max.work_units),
    })
}
fn final_format(policy: FinalQuantization) -> PixelFormat {
    PixelFormat::new(
        ColorMode::Rgb,
        match policy {
            FinalQuantization::Float32ThenU16Nearest => SampleType::U16,
            FinalQuantization::Float32 => SampleType::F32,
            FinalQuantization::Float32ThenPromoteF64 => SampleType::F64,
        },
        true,
    )
}
fn metadata(request: Request<'_>) -> Result<precision::Requirements> {
    match request.intermediate {
        IntermediatePrecision::F64UnfusedExistingTapOrder => (),
    }
    let prep = precision::requirements(request.preparation)?;
    if request.profile.context != request.preparation.current_context
        || request.preparation.current_format != final_format(request.final_quantization)
        || request.transform.pre_offset != [request.preparation.source_extent.x0, request.preparation.source_extent.y0]
        || request.profile.definition.color_space != ColorSpace::Rgb
        || request.profile.definition.a2b.iter().chain(&request.profile.definition.b2a).any(Option::is_some)
    {
        return Err(unsupported());
    }
    let curves = request.profile.definition.trc.as_ref().ok_or_else(unsupported)?;
    // First bounded candidate is analytical matrix/shaper only. Sampled LUT/TRC
    // inverse work/policy requires a separately declared and accounted extension.
    if curves.iter().any(|c| !matches!(c, Curve::Identity | Curve::Gamma(_) | Curve::Parametric { kind: 0..=4, .. })) {
        return Err(unsupported());
    }
    crate::gimp_link_cubic::qualify_geometry(
        request.preparation.source_extent.width(),
        request.preparation.source_extent.height(),
        request.transform,
        request.bounds,
    )?;
    checked_region_byte_len(request.bounds, request.preparation.current_format)?;
    Ok(prep)
}
/// Borrowed complete invocation admission before preparation/output allocation.
pub fn requirements(request: Request<'_>, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    if limits.resident_bytes > MAX_CHECKED_RASTER_BYTES || limits.work_units > 8_000_000_000 {
        return Err(limit());
    }
    let prep = metadata(request)?;
    let b = request.bounds;
    let first = TileCoord::containing(b.x0, b.y0);
    let last = TileCoord::containing(b.x1.checked_sub(1).ok_or_else(limit)?, b.y1.checked_sub(1).ok_or_else(limit)?);
    let tiles = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1)
        .ok()
        .and_then(|n| n.checked_mul(u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).ok()?))
        .ok_or_else(limit)?;
    let tile_bytes = (TILE_SIZE as u64)
        .checked_mul(TILE_SIZE as u64)
        .and_then(|n| n.checked_mul(request.preparation.current_format.bytes_per_pixel() as u64))
        .ok_or_else(limit)?;
    let output_retained_bytes = tiles.checked_mul(tile_bytes.checked_add(1024).ok_or_else(limit)?).and_then(|n| n.checked_add(4096)).ok_or_else(limit)?;
    let scratch_bytes = tile_bytes.checked_mul(3).and_then(|n| n.checked_add(tiles.checked_mul(32)?)).and_then(|n| n.checked_add(4096)).ok_or_else(limit)?;
    let base = prep.peak_resident_bytes.checked_add(output_retained_bytes).and_then(|n| n.checked_add(scratch_bytes)).ok_or_else(limit)?;
    // Fixed16 taps, direct analytical TRCs/inverses (including bounded inverse
    // fallback), native copies, map visits and pruning. No upstream budget refund.
    let mut work = prep
        .work_units
        .checked_add(b.size().area().checked_mul(16384).ok_or_else(limit)?)
        .and_then(|n| n.checked_add(output_retained_bytes.checked_mul(6)?))
        .and_then(|n| n.checked_add(tiles.checked_mul(tiles)?.checked_mul(512)?))
        .ok_or_else(limit)?;
    if base > limits.resident_bytes || work > limits.work_units {
        return Err(limit());
    }
    let left = Limits {
        resident_bytes: limits.resident_bytes.checked_sub(base).ok_or_else(limit)?,
        work_units: limits.work_units.checked_sub(work).ok_or_else(limit)?,
    };
    let cancelled = || c.cancelled();
    let pr = request.profile.definition.clone_requirements_with_control(cms_limits(left)?, &cancelled).map_err(cms_error)?;
    let profile_retained_bytes = pr.retained_bytes as u64;
    // Complete closed analytical profile validation includes retained-model
    // census, fixed matrix/point inspection, CMS257 samples/curve, and our511
    // direct strict samples/curve. No LUT/table validator is admitted here.
    let validation_work_units = (pr.retained_bytes as u64)
        .checked_mul(4)
        .and_then(|n| n.checked_add(pr.work_units.checked_mul(4)?))
        .and_then(|n| n.checked_add(3 * (257 + 511) * 128 + 4096))
        .ok_or_else(limit)?;
    work = work.checked_add(pr.work_units).and_then(|n| n.checked_add(validation_work_units)).ok_or_else(limit)?;
    let peak_resident_bytes = base.checked_add(profile_retained_bytes).ok_or_else(limit)?;
    if peak_resident_bytes > limits.resident_bytes || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(Requirements {
        source_preparation_bytes: prep.peak_resident_bytes,
        profile_retained_bytes,
        output_retained_bytes,
        scratch_bytes,
        peak_resident_bytes,
        work_units: work,
        validation_work_units,
    })
}
fn validate_curves(profile: &Profile, limits: Limits, c: &Interrupt<'_>) -> Result<()> {
    let cancelled = || c.cancelled();
    let direct_work = 3 * 511 * 128;
    let cms_quota = Limits { resident_bytes: limits.resident_bytes, work_units: limits.work_units.checked_sub(direct_work).ok_or_else(limit)? };
    photocraft_cms::xcf_color::validate_profile_with_control(profile, cms_limits(cms_quota)?, &cancelled).map_err(cms_error)?;
    let mut remaining_direct = direct_work;
    let curves = profile.trc.as_ref().ok_or_else(unsupported)?;
    // Same strict decoded and midpoint monotonicity conditions as the existing
    // TransferTables, without repeating whole-model validation511 times.
    for curve in curves {
        let (mut previous, mut midpoint) = (None, None);
        for i in 0..256 {
            poll(c)?;
            remaining_direct = remaining_direct.checked_sub(128).ok_or_else(limit)?;
            let v = curve.eval64(i as f64 / 255.);
            if !v.is_finite() || previous.is_some_and(|p| v <= p) {
                return Err(unsupported());
            }
            previous = Some(v);
            if i < 255 {
                remaining_direct = remaining_direct.checked_sub(128).ok_or_else(limit)?;
                let v = curve.eval64((i as f64 + 0.5) / 255.);
                if !v.is_finite() || midpoint.is_some_and(|p| v <= p) {
                    return Err(unsupported());
                }
                midpoint = Some(v);
            }
        }
    }
    poll(c)
}
fn input_pixel(source: &PreparedNativeSource, x: i64, y: i64, policy: InputUnitPrecision) -> Result<[f64; 4]> {
    let extent = source.extent();
    if x < 0 || y < 0 || x >= i64::from(extent.width()) || y >= i64::from(extent.height()) {
        return Ok([0.; 4]);
    }
    let bpp = source.format().bytes_per_pixel();
    let at = usize::try_from(y)
        .ok()
        .and_then(|n| n.checked_mul(extent.width() as usize))
        .and_then(|n| n.checked_add(usize::try_from(x).ok()?))
        .and_then(|n| n.checked_mul(bpp))
        .ok_or_else(limit)?;
    let end = at.checked_add(bpp).ok_or_else(limit)?;
    let pixel = source.bytes().get(at..end).ok_or_else(unsupported)?;
    let mut values = [0.; 4];
    for (ch, v) in values.iter_mut().enumerate() {
        *v = try_read_sample_f64(pixel, source.format().sample, ch).map_err(|_| RasterInputError::InvalidSample)?;
        if matches!(policy, InputUnitPrecision::RoundUnitToF32BeforeTransfer) {
            *v = f64::from(*v as f32);
        }
        if !v.is_finite() || !(0.0..=1.0).contains(v) {
            return Err(RasterInputError::InvalidSample.into());
        }
    }
    Ok(values)
}
/// Private full transform; originals/resources/cache metadata remain untouched.
pub fn prepare_transform(request: Request<'_>, limits: Limits, c: &Interrupt<'_>) -> Result<Surface> {
    let admitted = requirements(request, limits, c)?;
    validate_curves(request.profile.definition, Limits { resident_bytes: admitted.profile_retained_bytes, work_units: admitted.validation_work_units }, c)?;
    poll(c)?;
    let source = precision::prepare(request.preparation, limits, c)?;
    let curves = request.profile.definition.trc.as_ref().ok_or_else(unsupported)?;
    let (inverse, ox, oy) =
        crate::gimp_link_nearest::mapping_with_interpolation(source.extent().width(), source.extent().height(), request.transform, request.bounds, 2)?;
    let format = request.preparation.current_format;
    let bpp = format.bytes_per_pixel();
    let mut output = Surface::try_with_default_bytes(format, [0u8; 32].get(..bpp).ok_or_else(unsupported)?)?;
    for tile in request.bounds.tiles() {
        poll(c)?;
        let patch = tile.rect().intersect(&request.bounds);
        let count = checked_region_byte_len(patch, format)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(count).map_err(|_| RasterInputError::AllocationFailed)?;
        if bytes.capacity() as u64 > (count as u64).checked_mul(2).and_then(|n| n.checked_add(64)).ok_or_else(limit)?
            || bytes.capacity() as u64 > admitted.scratch_bytes
        {
            return Err(limit());
        }
        while bytes.len() < count {
            poll(c)?;
            let end = bytes.len().checked_add((count - bytes.len()).min(64 << 10)).ok_or_else(limit)?;
            bytes.resize(end, 0);
        }
        let mut cursor = 0usize;
        for y in patch.y0..patch.y1 {
            poll(c)?;
            for x in patch.x0..patch.x1 {
                poll(c)?;
                let end = cursor.checked_add(bpp).ok_or_else(limit)?;
                let (sx, sy) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (sx, sy) = (sx - ox - 0.5, sy - oy - 0.5);
                if !sx.is_finite() || !sy.is_finite() {
                    return Err(unsupported());
                }
                if sx >= -2. && sy >= -2. && sx < f64::from(source.extent().width()) + 1. && sy < f64::from(source.extent().height()) + 1. {
                    let (ix, iy) = (sx.floor() as i64, sy.floor() as i64);
                    let mut acc = [0.0f64; 4];
                    for dy in -1..=2 {
                        for dx in -1..=2 {
                            poll(c)?;
                            let (tx, ty) = (ix + dx, iy + dy);
                            if tx < 0 || ty < 0 || tx >= i64::from(source.extent().width()) || ty >= i64::from(source.extent().height()) {
                                continue;
                            }
                            let weight = crate::gimp_link_cubic::cubic_weight(sx - tx as f64) * crate::gimp_link_cubic::cubic_weight(sy - ty as f64);
                            let values = input_pixel(&source, tx, ty, request.input_precision)?;
                            let alpha = *values.get(3).ok_or_else(unsupported)?;
                            for ch in 0..3 {
                                let linear = curves.get(ch).ok_or_else(unsupported)?.eval64(*values.get(ch).ok_or_else(unsupported)?);
                                if !linear.is_finite() {
                                    return Err(RasterInputError::InvalidSample.into());
                                }
                                *acc.get_mut(ch).ok_or_else(unsupported)? += linear * alpha.max(1. / 65536.) * weight;
                            }
                            *acc.get_mut(3).ok_or_else(unsupported)? += alpha * weight;
                        }
                    }
                    let alpha = *acc.get(3).ok_or_else(unsupported)?;
                    let divisor = if alpha.abs() < 1. / 65536. { 1. / 65536. } else { alpha };
                    let mut values = [0.0f64; 4];
                    for ch in 0..3 {
                        let linear = acc.get(ch).ok_or_else(unsupported)? / divisor;
                        *values.get_mut(ch).ok_or_else(unsupported)? = curves.get(ch).ok_or_else(unsupported)?.eval_inverse64(linear);
                    }
                    if !alpha.is_finite() {
                        return Err(RasterInputError::InvalidSample.into());
                    }
                    *values.get_mut(3).ok_or_else(unsupported)? = alpha.clamp(0., 1.);
                    precision::write_cubic_result(values, format, request.final_quantization, bytes.get_mut(cursor..end).ok_or_else(unsupported)?)?;
                }
                cursor = end;
            }
        }
        poll(c)?;
        output.try_write_interleaved_with(patch, &bytes, c)?;
    }
    output.try_prune_with(c)?;
    poll(c)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use precision::PreparationPolicy;
    fn context() -> WorkingContext {
        WorkingContext { encoding: photocraft_doc::SampleEncoding::NonlinearProfile, profile_identity: [9; 32] }
    }
    fn source<'a>(bytes: &'a [u8], sample: SampleType) -> precision::Request<'a> {
        precision::Request {
            raw_source: bytes,
            source_format: PixelFormat::new(ColorMode::Rgb, SampleType::U16, true),
            current_format: PixelFormat::new(ColorMode::Rgb, sample, true),
            source_extent: Rect::new(254, -1, 256, 1),
            source_context: context(),
            current_context: context(),
            policy: if sample == SampleType::F32 { PreparationPolicy::NormalizeU16ToF32 } else { PreparationPolicy::NormalizeU16ToF32ThenPromoteF64 },
        }
    }
    // Unrun: real cubic/private publication, exact declared storage promotion,
    // tile seam/partial/hidden alpha and immutable source words.
    #[test]
    fn explicit_f64_storage_promotes_same_f32_candidate_result() {
        let words = [8193u16, 32769, 60001, 0, 12003, 48007, 24011, 16385, 60013, 12017, 36019, 49153, 44021, 32023, 24029, 65535];
        let bytes = words.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>();
        let state =
            TransformState { matrix_f64_bits: [1.0f64, 0., 0.25, 0., 1., 0.375, 0., 0., 1.].map(f64::to_bits), interpolation: 2, pre_offset: [254, -1] };
        let profile = photocraft_cms::builtin::srgb();
        let bounds = Rect::new(254, -1, 257, 2);
        let limits = Limits { resident_bytes: 64 << 20, work_units: 100_000_000 };
        let a = Request {
            preparation: source(&bytes, SampleType::F32),
            profile: BoundProfile { definition: &profile, context: context() },
            transform: &state,
            bounds,
            input_precision: InputUnitPrecision::RoundUnitToF32BeforeTransfer,
            intermediate: IntermediatePrecision::F64UnfusedExistingTapOrder,
            final_quantization: FinalQuantization::Float32,
        };
        let b = Request { preparation: source(&bytes, SampleType::F64), final_quantization: FinalQuantization::Float32ThenPromoteF64, ..a };
        let actual32 = prepare_transform(a, limits, &Interrupt::NONE).unwrap().try_read_interleaved(bounds).unwrap();
        let actual64 = prepare_transform(b, limits, &Interrupt::NONE).unwrap().try_read_interleaved(bounds).unwrap();
        for (p32, p64) in actual32.chunks_exact(16).zip(actual64.chunks_exact(32)) {
            for ch in 0..4 {
                assert_eq!(try_read_sample_f64(p32, SampleType::F32, ch).unwrap(), try_read_sample_f64(p64, SampleType::F64, ch).unwrap());
            }
        }
        assert_eq!(bytes, words.into_iter().flat_map(u16::to_ne_bytes).collect::<Vec<_>>());
        let req = requirements(a, limits, &Interrupt::NONE).unwrap();
        assert!(prepare_transform(a, Limits { resident_bytes: req.peak_resident_bytes - 1, work_units: req.work_units }, &Interrupt::NONE).is_err());
        assert!(prepare_transform(a, Limits { resident_bytes: req.peak_resident_bytes, work_units: req.work_units - 1 }, &Interrupt::NONE).is_err());
        assert!(prepare_transform(a, limits, &Interrupt::cancel_only(&|| true)).is_err());
        let wrong =
            Request { preparation: precision::Request { current_context: WorkingContext { profile_identity: [10; 32], ..context() }, ..a.preparation }, ..a };
        assert!(prepare_transform(wrong, limits, &Interrupt::NONE).is_err());
    }
}
