//! Exact v12 physical-Lab metadata; callers independently pay preparation and
//! evaluation and validate source/profile/native domains. The canonical policy
//! retains Lab authority even when its physical import hint is absent.
use super::{FlatSourceRgbDomain, GimpOperator, GimpSpace, ResolvedGimpComposition};
pub const XCF12_FLAT_LAB_SOURCE_EPOCH: &str = "xcf-12-authored-blend-auto";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatLabSourceRule {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
    pub source_rgb_domain: FlatSourceRgbDomain,
}
impl FlatLabSourceRule {
    pub fn resolved(self) -> ResolvedGimpComposition {
        ResolvedGimpComposition { blend_space: self.blend_space, composite_space: self.composite_space, operator: self.operator }
    }
}
pub fn flat_lab_source_rule(version: u32, mode: i32, blend_raw: i32, composite_raw: i32, operator_raw: i32) -> Option<FlatLabSourceRule> {
    if version != 12 || !matches!(mode, 24..=27) || (blend_raw, composite_raw, operator_raw) != (-3, -1, -2) {
        return None;
    }
    Some(FlatLabSourceRule {
        blend_space: GimpSpace::Lab,
        composite_space: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        source_rgb_domain: FlatSourceRgbDomain::Unit,
    })
}
