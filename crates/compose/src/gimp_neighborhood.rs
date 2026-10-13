//! Private ordered neighborhood stages and bounded job-local tile cache.
use crate::{
    gimp_composite::{CompositionError, Result, WorkingSpace},
    gimp_filters::PointStack,
};
use photocraft_algo::gimp::neighborhood::{GaussianAbyss, GaussianLimits, GaussianParams, try_gaussian_young_recursive};
use photocraft_cms::{Profile, WideWork};
use photocraft_color::PixelFormat;
use photocraft_doc::{
    SampleEncoding,
    import_compositing::GimpSpace,
    xcf_filters::{ExactNumber, FilterNode, FilterStack, FilterValue},
};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Interrupt, Surface};
#[path = "gimp_gaussian_raster.rs"]
mod raster_gaussian;
pub(crate) use raster_gaussian::EPOCH as FINITE_GAUSSIAN_EPOCH;
#[path = "gimp_box.rs"]
mod box_blur;
pub(crate) use box_blur::EPOCH as BOX_BLUR_EPOCH;
#[path = "gimp_median.rs"]
mod median_blur;
pub(crate) use median_blur::{COLOR_EPOCH as MEDIAN_COLOR_EPOCH, EPOCH as MEDIAN_BLUR_EPOCH, GRAY_EPOCH as MEDIAN_GRAY_EPOCH};

#[path = "gimp_motion.rs"]
mod motion_blur;
pub(crate) use motion_blur::EPOCH as MOTION_BLUR_EPOCH;

#[path = "gimp_pixelize.rs"]
mod pixelize;
pub(crate) use pixelize::EPOCH as PIXELIZE_EPOCH;

