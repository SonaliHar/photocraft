//! Explicit native ROI candidate. Not wired into qualified/default rendering.
//! Source pixels must already be in the caller's exact working profile/transfer.
use super::gimp_configured_candidate::{SourceBinding, bind_source_node_with_sample_policy};

#[path = "gimp_configured_native_cells.rs"]
pub(crate) mod native_cells;
use photocraft_algo::gimp::configured::{CurveSamplePolicy, Limits, WorkingContext};
use photocraft_color::{
    ColorMode,
    checked_arithmetic::{self, Destination, Domain, Operand, Policy, Storage},
};
use photocraft_doc::xcf_filters::FilterNode;
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{
    Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError, Surface, checked_region_byte_len,
    staged::{StagedSurfaceBuilder, StagingBudget},
};
#[derive(Clone, Copy, Debug)]
pub enum CoveragePolicy {
    PreserveSource,
    EvaluatedRejectUnit,
    EvaluatedClampUnit,
    EvaluatedFinite,
}
#[derive(Clone, Copy, Debug)]
pub struct OutputPolicy {
    pub colors: Policy,
    pub alpha: Policy,
    pub coverage: CoveragePolicy,
}
/// Only `roi` pixels are defined by this result. It is a filter-render window,
/// not a replacement full source; callers must not read its padding as input.
pub struct RenderedCandidate {
    pub surface: Surface,
    pub roi: Rect,
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
fn poll(control: &Interrupt<'_>) -> Result<(), RasterInputError> {
    control.check().map_err(|_| RasterInputError::Cancelled)
}
fn tile_count(roi: Rect) -> Result<u64, RasterInputError> {
    if roi.is_empty() {
        return Ok(0);
    }
    let first = TileCoord::containing(roi.x0, roi.y0);
    let last =
        TileCoord::containing(roi.x1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?, roi.y1.checked_sub(1).ok_or(RasterInputError::InvalidBounds)?);
    let x = u64::try_from(i64::from(last.tx) - i64::from(first.tx) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    let y = u64::try_from(i64::from(last.ty) - i64::from(first.ty) + 1).map_err(|_| RasterInputError::InvalidBounds)?;
    x.checked_mul(y).ok_or(RasterInputError::LimitExceeded)
}
/// Actual typed source -> prepared evaluator -> exact native final quantization.
/// No stored locator access, implicit CMS, compositing envelope or admission.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn render_source_candidate(
    node: &FilterNode,
    binding: SourceBinding<'_>,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual: WorkingContext,
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<RenderedCandidate, RasterInputError> {
    render_source_candidate_with_sample_policy(node, binding, CurveSamplePolicy::RequireExact256, source, extent, roi, actual, policy, limits, control)
}
/// Explicit retained-table opt-in; requested metadata remains caller-bound.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn render_source_candidate_with_sample_policy(
    node: &FilterNode,
    binding: SourceBinding<'_>,
    sampling: CurveSamplePolicy,
    source: &Surface,
    extent: Rect,
    roi: Rect,
    actual: WorkingContext,
    policy: OutputPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<RenderedCandidate, RasterInputError> {
    poll(control)?;
    let format = source.format();
    let colors = match format.mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        _ => return Err(RasterInputError::UnsupportedConversion),
    };
    if actual != binding.working
        || actual.color_channels != colors
        || actual.alpha != format.alpha
        || !matches!(format.channels(), 1..=4)
        || extent.is_empty()
        || roi.is_empty()
        || roi.intersect(&extent) != roi
        || limits.resident_bytes == 0
        || limits.resident_bytes > MAX_CHECKED_RASTER_BYTES
    {
        return Err(RasterInputError::UnsupportedConversion);
    }
    let bpp = format.bytes_per_pixel();
    let count = tile_count(roi)?;
    let tile_bytes = (TILE_SIZE as u64).checked_mul(TILE_SIZE as u64).and_then(|n| n.checked_mul(bpp as u64)).ok_or(RasterInputError::LimitExceeded)?;
    let output = count.checked_mul(tile_bytes.checked_add(1024).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    let pixels = u64::try_from(checked_region_byte_len(roi, format)? / bpp).map_err(|_| RasterInputError::LimitExceeded)?;
    let scratch = 4096u64.checked_mul(bpp as u64).ok_or(RasterInputError::LimitExceeded)?;
    let source_lower =
        (source.tile_count() as u64).checked_mul(tile_bytes.checked_add(256).ok_or(RasterInputError::LimitExceeded)?).ok_or(RasterInputError::LimitExceeded)?;
    if source_lower > limits.resident_bytes || (source.tile_count() as u64).checked_mul(8).is_none_or(|work| work > limits.work_units) {
        return Err(RasterInputError::LimitExceeded);
    }
    let retained = source.resident_accounted_bytes()?;
    // Fixed bounded config/point scratch + owned prepared table envelope.
    let preparation = 64 * 1024u64;
    let peak =
        retained.checked_add(output).and_then(|n| n.checked_add(scratch)).and_then(|n| n.checked_add(preparation)).ok_or(RasterInputError::LimitExceeded)?;
    let work = pixels
        .checked_mul(4096)
        .and_then(|n| n.checked_add(output.checked_mul(16)?))
        .and_then(|n| n.checked_add(16384))
        .and_then(|n| n.checked_add((source.tile_count() as u64).checked_mul(8)?))
        .ok_or(RasterInputError::LimitExceeded)?;
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(RasterInputError::LimitExceeded);
    }
    source.try_validate_tile_origins_with(control)?;
    let bound = bind_source_node_with_sample_policy(node, binding, sampling, control)?;
    let preparation_limits = Limits { resident_bytes: preparation, work_units: 16384 };
    bound.with_prepared(actual, preparation_limits, control, |prepared| {
        render_prepared_window(
            prepared,
            source,
            roi,
            actual,
            policy,
            output,
            scratch,
            limits.work_units.checked_sub(16384).ok_or(RasterInputError::LimitExceeded)?,
            control,
        )
    })
}

// Shared numerical/staging consumer. Callers must preadmit output, scratch and
// work and validate exact source/context/ROI before entering this private seam.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_prepared_window(
    prepared: &photocraft_algo::gimp::configured::PreparedConfiguration<'_>,
    source: &Surface,
    roi: Rect,
    actual: WorkingContext,
    policy: OutputPolicy,
    output_bytes: u64,
    scratch: u64,
    work_units: u64,
    control: &Interrupt<'_>,
) -> Result<RenderedCandidate, RasterInputError> {
    let format = source.format();
    let bpp = format.bytes_per_pixel();
    let colors = actual.color_channels;
    let mut native_cells = native_cells::NativeCells::new(scratch, control)?;
    let mut prepaid = Prepaid { bytes: output_bytes, work: work_units };
    let mut builder = StagedSurfaceBuilder::new(format, source.default_bytes(), roi, &mut prepaid, control)?;
    // Match the builder's ROI-relative 64px coverage cells, not tile origins.
    for y in (roi.y0..roi.y1).step_by(64) {
        for x in (roi.x0..roi.x1).step_by(64) {
            poll(control)?;
            let part = Rect::new(
                x,
                y,
                x.checked_add(64).ok_or(RasterInputError::InvalidBounds)?.min(roi.x1),
                y.checked_add(64).ok_or(RasterInputError::InvalidBounds)?.min(roi.y1),
            );
            let raw = native_cells.read(source, part, control)?;
            for native in raw.chunks_exact_mut(bpp) {
                poll(control)?;
                prepaid.work(4096)?;
                let mut input = [0.0; 4];
                let mut output = [0.0; 4];
                for (index, sample) in input.iter_mut().take(format.channels()).enumerate() {
                    *sample = checked_arithmetic::decode_finite(
                        Operand { bytes: native, storage: Storage::Native(format.sample), index },
                        if index == colors { Domain::Unit } else { Domain::FloatHdr },
                    )
                    .map_err(|_| RasterInputError::InvalidSample)?;
                }
                prepared.try_eval_pixel(
                    input.get(..format.channels()).ok_or(RasterInputError::LengthMismatch)?,
                    output.get_mut(..format.channels()).ok_or(RasterInputError::LengthMismatch)?,
                    actual,
                    control,
                )?;
                let mut encoded = [0u8; 32];
                let encoded = encoded.get_mut(..bpp).ok_or(RasterInputError::LengthMismatch)?;
                encoded.copy_from_slice(native);
                for (index, sample) in output.iter().take(colors).enumerate() {
                    checked_arithmetic::encode_finite(*sample, Destination { bytes: encoded, storage: Storage::Native(format.sample), index }, policy.colors)
                        .map_err(|_| RasterInputError::InvalidSample)?;
                }
                if format.alpha && !matches!(policy.coverage, CoveragePolicy::PreserveSource) {
                    let mut alpha = *output.get(colors).ok_or(RasterInputError::LengthMismatch)?;
                    match policy.coverage {
                        CoveragePolicy::EvaluatedRejectUnit if !(0.0..=1.0).contains(&alpha) => return Err(RasterInputError::InvalidSample),
                        CoveragePolicy::EvaluatedClampUnit => alpha = alpha.clamp(0.0, 1.0),
                        _ => {}
                    }
                    checked_arithmetic::encode_finite(
                        alpha,
                        Destination { bytes: encoded, storage: Storage::Native(format.sample), index: colors },
                        policy.alpha,
                    )
                    .map_err(|_| RasterInputError::InvalidSample)?;
                }
                native.copy_from_slice(encoded);
            }
            builder.push_region(part, raw, &mut prepaid, control)?;
        }
    }
    let staged = builder.finalize(&mut prepaid, control)?;
    let (surface, ()) = staged.into_parts();
    poll(control)?;
    Ok(RenderedCandidate { surface, roi })
}
