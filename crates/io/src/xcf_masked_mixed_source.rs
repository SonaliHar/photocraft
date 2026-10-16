//! Masked MIXED scalar identities. Not ordinary maskedPoint or oldemptyMixed.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Exposure,
    InvertLinear,
    InvertGamma,
    BrightnessContrast,
    Levels,
}
impl Kind {
    pub(crate) fn epoch(self) -> &'static str {
        match self {
            Self::Exposure => photocraft_compose::EXPOSURE_MASKED_MIXED_EPOCH,
            Self::InvertLinear => photocraft_compose::INVERT_LINEAR_MASKED_MIXED_EPOCH,
            Self::InvertGamma => photocraft_compose::INVERT_GAMMA_MASKED_MIXED_EPOCH,
            Self::BrightnessContrast => photocraft_compose::BRIGHTNESS_CONTRAST_MASKED_MIXED_EPOCH,
            Self::Levels => photocraft_compose::GEGL_LEVELS_MASKED_MIXED_EPOCH,
        }
    }
    pub(crate) fn family(self) -> PointReplaceFamily {
        match self {
            Self::Exposure => PointReplaceFamily::MaskedMixedExposure,
            Self::InvertLinear => PointReplaceFamily::MaskedMixedInvertLinear,
            Self::InvertGamma => PointReplaceFamily::MaskedMixedInvertGamma,
            Self::BrightnessContrast => PointReplaceFamily::MaskedMixedBrightnessContrast,
            Self::Levels => PointReplaceFamily::MaskedMixedGeglLevels,
        }
    }
}
pub(crate) fn parameters<P>(e: &Effect<P>) -> Option<Kind> {
    if e.operation == "gegl:exposure" {
        super::mixed_exposure_source::parameters(e)?;
        return Some(Kind::Exposure);
    }
    Some(match crate::xcf_point_replace_source::masked_stock_source::parameters(e)? {
        crate::xcf_point_replace_source::masked_stock_source::Kind::InvertLinear => Kind::InvertLinear,
        crate::xcf_point_replace_source::masked_stock_source::Kind::InvertGamma => Kind::InvertGamma,
        crate::xcf_point_replace_source::masked_stock_source::Kind::BrightnessContrast => Kind::BrightnessContrast,
        crate::xcf_point_replace_source::masked_stock_source::Kind::Levels => Kind::Levels,
    })
}