const CACHE_BYTES: u64 = 16 * 1024 * 1024;
const JOB_BYTES: u64 = CACHE_BYTES * 2;
enum Stage {
    Point(PointStack),
    Curves(Box<crate::gimp_curves::CurvesNode>),
    Levels(Box<crate::gimp_levels::LevelsNode>),
    ColorBalance(Box<crate::gimp_color_balance::ColorBalanceNode>),
    Gaussian { working: usize, params: GaussianParams },
    GaussianFinite { working: usize, params: GaussianParams, filter: photocraft_algo::gimp::neighborhood::GaussianFilter },
    BoxBlur { working: usize, radius: u32 },
    MotionBlur { working: usize, params: photocraft_algo::gimp::neighborhood::MotionBlurParams },
    Pixelize { working: usize, params: photocraft_algo::gimp::neighborhood::PixelizeParams },
    MedianBlur { params: photocraft_algo::gimp::neighborhood::MedianParams },
}
pub(crate) struct OrderedStack {
    stages: Vec<Stage>,
    extent: Rect,
    output_extent: Rect,
    spaces: Vec<WorkingSpace>,
    integer_point_storage: Option<u64>,
    current_point_sample: Option<photocraft_color::SampleType>,
    filter_output_precision: photocraft_doc::FilterOutputPrecision,
}
/// Conservative capacity for one scalar stage/space per source node. Scalar
/// PointStack ownership and analytic profile clones are charged separately.
pub(crate) fn scalar_preparation_bytes(nodes: usize) -> Result<u64> {
    let bytes = nodes
        .checked_mul(std::mem::size_of::<Stage>() + std::mem::size_of::<WorkingSpace>())
        .ok_or(CompositionError::Invalid("ordered preparation size overflow"))?;
    u64::try_from(bytes).map_err(|_| CompositionError::Invalid("ordered preparation size overflow"))
}
impl OrderedStack {
    /// Actual Vec capacities for the scalar-only preparation class. Complex
    /// stages remain accounting-unsupported even if separately render-qualified.
    pub(crate) fn scalar_retained_bytes(&self) -> Result<u64> {
        let mut bytes = self
            .stages
            .capacity()
            .checked_mul(std::mem::size_of::<Stage>())
            .and_then(|n| self.spaces.capacity().checked_mul(std::mem::size_of::<WorkingSpace>()).and_then(|s| n.checked_add(s)))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(CompositionError::Invalid("ordered capacity overflow"))?;
        for stage in &self.stages {
            let Stage::Point(point) = stage else {
                return Err(CompositionError::Unsupported("non-scalar retained preparation owner"));
            };
            bytes = bytes.checked_add(point.scalar_retained_bytes()?).ok_or(CompositionError::Invalid("ordered capacity overflow"))?;
        }
        Ok(bytes)
    }
    #[cfg(test)]
    pub(crate) fn new(source: &FilterStack, profile: &Profile, stored: SampleEncoding, version: u32, extent: Rect, qualified: [bool; 3]) -> Result<Self> {
        Self::new_with_interrupt(source, profile, stored, version, extent, qualified, &Interrupt::NONE)
    }
    pub(crate) fn new_with_interrupt(source: &FilterStack, profile: &Profile, stored: SampleEncoding, version: u32, extent: Rect, qualified: [bool; 3], control: &Interrupt<'_>) -> Result<Self> {
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        let result = Self::prepare_with_interrupt(source, profile, stored, version, extent, qualified, control);
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        result
    }
    fn prepare_with_interrupt(source: &FilterStack, profile: &Profile, stored: SampleEncoding, version: u32, extent: Rect, qualified: [bool; 3], control: &Interrupt<'_>) -> Result<Self> {
        if source.nodes.len() > 256 {
            return Err(CompositionError::Invalid("ordered filter stack limit"));
        }
        if source.nodes.len() > 32 && source.nodes.iter().any(|n| n.enabled && n.operation.name == "gegl:gaussian-blur") {
            return Err(CompositionError::Invalid("neighborhood stage depth limit"));
        }
        if source.nodes.iter().any(|node| node.enabled && node.operation.name == "gimp:levels") && source.nodes.iter().filter(|node| node.enabled).count() != 1
        {
            return Err(CompositionError::Unsupported("structured Levels intermediate Alpha boundary needs a stack witness"));
        }
        control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
        let mut stages = Vec::new();
        let mut output_extent = extent;
        let mut spaces = Vec::new();
        spaces.try_reserve_exact(source.nodes.len()).map_err(|_| CompositionError::Invalid("ordered space allocation denied"))?;
        stages.try_reserve_exact(source.nodes.len()).map_err(|_| CompositionError::Invalid("ordered preparation allocation denied"))?;
        if source
            .nodes
            .iter()
            .any(|node| node.operation.name == "gegl:unpremultiply"
                || crate::gimp_video_degradation::claims_video(node)
                || crate::gimp_red_eye::claims_geometry(node)
                || node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH))
        {
            // Disabled current nodes still require their closed typed schema;
            // activation must not be an escape from source-state validation.
            stages.push(Stage::Point(PointStack::from_nodes_at_with_interrupt(&source.nodes, profile, stored, version, extent, control)?));
            return Ok(Self {
                stages,
                extent,
                output_extent,
                spaces,
                integer_point_storage: None,
                current_point_sample: None,
                filter_output_precision: photocraft_doc::FilterOutputPrecision::Canonical,
            });
        }
        if source.nodes.iter().any(|node| node.operation.schema_epoch == crate::gimp_filters::STACK_EDITS_EPOCH) {
            if extent != Rect::from_xywh(0, 0, 8, 6) {
                return Err(CompositionError::Unsupported("witnessed point stack extent"));
            }
            // This source order is one qualified point pipeline, not independent
            // singleton stages. Its caller validates every canonical node/mask;
            // PointStack validates the exact epoch/envelope and schedules bottom first.
            stages.push(Stage::Point(PointStack::from_nodes_at_with_interrupt(&source.nodes, profile, stored, version, extent, control)?));
            return Ok(Self {
                stages,
                extent,
                output_extent,
                spaces,
                integer_point_storage: None,
                current_point_sample: None,
                filter_output_precision: photocraft_doc::FilterOutputPrecision::Canonical,
            });
        }
        for node in source.nodes.iter().filter(|n| n.enabled) {
            control.check().map_err(|_| photocraft_raster::RasterInputError::Cancelled)?;
            if node.operation.name == "gegl:gaussian-blur" {
                if !qualified[0] {
                    return Err(CompositionError::Unsupported("Gaussian source color/extent domain"));
                }
                if node.operation.schema_epoch == FINITE_GAUSSIAN_EPOCH {
                    if extent != Rect::from_xywh(0, 0, 16, 12)
                        || profile.channels() != 3
                        || stored != SampleEncoding::NonlinearProfile
                        || source.nodes.len() != 1
                    {
                        return Err(CompositionError::Unsupported("finite Gaussian raster source context"));
                    }
                    let (params, filter) = raster_gaussian::parameters(node, version)?;
                    spaces.push(WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?);
                    stages.push(Stage::GaussianFinite { working: spaces.len() - 1, params, filter });
                    continue;
                }
                let params = gaussian(node, version)?;
                spaces.push(WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?);
                stages.push(Stage::Gaussian { working: spaces.len() - 1, params });
            } else if node.operation.name == "gegl:box-blur" {
                if !qualified[0] || profile.channels() != 3 || stored != SampleEncoding::NonlinearProfile || source.nodes.len() != 1 {
                    return Err(CompositionError::Unsupported("Box Blur source context"));
                }
                let radius = box_blur::radius(node, version, extent)?;
                output_extent = photocraft_algo::gimp::neighborhood::box_blur_extent(extent, radius)?;
                spaces.push(WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?);
                stages.push(Stage::BoxBlur { working: spaces.len() - 1, radius });
            } else if node.operation.name == "gegl:motion-blur-linear" {
                if !qualified[0] || profile.channels() != 3 || stored != SampleEncoding::NonlinearProfile || source.nodes.len() != 1 {
                    return Err(CompositionError::Unsupported("Motion source context"));
                }
                let params = motion_blur::parameters(node, version, extent)?;
                output_extent = photocraft_algo::gimp::neighborhood::motion_blur_extent(extent, params)?;
                spaces.push(WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?);
                stages.push(Stage::MotionBlur { working: spaces.len() - 1, params });
            } else if node.operation.name == "gegl:pixelize" {
                if !qualified[0] || profile.channels() != 3 || stored != SampleEncoding::NonlinearProfile || source.nodes.len() != 1 {
                    return Err(CompositionError::Unsupported("Pixelize source context"));
                }
                let params = pixelize::parameters(node, version, extent)?;
                spaces.push(WorkingSpace::new_with_interrupt(profile, stored, GimpSpace::LinearRgb, control)?);
                stages.push(Stage::Pixelize { working: spaces.len() - 1, params });
            } else if node.operation.name == "gegl:median-blur" {
                let gray = node.operation.schema_epoch == MEDIAN_GRAY_EPOCH;
                if !qualified[0] || profile.channels() != if gray { 1 } else { 3 } || stored != SampleEncoding::NonlinearProfile || source.nodes.len() != 1 {
                    return Err(CompositionError::Unsupported("Median source context"));
                }
                let separable = if gray { profile.gray_trc.is_some() } else { profile.matrix.is_some() && profile.trc.is_some() };
                if !separable || profile.a2b.iter().any(Option::is_some) || profile.b2a.iter().any(Option::is_some) {
                    return Err(CompositionError::Unsupported("Median separable profile domain"));
                }
                stages.push(Stage::MedianBlur { params: median_blur::parameters(node, version, extent)? });
            } else if node.operation.name == "gimp:curves" {
                if !qualified[1] {
                    return Err(CompositionError::Unsupported("FREE curves source color/extent domain"));
                }
                stages.push(Stage::Curves(Box::new(crate::gimp_curves::CurvesNode::new(node, version)?)));
            } else if node.operation.name == "gimp:color-balance" {
                if source.nodes.len() != 1 || extent != Rect::new(0, 0, 8, 6) {
                    return Err(CompositionError::Unsupported("Color Balance stack/extent domain"));
                }
                stages.push(Stage::ColorBalance(Box::new(crate::gimp_color_balance::ColorBalanceNode::new(node, profile, stored, version)?)));
            } else if node.operation.name == "gimp:levels" {
                if !qualified[2] {
                    return Err(CompositionError::Unsupported("structured Levels source color/extent domain"));
                }
                stages.push(Stage::Levels(Box::new(crate::gimp_levels::LevelsNode::new(node, profile, stored, version)?)));
            } else {
                stages.push(Stage::Point(PointStack::from_nodes_at_with_interrupt(std::slice::from_ref(node), profile, stored, version, extent, control)?));
            }
        }
        Ok(Self {
            stages,
            extent,
            output_extent,
            spaces,
            integer_point_storage: None,
            current_point_sample: None,
            filter_output_precision: photocraft_doc::FilterOutputPrecision::Canonical,
        })
    }
    /// The qualified U8/U32 point source writes an integer drawable buffer before
    /// layer composition. Round stored colors at that boundary, preserving
    /// the unchanged source alpha. Other point/native precision paths stay wide.
    pub(crate) fn with_current_point_storage(mut self, sample: Option<photocraft_color::SampleType>) -> Self {
        self.current_point_sample = sample;
        for stage in &mut self.stages {
            if let Stage::Point(point) = stage {
                point.set_current_sample(sample);
            }
        }
        self.integer_point_storage = match sample {
            Some(photocraft_color::SampleType::U8) => Some(255),
            Some(photocraft_color::SampleType::U32) => Some(4294967295),
            _ => None,
        };
        self
    }
    /// Current buffer policy is supplied from mutable canonical state, never provenance.
    pub(crate) fn with_filter_output_precision(mut self, precision: photocraft_doc::FilterOutputPrecision) -> Self {
        self.filter_output_precision = precision;
        self
    }
    fn checked_stored_point_result(&self, pixel: [f64; 4], changes_alpha: bool) -> Result<[f64; 4]> {
        let mut output = self.stored_point_result(pixel, changes_alpha);
        // Expanded Mono and Channel Mixer expose finite signed/HDR generated values.
        // Its live buffer is stored at the current precision before composition.
        if self.stages.iter().any(|stage| matches!(stage, Stage::Point(point) if point.has_signed_generated_storage()))
            && let Some(sample) = self.current_point_sample
        {
            for value in &mut output {
                let mut bytes = [0; 8];
                photocraft_color::try_write_sample_f64(&mut bytes, sample, 0, *value)
                    .map_err(|_| CompositionError::Invalid("Mono Mixer generated storage write"))?;
                *value =
                    photocraft_color::try_read_sample_f64(&bytes, sample, 0).map_err(|_| CompositionError::Invalid("Mono Mixer generated storage read"))?;
            }
        }
        if self.filter_output_precision == photocraft_doc::FilterOutputPrecision::Half && changes_alpha {
            for component in &mut output {
                let bits = photocraft_color::try_round_f32_to_f16_bits(*component as f32)
                    .map_err(|_| CompositionError::Invalid("generated Half filter buffer overflow"))?;
                *component = f64::from(f32::from_bits(photocraft_color::widen_f16_to_f32_bits(bits)));
            }
        }
        Ok(output)
    }
    fn stored_point_result(&self, mut pixel: [f64; 4], changes_alpha: bool) -> [f64; 4] {
        if let Some(denominator) = self.integer_point_storage {
            let denominator = denominator as f64;
            // Public Color to Alpha U8 projection independently proves the
            // same current storage boundary applies to generated coverage.
            let count = if changes_alpha { 4 } else { 3 };
            for component in pixel.iter_mut().take(count) {
                *component = (component.clamp(0.0, 1.0) * denominator).round() / denominator;
            }
        }
        pixel
    }
    pub(crate) fn has_neighborhood(&self) -> bool {
        self.stages.iter().any(|n| {
            matches!(
                n,
                Stage::Gaussian { .. }
                    | Stage::GaussianFinite { .. }
                    | Stage::BoxBlur { .. }
                    | Stage::MotionBlur { .. }
                    | Stage::Pixelize { .. }
                    | Stage::MedianBlur { .. }
            )
        })
    }
    /// A point effect cannot generate coverage outside its declared output extent.
    /// In particular, opaque component extraction must preserve transparent abyss.
    pub(crate) fn point_at(&self, pixel: [f64; 4], position: (i32, i32), poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        if !self.output_extent.contains(position.0, position.1) {
            return Ok(pixel);
        }
        self.point_with(pixel, Some(position), poll)
    }
    #[cfg(test)]
    pub(crate) fn point(&self, pixel: [f64; 4], poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        self.point_with(pixel, None, poll)
    }
    fn point_with(&self, mut pixel: [f64; 4], position: Option<(i32, i32)>, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<[f64; 4]> {
        for stage in &self.stages {
            pixel = match stage {
                Stage::Point(point) => {
                    let result = match position {
                        Some(position) => point.apply_current_at(position, pixel, self.integer_point_storage == Some(4294967295), poll)?,
                        None => point.apply_current(pixel, self.integer_point_storage == Some(4294967295), poll)?,
                    };
                    self.checked_stored_point_result(result, point.changes_alpha())?
                }
                Stage::Curves(curves) => curves.apply(pixel, poll)?,
                Stage::Levels(levels) => levels.apply(pixel, poll)?,
                Stage::ColorBalance(balance) => balance.apply(pixel, poll)?,
                Stage::Gaussian { .. }
                | Stage::GaussianFinite { .. }
                | Stage::BoxBlur { .. }
                | Stage::MotionBlur { .. }
                | Stage::Pixelize { .. }
                | Stage::MedianBlur { .. } => {
                    return Err(CompositionError::Invalid("neighborhood cache missing"));
                }
            };
        }
        Ok(pixel)
    }
    pub(crate) fn evaluate(&self, source: &Surface, output: Rect, poll: &mut impl FnMut(WideWork) -> Result<()>, interrupt: &Interrupt<'_>) -> Result<Surface> {
        self.prefix(source, self.stages.len(), output.intersect(&self.output_extent), poll, interrupt)
    }
    fn prefix(&self, source: &Surface, count: usize, roi: Rect, poll: &mut impl FnMut(WideWork) -> Result<()>, interrupt: &Interrupt<'_>) -> Result<Surface> {
        if interrupt.cancelled() {
            return Err(photocraft_raster::RasterInputError::Cancelled.into());
        }
        if count == 0 {
            return map_pixels(source, roi, poll, interrupt, Ok);
        }
        let stage = self.stages.get(count - 1).ok_or(CompositionError::Invalid("ordered stage reference"))?;
        match stage {
            Stage::MedianBlur { params } => {
                let input = self.prefix(source, count - 1, self.extent, poll, interrupt)?;
                let window = u64::try_from(params.radius).map_err(|_| CompositionError::Invalid("Median radius"))? * 2 + 1;
                let units = u64::from(roi.width()) * u64::from(roi.height()) * window.pow(2) * 4 * 16;
                poll(WideWork::SampleUnits(units))?;
                let limits = GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units };
                Ok(if params.high_precision {
                    photocraft_algo::gimp::neighborhood::try_median_blur(&input, roi, self.extent, *params, limits, interrupt)?
                } else {
                    photocraft_algo::gimp::neighborhood::try_median_blur_quantized_encoded(&input, roi, self.extent, *params, limits, interrupt)?
                })
            }
            Stage::Pixelize { working, params } => {
                let working = self.spaces.get(*working).ok_or(CompositionError::Invalid("Pixelize working space"))?;
                let input = self.prefix(source, count - 1, self.extent, poll, interrupt)?;
                let linear = map_pixels(&input, self.extent, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], true)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })?;
                let units =
                    u64::from(self.extent.width()) * u64::from(self.extent.height()) * 4 * 32 + u64::from(roi.width()) * u64::from(roi.height()) * 4 * 16;
                poll(WideWork::SampleUnits(units))?;
                let filtered = photocraft_algo::gimp::neighborhood::try_pixelize_square(
                    &linear,
                    roi,
                    self.extent,
                    *params,
                    GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units },
                    interrupt,
                )?;
                map_pixels(&filtered, roi, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], false)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })
            }
            Stage::MotionBlur { working, params } => {
                let working = self.spaces.get(*working).ok_or(CompositionError::Invalid("Motion working space"))?;
                let input = self.prefix(source, count - 1, self.extent, poll, interrupt)?;
                let linear = map_pixels(&input, self.extent, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], true)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })?;
                let samples = if params.length == 10. { 11u64 } else { 12u64 };
                let units = u64::from(roi.width()) * u64::from(roi.height()) * samples * 4 * 32;
                poll(WideWork::SampleUnits(units))?;
                let filtered = photocraft_algo::gimp::neighborhood::try_motion_blur_linear(
                    &linear,
                    roi,
                    self.extent,
                    *params,
                    GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units },
                    interrupt,
                )?;
                map_pixels(&filtered, roi, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], false)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })
            }
            Stage::BoxBlur { working, radius } => {
                let working = self.spaces.get(*working).ok_or(CompositionError::Invalid("Box Blur working space"))?;
                let input = self.prefix(source, count - 1, self.extent, poll, interrupt)?;
                let linear = map_pixels(&input, self.extent, poll, interrupt, |p| {
                    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
                        return Err(CompositionError::Unsupported("Box Blur color/alpha domain"));
                    }
                    let rgb = working.convert([p[0], p[1], p[2]], true)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })?;
                let units = box_blur::work(roi, *radius)?;
                poll(WideWork::SampleUnits(units))?;
                let filtered = photocraft_algo::gimp::neighborhood::try_box_blur(
                    &linear,
                    roi,
                    self.extent,
                    *radius,
                    GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units },
                    interrupt,
                )?;
                map_pixels(&filtered, roi, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], false)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })
            }
            Stage::GaussianFinite { working, params, filter } => {
                let working = self.spaces.get(*working).ok_or(CompositionError::Invalid("finite Gaussian space"))?;
                let input = self.prefix(source, count - 1, self.extent, poll, interrupt)?;
                let linear = map_pixels(&input, self.extent, poll, interrupt, |p| {
                    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
                        return Err(CompositionError::Unsupported("finite Gaussian color/alpha domain"));
                    }
                    let rgb = working.convert([p[0], p[1], p[2]], true)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })?;
                let units = u64::from(self.extent.width()) * u64::from(self.extent.height()) * 4 * 1024;
                poll(WideWork::SampleUnits(units))?;
                let filtered = photocraft_algo::gimp::neighborhood::try_gaussian_finite_clamp(
                    &linear,
                    self.extent,
                    self.extent,
                    *params,
                    *filter,
                    GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units },
                    interrupt,
                )?;
                map_pixels(&filtered, roi, poll, interrupt, |p| {
                    let rgb = working.convert([p[0], p[1], p[2]], false)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })
            }
            Stage::Point(point) => {
                let input = self.prefix(source, count - 1, roi, poll, interrupt)?;
                map_pixels_at(&input, roi, poll, interrupt, |position, p| {
                    self.checked_stored_point_result(
                        point.apply_current_at(position, p, self.integer_point_storage == Some(4294967295), &mut |_| Ok(()))?,
                        point.changes_alpha(),
                    )
                })
            }
            Stage::Curves(curves) => {
                let input = self.prefix(source, count - 1, roi, poll, interrupt)?;
                map_pixels(&input, roi, poll, interrupt, |p| curves.apply(p, &mut |_| Ok(())))
            }
            Stage::ColorBalance(balance) => {
                let input = self.prefix(source, count - 1, roi, poll, interrupt)?;
                map_pixels(&input, roi, poll, interrupt, |p| balance.apply(p, &mut |_| Ok(())))
            }
            Stage::Levels(levels) => {
                let input = self.prefix(source, count - 1, roi, poll, interrupt)?;
                map_pixels(&input, roi, poll, interrupt, |p| levels.apply(p, &mut |_| Ok(())))
            }
            Stage::Gaussian { working, params } => {
                let working = self.spaces.get(*working).ok_or(CompositionError::Invalid("ordered space reference"))?;
                // Qualified sigmas have a bounded recursive guard: twice ceil(12*sigma)+32.
                let gx = 2 * ((params.std_dev_x * 12.0).ceil() as i32 + 32);
                let gy = 2 * ((params.std_dev_y * 12.0).ceil() as i32 + 32);
                let halo = Rect::new(
                    roi.x0.checked_sub(gx).ok_or(CompositionError::Invalid("Gaussian halo"))?,
                    roi.y0.checked_sub(gy).ok_or(CompositionError::Invalid("Gaussian halo"))?,
                    roi.x1.checked_add(gx).ok_or(CompositionError::Invalid("Gaussian halo"))?,
                    roi.y1.checked_add(gy).ok_or(CompositionError::Invalid("Gaussian halo"))?,
                )
                .intersect(&self.extent);
                let input = self.prefix(source, count - 1, halo, poll, interrupt)?;
                let linear = map_pixels(&input, halo, poll, interrupt, |p| {
                    if p[3] != 1.0 {
                        return Err(CompositionError::Unsupported("Gaussian opaque source domain"));
                    }
                    let rgb = working.convert([p[0], p[1], p[2]], true)?;
                    Ok([rgb[0], rgb[1], rgb[2], p[3]])
                })?;
                let units = u64::from(roi.width())
                    .checked_add((gx as u64) * 2)
                    .and_then(|w| w.checked_mul(u64::from(roi.height()) + (gy as u64) * 2))
                    .and_then(|v| v.checked_mul(4 * 64))
                    .ok_or(CompositionError::Invalid("Gaussian work overflow"))?;
                poll(WideWork::SampleUnits(units))?;
                let filtered = try_gaussian_young_recursive(
                    &linear,
                    roi,
                    self.extent,
                    *params,
                    GaussianLimits { resident_bytes: CACHE_BYTES, work_units: units },
                    interrupt,
                )?;
                map_pixels(&filtered, roi, poll, interrupt, |p| {
                    // This admitted domain proved constant opaque input with Clamp.
                    // Its exact alpha is one; recurrence roundoff is not coverage.
                    if !p[3].is_finite() || (p[3] - 1.0).abs() > 1e-12 {
                        return Err(CompositionError::Invalid("Gaussian opaque alpha invariant"));
                    }
                    let rgb = working.convert([p[0], p[1], p[2]], false)?;
                    Ok([rgb[0], rgb[1], rgb[2], 1.0])
                })
            }
        }
    }
}
fn gaussian(node: &FilterNode, version: u32) -> Result<GaussianParams> {
    let c = node.composition.as_ref().ok_or(CompositionError::Unsupported("missing Gaussian effect policy"))?;
    if version != 22
        || node.operation.stored_version != "0:0"
        || c.source_mode != 62
        || c.legacy
        || c.pass_through
        || c.resolved.is_some()
        || c.stored_blend_space != -856260015
        || c.stored_composite_space != -856260015
        || c.stored_operator != -856260015
        || node.opacity_f64_bits != 1.0_f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || node.arguments.len() != 5
    {
        return Err(CompositionError::Unsupported("unqualified Gaussian Replace envelope"));
    }
    let get = |name: &str| -> Result<&FilterValue> {
        let mut found = node.arguments.iter().filter(|a| a.name == name);
        let value = &found.next().ok_or(CompositionError::Unsupported("Gaussian argument absent"))?.value;
        if found.next().is_some() {
            return Err(CompositionError::Unsupported("duplicate Gaussian argument"));
        }
        Ok(value)
    };
    let number = |name| -> Result<f64> {
        match get(name)? {
            FilterValue::Number(ExactNumber::Float32Bits(bits)) => Ok(f64::from(f32::from_bits(*bits))),
            _ => Err(CompositionError::Unsupported("Gaussian argument stored type")),
        }
    };
    let x = number("std-dev-x")?;
    let y = number("std-dev-y")?;
    if !matches!(x, 2.25 | 3.75)
        || y != 1.5
        || !matches!(get("filter")?, FilterValue::Enum { domain, value: 0 } if domain == "gegl:gaussian-blur::filter")
        || !matches!(get("abyss-policy")?, FilterValue::Enum { domain, value: 1 } if domain == "gegl:gaussian-blur::abyss-policy")
        || get("clip-extent")? != &FilterValue::Bool(true)
    {
        return Err(CompositionError::Unsupported("unqualified Gaussian Auto/Clamp/clip domain"));
    }
    Ok(GaussianParams { std_dev_x: x, std_dev_y: y, abyss: GaussianAbyss::Clamp, clip_extent: true })
}
pub(crate) fn qualify_selection_mask(state: &photocraft_doc::XcfState, node: &FilterNode, owner: photocraft_doc::LayerId, extent: Rect) -> Result<()> {
    if crate::gimp_red_eye::claims_geometry(node) {
        let mask = node.mask.and_then(|reference| match reference {
            photocraft_doc::item_identity::ItemRef::Mask(id) => state.filter_masks.iter().find(|mask| mask.id == id),
            _ => None,
        }).ok_or(CompositionError::Unsupported("Red Eye owner geometry requires its saved canvas mask"))?;
        if node.operation.name != "gegl:red-eye-removal" || node.operation.schema_epoch != crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH
            || !node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == crate::gimp_red_eye::RED_EYE_OWNER_GEOMETRY_EPOCH)
            || !matches!(extent, Rect { x0: 0, y0: 0, x1: 37, y1: 29 } | Rect { x0: 0, y0: 0, x1: 259, y1: 17 })
            || mask.surface.format() != PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false)
            || state.items.iter().any(|item| item.item == photocraft_doc::item_identity::ItemRef::Mask(mask.id) && (item.composition.is_some() || item.source_flags != 0)) {
            return Err(CompositionError::Unsupported("Red Eye owner geometry exact canvas mask context"));
        }
        return qualify_point_canvas_mask(state, node, owner, extent);
    }
    if crate::gimp_video_degradation::claims_options_epoch(node) {
        let mask = node.mask.and_then(|reference| match reference {
            photocraft_doc::item_identity::ItemRef::Mask(id) => state.filter_masks.iter().find(|mask| mask.id == id),
            _ => None,
        }).ok_or(CompositionError::Unsupported("Video options requires its saved canvas selection mask"))?;
        if extent != Rect::new(0, 0, 16, 16)
            || mask.surface.format() != PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false)
            || state.items.iter().any(|item| item.item == photocraft_doc::item_identity::ItemRef::Mask(mask.id) && item.composition.is_some())
        {
            return Err(CompositionError::Unsupported("Video options exact Float canvas mask context"));
        }
        return qualify_point_canvas_mask(state, node, owner, extent);
    }
    if node.operation.name == "gegl:unpremultiply" {
        let mask = node.mask.and_then(|reference| match reference { photocraft_doc::item_identity::ItemRef::Mask(id) => state.filter_masks.iter().find(|mask| mask.id == id), _ => None })
            .ok_or(CompositionError::Unsupported("Unpremultiply requires its saved selection mask"))?;
        if extent != Rect::new(0, 0, 8, 6) || mask.surface.format() != PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false) {
            return Err(CompositionError::Unsupported("Unpremultiply exact Float mask extent"));
        }
        if state.items.iter().any(|item| item.item == photocraft_doc::item_identity::ItemRef::Mask(mask.id) && item.composition.is_some()) {
            return Err(CompositionError::Unsupported("Unpremultiply mask current composition"));
        }
        return qualify_point_canvas_mask(state, node, owner, extent);
    }
    if node.operation.schema_epoch == crate::gimp_filters::COMPONENT_EXTRACT_GEOMETRY_EPOCH {
        let mask = node
            .mask
            .and_then(|reference| match reference {
                photocraft_doc::item_identity::ItemRef::Mask(id) => state.filter_masks.iter().find(|mask| mask.id == id),
                _ => None,
            })
            .ok_or(CompositionError::Unsupported("Component Extract geometry requires canvas mask"))?;
        if !matches!((extent.width(), extent.height(), extent.x0, extent.y0), (37, 29, 0, 0) | (259, 17, 0, 0))
            || mask.surface.format() != PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false)
        {
            return Err(CompositionError::Unsupported("Component Extract geometry canvas mask domain"));
        }
    }
    if node.composition.as_ref().is_some_and(|policy| policy.compatibility_epoch == crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH) {
        return qualify_point_canvas_mask(state, node, owner, extent);
    }
    if node.operation.schema_epoch == crate::gimp_filters::EXPOSURE_GEOMETRY_EPOCH {
        return qualify_exposure_canvas_mask(state, node, owner);
    }
    let id = match node.mask {
        None => return Ok(()),
        Some(photocraft_doc::item_identity::ItemRef::Mask(id)) => id,
        _ => return Err(CompositionError::Invalid("filter mask reference type")),
    };
    let mut masks = state.filter_masks.iter().filter(|m| m.id == id);
    let mask = masks.next().ok_or(CompositionError::Invalid("filter mask missing"))?;
    let format = mask.surface.format();
    let rgb_clip_mask = node.operation.name == "gegl:rgb-clip"
        && node.operation.schema_epoch == crate::gimp_filters::RGB_CLIP_HDR_EPOCH
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format == PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false);
    let value_precision_mask = node.operation.name == "gegl:value-invert"
        && node.operation.schema_epoch == crate::gimp_filters::VALUE_INVERT_PRECISION_EPOCH
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64);
    let brightness_precision_mask = node.operation.name == "gegl:brightness-contrast"
        && node.operation.schema_epoch == crate::gimp_filters::BRIGHTNESS_CONTRAST_PRECISION_EPOCH
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64);
    let gegl_levels_precision_mask = node.operation.name == "gegl:levels"
        && node.operation.schema_epoch == crate::gimp_filters::GEGL_LEVELS_PRECISION_EPOCH
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64);
    let exposure_black_precision_mask = node.operation.name == "gegl:exposure"
        && node.operation.schema_epoch == crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64);
    let color_temperature_precision_mask = (node.operation.name == "gimp:threshold-alpha"
        && node.operation.schema_epoch == crate::gimp_filters::THRESHOLD_ALPHA_PRECISION_EPOCH
        || node.operation.name == "gimp:threshold" && node.operation.schema_epoch == crate::gimp_filters::THRESHOLD_PRECISION_EPOCH
        || node.operation.name == "gegl:saturation"
            && matches!(
                node.operation.schema_epoch.as_str(),
                crate::gimp_filters::SATURATION_PRECISION_EPOCH | crate::gimp_filters::SATURATION_GRAY_PRECISION_EPOCH
            )
        || node.operation.name == "gegl:sepia" && node.operation.schema_epoch == crate::gimp_filters::SEPIA_PRECISION_EPOCH
        || node.operation.name == "gegl:alien-map"
            && matches!(
                node.operation.schema_epoch.as_str(),
                crate::gimp_filters::ALIEN_MAP_PRECISION_EPOCH | crate::gimp_filters::ALIEN_MAP_HSL_PRECISION_EPOCH
            )
        || node.operation.name == "gimp:desaturate" && node.operation.schema_epoch == crate::gimp_filters::DESATURATE_PRECISION_EPOCH
        || node.operation.name == "gegl:color-temperature"
            && matches!(
                node.operation.schema_epoch.as_str(),
                crate::gimp_filters::COLOR_TEMPERATURE_PRECISION_EPOCH | crate::gimp_filters::COLOR_TEMPERATURE_OFFGRID_EPOCH
            ))
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64);
    let saturation_yuv_node = node.operation.name == "gegl:saturation"
        && (node.operation.schema_epoch == crate::gimp_filters::SATURATION_RGB_YUV_PRECISION_EPOCH
            || node.operation.schema_epoch == crate::gimp_filters::SATURATION_PRECISION_EPOCH
                && node
                    .arguments
                    .iter()
                    .any(|a| a.name == "colorspace" && matches!(&a.value, FilterValue::Enum { domain, value: 2 } if domain == "GeglSaturationType")));
    if saturation_yuv_node && !matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32) {
        return Err(CompositionError::Unsupported("unqualified RGB Yuv selection mask precision"));
    }
    let saturation_yuv_mask = saturation_yuv_node
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32);
    let color_balance_depth = node.operation.schema_epoch == crate::gimp_color_balance::DEPTH_EPOCH;
    let color_balance_mask = node.operation.name == "gimp:color-balance"
        && matches!(node.operation.schema_epoch.as_str(), crate::gimp_color_balance::EPOCH | crate::gimp_color_balance::DEPTH_EPOCH)
        && extent == Rect::from_xywh(0, 0, 8, 6)
        && format.mode == photocraft_color::ColorMode::Grayscale
        && !format.alpha
        && (matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
            || color_balance_depth && matches!(format.sample, photocraft_color::SampleType::U8 | photocraft_color::SampleType::U32));
    if (rgb_clip_mask
        || value_precision_mask
        || brightness_precision_mask
        || gegl_levels_precision_mask
        || exposure_black_precision_mask
        || color_temperature_precision_mask
        || saturation_yuv_mask
        || color_balance_mask)
        && (mask.surface.default_bytes().iter().any(|byte| *byte != 0) || mask.surface.try_read_interleaved(extent)?.iter().any(|byte| *byte != 0))
    {
        return Err(CompositionError::Unsupported("unqualified precision point selection mask bits"));
    }
    let qualified_format = color_temperature_precision_mask
        || saturation_yuv_mask
        || color_balance_mask
        || exposure_black_precision_mask
        || gegl_levels_precision_mask
        || brightness_precision_mask
        || value_precision_mask
        || rgb_clip_mask
        || format == PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U8, false)
        || (node.operation.name == "gimp:levels"
            && format == PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false))
        || (matches!(node.operation.schema_epoch.as_str(), FINITE_GAUSSIAN_EPOCH | BOX_BLUR_EPOCH | MEDIAN_COLOR_EPOCH | MEDIAN_GRAY_EPOCH)
            && extent == Rect::from_xywh(0, 0, 16, 12)
            && matches!(
                format,
                PixelFormat {
                    mode: photocraft_color::ColorMode::Grayscale,
                    sample: photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64,
                    alpha: false
                }
            ));
    if masks.next().is_some()
        || mask.owner != node.id
        || mask.bounds != extent
        || !qualified_format
        || mask.surface.resident_payload_bytes()?
            > if ((value_precision_mask
                || brightness_precision_mask
                || gegl_levels_precision_mask
                || exposure_black_precision_mask
                || color_temperature_precision_mask
                || color_balance_mask)
                && format.sample == photocraft_color::SampleType::F64)
                || matches!(node.operation.schema_epoch.as_str(), FINITE_GAUSSIAN_EPOCH | BOX_BLUR_EPOCH | MEDIAN_COLOR_EPOCH | MEDIAN_GRAY_EPOCH)
            {
                8 * 65536 + 8
            } else {
                4 * 65536 + 4
            }
    {
        return Err(CompositionError::Unsupported("unqualified selection filter mask domain"));
    }
    let mut items = state.items.iter().filter(|item| item.item == photocraft_doc::item_identity::ItemRef::Mask(id));
    let item = items.next().ok_or(CompositionError::Invalid("filter mask registry missing"))?;
    let metadata = item.mask.as_ref().ok_or(CompositionError::Invalid("filter mask metadata missing"))?;
    if items.next().is_some()
        || ((rgb_clip_mask
            || value_precision_mask
            || brightness_precision_mask
            || gegl_levels_precision_mask
            || exposure_black_precision_mask
            || color_temperature_precision_mask
            || color_balance_mask)
            && !item.visible)
        || metadata.owner != photocraft_doc::item_identity::ItemRef::Layer(owner)
        || metadata.bounds != mask.bounds
        || metadata.offset != [mask.bounds.x0, mask.bounds.y0]
        || metadata.outside_f64_bits != 0.0_f64.to_bits()
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(CompositionError::Invalid("filter mask relation or bounds"));
    }
    consume_empty_selection_state(state, node, owner, mask.bounds, mask.surface.format())
}
/// For the measured Replace compatibility epoch, `canvas` is the document
/// canvas rather than the drawable extent. The caller validates operation,
/// source model/precision/profile and matching mask sample depth separately.
/// No absent mask or nonzero selection is inferred as unrestricted filtering.
fn qualify_point_canvas_mask(state: &photocraft_doc::XcfState, node: &FilterNode, owner: photocraft_doc::LayerId, canvas: Rect) -> Result<()> {
    use photocraft_doc::item_identity::ItemRef;
    let Some(ItemRef::Mask(id)) = node.mask else {
        return Err(CompositionError::Unsupported("point Replace requires its saved canvas selection mask"));
    };
    if canvas.x0 != 0 || canvas.y0 != 0 || canvas.is_empty() || canvas.width() > 262144 || canvas.height() > 262144 {
        return Err(CompositionError::Invalid("point Replace canvas bounds"));
    }
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let mask = masks.next().ok_or(CompositionError::Invalid("point Replace canvas mask missing"))?;
    let format = mask.surface.format();
    if masks.next().is_some()
        || node.source_region != Some(0)
        || mask.owner != node.id
        || mask.bounds != canvas
        || format.mode != photocraft_color::ColorMode::Grayscale
        || format.alpha
        || !matches!(
            format.sample,
            photocraft_color::SampleType::U8
                | photocraft_color::SampleType::U16
                | photocraft_color::SampleType::U32
                | photocraft_color::SampleType::F32
                | photocraft_color::SampleType::F64
        )
        || mask.surface.resident_payload_bytes()? > CACHE_BYTES
        || mask.surface.default_bytes().iter().any(|byte| *byte != 0)
        || mask.surface.tiles().any(|(_, tile)| tile.bytes().iter().any(|byte| *byte != 0))
    {
        return Err(CompositionError::Unsupported("unqualified point Replace canvas selection mask"));
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Mask(id));
    let item = items.next().ok_or(CompositionError::Invalid("point Replace canvas mask registry missing"))?;
    let metadata = item.mask.as_ref().ok_or(CompositionError::Invalid("point Replace canvas mask metadata missing"))?;
    if items.next().is_some()
        || !item.visible
        || metadata.owner != ItemRef::Layer(owner)
        || metadata.bounds != canvas
        || metadata.offset != [0, 0]
        || metadata.outside_f64_bits != 0f64.to_bits()
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(CompositionError::Invalid("point Replace canvas mask relationship"));
    }
    consume_empty_selection_state(state, node, owner, mask.bounds, mask.surface.format())
}
/// The witnessed offset Exposure mask is canvas-relative, not drawable-relative.
/// The caller separately qualifies the document canvas, owner extent and sample
/// depth. This checks the exact saved zero-selection mask without allocating a
/// contiguous copy or inferring an absent source offset from the layer offset.
fn qualify_exposure_canvas_mask(state: &photocraft_doc::XcfState, node: &FilterNode, owner: photocraft_doc::LayerId) -> Result<()> {
    use photocraft_doc::item_identity::ItemRef;
    let Some(ItemRef::Mask(id)) = node.mask else {
        return Err(CompositionError::Unsupported("Exposure geometry requires its saved canvas mask"));
    };
    let mut masks = state.filter_masks.iter().filter(|mask| mask.id == id);
    let mask = masks.next().ok_or(CompositionError::Invalid("Exposure canvas mask missing"))?;
    let format = mask.surface.format();
    if masks.next().is_some()
        || node.operation.name != "gegl:exposure"
        || node.source_region != Some(0)
        || mask.owner != node.id
        || mask.bounds != Rect::from_xywh(0, 0, 295, 160)
        || format.mode != photocraft_color::ColorMode::Grayscale
        || format.alpha
        || !matches!(format.sample, photocraft_color::SampleType::U16 | photocraft_color::SampleType::F32 | photocraft_color::SampleType::F64)
        // Two complete 256Ãƒâ€šÃ‚Â² tiles plus a default sample, bounded before scanning.
        || mask.surface.resident_payload_bytes()? > 2 * 65536 * 8 + 8
        || mask.surface.tiles().any(|(coord, _)| !matches!(coord.tx, 0 | 1) || coord.ty != 0)
        || mask.surface.default_bytes().iter().any(|byte| *byte != 0)
        || mask.surface.tiles().any(|(_, tile)| tile.bytes().iter().any(|byte| *byte != 0))
    {
        return Err(CompositionError::Unsupported("unqualified Exposure canvas selection mask"));
    }
    let mut items = state.items.iter().filter(|item| item.item == ItemRef::Mask(id));
    let item = items.next().ok_or(CompositionError::Invalid("Exposure canvas mask registry missing"))?;
    let metadata = item.mask.as_ref().ok_or(CompositionError::Invalid("Exposure canvas mask metadata missing"))?;
    if items.next().is_some()
        || !item.visible
        || metadata.owner != ItemRef::Layer(owner)
        || metadata.bounds != mask.bounds
        || metadata.offset != [0, 0]
        || metadata.outside_f64_bits != 0f64.to_bits()
        || metadata.shown
        || metadata.editing
        || metadata.linked
    {
        return Err(CompositionError::Invalid("Exposure canvas mask relationship"));
    }
    consume_empty_selection_state(state, node, owner, mask.bounds, mask.surface.format())
}
fn consume_empty_selection_state(state: &photocraft_doc::XcfState, node: &FilterNode, owner: photocraft_doc::LayerId, bounds: Rect, format: PixelFormat) -> Result<()> {
    let prepared = crate::gimp_filter_mask_state::prepare_empty_selection(
        state, node, owner, bounds, format,
        crate::gimp_filter_mask_state::MaskStateLimits { resident_bytes: CACHE_BYTES, work_units: CACHE_BYTES },
        &Interrupt::NONE,
    )?;
    if !prepared.permits_unrestricted_support() {
        return Err(CompositionError::Unsupported("filter mask support disposition"));
    }
    Ok(())
}
fn map_pixels(
    source: &Surface,
    roi: Rect,
    poll: &mut impl FnMut(WideWork) -> Result<()>,
    interrupt: &Interrupt<'_>,
    mut f: impl FnMut([f64; 4]) -> Result<[f64; 4]>,
) -> Result<Surface> {
    map_pixels_at(source, roi, poll, interrupt, |_, pixel| f(pixel))
}
fn map_pixels_at(
    source: &Surface,
    roi: Rect,
    poll: &mut impl FnMut(WideWork) -> Result<()>,
    interrupt: &Interrupt<'_>,
    mut f: impl FnMut((i32, i32), [f64; 4]) -> Result<[f64; 4]>,
) -> Result<Surface> {
    let mut out = Surface::new(PixelFormat::RGBA64F);
    let count =
        u64::from(roi.width()).checked_mul(u64::from(roi.height())).and_then(|v| v.checked_mul(4)).ok_or(CompositionError::Invalid("filter scratch size"))?;
    if count.checked_mul(8).ok_or(CompositionError::Invalid("filter scratch size"))? > CACHE_BYTES {
        return Err(CompositionError::Invalid("filter scratch cap"));
    }
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(usize::try_from(count).map_err(|_| CompositionError::Invalid("filter scratch size"))?)
        .map_err(|_| CompositionError::Invalid("filter scratch allocation denied"))?;
    for y in roi.y0..roi.y1 {
        poll(WideWork::SampleUnits(u64::from(roi.width()) * 128))?;
        if interrupt.cancelled() {
            return Err(photocraft_raster::RasterInputError::Cancelled.into());
        }
        for x in roi.x0..roi.x1 {
            let mut p = [0.0; 4];
            let format = source.format();
            source.try_read_pixel_f64(x, y, p.get_mut(..format.channels()).ok_or(CompositionError::Unsupported("neighborhood source channels"))?)?;
            if format.mode == photocraft_color::ColorMode::Grayscale {
                p = [p[0], p[0], p[0], if format.alpha { p[1] } else { 1. }];
            } else if format.mode == photocraft_color::ColorMode::Rgb {
                if !format.alpha {
                    p[3] = 1.;
                }
            } else {
                return Err(CompositionError::Unsupported("neighborhood source color model"));
            }
            samples.extend_from_slice(&f((x, y), p)?);
        }
    }
    out.try_write_region_f64_with(roi, &samples, interrupt)?;
    Ok(out)
}

