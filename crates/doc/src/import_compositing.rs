//! Opt-in normal/source-over import policy. No arbitrary operators are representable.
use crate::SampleEncoding;
use serde::{Deserialize, Serialize};
#[path = "flat_modern_source.rs"]
mod flat_modern_source;
pub use flat_modern_source::{FlatModernSourceRule, FlatSourceRgbDomain, XCF_FLAT_MODERN_IMPLICIT_SOURCE_EPOCH, flat_modern_source_rule};
/// Explicit structural behavior identity, independent of current profile assignment.
pub const XCF13_ISOLATED_MULTIPLY_SEED_EPOCH: &str = "xcf13-isolated-multiply-seed-v1";
/// Finite signed/over-one source alpha over an empty or opaque backdrop only.
pub const XCF12_FINITE_NORMAL_SOURCE_EPOCH: &str = "xcf12-finite-normal-source-v1";

/// Normal/mask composition qualified for a bounded analytical assigned ICC class.
pub const XCF12_ANALYTIC_NORMAL_MASK_EPOCH: &str = "xcf12-analytic-normal-mask-v1";
/// Analytical assigned primaries with explicitly linear current transfer curves.
pub const XCF12_ANALYTIC_LINEAR_NORMAL_MASK_EPOCH: &str = "xcf12-analytic-linear-normal-mask-v1";
/// Analytical assigned primaries with explicitly perceptual current transfer curves.
pub const XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_EPOCH: &str = "xcf12-analytic-perceptual-normal-mask-v1";
/// Bounded independently observed owner/mask geometry with analytical assigned ICC.
pub const XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH: &str = "xcf12-analytic-normal-mask-geometry-v1";
/// The same bounded geometry with explicitly linear current sample interpretation.
pub const XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH: &str = "xcf12-analytic-linear-normal-mask-geometry-v1";
/// The same bounded geometry with explicitly perceptual current sample interpretation.
pub const XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH: &str = "xcf12-analytic-perceptual-normal-mask-geometry-v1";
/// Bounded native U16/U32 geometry with nonlinear analytical assigned ICC.
pub const XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH: &str = "xcf12-analytic-integer-normal-mask-geometry-v1";
/// Ordinary two-owner Normal-mask scenes require caller-owned job accounting.
pub const XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH: &str = "xcf12-analytic-ordinary-normal-mask-v1";

/// Stored current interpretation required by a qualified analytical composition.
pub fn analytic_normal_encoding(epoch: &str) -> Option<SampleEncoding> {
    match epoch {
        XCF12_ANALYTIC_NORMAL_MASK_EPOCH
        | XCF12_ANALYTIC_NORMAL_MASK_GEOMETRY_EPOCH
        | XCF12_ANALYTIC_INTEGER_NORMAL_MASK_GEOMETRY_EPOCH
        | XCF12_ANALYTIC_ORDINARY_NORMAL_MASK_EPOCH => Some(SampleEncoding::NonlinearProfile),
        XCF12_ANALYTIC_LINEAR_NORMAL_MASK_EPOCH | XCF12_ANALYTIC_LINEAR_NORMAL_MASK_GEOMETRY_EPOCH => Some(SampleEncoding::Linear),
        XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_EPOCH | XCF12_ANALYTIC_PERCEPTUAL_NORMAL_MASK_GEOMETRY_EPOCH => Some(SampleEncoding::Perceptual),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCompositing {
    pub blend_encoding: SampleEncoding,
    pub composite_encoding: SampleEncoding,
    pub pass_through: bool,
}

/// Source and resolved policies stay distinct; unknown raw effect domains remain unresolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GimpComposition {
    pub compatibility_epoch: String,
    pub source_mode: i32,
    pub legacy: bool,
    pub stored_blend_space: i32,
    pub stored_composite_space: i32,
    pub stored_operator: i32,
    pub resolved: Option<ResolvedGimpComposition>,
    pub pass_through: bool,
}

impl GimpComposition {
    /// Admit the already stored resolution without recomputing Auto/default
    /// choices from current profiles, storage precision or a reopening runtime.
    /// Positive serialized enums are explicit: they cannot contradict the cached
    /// resolution. The small nonpositive domains retain the importer's qualified
    /// defaults; no opaque serialized word is treated as Auto.
    /// This is structural admission, not operation/epoch evaluator qualification.
    pub fn checked_resolved(&self) -> Result<&ResolvedGimpComposition, crate::XcfValidationError> {
        let invalid = |code| crate::XcfValidationError::new(code, "invalid or unresolved imported composition policy");
        if self.compatibility_epoch.is_empty() || self.compatibility_epoch.len() > 1 << 20 || self.compatibility_epoch.contains('\0') {
            return Err(invalid("composition_identity"));
        }
        if !(-4..=4).contains(&self.stored_blend_space) || !(-4..=4).contains(&self.stored_composite_space) || !(-2..=4).contains(&self.stored_operator) {
            return Err(invalid("composition_unknown"));
        }
        let resolved = self.resolved.as_ref().ok_or_else(|| invalid("composition_unresolved"))?;
        let explicit_space = |stored| match stored {
            1 => Some(GimpSpace::LinearRgb),
            2 => Some(GimpSpace::ProfileRgb),
            3 => Some(GimpSpace::Lab),
            4 => Some(GimpSpace::PerceptualRgb),
            _ => None,
        };
        let explicit_operator = match self.stored_operator {
            1 => Some(GimpOperator::Union),
            2 => Some(GimpOperator::ClipToBackdrop),
            3 => Some(GimpOperator::ClipToLayer),
            4 => Some(GimpOperator::Intersection),
            _ => None,
        };
        if explicit_space(self.stored_blend_space).is_some_and(|space| space != resolved.blend_space)
            || explicit_space(self.stored_composite_space).is_some_and(|space| space != resolved.composite_space)
            || explicit_operator.is_some_and(|operator| operator != resolved.operator)
        {
            return Err(invalid("composition_resolution"));
        }
        Ok(resolved)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedGimpComposition {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GimpSpace {
    LinearRgb,
    ProfileRgb,
    Lab,
    PerceptualRgb,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GimpOperator {
    Union,
    ClipToBackdrop,
    ClipToLayer,
    Intersection,
}
