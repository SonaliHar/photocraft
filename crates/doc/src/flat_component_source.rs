//! Exact metadata rules for witnessed RGB component and luminance source modes.
//! Storage, source eligibility, profile and native/intermediate domains remain
//! independently controlled by callers. This table grants none of them.
use super::{FlatSourceRgbDomain, GimpOperator, GimpSpace, ResolvedGimpComposition};
pub const XCF12_FLAT_COMPONENT_SOURCE_EPOCH: &str = "xcf-12-authored-blend-auto";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatComponentSourceRule {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
    pub source_rgb_domain: FlatSourceRgbDomain,
}
impl FlatComponentSourceRule {
    pub fn resolved(self) -> ResolvedGimpComposition {
        ResolvedGimpComposition { blend_space: self.blend_space, composite_space: self.composite_space, operator: self.operator }
    }
}
pub fn flat_component_source_rule(version: u32, mode: i32, blend_raw: i32, composite_raw: i32, operator_raw: i32) -> Option<FlatComponentSourceRule> {
    if version != 12 || (composite_raw, operator_raw) != (-1, -2) {
        return None;
    }
    let blend_space = match (mode, blend_raw) {
        (37..=40, -2) => GimpSpace::ProfileRgb,
        (54..=56, -1) => GimpSpace::LinearRgb,
        _ => return None,
    };
    Some(FlatComponentSourceRule {
        blend_space,
        composite_space: GimpSpace::LinearRgb,
        operator: GimpOperator::ClipToBackdrop,
        source_rgb_domain: FlatSourceRgbDomain::Unit,
    })
}
