//! Original residual source identities, separate from PointReplace authority.
//! A hint selects one strict validator; it never qualifies storage or retries.
use super::*;
use crate::xcf_filters::NeighborhoodDomain;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::xcf_import) enum ResidualPointFamily {
    Desaturate,
    SaturationRgb,
    SaturationGray,
    SaturationYuv,
    ValueInvertPrecision,
    Posterize,
}
impl ResidualPointFamily {
    pub(super) fn hint<P: ExactParsedPlane>(file: &XcfFile<P>) -> Option<Self> {
        let effect = file.layers.first()?.effects.first()?;
        if effect.arguments.len() > 10 {
            return None;
        }
        Some(match effect.operation.as_str() {
            "gimp:desaturate" => Self::Desaturate,
            "gegl:saturation" if file.model == Model::Gray => Self::SaturationGray,
            "gegl:saturation" if effect.arguments.iter().any(|a| a.name == "colorspace" && a.value == ArgumentValue::Enum(2)) => Self::SaturationYuv,
            "gegl:saturation" => Self::SaturationRgb,
            "gegl:value-invert" => Self::ValueInvertPrecision,
            "gimp:posterize" => Self::Posterize,
            _ => return None,
        })
    }
    pub(super) fn domain(self) -> NeighborhoodDomain {
        match self {
            Self::Desaturate => NeighborhoodDomain::DesaturatePrecision,
            Self::SaturationRgb => NeighborhoodDomain::SaturationPrecision,
            Self::SaturationGray => NeighborhoodDomain::SaturationGrayPrecision,
            Self::SaturationYuv => NeighborhoodDomain::SaturationRgbYuvPrecision,
            Self::ValueInvertPrecision => NeighborhoodDomain::ValueInvertPrecision,
            Self::Posterize => NeighborhoodDomain::PosterizeOriginal,
        }
    }
    pub(super) fn qualify<P: ExactParsedPlane>(
        self,
        file: &XcfFile<P>,
        ctl: &Interrupt<'_>,
        budget: Option<&photocraft_xcf::Budget<'_>>,
    ) -> Result<bool, IoError> {
        // Repeat the bounded hint equality before any source sample scan. It is
        // still the original whole-file qualifier and mapper that grant scope.
        if Self::hint(file) != Some(self) {
            return Ok(false);
        }
        let operation = match self {
            Self::Desaturate => "gimp:desaturate",
            Self::SaturationRgb | Self::SaturationGray | Self::SaturationYuv => "gegl:saturation",
            Self::ValueInvertPrecision => "gegl:value-invert",
            Self::Posterize => return opaque_original(file, "gimp:posterize", ctl, budget),
        };
        if matches!(self, Self::SaturationRgb | Self::SaturationYuv) && file.model != Model::Rgb {
            return Ok(false);
        }
        precision_point_source::qualify_origin_geometry(file, operation, false, ctl, budget)
    }
    pub(super) fn set_late(self, domains: &mut admission::LateAdmissionDomains) {
        match self {
            Self::Desaturate => domains.desaturate_domain = true,
            Self::SaturationRgb => domains.saturation_domain = true,
            Self::SaturationGray => domains.saturation_gray_domain = true,
            Self::SaturationYuv => domains.saturation_yuv_domain = true,
            Self::ValueInvertPrecision => domains.value_precision_domain = true,
            Self::Posterize => domains.posterize_domain = true,
        }
    }
}
fn poll(ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>, units: u64) -> Result<(), IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    if let Some(budget) = budget {
        budget.consume_work(units, "residual opaque Point source").map_err(accounting_error)?;
    }
    ctl.check().map_err(|_| IoError::Cancelled)
}
// Exact existing qualified_opaque_neighborhood metadata and alpha predicate,
// using bounded real native visitors instead of materializing an eager Plane.
// Existing eager/default function remains unchanged for all its old callers.
pub(super) fn opaque_original<P: ExactParsedPlane>(file: &XcfFile<P>, operation: &str, ctl: &Interrupt<'_>, budget: Option<&photocraft_xcf::Budget<'_>>) -> Result<bool, IoError> {
    poll(ctl, budget, 32)?;
    if !(file.version == 22
        && file.precision == 150
        && file.model == Model::Rgb
        && file.sample == Sample::U8
        && file.encoding == Encoding::NonlinearProfile
        && file.icc.is_empty()
        && file.width == 32
        && file.height == 32
        && file.channels.is_empty()
        && file.paths.is_empty()
        && file.layers.len() == 1)
    {
        return Ok(false);
    }
    let Some(d) = file.layers.first() else {
        return Ok(false);
    };
    if !(!d.group
        && d.path.len() <= 1
        && d.visible
        && d.alpha
        && d.model == Model::Rgb
        && d.width == 32
        && d.height == 32
        && d.offset == (0, 0)
        && d.mode == 28
        && (d.blend_space, d.composite_space, d.composite_mode) == (0, -1, -1)
        && d.opacity.to_bits() == 1.0_f32.to_bits()
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
    let Some(plane) = d.plane.as_ref() else {
        return Ok(false);
    };
    let descriptor = plane.descriptor();
    if descriptor.sample != Sample::U8 || descriptor.channels != 4 || plane.native_byte_len() != Some(32 * 32 * 4) {
        return Ok(false);
    }
    let mut seen = 0usize;
    let mut valid = true;
    plane.visit_native(ctl, &mut |bytes| {
        if bytes.is_empty() || bytes.len() > 256 || bytes.len() % 4 != 0 {
            return Err(IoError::Unsupported("opaque Point native block".into()));
        }
        seen = seen.checked_add(bytes.len()).filter(|n| *n <= 4096).ok_or_else(|| IoError::Unsupported("opaque Point extra bytes".into()))?;
        poll(ctl, budget, (bytes.len() + bytes.len() / 4) as u64)?;
        valid &= bytes.chunks_exact(4).all(|pixel| pixel.last() == Some(&255));
        Ok(())
    })?;
    if seen != 4096 {
        return Err(IoError::Unsupported("opaque Point incomplete native bytes".into()));
    }
    poll(ctl, budget, 0)?;
    Ok(valid)
}

#[cfg(test)]
#[path = "staged_residual_point_tests.rs"]
mod tests;
