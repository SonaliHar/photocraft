//! Distinct ordered mixed-mask scalar roles. No ordinary Point/source alias.
use crate::{backed_normal::BackedNormalError as E, backed_stock_point_context::PlanBudget, gimp_composite::CompositionError};
use photocraft_doc::xcf_filters::{FilterArgument, FilterNode};
use photocraft_geom::Rect;
use photocraft_raster::{
    Interrupt,
    backing::{
        ReadControl,
        quota::{BackingQuota, QuotaLease},
    },
};
pub const EXPOSURE_EPOCH: &str = "xcf22-gegl-exposure-masked-mixed-wide-v1";
pub const INVERT_LINEAR_EPOCH: &str = "xcf22-gegl-invert-linear-masked-mixed-wide-v1";
pub const INVERT_GAMMA_EPOCH: &str = "xcf22-gegl-invert-gamma-masked-mixed-wide-v1";
pub const BRIGHTNESS_CONTRAST_EPOCH: &str = "xcf22-gegl-brightness-contrast-masked-mixed-wide-v1";
pub const GEGL_LEVELS_EPOCH: &str = "xcf22-gegl-levels-masked-mixed-wide-v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Exposure,
    InvertLinear,
    InvertGamma,
    BrightnessContrast,
    GeglLevels,
}
impl Kind {
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::Exposure => "gegl:exposure",
            Self::InvertLinear => "gegl:invert-linear",
            Self::InvertGamma => "gegl:invert-gamma",
            Self::BrightnessContrast => "gegl:brightness-contrast",
            Self::GeglLevels => "gegl:levels",
        }
    }
    pub fn version(self) -> &'static str {
        if self == Self::Exposure { "1:0" } else { "0:0" }
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::Exposure => EXPOSURE_EPOCH,
            Self::InvertLinear => INVERT_LINEAR_EPOCH,
            Self::InvertGamma => INVERT_GAMMA_EPOCH,
            Self::BrightnessContrast => BRIGHTNESS_CONTRAST_EPOCH,
            Self::GeglLevels => GEGL_LEVELS_EPOCH,
        }
    }
    fn scalar(self) -> Option<crate::MaskedScalarKind> {
        match self {
            Self::Exposure => None,
            Self::InvertLinear => Some(crate::MaskedScalarKind::InvertLinear),
            Self::InvertGamma => Some(crate::MaskedScalarKind::InvertGamma),
            Self::BrightnessContrast => Some(crate::MaskedScalarKind::BrightnessContrast),
            Self::GeglLevels => Some(crate::MaskedScalarKind::GeglLevels),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Parameters {
    Exposure(crate::gimp_exposure::Parameters),
    Scalar(crate::MaskedScalarParameters),
}
impl Parameters {
    pub fn kind(self) -> Kind {
        match self {
            Self::Exposure(_) => Kind::Exposure,
            Self::Scalar(p) => match p.kind() {
                crate::MaskedScalarKind::InvertLinear => Kind::InvertLinear,
                crate::MaskedScalarKind::InvertGamma => Kind::InvertGamma,
                crate::MaskedScalarKind::BrightnessContrast => Kind::BrightnessContrast,
                crate::MaskedScalarKind::GeglLevels => Kind::GeglLevels,
            },
        }
    }
}
fn shape() -> CompositionError {
    CompositionError::Unsupported("exact masked mixed scalar role")
}
pub fn parameters(kind: Kind, args: &[FilterArgument]) -> Result<Parameters, CompositionError> {
    if args.len() > 4 || args.iter().any(|a| a.name.len() > 16) {
        return Err(shape());
    }
    if kind == Kind::Exposure {
        Ok(Parameters::Exposure(crate::gimp_exposure::parameters(args)?))
    } else {
        Ok(Parameters::Scalar(crate::gimp_masked_scalar::parameters(kind.scalar().ok_or_else(shape)?, args)?))
    }
}
pub(crate) fn claims(node: &FilterNode) -> bool {
    [EXPOSURE_EPOCH, INVERT_LINEAR_EPOCH, INVERT_GAMMA_EPOCH, BRIGHTNESS_CONTRAST_EPOCH, GEGL_LEVELS_EPOCH].contains(&node.operation.schema_epoch.as_str())
        || node.composition.as_ref().is_some_and(|c| {
            [EXPOSURE_EPOCH, INVERT_LINEAR_EPOCH, INVERT_GAMMA_EPOCH, BRIGHTNESS_CONTRAST_EPOCH, GEGL_LEVELS_EPOCH].contains(&c.compatibility_epoch.as_str())
        })
}
pub(crate) fn canonical(node: &FilterNode, extent: Rect) -> Result<Parameters, CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    if node.operation.name.len() > 32 || node.operation.schema_epoch.len() > 64 || node.operation.stored_version.len() > 8 {
        return Err(shape());
    }
    let kind = match node.operation.name.as_str() {
        "gegl:exposure" => Kind::Exposure,
        "gegl:invert-linear" => Kind::InvertLinear,
        "gegl:invert-gamma" => Kind::InvertGamma,
        "gegl:brightness-contrast" => Kind::BrightnessContrast,
        "gegl:levels" => Kind::GeglLevels,
        _ => return Err(shape()),
    };
    if node.operation.stored_version != kind.version()
        || node.operation.schema_epoch != kind.epoch()
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node
            .composition
            .as_ref()
            .is_none_or(|p| p.compatibility_epoch != kind.epoch() || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(shape());
    }
    parameters(kind, &node.arguments)
}
pub(crate) enum Prepared {
    Exposure(crate::gimp_masked_exposure::PreparedMaskedExposure),
    Scalar(crate::gimp_masked_scalar::PreparedMaskedScalar),
}
impl Prepared {
    pub(crate) fn prepare<B: PlanBudget>(p: Parameters, outer: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        match p {
            Parameters::Exposure(p) => {
                let (value, fee) = crate::gimp_masked_exposure::PreparedMaskedExposure::prepare(p, outer, i, b)?;
                Ok((Self::Exposure(value), fee))
            }
            Parameters::Scalar(p) => {
                let (value, fee) = crate::gimp_masked_scalar::PreparedMaskedScalar::prepare(p, outer, i, b)?;
                Ok((Self::Scalar(value), fee))
            }
        }
    }
    pub(crate) fn reserve_runtime(&self, q: &BackingQuota, c: &ReadControl<'_>) -> Result<QuotaLease, E> {
        match self {
            Self::Exposure(p) => p.reserve_runtime(q, c),
            Self::Scalar(p) => p.reserve_runtime(q, c),
        }
    }
    pub(crate) fn evaluate(&self, before: [f64; 4], enabled: bool, derived: bool, raw: f64, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        match self {
            Self::Exposure(p) => {
                if derived {
                    p.evaluate_derived(before, enabled, raw, q, c)
                } else {
                    p.evaluate(before, enabled, raw, q, c)
                }
            }
            Self::Scalar(p) => {
                if derived {
                    p.evaluate_derived(before, enabled, raw, q, c)
                } else {
                    p.evaluate(before, enabled, raw, q, c)
                }
            }
        }
    }
}

#[cfg(test)]
#[path="gimp_masked_mixed_tests.rs"]
pub(crate) mod tests;
