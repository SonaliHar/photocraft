//! Allocation-free conservative input requirements for explicit native samplers.
//! Plans describe logical coverage, not source/profile/operation qualification.
//! No surface, displacement-field scan, external read or render is performed.
use super::{
    geometry_displace_nearest::{FieldCoordinates, FieldParams, VectorUnits},
    geometry_nearest::{Abyss, Affine, Params, Ties},
    wide_point::WorkingContext,
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, RasterInputError, checked_region_byte_len};

#[derive(Clone, Copy, Debug)]
pub struct SourceMetadata {
    pub format: PixelFormat,
    pub extent: Rect,
}
#[derive(Clone, Copy, Debug)]
pub struct FieldMetadata {
    pub format: PixelFormat,
    pub extent: Rect,
}
/// Caller-proven inclusive bounds of the two selected numeric field channels
/// throughout the requested in-field output grid. No zero/default is assumed.
/// This declaration does not prove its relationship to any actual field bytes.
#[derive(Clone, Copy, Debug)]
pub struct ProvenVectorBounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UncertaintyPolicy {
    Reject,
    RequestFullSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRequirement {
    None,
    Rectangle(Rect),
    /// Finite declared vector bounds could not be evaluated/rounded within the
    /// sampler's finite binary64/i32 domain. This permits conservative fetching,
    /// not execution: the sampler still validates every actual coordinate.
    FullExtentForUncertainDisplacement(Rect),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplacementPlan {
    pub source: SourceRequirement,
    /// Output-grid intersection with the field's own extent; outside it uses
    /// the explicitly declared outside vector and needs no field samples.
    pub field: Option<Rect>,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub work_units: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Raster(RasterInputError),
    Coordinate,
    FieldFormat,
    FieldContext,
    VectorBounds,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native sampling ROI: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<RasterInputError> for Error {
    fn from(error: RasterInputError) -> Self {
        Self::Raster(error)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn bounds() -> Error {
    RasterInputError::InvalidBounds.into()
}
fn finite(value: f64) -> Result<f64> {
    if value.is_finite() { Ok(value) } else { Err(Error::Coordinate) }
}
struct Meter<'a, 'interrupt> {
    remaining: u64,
    control: &'a Interrupt<'interrupt>,
}
impl Meter<'_, '_> {
    fn charge(&mut self, work: u64) -> Result<()> {
        self.control.check().map_err(|_| RasterInputError::Cancelled)?;
        self.remaining = self.remaining.checked_sub(work).ok_or(RasterInputError::LimitExceeded)?;
        Ok(())
    }
}
fn validate(source: SourceMetadata, roi: Rect, params: Params, meter: &mut Meter<'_, '_>) -> Result<()> {
    meter.charge(8)?;
    if source.extent.is_empty() || source.extent.x1 < source.extent.x0 || source.extent.y1 < source.extent.y0 || roi.x1 < roi.x0 || roi.y1 < roi.y0 {
        return Err(bounds());
    }
    checked_region_byte_len(source.extent, source.format)?;
    checked_region_byte_len(roi, source.format)?;
    let bpp = source.format.bytes_per_pixel();
    if usize::from(params.output_default.length) != bpp || bpp > params.output_default.bytes.len() {
        return Err(RasterInputError::LengthMismatch.into());
    }
    if let Abyss::ConstantNative(pixel) = params.abyss
        && (usize::from(pixel.length) != bpp || bpp > pixel.bytes.len())
    {
        return Err(RasterInputError::LengthMismatch.into());
    }
    for value in [params.map.xx, params.map.xy, params.map.yx, params.map.yy, params.map.tx, params.map.ty] {
        finite(value)?;
    }
    Ok(())
}
fn mapped(map: Affine, x: i32, y: i32) -> Result<[f64; 2]> {
    let x = f64::from(x) + 0.5;
    let y = f64::from(y) + 0.5;
    let xx = finite(map.xx * x)?;
    let xy = finite(map.xy * y)?;
    let sx0 = finite(xx + xy)?;
    let sx = finite(sx0 + map.tx)?;
    let yx = finite(map.yx * x)?;
    let yy = finite(map.yy * y)?;
    let sy0 = finite(yx + yy)?;
    let sy = finite(sy0 + map.ty)?;
    Ok([sx, sy])
}
fn center_extrema(roi: Rect, map: Affine, meter: &mut Meter<'_, '_>) -> Result<ProvenVectorBounds> {
    if roi.is_empty() {
        return Err(bounds());
    }
    let last_x = roi.x1.checked_sub(1).ok_or_else(bounds)?;
    let last_y = roi.y1.checked_sub(1).ok_or_else(bounds)?;
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for (x, y) in [(roi.x0, roi.y0), (last_x, roi.y0), (roi.x0, last_y), (last_x, last_y)] {
        meter.charge(16)?;
        let value = mapped(map, x, y)?;
        for axis in 0..2 {
            min[axis] = min[axis].min(value[axis]);
            max[axis] = max[axis].max(value[axis]);
        }
    }
    // Separate finite binary64 multiply/add operations are monotone along each
    // axis for fixed coefficients. Corner extrema therefore include interior
    // computed coordinates, including cancellation/rounding in these stages.
    Ok(ProvenVectorBounds { min, max })
}
fn nearest(value: f64, ties: Ties) -> Result<i32> {
    let shifted = finite(value - 0.5)?;
    let lower = shifted.floor();
    let fraction = finite(shifted - lower)?;
    let upper = fraction > 0.5
        || (fraction == 0.5
            && match ties {
                Ties::Lower => false,
                Ties::Upper => true,
                Ties::ToEven => lower.rem_euclid(2.0) != 0.0,
            });
    let selected = finite(if upper { lower + 1.0 } else { lower })?;
    if selected < f64::from(i32::MIN) || selected > f64::from(i32::MAX) {
        return Err(Error::Coordinate);
    }
    Ok(selected as i32)
}
fn requirement(coordinates: ProvenVectorBounds, ties: Ties, extent: Rect, abyss: Abyss, meter: &mut Meter<'_, '_>) -> Result<SourceRequirement> {
    meter.charge(8)?;
    let mut min = [nearest(coordinates.min[0], ties)?, nearest(coordinates.min[1], ties)?];
    let mut max = [nearest(coordinates.max[0], ties)?, nearest(coordinates.max[1], ties)?];
    let last_x = extent.x1.checked_sub(1).ok_or_else(bounds)?;
    let last_y = extent.y1.checked_sub(1).ok_or_else(bounds)?;
    match abyss {
        Abyss::Clamp => {
            min[0] = min[0].clamp(extent.x0, last_x);
            max[0] = max[0].clamp(extent.x0, last_x);
            min[1] = min[1].clamp(extent.y0, last_y);
            max[1] = max[1].clamp(extent.y0, last_y);
        }
        Abyss::ConstantNative(_) => {
            min[0] = min[0].max(extent.x0);
            max[0] = max[0].min(last_x);
            min[1] = min[1].max(extent.y0);
            max[1] = max[1].min(last_y);
            if min[0] > max[0] || min[1] > max[1] {
                return Ok(SourceRequirement::None);
            }
        }
    }
    // Construct AFTER clipping/clamping, so inclusive i32::MAX map extrema
    // never require overflowing a half-open rectangle outside declared input.
    Ok(SourceRequirement::Rectangle(Rect::new(min[0], min[1], max[0].checked_add(1).ok_or_else(bounds)?, max[1].checked_add(1).ok_or_else(bounds)?)))
}

/// Conservative source rectangle for nearest sampling. It bounds all interior
/// taps, rather than merely fetching corners: an affine rectangle can cross the
/// input even when none of its four mapped corners lie inside the input.
/// Clamp includes edge taps; constant abyss excludes outside native reads.
pub fn plan_nearest(source: SourceMetadata, output_roi: Rect, params: Params, limits: Limits, control: &Interrupt<'_>) -> Result<SourceRequirement> {
    let mut meter = Meter { remaining: limits.work_units, control };
    validate(source, output_roi, params, &mut meter)?;
    if output_roi.is_empty() {
        meter.charge(1)?;
        return Ok(SourceRequirement::None);
    }
    let extrema = center_extrema(output_roi, params.map, &mut meter)?;
    let required = requirement(extrema, params.ties, source.extent, params.abyss, &mut meter)?;
    meter.charge(1)?;
    Ok(required)
}

/// Uses caller-proven field bounds, never reads/rescans the displacement field.
/// Bounds refer to selected numeric channels BEFORE scale. Outside vectors are
/// included if output coverage extends outside the field's declared extent.
/// Source format is raw-copy metadata; field context equality is an explicit
/// interpretation assertion, not a profile/CMS/GEGL admission proof.
// Keep the fixed math/authority inputs explicit; regrouping them changes the established API.
#[allow(clippy::too_many_arguments)]
pub fn plan_displacement(
    source: SourceMetadata,
    field: FieldMetadata,
    output_roi: Rect,
    params: Params,
    field_params: FieldParams,
    expected_field_context: WorkingContext,
    proven: ProvenVectorBounds,
    uncertainty: UncertaintyPolicy,
    limits: Limits,
    control: &Interrupt<'_>,
) -> Result<DisplacementPlan> {
    let mut meter = Meter { remaining: limits.work_units, control };
    validate(source, output_roi, params, &mut meter)?;
    meter.charge(8)?;
    if field.extent.is_empty() || field.extent.x1 < field.extent.x0 || field.extent.y1 < field.extent.y0 {
        return Err(bounds());
    }
    checked_region_byte_len(field.extent, field.format)?;
    if !matches!(field.format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(field.format.sample, SampleType::F32 | SampleType::F64)
        || field_params.channels.iter().any(|channel| *channel >= field.format.channels())
    {
        return Err(Error::FieldFormat);
    }
    if field_params.context != expected_field_context {
        return Err(Error::FieldContext);
    }
    match (field_params.coordinates, field_params.units) {
        (FieldCoordinates::OutputPixelGrid, VectorUnits::SourcePixels) => {}
    }
    for axis in 0..2 {
        finite(field_params.scales[axis])?;
        finite(field_params.outside[axis])?;
        if !proven.min[axis].is_finite() || !proven.max[axis].is_finite() || proven.min[axis] > proven.max[axis] {
            return Err(Error::VectorBounds);
        }
    }
    if output_roi.is_empty() {
        meter.charge(1)?;
        return Ok(DisplacementPlan { source: SourceRequirement::None, field: None });
    }
    let intersection = field.extent.intersect(&output_roi);
    let field_roi = if intersection.is_empty() { None } else { Some(intersection) };
    let mut vectors = if field_roi.is_some() { proven } else { ProvenVectorBounds { min: field_params.outside, max: field_params.outside } };
    if !field.extent.contains_rect(&output_roi) {
        for axis in 0..2 {
            vectors.min[axis] = vectors.min[axis].min(field_params.outside[axis]);
            vectors.max[axis] = vectors.max[axis].max(field_params.outside[axis]);
        }
    }
    let mut extrema = center_extrema(output_roi, params.map, &mut meter)?;
    meter.charge(16)?;
    let arithmetic = (|| -> Result<ProvenVectorBounds> {
        for axis in 0..2 {
            let a = finite(vectors.min[axis] * field_params.scales[axis])?;
            let b = finite(vectors.max[axis] * field_params.scales[axis])?;
            extrema.min[axis] = finite(extrema.min[axis] + a.min(b))?;
            extrema.max[axis] = finite(extrema.max[axis] + a.max(b))?;
        }
        Ok(extrema)
    })();
    let required = match arithmetic {
        Ok(extrema) => requirement(extrema, params.ties, source.extent, params.abyss, &mut meter),
        Err(error) => Err(error),
    };
    let source_requirement = match required {
        Ok(required) => required,
        Err(Error::Coordinate) if uncertainty == UncertaintyPolicy::RequestFullSource => SourceRequirement::FullExtentForUncertainDisplacement(source.extent),
        Err(error) => return Err(error),
    };
    meter.charge(1)?;
    Ok(DisplacementPlan { source: source_requirement, field: field_roi })
}

// Unrun exhaustive small-coordinate planner drafts; no stock/source admission.
#[cfg(test)]
mod tests {
    use super::super::geometry_nearest::NativePixel;
    use super::*;
    use photocraft_doc::SampleEncoding;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn source() -> SourceMetadata {
        SourceMetadata { format: PixelFormat::new(ColorMode::Rgb, SampleType::U32, true), extent: Rect::new(-2, -2, 3, 3) }
    }
    fn params() -> Params {
        Params {
            map: Affine { xx: 1.0, xy: 0.0, yx: 0.0, yy: 1.0, tx: 0.0, ty: 0.0 },
            ties: Ties::Lower,
            abyss: Abyss::Clamp,
            output_default: NativePixel::from_slice(&[0; 16]).unwrap(),
        }
    }
    fn limits() -> Limits {
        Limits { work_units: 512 }
    }
    fn context() -> WorkingContext {
        WorkingContext { encoding: SampleEncoding::Linear, profile_identity: [21; 32] }
    }
    fn field_params() -> FieldParams {
        FieldParams {
            channels: [0, 0],
            scales: [1.0, -2.0],
            outside: [0.75, -0.25],
            coordinates: FieldCoordinates::OutputPixelGrid,
            units: VectorUnits::SourcePixels,
            context: context(),
        }
    }
    fn covers(required: SourceRequirement, x: i32, y: i32) -> bool {
        match required {
            SourceRequirement::None => false,
            SourceRequirement::Rectangle(rect) | SourceRequirement::FullExtentForUncertainDisplacement(rect) => rect.contains(x, y),
        }
    }
    fn tap(source: SourceMetadata, params: Params, coordinate: [f64; 2]) -> Option<(i32, i32)> {
        let x = nearest(coordinate[0], params.ties).unwrap();
        let y = nearest(coordinate[1], params.ties).unwrap();
        match params.abyss {
            Abyss::Clamp => Some((x.clamp(source.extent.x0, source.extent.x1 - 1), y.clamp(source.extent.y0, source.extent.y1 - 1))),
            Abyss::ConstantNative(_) => source.extent.contains(x, y).then_some((x, y)),
        }
    }
    #[test]
    fn corner_extrema_cover_exhaustive_small_affine_taps_and_all_ties() {
        let s = source();
        let roi = Rect::new(-5, -4, 6, 5);
        for map in [
            params().map,
            Affine { xx: -1.0, xy: 0.5, yx: 0.25, yy: -1.0, tx: 0.5, ty: -0.5 },
            Affine { xx: 0.0, xy: 0.0, yx: 0.0, yy: 0.0, tx: 1.0, ty: -1.0 },
        ] {
            for ties in [Ties::Lower, Ties::Upper, Ties::ToEven] {
                for abyss in [Abyss::Clamp, Abyss::ConstantNative(NativePixel::from_slice(&[0; 16]).unwrap())] {
                    let p = Params { map, ties, abyss, ..params() };
                    let plan = plan_nearest(s, roi, p, limits(), &Interrupt::NONE).unwrap();
                    for y in roi.y0..roi.y1 {
                        for x in roi.x0..roi.x1 {
                            if let Some((sx, sy)) = tap(s, p, mapped(map, x, y).unwrap()) {
                                assert!(covers(plan, sx, sy));
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn all_corners_outside_does_not_discard_interior_and_clamp_fetches_edges() {
        let s = source();
        let roi = Rect::new(-5, -5, 6, 6);
        let mut p = params();
        p.abyss = Abyss::ConstantNative(NativePixel::from_slice(&[0; 16]).unwrap());
        assert_eq!(plan_nearest(s, roi, p, limits(), &Interrupt::NONE).unwrap(), SourceRequirement::Rectangle(s.extent));
        p.map.tx = 100.0;
        assert_eq!(plan_nearest(s, roi, p, limits(), &Interrupt::NONE).unwrap(), SourceRequirement::None);
        p.abyss = Abyss::Clamp;
        let plan = plan_nearest(s, roi, p, limits(), &Interrupt::NONE).unwrap();
        assert!(covers(plan, 2, -2));
        assert!(covers(plan, 2, 2));
        assert!(!covers(plan, 1, 0));
    }
    #[test]
    fn proven_displacement_bounds_cover_signed_scales_and_outside_field_vectors() {
        let s = source();
        let field = FieldMetadata { format: PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), extent: Rect::new(-1, -1, 2, 2) };
        let roi = Rect::new(-4, -3, 4, 3);
        let fp = field_params();
        let vectors = ProvenVectorBounds { min: [-1.0, -0.5], max: [1.25, 0.75] };
        let plan = plan_displacement(s, field, roi, params(), fp, context(), vectors, UncertaintyPolicy::Reject, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(plan.field, Some(field.extent));
        for y in roi.y0..roi.y1 {
            for x in roi.x0..roi.x1 {
                for vector in [vectors.min, vectors.max, [0.0, 0.0]] {
                    let vector = if field.extent.contains(x, y) { vector } else { fp.outside };
                    let mut point = mapped(params().map, x, y).unwrap();
                    for axis in 0..2 {
                        point[axis] += vector[axis] * fp.scales[axis];
                    }
                    let (sx, sy) = tap(s, params(), point).unwrap();
                    assert!(covers(plan.source, sx, sy));
                }
            }
        }
    }
    #[test]
    fn field_absent_empty_and_uncertain_results_are_explicit() {
        let s = source();
        let field = FieldMetadata { format: PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), extent: Rect::new(10, 10, 11, 11) };
        let vectors = ProvenVectorBounds { min: [0.0; 2], max: [0.0; 2] };
        let p = plan_displacement(
            s,
            field,
            Rect::new(0, 0, 1, 1),
            params(),
            field_params(),
            context(),
            vectors,
            UncertaintyPolicy::Reject,
            limits(),
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(p.field, None);
        assert_eq!(plan_nearest(s, Rect::new(0, 0, 0, 0), params(), limits(), &Interrupt::NONE).unwrap(), SourceRequirement::None);
        let huge = ProvenVectorBounds { min: [-f64::MAX; 2], max: [f64::MAX; 2] };
        let mut fp = field_params();
        fp.scales = [2.0; 2];
        let p =
            plan_displacement(s, field, field.extent, params(), fp, context(), huge, UncertaintyPolicy::RequestFullSource, limits(), &Interrupt::NONE).unwrap();
        assert_eq!(p.source, SourceRequirement::FullExtentForUncertainDisplacement(s.extent));
        assert!(plan_displacement(s, field, field.extent, params(), fp, context(), huge, UncertaintyPolicy::Reject, limits(), &Interrupt::NONE).is_err());
    }
    #[test]
    fn invalid_context_bounds_coordinate_limits_and_cancellation_refuse() {
        let s = source();
        let roi = Rect::new(0, 0, 1, 1);
        let field = FieldMetadata { format: PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), extent: roi };
        let bad = ProvenVectorBounds { min: [2.0; 2], max: [1.0; 2] };
        assert!(matches!(
            plan_displacement(s, field, roi, params(), field_params(), context(), bad, UncertaintyPolicy::RequestFullSource, limits(), &Interrupt::NONE),
            Err(Error::VectorBounds)
        ));
        let mut expected = context();
        expected.profile_identity[0] = 0;
        assert!(matches!(
            plan_displacement(
                s,
                field,
                roi,
                params(),
                field_params(),
                expected,
                ProvenVectorBounds { min: [0.0; 2], max: [0.0; 2] },
                UncertaintyPolicy::Reject,
                limits(),
                &Interrupt::NONE
            ),
            Err(Error::FieldContext)
        ));
        let mut p = params();
        p.map.tx = f64::from(i32::MAX) + 2.0;
        assert!(matches!(plan_nearest(s, roi, p, limits(), &Interrupt::NONE), Err(Error::Coordinate)));
        assert!(matches!(plan_nearest(s, roi, params(), Limits { work_units: 1 }, &Interrupt::NONE), Err(Error::Raster(RasterInputError::LimitExceeded))));
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) > 2;
        assert!(matches!(plan_nearest(s, roi, params(), limits(), &Interrupt::cancel_only(&cancel)), Err(Error::Raster(RasterInputError::Cancelled))));
    }
}
