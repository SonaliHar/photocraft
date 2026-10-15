//! Allocation-free raw policy rules for the implicit RGB flat-source lane.
//! Geometry, storage, flags, profile identity and native samples require separate
//! controlled validation. A returned rule is not source or renderer authority.
use super::{GimpOperator, GimpSpace, ResolvedGimpComposition};

pub const XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH: &str = "xcf-flat-modern-implicit-v1";

/// Native source restrictions of the existing scoped channel equations.
/// Every source alpha must independently be finite and in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlatSourceRgbDomain {
    Finite,
    Unit,
    PositiveUnit,
    BelowOneUnit,
    StrictUnit,
}
impl FlatSourceRgbDomain {
    pub fn accepts(self, value: f64) -> bool {
        value.is_finite()
            && match self {
                Self::Finite => true,
                Self::Unit => (0.0..=1.0).contains(&value),
                Self::PositiveUnit => value > 0.0 && value <= 1.0,
                Self::BelowOneUnit => (0.0..1.0).contains(&value),
                Self::StrictUnit => value > 0.0 && value < 1.0,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatModernSourceRule {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
    pub source_rgb_domain: FlatSourceRgbDomain,
}
impl FlatModernSourceRule {
    pub fn resolved(self) -> ResolvedGimpComposition {
        ResolvedGimpComposition { blend_space: self.blend_space, composite_space: self.composite_space, operator: self.operator }
    }
}
fn space(raw: i32) -> Option<GimpSpace> {
    match raw {
        1 => Some(GimpSpace::LinearRgb),
        2 => Some(GimpSpace::ProfileRgb),
        4 => Some(GimpSpace::PerceptualRgb),
        _ => None,
    }
}
fn operator(raw: i32) -> Option<GimpOperator> {
    match raw {
        1 => Some(GimpOperator::Union),
        2 => Some(GimpOperator::ClipToBackdrop),
        3 => Some(GimpOperator::ClipToLayer),
        4 => Some(GimpOperator::Intersection),
        _ => None,
    }
}
/// Exact retained Auto patterns plus the existing explicit CURRENT wire subset.
/// No absolute-value conversion of arbitrary negative fields is permitted.
/// The mixed-version additions are observed v23 Multiply/Subtract/Lighten;
/// other mode/version combinations keep their existing closed raw patterns.
pub fn flat_modern_source_rule(version: u32, mode: i32, blend_raw: i32, composite_raw: i32, operator_raw: i32) -> Option<FlatModernSourceRule> {
    use GimpOperator::{ClipToBackdrop, Union};
    use GimpSpace::{LinearRgb, PerceptualRgb, ProfileRgb};
    if !matches!(version, 12 | 23) || !matches!(mode, 23 | 28 | 30..=36 | 41..=53) {
        return None;
    }
    let (blend_space, composite_space, operator) = match (version, mode, blend_raw, composite_raw, operator_raw) {
        (12 | 23, 28, 0, -1, -1) => (LinearRgb, LinearRgb, Union),
        // GIMP's explicit Normal control retains Auto blend (0), rather than
        // the attempted positive blend word. Only this witnessed mixed shape
        // is added; it is not a blanket interpretation of arbitrary Auto.
        (23, 28, 0, 1, 1) => (LinearRgb, LinearRgb, Union),
        (12 | 23, 30, -1, -1, -2) => (LinearRgb, LinearRgb, ClipToBackdrop),
        (12, 30, 2, -1, -2) => (ProfileRgb, LinearRgb, ClipToBackdrop),
        (23, 30, 4, -1, -2) => (PerceptualRgb, LinearRgb, ClipToBackdrop),
        (12, 30, -1, 2, -2) => (LinearRgb, ProfileRgb, ClipToBackdrop),
        (23, 30, -1, 4, -2) => (LinearRgb, PerceptualRgb, ClipToBackdrop),
        (23, 23 | 31 | 32 | 42..=53, -4, -1, -2) => (PerceptualRgb, LinearRgb, ClipToBackdrop),
        (12, 33 | 34 | 41, -1, -1, -2) | (23, 34, -1, -1, -2) => (LinearRgb, LinearRgb, ClipToBackdrop),
        // Profile/perceptual equivalence is confined to the separate exact
        // implicit-sRGB lane. This does not resolve an assigned-profile Auto.
        (12, 35 | 36, 0, -1, -2) | (23, 36, 0, -1, -2) => (ProfileRgb, LinearRgb, ClipToBackdrop),
        _ if !matches!(mode, 46 | 47) => (space(blend_raw)?, space(composite_raw)?, operator(operator_raw)?),
        _ => return None,
    };
    let source_rgb_domain = match mode {
        41 | 43 => FlatSourceRgbDomain::PositiveUnit,
        42 => FlatSourceRgbDomain::BelowOneUnit,
        48 => FlatSourceRgbDomain::StrictUnit,
        44..=47 | 49 | 51 | 53 => FlatSourceRgbDomain::Unit,
        _ => FlatSourceRgbDomain::Finite,
    };
    Some(FlatModernSourceRule { blend_space, composite_space, operator, source_rgb_domain })
}
