//! Exact v12 alpha/color composition metadata for independently gated RGB/Gray.
//! Model, implicit profile, complete native unit inputs and source authority
//! are caller obligations. Native hidden RGB is never normalized by this table.
use super::{FlatSourceRgbDomain, GimpOperator, GimpSpace, ResolvedGimpComposition};
pub const XCF12_FLAT_ALPHA_SOURCE_EPOCH: &str = "xcf-12-authored-blend-auto";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatAlphaSourceRule {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
    pub source_rgb_domain: FlatSourceRgbDomain,
}
impl FlatAlphaSourceRule {
    pub fn resolved(self) -> ResolvedGimpComposition {
        ResolvedGimpComposition { blend_space: self.blend_space, composite_space: self.composite_space, operator: self.operator }
    }
}
pub fn flat_alpha_source_rule(version: u32, mode: i32, blend_raw: i32, composite_raw: i32, operator_raw: i32) -> Option<FlatAlphaSourceRule> {
    use GimpOperator::{ClipToBackdrop, Union};
    use GimpSpace::{LinearRgb, ProfileRgb};
    if version != 12 {
        return None;
    }
    let (blend_space, composite_space, operator) = match (mode, blend_raw, composite_raw, operator_raw) {
        (57, -1, -1, -2) => (LinearRgb, LinearRgb, ClipToBackdrop),
        (58, 0, -1, -2) => (ProfileRgb, LinearRgb, ClipToBackdrop),
        (59, 0, -1, -1) => (ProfileRgb, LinearRgb, Union),
        (60, 0, 0, -2) => (ProfileRgb, ProfileRgb, ClipToBackdrop),
        _ => return None,
    };
    Some(FlatAlphaSourceRule { blend_space, composite_space, operator, source_rgb_domain: FlatSourceRgbDomain::Unit })
}