/// Cache belongs to one immutable renderer invocation; it is never document state.
pub(crate) struct TileCache {
    entries: Vec<(photocraft_doc::LayerId, TileCoord, Surface)>,
    reserved: bool,
}
impl TileCache {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new(), reserved: false }
    }
    pub(crate) fn sample(
        &mut self,
        plan: &OrderedStack,
        source: &Surface,
        id: photocraft_doc::LayerId,
        position: (i32, i32),
        poll: &mut impl FnMut(WideWork) -> Result<()>,
        interrupt: &Interrupt<'_>,
    ) -> Result<[f64; 4]> {
        let (x, y) = position;
        if !plan.output_extent.contains(x, y) {
            let mut p = [0.0; 4];
            source.try_read_pixel_f64(x, y, &mut p)?;
            return Ok(p);
        }
        let tile = TileCoord::containing(x, y);
        if let Some((_, _, surface)) = self.entries.iter().find(|(owner, key, _)| *owner == id && *key == tile) {
            let mut p = [0.0; 4];
            surface.try_read_pixel_f64(x, y, &mut p)?;
            return Ok(p);
        }
        if !self.reserved {
            poll(WideWork::ReserveBytes(JOB_BYTES))?;
            self.reserved = true;
            self.entries.try_reserve_exact(7).map_err(|_| CompositionError::Invalid("filter cache allocation denied"))?;
        }
        if self.entries.len() == 7 {
            self.entries.remove(0);
        }
        let surface = plan.evaluate(source, tile.rect(), poll, interrupt)?;
        if surface.resident_payload_bytes()? > 2 * 1024 * 1024 + 32 {
            return Err(CompositionError::Invalid("filter tile cache cap"));
        }
        let mut p = [0.0; 4];
        surface.try_read_pixel_f64(x, y, &mut p)?;
        self.entries.push((id, tile, surface));
        Ok(p)
    }
    pub(crate) fn finish(&mut self, poll: &mut impl FnMut(WideWork) -> Result<()>) -> Result<()> {
        self.entries.clear();
        if self.reserved {
            self.reserved = false;
            poll(WideWork::ReleaseBytes(JOB_BYTES))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod gray_scratch_tests {
    use super::*;
    #[test]
    fn u8_point_storage_rounds_before_transfer_without_rounding_alpha() {
        let wide = OrderedStack {
            stages: vec![],
            extent: Rect::EMPTY,
            output_extent: Rect::EMPTY,
            spaces: vec![],
            integer_point_storage: None,
            current_point_sample: None,
            filter_output_precision: photocraft_doc::FilterOutputPrecision::Canonical,
        };
        // A Levels result this small must become zero in the U8 buffer before
        // nonlinear display conversion. Coverage remains the original sample.
        let pixel = [0.00065, 0.5, 1.25, 0.123456789];
        assert_eq!(wide.stored_point_result(pixel, false), pixel);
        let narrow = wide.with_current_point_storage(Some(photocraft_color::SampleType::U8));
        assert_eq!(narrow.stored_point_result(pixel, false), [0.0, 128.0 / 255.0, 1.0, pixel[3]]);
        let hidden = narrow.stored_point_result([0.75, 0.25, 0.5, -0.0], false);
        assert_eq!(hidden[0], 191.0 / 255.0);
        assert_eq!(hidden[3].to_bits(), (-0.0_f64).to_bits());
    }
    #[test]
    fn u32_point_storage_clamps_and_rounds_current_codes_without_rounding_alpha() {
        let wide = OrderedStack {
            stages: vec![],
            extent: Rect::EMPTY,
            output_extent: Rect::EMPTY,
            spaces: vec![],
            integer_point_storage: None,
            current_point_sample: None,
            filter_output_precision: photocraft_doc::FilterOutputPrecision::Canonical,
        };
        let input = [-0.125, 1.125, 0.25 / 4294967295.0, -0.0];
        assert_eq!(wide.stored_point_result(input, false), input);
        let integer = wide.with_current_point_storage(Some(photocraft_color::SampleType::U32));
        let output = integer.stored_point_result(input, false);
        assert_eq!(output[..3], [0.0, 1.0, 0.0]);
        assert_eq!(output[3].to_bits(), input[3].to_bits());
        assert_eq!(integer.stored_point_result([0.5, 0.5, 0.5, 0.75], false)[0], 2147483648.0 / 4294967295.0);
        // Original Half precision is inert provenance: current F32 remains wide.
        let current_float = integer.with_current_point_storage(Some(photocraft_color::SampleType::F32));
        assert_eq!(current_float.stored_point_result(input, false), input);
    }
    #[test]
    fn gray_alpha_is_coverage_and_gray_without_alpha_is_opaque() {
        for alpha in [true, false] {
            let format = PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F64, alpha);
            let raw: Vec<u8> = if alpha { vec![-0.25f64, 0.5] } else { vec![-0.25f64] }.into_iter().flat_map(f64::to_ne_bytes).collect();
            let source = Surface::try_with_default_bytes(format, &raw).unwrap();
            let before = source.clone();
            let scratch = map_pixels(&source, Rect::new(-1, -1, 2, 2), &mut |_| Ok(()), &Interrupt::NONE, Ok).unwrap();
            let mut actual = [0.; 4];
            scratch.try_read_pixel_f64(-1, -1, &mut actual).unwrap();
            assert_eq!(actual, [-0.25, -0.25, -0.25, if alpha { 0.5 } else { 1. }]);
            assert_eq!(source, before);
        }
    }
}

#[cfg(test)]
mod exposure_canvas_mask_tests {
    use super::*;
    use photocraft_doc::{
        FilterMask, XcfState,
        item_identity::{FilterId, ItemRef, MaskId},
        xcf_filters::OperationIdentity,
        xcf_state::{ItemLocks, ItemState, MaskState},
    };
    #[test]
    fn canvas_mask_is_independent_of_offset_extent_and_rejects_changed_bits_and_relations() {
        let owner = photocraft_doc::LayerId(100);
        let id = MaskId(3);
        let node = FilterNode {
            id: FilterId(2),
            operation: OperationIdentity {
                name: "gegl:exposure".into(),
                stored_version: "1:0".into(),
                schema_epoch: crate::gimp_filters::EXPOSURE_GEOMETRY_EPOCH.into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(id)),
            auxiliary: vec![],
            composition: None,
        };
        let bounds = Rect::from_xywh(0, 0, 295, 160);
        for sample in [photocraft_color::SampleType::U16, photocraft_color::SampleType::F32, photocraft_color::SampleType::F64] {
            let surface = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, sample, false));
            let mut state = XcfState::default();
            state.filter_masks.push(FilterMask::try_new(id, node.id, surface, bounds).unwrap());
            state.items.push(ItemState {
                item: ItemRef::Mask(id),
                name: "own selection".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(bounds),
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(owner),
                    bounds,
                    offset: [0, 0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: false,
                }),
            });
            for extent in [Rect::from_xywh(-13, 13, 257, 129), Rect::from_xywh(17, -11, 257, 129)] {
                qualify_selection_mask(&state, &node, owner, extent).unwrap();
            }
            let mut disabled = state.clone();
            disabled.items[0].mask.as_mut().unwrap().enabled = false;
            qualify_selection_mask(&disabled, &node, owner, Rect::from_xywh(-13, 13, 257, 129)).unwrap();
            disabled.filter_masks[0].surface.try_write_pixel_f64(294, 159, &[0.5]).unwrap();
            assert!(qualify_selection_mask(&disabled, &node, owner, Rect::from_xywh(-13, 13, 257, 129)).is_err());
            let before = state.clone();
            for mutation in 0..7 {
                let mut invalid = state.clone();
                match mutation {
                    0 => invalid.filter_masks[0].surface.try_write_pixel_f64(294, 159, &[0.5]).unwrap(),
                    1 => invalid.filter_masks[0].bounds.x0 = -13,
                    2 => invalid.items[0].mask.as_mut().unwrap().offset = [-13, 13],
                    3 => invalid.items[0].visible = false,
                    4 => invalid.items[0].mask.as_mut().unwrap().editing = true,
                    5 => invalid.filter_masks.push(invalid.filter_masks[0].clone()),
                    _ => invalid.filter_masks[0].owner = FilterId(9),
                }
                assert!(qualify_selection_mask(&invalid, &node, owner, Rect::from_xywh(-13, 13, 257, 129)).is_err(), "{mutation}");
            }
            let mut absent = node.clone();
            absent.mask = None;
            assert!(qualify_selection_mask(&state, &absent, owner, bounds).is_err());
            assert_eq!(state, before);
        }
    }
    #[test]
    fn generic_point_mask_requires_exact_canvas_bits_and_relations() {
        point_mask_samples(&[
            photocraft_color::SampleType::U8,
            photocraft_color::SampleType::U16,
            photocraft_color::SampleType::F32,
            photocraft_color::SampleType::F64,
        ]);
    }
    #[test]
    fn generic_u32_point_mask_requires_exact_canvas_bits_and_relations() {
        point_mask_samples(&[photocraft_color::SampleType::U32]);
    }
    fn point_mask_samples(samples: &[photocraft_color::SampleType]) {
        let owner = photocraft_doc::LayerId(100);
        let id = MaskId(3);
        let node = FilterNode {
            id: FilterId(2),
            operation: OperationIdentity {
                name: "gegl:exposure".into(),
                stored_version: "1:0".into(),
                schema_epoch: crate::gimp_filters::EXPOSURE_BLACK_PRECISION_EPOCH.into(),
            },
            source_display_name: None,
            source_icon: None,
            arguments: vec![],
            enabled: true,
            opacity_f64_bits: 1f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: Some(ItemRef::Mask(id)),
            auxiliary: vec![],
            composition: Some(photocraft_doc::import_compositing::GimpComposition {
                compatibility_epoch: crate::gimp_filters::POINT_REPLACE_COMPATIBILITY_EPOCH.into(),
                source_mode: 62,
                legacy: false,
                stored_blend_space: i32::MIN,
                stored_composite_space: i32::MAX,
                stored_operator: 0x12345678,
                resolved: None,
                pass_through: false,
            }),
        };
        let bounds = Rect::from_xywh(0, 0, 295, 160);
        for &sample in samples {
            let surface = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, sample, false));
            let mut state = XcfState::default();
            state.filter_masks.push(FilterMask::try_new(id, node.id, surface, bounds).unwrap());
            state.items.push(ItemState {
                item: ItemRef::Mask(id),
                name: "own selection".into(),
                custom_name: false,
                visible: true,
                locks: ItemLocks::default(),
                parent: None,
                source_tattoo: None,
                source_flags: 0,
                raster_bounds: Some(bounds),
                composition: None,
                mask: Some(MaskState {
                    owner: ItemRef::Layer(owner),
                    bounds,
                    offset: [0, 0],
                    outside_f64_bits: 0f64.to_bits(),
                    enabled: true,
                    shown: false,
                    editing: false,
                    linked: false,
                }),
            });
            qualify_selection_mask(&state, &node, owner, bounds).unwrap();
            assert!(qualify_selection_mask(&state, &node, owner, Rect::from_xywh(0, 0, 294, 160)).is_err());
            let mut disabled = state.clone();
            disabled.items[0].mask.as_mut().unwrap().enabled = false;
            qualify_selection_mask(&disabled, &node, owner, bounds).unwrap();
            disabled.filter_masks[0].surface.try_write_pixel_f64(294, 159, &[0.5]).unwrap();
            assert!(qualify_selection_mask(&disabled, &node, owner, bounds).is_err());
            let before = state.clone();
            for mutation in 0..7 {
                let mut invalid = state.clone();
                match mutation {
                    0 => invalid.filter_masks[0].surface.try_write_pixel_f64(294, 159, &[0.5]).unwrap(),
                    1 => invalid.filter_masks[0].bounds.x0 = -13,
                    2 => invalid.items[0].mask.as_mut().unwrap().offset = [-13, 13],
                    3 => invalid.items[0].visible = false,
                    4 => invalid.items[0].mask.as_mut().unwrap().editing = true,
                    5 => invalid.filter_masks.push(invalid.filter_masks[0].clone()),
                    _ => invalid.filter_masks[0].owner = FilterId(9),
                }
                assert!(qualify_selection_mask(&invalid, &node, owner, bounds).is_err(), "{mutation}");
            }
            let mut absent = node.clone();
            absent.mask = None;
            assert!(qualify_selection_mask(&state, &absent, owner, bounds).is_err());
            assert_eq!(state, before);
        }
    }
}
