//! Explicit source->working CMS->configured candidate->inverse CMS native ROI.
//! Transforms are borrowed caller authority, not inferred profile/TRC defaults.
use super::{
    gimp_configured_candidate::SourceBinding,
    gimp_configured_render_candidate::{
        CoveragePolicy, OutputPolicy, RenderedCandidate, native_cells::NativeCells, render_source_candidate_with_sample_policy,
    },
};
use photocraft_algo::gimp::configured::{CurveSamplePolicy, Limits, WorkingContext};
use photocraft_cms::{Transform, curve::Curve, pipeline::Stage};
use photocraft_color::{
    ColorMode, PixelFormat, SampleType,
    checked_arithmetic::{self, Destination, Domain, Operand, Policy, Rounding, Storage, UnitOverflow},
    try_read_sample_f64, try_write_sample_f64,
};
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
pub struct TransformPair<'a> {
    pub source: WorkingContext,
    pub working: WorkingContext,
    pub forward: &'a Transform,
    pub inverse: &'a Transform,
    /// Compatibility entry points retain this caller-bound total. The owned
    /// factory derives it from the complete controlled Transform capacity census,
    /// including acceleration/Post storage beyond the visible pipeline.
    pub retained_bytes: u64,
    pub work_per_pixel: u64,
}
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn add(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_add(b).ok_or(RasterInputError::LimitExceeded)
}
fn mul(a: u64, b: u64) -> Result<u64, RasterInputError> {
    a.checked_mul(b).ok_or(RasterInputError::LimitExceeded)
}
fn transform_requirements(transform: &Transform, control: &Interrupt<'_>) -> Result<(u64, u64), RasterInputError> {
    let pipeline = transform.pipeline();
    if pipeline.stages.len() > 32 || !matches!(pipeline.inputs, 1 | 3) || !matches!(pipeline.outputs, 1 | 3) {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let mut retained = mul(pipeline.stages.capacity() as u64, std::mem::size_of::<Stage>() as u64)?;
    let mut channels = pipeline.inputs;
    for stage in &pipeline.stages {
        poll(control)?;
        match stage {
            Stage::Curves(curves) | Stage::InvCurves(curves) => {
                if curves.len() != channels {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                retained = add(retained, mul(curves.capacity() as u64, std::mem::size_of::<Curve>() as u64)?)?;
                for curve in curves {
                    poll(control)?;
                    match curve {
                        Curve::Identity => {}
                        Curve::Gamma(g) if g.is_finite() && *g > 0.0 => {}
                        Curve::Parametric { kind, p } if *kind <= 4 && p.iter().all(|v| v.is_finite()) => {}
                        Curve::Table(values) if values.len() >= 2 && values.len() <= 4096 => {
                            retained = add(retained, mul(values.capacity() as u64, 4)?)?;
                            for value in values {
                                poll(control)?;
                                if !value.is_finite() {
                                    return Err(RasterInputError::InvalidSample);
                                }
                            }
                        }
                        Curve::Table64(values) if values.len() >= 2 && values.len() <= 4096 => {
                            retained = add(retained, mul(values.capacity() as u64, 8)?)?;
                            for value in values {
                                poll(control)?;
                                if !value.is_finite() {
                                    return Err(RasterInputError::InvalidSample);
                                }
                            }
                        }
                        _ => return Err(RasterInputError::UnsupportedConversion),
                    }
                }
            }
            Stage::Matrix { rows, cols, m, offset } => {
                if !matches!(rows, 1 | 3)
                    || *cols != channels
                    || rows.checked_mul(*cols) != Some(m.len())
                    || offset.len() != *rows
                    || m.iter().chain(offset).any(|v| !v.is_finite())
                {
                    return Err(RasterInputError::UnsupportedConversion);
                }
                retained = add(retained, mul(add(m.capacity() as u64, offset.capacity() as u64)?, 8)?)?;
                channels = *rows;
            }
            Stage::Clut(_) => return Err(RasterInputError::UnsupportedConversion),
            Stage::Clamp01 => {}
            _ if channels == 3 => {}
            _ => return Err(RasterInputError::UnsupportedConversion),
        }
    }
    if channels != pipeline.outputs {
        return Err(RasterInputError::UnsupportedConversion);
    }
    // Bounded matrix/TRC/analytic call; no internal token is available in L0.
    // The conservative stage work bound includes table inverse/search schedules.
    Ok((retained, mul(add(pipeline.stages.len() as u64, 1)?, 65536)?))
}
struct Prepaid {
    bytes: u64,
    work: u64,
}
impl StagingBudget for Prepaid {
    type Lease = ();
    fn reserve(&mut self, bytes: u64) -> Result<(), RasterInputError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
    fn work(&mut self, units: u64) -> Result<(), RasterInputError> {
        self.work = self.work.checked_sub(units).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn count(roi: Rect) -> Result<u64, RasterInputError> {
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    mul(
        u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
        u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?,
    )
}
fn cells(roi: Rect, mut f: impl FnMut(Rect) -> Result<(), RasterInputError>) -> Result<(), RasterInputError> {
    for tile in roi.tiles() {
        let x0 = tile.tx.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        let y0 = tile.ty.checked_mul(TILE_SIZE).ok_or(RasterInputError::InvalidBounds)?;
        for cy in (0..TILE_SIZE).step_by(64) {
            for cx in (0..TILE_SIZE).step_by(64) {
                let x = x0.checked_add(cx).ok_or(RasterInputError::InvalidBounds)?;
                let y = y0.checked_add(cy).ok_or(RasterInputError::InvalidBounds)?;
                let part =
                    Rect::new(x, y, x.checked_add(64).ok_or(RasterInputError::InvalidBounds)?, y.checked_add(64).ok_or(RasterInputError::InvalidBounds)?)
                        .intersect(&roi);
                if !part.is_empty() {
                    f(part)?;
                }
            }
        }
    }
    Ok(())
}
fn scratch(len: usize, envelope: u64, control: &Interrupt<'_>) -> Result<Vec<u8>, RasterInputError> {
    poll(control)?;
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| RasterInputError::AllocationFailed)?;
    if v.capacity() as u64 > envelope {
        return Err(RasterInputError::LimitExceeded);
    }
    while v.len() < len {
        poll(control)?;
        let end = v.len().checked_add(4096).ok_or(RasterInputError::LimitExceeded)?.min(len);
        v.resize(end, 0);
    }
    Ok(v)
}
/// Conservative known live owners for explicit candidate orchestration.
/// Private Transform caches remain included by the caller's retained charge.
#[derive(Clone, Copy, Debug)]
pub struct ColorRequirements {
    pub working_format: PixelFormat,
    pub working_output_bytes: u64,
    pub native_output_bytes: u64,
    pub index_bytes: u64,
    pub peak_resident_bytes: u64,
    pub work_units: u64,
}
/// Allocation-free conservative work charge before detailed pipeline/table
/// and source-resident traversal. Existing admitted maxima are unchanged.
pub(crate) fn requirements_preparation_work(source: &Surface, pair: &TransformPair<'_>) -> Result<u64, RasterInputError> {
    let mut work = 256u64;
    for transform in [pair.forward, pair.inverse] {
        let pipeline = transform.pipeline();
        if pipeline.stages.len() > 32 || !matches!(pipeline.inputs, 1 | 3) || !matches!(pipeline.outputs, 1 | 3) {
            return Err(RasterInputError::UnsupportedConversion);
        }
        // At most three curves of <=4096 values per admitted stage, plus
        // bounded matrix/channel checks. No table values are read here.
        work = add(work, mul(pipeline.stages.len() as u64, 3 * 4096 + 256)?)?;
    }
    let tiles = u64::try_from(source.tile_count()).map_err(|_| RasterInputError::LimitExceeded)?;
    add(work, mul(tiles, 16)?)
}
pub fn requirements_color_candidate(
    source: &Surface,
    roi: Rect,
    pair: &TransformPair<'_>,
    control: &Interrupt<'_>,
) -> Result<ColorRequirements, RasterInputError> {
    poll(control)?;
    let source_format = source.format();
    if !matches!(source_format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || roi.is_empty()
        || pair.source.color_channels != source_format.mode.color_channels()
        || pair.source.alpha != source_format.alpha
        || pair.working.alpha != source_format.alpha
        || !matches!(pair.working.color_channels, 1 | 3)
        || pair.forward.inputs() != pair.source.color_channels
        || pair.forward.outputs() != pair.working.color_channels
        || pair.inverse.inputs() != pair.working.color_channels
        || pair.inverse.outputs() != pair.source.color_channels
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let (forward_bytes, forward_work) = transform_requirements(pair.forward, control)?;
    let (inverse_bytes, inverse_work) = transform_requirements(pair.inverse, control)?;
    if pair.retained_bytes < add(forward_bytes, inverse_bytes)? || pair.work_per_pixel < add(forward_work, inverse_work)? {
        return Err(RasterInputError::LimitExceeded);
    }
    let working_format =
        PixelFormat::new(if pair.working.color_channels == 3 { ColorMode::Rgb } else { ColorMode::Grayscale }, SampleType::F64, source_format.alpha);
    let tiles = count(roi)?;
    let work_tile = mul(65536, working_format.bytes_per_pixel() as u64)?;
    let native_tile = mul(65536, source_format.bytes_per_pixel() as u64)?;
    let source_lower = mul(source.tile_count() as u64, add(native_tile, 256)?)?;
    if source_lower > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    let work_charge = add(mul(tiles, add(work_tile, 1024)?)?, working_format.bytes_per_pixel() as u64)?;
    let native_charge = add(mul(tiles, add(native_tile, 1024)?)?, source_format.bytes_per_pixel() as u64)?;
    let pixels = mul(u64::from(roi.width()), u64::from(roi.height()))?;
    let scratch_bytes = add(mul(4096, add(working_format.bytes_per_pixel() as u64, source_format.bytes_per_pixel() as u64)?)?, 65536)?;
    let peak = add(pair.retained_bytes, add(source.resident_accounted_bytes()?, add(mul(work_charge, 2)?, add(native_charge, scratch_bytes)?)?)?)?;
    let work = add(
        mul(pixels, add(pair.work_per_pixel, 16384)?)?,
        add(mul(add(mul(work_charge, 2)?, native_charge)?, 32)?, add(32768, mul(source.tile_count() as u64, 16)?)?)?,
    )?;
    if peak > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    poll(control)?;
    Ok(ColorRequirements {
        working_format,
        working_output_bytes: work_charge,
        native_output_bytes: native_charge,
        index_bytes: mul(tiles, 1024)?,
        peak_resident_bytes: peak,
        work_units: work,
    })
}
/// All intermediate working samples use F64 storage solely as lossless binary64
/// staging. Original native precision is quantized once, after inverse CMS.
/// ROI-only output; no ICC/profile-cache/default or source qualification claim.
pub fn render_color_candidate(
    node: &FilterNode,
    binding: SourceBinding<'_>,
    sampling: CurveSamplePolicy,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual_source: WorkingContext,
    pair: &TransformPair<'_>,
    working_default: &[u8],
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<RenderedCandidate, RasterInputError> {
    render_color_candidate_with_requirements(node, binding, sampling, source, extent, roi, actual_source, pair, working_default, policy, limits, control, None)
}

pub(crate) fn render_color_candidate_with_requirements(
    node: &FilterNode,
    binding: SourceBinding<'_>,
    sampling: CurveSamplePolicy,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual_source: WorkingContext,
    pair: &TransformPair<'_>,
    working_default: &[u8],
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
    requirements: Option<ColorRequirements>,
) -> Result<RenderedCandidate, RasterInputError> {
    render_color_execution(
        source,
        extent,
        roi,
        actual_source,
        binding.working,
        pair,
        working_default,
        policy,
        limits,
        control,
        requirements,
        || {
            let configured = super::gimp_configured_candidate::bind_source_node_with_sample_policy(node, binding, sampling, control)?;
            configured.with_prepared(pair.working, Limits { resident_bytes: 65536, work_units: 16384 }, control, |_| Ok(()))
        },
        |working, output_policy, middle_limits, _charge| {
            render_source_candidate_with_sample_policy(node, binding, sampling, working, extent, roi, pair.working, output_policy, middle_limits, control)
        },
    )
}

/// Reuse one caller-prepared exact typed configuration across CMS graph cells.
/// Preparation authority and original configuration remain borrowed; no source
/// identity is inferred and the same paired transform/default policies apply.
pub fn render_prepared_color_candidate(
    prepared: &photocraft_algo::gimp::configured::PreparedConfiguration<'_>,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual_source: WorkingContext,
    pair: &TransformPair<'_>,
    working_default: &[u8],
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<RenderedCandidate, RasterInputError> {
    render_prepared_color_candidate_with_requirements(prepared, source, extent, roi, actual_source, pair, working_default, policy, limits, control, None)
}

pub(crate) fn render_prepared_color_candidate_with_requirements(
    prepared: &photocraft_algo::gimp::configured::PreparedConfiguration<'_>,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual_source: WorkingContext,
    pair: &TransformPair<'_>,
    working_default: &[u8],
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
    requirements: Option<ColorRequirements>,
) -> Result<RenderedCandidate, RasterInputError> {
    poll(control)?;
    if limits.resident_bytes == 0 || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES {
        return Err(RasterInputError::LimitExceeded);
    }
    let remaining = limits.resident_bytes.checked_sub(prepared.retained_bytes()).ok_or(RasterInputError::LimitExceeded)?;
    render_color_execution(
        source,
        extent,
        roi,
        actual_source,
        prepared.working_context(),
        pair,
        working_default,
        policy,
        Limits { resident_bytes: remaining, work_units: limits.work_units },
        control,
        requirements,
        || poll(control),
        |working, output_policy, middle_limits, charge| {
            super::gimp_configured_render_candidate::render_prepared_window(
                prepared,
                working,
                roi,
                pair.working,
                output_policy,
                charge,
                mul(4096, working.format().bytes_per_pixel() as u64)?,
                middle_limits.work_units,
                control,
            )
        },
    )
}

// The retained-source and prepared entry points share all CMS and final-native
// numerical/staging code; only explicit configuration preparation differs.
fn render_color_execution(
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual_source: WorkingContext,
    configured_working: WorkingContext,
    pair: &TransformPair<'_>,
    working_default: &[u8],
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
    requirements: Option<ColorRequirements>,
    validate_configuration: impl FnOnce() -> Result<(), RasterInputError>,
    evaluate_working: impl FnOnce(&Surface, OutputPolicy, Limits, u64) -> Result<RenderedCandidate, RasterInputError>,
) -> Result<RenderedCandidate, RasterInputError> {
    poll(control)?;
    let source_format = source.format();
    if !matches!(source_format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || actual_source != pair.source
        || pair.working != configured_working
        || pair.source.color_channels != source_format.mode.color_channels()
        || pair.source.alpha != source_format.alpha
        || pair.working.alpha != source_format.alpha
        || !matches!(pair.working.color_channels, 1 | 3)
        || extent.is_empty()
        || roi.is_empty()
        || roi.intersect(&extent) != roi
        || limits.resident_bytes == 0
        || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let required = match requirements {
        Some(required) => required,
        None => requirements_color_candidate(source, roi, pair, control)?,
    };
    let working_format = required.working_format;
    if working_default.len() != working_format.bytes_per_pixel() {
        return Err(RasterInputError::LengthMismatch);
    }
    for i in 0..working_format.channels() {
        let sample = try_read_sample_f64(working_default, SampleType::F64, i).map_err(|_| RasterInputError::InvalidSample)?;
        if !sample.is_finite() || (working_format.alpha && i == pair.working.color_channels && !(0.0..=1.0).contains(&sample)) {
            return Err(RasterInputError::InvalidSample);
        }
    }
    let work_charge = required.working_output_bytes;
    let native_charge = required.native_output_bytes;
    let peak = required.peak_resident_bytes;
    let work = required.work_units;
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    source.try_validate_tile_origins_with(control)?;
    // Refuse malformed/unsupported retained config before any raster allocation.
    validate_configuration()?;
    let mut prepaid = Prepaid { bytes: work_charge, work };
    let mut forward = StagedSurfaceBuilder::new(working_format, working_default, roi, &mut prepaid, control)?;
    let mut source_cells = NativeCells::new(mul(4096, source_format.bytes_per_pixel() as u64)?, control)?;
    cells(roi, |part| {
        poll(control)?;
        let native = source_cells.read(source, part, control)?;
        let len = (native.len() / source_format.bytes_per_pixel()).checked_mul(working_format.bytes_per_pixel()).ok_or(RasterInputError::LimitExceeded)?;
        let mut encoded = scratch(len, mul(4096, working_format.bytes_per_pixel() as u64)?, control)?;
        for (src, dst) in native.chunks_exact(source_format.bytes_per_pixel()).zip(encoded.chunks_exact_mut(working_format.bytes_per_pixel())) {
            poll(control)?;
            let mut input = [0.0; 3];
            let mut output = [0.0; 3];
            for (i, v) in input.iter_mut().take(pair.source.color_channels).enumerate() {
                *v = checked_arithmetic::decode_finite(Operand { bytes: src, storage: Storage::Native(source_format.sample), index: i }, Domain::FloatHdr)
                    .map_err(|_| RasterInputError::InvalidSample)?;
            }
            pair.forward
                .try_eval_f64(
                    input.get(..pair.source.color_channels).ok_or(RasterInputError::LengthMismatch)?,
                    output.get_mut(..pair.working.color_channels).ok_or(RasterInputError::LengthMismatch)?,
                )
                .map_err(|_| RasterInputError::UnsupportedConversion)?;
            poll(control)?;
            for (i, v) in output.iter().take(pair.working.color_channels).enumerate() {
                try_write_sample_f64(dst, SampleType::F64, i, *v).map_err(|_| RasterInputError::InvalidSample)?;
            }
            if source_format.alpha {
                let alpha = checked_arithmetic::decode_finite(
                    Operand { bytes: src, storage: Storage::Native(source_format.sample), index: pair.source.color_channels },
                    Domain::Unit,
                )
                .map_err(|_| RasterInputError::InvalidSample)?;
                try_write_sample_f64(dst, SampleType::F64, pair.working.color_channels, alpha).map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
        forward.push_region(part, &encoded, &mut prepaid, control)
    })?;
    let (working, ()) = forward.finalize(&mut prepaid, control)?.into_parts();
    drop(source_cells);
    let lossless = Policy { domain: Domain::FloatHdr, unit_overflow: UnitOverflow::Reject, rounding: Rounding::Exact };
    let evaluated = evaluate_working(
        &working,
        OutputPolicy { colors: lossless, alpha: lossless, coverage: policy.coverage },
        Limits { resident_bytes: peak, work_units: work },
        work_charge,
    )?;
    drop(working);
    let mut prepaid = Prepaid { bytes: native_charge, work };
    let mut result = StagedSurfaceBuilder::new(source_format, source.default_bytes(), roi, &mut prepaid, control)?;
    let mut filtered_cells = NativeCells::new(mul(4096, working_format.bytes_per_pixel() as u64)?, control)?;
    let mut source_cells = NativeCells::new(mul(4096, source_format.bytes_per_pixel() as u64)?, control)?;
    cells(roi, |part| {
        poll(control)?;
        let filtered = filtered_cells.read(&evaluated.surface, part, control)?;
        let native = source_cells.read(source, part, control)?;
        for (src, dst) in filtered.chunks_exact(working_format.bytes_per_pixel()).zip(native.chunks_exact_mut(source_format.bytes_per_pixel())) {
            poll(control)?;
            let mut input = [0.0; 3];
            let mut output = [0.0; 3];
            for (i, v) in input.iter_mut().take(pair.working.color_channels).enumerate() {
                *v = try_read_sample_f64(src, SampleType::F64, i).map_err(|_| RasterInputError::InvalidSample)?;
            }
            pair.inverse
                .try_eval_f64(
                    input.get(..pair.working.color_channels).ok_or(RasterInputError::LengthMismatch)?,
                    output.get_mut(..pair.source.color_channels).ok_or(RasterInputError::LengthMismatch)?,
                )
                .map_err(|_| RasterInputError::UnsupportedConversion)?;
            poll(control)?;
            for (i, v) in output.iter().take(pair.source.color_channels).enumerate() {
                checked_arithmetic::encode_finite(*v, Destination { bytes: dst, storage: Storage::Native(source_format.sample), index: i }, policy.colors)
                    .map_err(|_| RasterInputError::InvalidSample)?;
            }
            if source_format.alpha && !matches!(policy.coverage, CoveragePolicy::PreserveSource) {
                let alpha = try_read_sample_f64(src, SampleType::F64, pair.working.color_channels).map_err(|_| RasterInputError::InvalidSample)?;
                checked_arithmetic::encode_finite(
                    alpha,
                    Destination { bytes: dst, storage: Storage::Native(source_format.sample), index: pair.source.color_channels },
                    policy.alpha,
                )
                .map_err(|_| RasterInputError::InvalidSample)?;
            }
        }
        result.push_region(part, native, &mut prepaid, control)
    })?;
    let (surface, ()) = result.finalize(&mut prepaid, control)?.into_parts();
    poll(control)?;
    Ok(RenderedCandidate { surface, roi })
}

#[path = "gimp_configured_color_candidate/owned.rs"]
mod owned;
pub use owned::{OwnedConfiguredColorCandidate, ProfileOwnedColorRequirements, profile_owned_color_requirements, prepare_owned_color_candidate, prepare_retained_color_candidate, prepare_profile_owned_color_candidate};
