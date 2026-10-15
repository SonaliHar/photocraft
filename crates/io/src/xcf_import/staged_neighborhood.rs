//! Existing neighborhood source domains; no Point evaluator authority.
use super::*;
use crate::xcf_filters::NeighborhoodDomain;
#[path = "staged_neighborhood/original_gaussian.rs"]
mod original_gaussian;
#[path="staged_neighborhood/assigned_box.rs"]
mod assigned_box;
#[path="staged_neighborhood/assigned_median.rs"]
mod assigned_median;
#[path="staged_neighborhood/assigned_gaussian.rs"]
mod assigned_gaussian;
#[path="staged_neighborhood/box_geometry.rs"]
mod box_geometry;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::xcf_import) enum NeighborhoodSourceFamily {
    BoxGeometryF32,
    BoxGeometryF64,
    BoxGeometryLinearF32,
    BoxGeometryLinearF64,
    BoxGeometryPerceptualF32,
    BoxGeometryPerceptualF64,
    BoxGeometryAssignedF32,
    BoxGeometryAssignedF64,
    BoxOriginal,
    BoxFinite,
    BoxAssignedF32,
    MedianAssignedF32,
    MedianOriginal,
    MedianFinite,
    MedianGray,
    MotionOriginal,
    PixelizeOriginal,
    GaussianAssignedF32,
    RasterFinite,
    OriginalGaussian,
}
impl NeighborhoodSourceFamily {
    pub(super) fn hint<P: ExactParsedPlane>(file: &XcfFile<P>) -> Option<Self> {
        let effect = file.layers.first()?.effects.first()?;
        if effect.arguments.len() > 10 {
            return None;
        }
        let finite = matches!((file.sample, file.precision), (Sample::F32, 650) | (Sample::F64, 750));
        Some(match effect.operation.as_str() {
            "gegl:box-blur" if box_geometry::hint(file) && file.encoding==Encoding::Perceptual => if file.sample==Sample::F64{Self::BoxGeometryPerceptualF64}else{Self::BoxGeometryPerceptualF32},
            "gegl:box-blur" if box_geometry::hint(file) && file.encoding==Encoding::Linear => if file.sample==Sample::F64{Self::BoxGeometryLinearF64}else{Self::BoxGeometryLinearF32},
            "gegl:box-blur" if box_geometry::hint(file) => if !file.icc.is_empty(){if file.sample==Sample::F64{Self::BoxGeometryAssignedF64}else{Self::BoxGeometryAssignedF32}}else if file.sample==Sample::F64{Self::BoxGeometryF64}else{Self::BoxGeometryF32},
            "gegl:box-blur" if file.sample==Sample::F32 && file.precision==650 && !file.icc.is_empty() => Self::BoxAssignedF32,
            "gegl:box-blur" if finite => Self::BoxFinite,
            "gegl:box-blur" => Self::BoxOriginal,
            "gegl:median-blur" if file.model==Model::Rgb && file.sample==Sample::F32 && file.precision==650 && !file.icc.is_empty() => Self::MedianAssignedF32,
            "gegl:median-blur" if file.model == Model::Gray => Self::MedianGray,
            "gegl:median-blur" if finite => Self::MedianFinite,
            "gegl:median-blur" => Self::MedianOriginal,
            "gegl:motion-blur-linear" => Self::MotionOriginal,
            "gegl:pixelize" => Self::PixelizeOriginal,
            "gegl:gaussian-blur" if file.model==Model::Rgb && file.sample==Sample::F32 && file.precision==650 && !file.icc.is_empty() => Self::GaussianAssignedF32,
            "gegl:gaussian-blur" if finite => Self::RasterFinite,
            // Routing only. The separate exact legacy predicate must pass;
            // this does not enroll arbitrary Auto, geometry, alpha or profiles.
            "gegl:gaussian-blur" if file.model == Model::Rgb && file.sample == Sample::U8 && file.precision == 150 && (file.width, file.height) == (96, 80) => {
                Self::OriginalGaussian
            }
            _ => return None,
        })
    }
    pub(super) fn domain(self) -> NeighborhoodDomain {
        match self {
            Self::BoxGeometryF32 | Self::BoxGeometryF64 | Self::BoxGeometryLinearF32 | Self::BoxGeometryLinearF64 | Self::BoxGeometryPerceptualF32 | Self::BoxGeometryPerceptualF64 | Self::BoxGeometryAssignedF32 | Self::BoxGeometryAssignedF64 => NeighborhoodDomain::BoxGeometry,
            Self::BoxOriginal => NeighborhoodDomain::BoxOriginal,
            Self::BoxFinite | Self::BoxAssignedF32 => NeighborhoodDomain::BoxFinite,
            Self::MedianOriginal => NeighborhoodDomain::MedianOriginal,
            Self::MedianFinite | Self::MedianAssignedF32 => NeighborhoodDomain::MedianFinite,
            Self::MedianGray => NeighborhoodDomain::MedianGray,
            Self::MotionOriginal => NeighborhoodDomain::MotionOriginal,
            Self::PixelizeOriginal => NeighborhoodDomain::PixelizeOriginal,
            Self::RasterFinite | Self::GaussianAssignedF32 => NeighborhoodDomain::RasterFinite,
            Self::OriginalGaussian => NeighborhoodDomain::RasterOriginal,
        }
    }
    pub(super) fn set_admission(self, domains: &mut admission::AdmissionDomains) {
        domains.owned_neighborhood_domain = Some(self.domain());
        match self {
            Self::BoxGeometryF32 | Self::BoxGeometryF64 | Self::BoxGeometryLinearF32 | Self::BoxGeometryLinearF64 | Self::BoxGeometryPerceptualF32 | Self::BoxGeometryPerceptualF64 | Self::BoxGeometryAssignedF32 | Self::BoxGeometryAssignedF64 => {domains.finite_box_domain=true;domains.observed_box_geometry_domain=true;},
            Self::BoxOriginal => domains.opaque_box_domain = true,
            Self::BoxFinite => domains.finite_box_domain = true,
            Self::BoxAssignedF32 => { domains.finite_box_domain=true; domains.owned_assigned_box_domain=true; },
            Self::MedianOriginal => domains.opaque_median_domain = true,
            Self::MedianFinite => domains.finite_median_domain = true,
            Self::MedianAssignedF32 => { domains.finite_median_domain=true; domains.owned_assigned_median_domain=true; },
            Self::MedianGray => domains.gray_median_domain = true,
            Self::MotionOriginal => domains.opaque_motion_domain = true,
            Self::PixelizeOriginal => domains.opaque_pixelize_domain = true,
            Self::RasterFinite => domains.finite_gaussian_domain = true,
            Self::GaussianAssignedF32 => {domains.finite_gaussian_domain=true;domains.owned_assigned_gaussian_domain=true;},
            Self::OriginalGaussian => {}
        }
    }
    pub(super) fn qualify<P: ExactParsedPlane>(
        self,
        file: &XcfFile<P>,
        ctl: &Interrupt<'_>,
        budget: Option<&photocraft_xcf::Budget<'_>>,
    ) -> Result<bool, IoError> {
        if Self::hint(file) != Some(self) {
            return Ok(false);
        }
        match self {
            Self::BoxGeometryF32 | Self::BoxGeometryF64 | Self::BoxGeometryLinearF32 | Self::BoxGeometryLinearF64 | Self::BoxGeometryPerceptualF32 | Self::BoxGeometryPerceptualF64 | Self::BoxGeometryAssignedF32 | Self::BoxGeometryAssignedF64 => box_geometry::qualify(file,ctl,budget),
            Self::BoxAssignedF32 => assigned_box::qualify(file,ctl,budget),
            Self::MedianAssignedF32 => assigned_median::qualify(file,ctl,budget),
            Self::GaussianAssignedF32 => assigned_gaussian::qualify(file,ctl,budget),
            Self::BoxOriginal => residual::opaque_original(file, "gegl:box-blur", ctl, budget),
            Self::MedianOriginal => residual::opaque_original(file, "gegl:median-blur", ctl, budget),
            Self::MotionOriginal => residual::opaque_original(file, "gegl:motion-blur-linear", ctl, budget),
            Self::PixelizeOriginal => residual::opaque_original(file, "gegl:pixelize", ctl, budget),
            Self::BoxFinite => finite(file, "gegl:box-blur", Model::Rgb, ctl, budget),
            Self::MedianFinite => finite(file, "gegl:median-blur", Model::Rgb, ctl, budget),
            Self::MedianGray => finite(file, "gegl:median-blur", Model::Gray, ctl, budget),
            Self::RasterFinite => finite(file, "gegl:gaussian-blur", Model::Rgb, ctl, budget),
            Self::OriginalGaussian => original_gaussian::qualify(file, ctl, budget),
        }
    }
}
fn poll(ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget.consume_work(units, "neighborhood original source").map_err(accounting_error)?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
// Same metadata and native predicate as qualified_finite_raster_filter. The
// existing structural owner supplies the narrower implicit-original-ICC lane.
fn finite<P: ExactParsedPlane>(
    file: &XcfFile<P>,
    operation: &str,
    model: Model,
    ctl: &Interrupt<'_>,
    budget: Option<&photocraft_xcf::Budget<'_>>,
) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if !(file.version == 22
        && file.model == model
        && matches!((file.sample, file.precision), (Sample::F32, 650) | (Sample::F64, 750))
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 16
        && file.height == 12
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1)
    {
        return Ok(false);
    }
    let Some(d) = file.layers.first() else { return Ok(false) };
    if !(!d.group
        && d.path.len() <= 1
        && d.visible
        && d.alpha
        && d.model == model
        && d.width == 16
        && d.height == 12
        && d.offset == (0, 0)
        && d.mode == 28
        && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
        && d.opacity.to_bits() == 1.0f32.to_bits()
        && d.mask.is_none()
        && d.text.is_none()
        && d.vector.is_none()
        && d.link.is_none()
        && d.transform.is_none()
        && d.floating_attachment.is_none()
        && d.effects.len() == 1
        && d.effects.first().is_some_and(|e| e.operation == operation))
    {
        return Ok(false);
    }
    let Some(plane) = d.plane.as_ref() else { return Ok(false) };
    let width = if file.sample == Sample::F64 { 8 } else { 4 };
    let channels = if model == Model::Gray { 2 } else { 4 };
    let expected = 16usize * 12 * channels * width;
    let descriptor = plane.descriptor();
    if descriptor.sample != file.sample || descriptor.channels != channels || plane.native_byte_len() != Some(expected as u64) {
        return Ok(false);
    }
    let mut seen = 0usize;
    let mut accepted = true;
    plane.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > 64 * channels * width || bytes.len() % (channels * width) != 0 {
            return Err(IoError::Unsupported("neighborhood native block".into()));
        }
        seen = seen.checked_add(bytes.len()).filter(|n| *n <= expected).ok_or_else(|| IoError::Unsupported("neighborhood extra native bytes".into()))?;
        poll(ctl, budget, bytes.len() as u64)?;
        for (index, bytes) in bytes.chunks_exact(width).enumerate() {
            let value = if width == 8 {
                f64::from_ne_bytes(bytes.try_into().map_err(|_| IoError::Unsupported("neighborhood F64 word".into()))?)
            } else {
                f64::from(f32::from_ne_bytes(bytes.try_into().map_err(|_| IoError::Unsupported("neighborhood F32 word".into()))?))
            };
            accepted &= value.is_finite() && (index % channels != channels - 1 || (0.0..=1.0).contains(&value));
        }
        Ok(())
    })?;
    if seen != expected {
        return Err(IoError::Unsupported("neighborhood incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(accepted)
}
#[cfg(test)]
#[path = "staged_neighborhood_tests.rs"]
mod tests;

#[cfg(all(test, feature = "corpus"))]
#[path = "staged_neighborhood_disabled_tests.rs"]
mod disabled_tests;
