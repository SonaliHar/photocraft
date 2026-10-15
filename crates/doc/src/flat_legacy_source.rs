//! Exact metadata-only legacy flat-source composition rules.
//! Callers retain independent storage, source, profile and native-domain gates.
use super::{FlatSourceRgbDomain, GimpOperator, GimpSpace, ResolvedGimpComposition};

/// The existing authored v12 legacy epoch; this is not a new source grant.
pub const XCF12_FLAT_LEGACY_SOURCE_EPOCH: &str = "xcf-12-authored-blend-auto";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlatLegacySourceRule {
    pub blend_space: GimpSpace,
    pub composite_space: GimpSpace,
    pub operator: GimpOperator,
    pub source_rgb_domain: FlatSourceRgbDomain,
}
impl FlatLegacySourceRule {
    pub fn resolved(self) -> ResolvedGimpComposition {
        ResolvedGimpComposition { blend_space: self.blend_space, composite_space: self.composite_space, operator: self.operator }
    }
}
/// Only the eighteen independently observed legacy RGB modes.
/// Their endpoint handling is distinct from modern Divide/Dodge/Burn domains.
/// Normal0, requested Overlay5 and unknown tuples stay closed.
pub fn flat_legacy_source_rule(version: u32, mode: i32, blend_raw: i32, composite_raw: i32, operator_raw: i32) -> Option<FlatLegacySourceRule> {
    if version != 12 || !matches!(mode, 3 | 4 | 6..=21) || (blend_raw, composite_raw, operator_raw) != (-2, -2, -2) {
        return None;
    }
    Some(FlatLegacySourceRule {
        blend_space: GimpSpace::ProfileRgb,
        composite_space: GimpSpace::ProfileRgb,
        operator: GimpOperator::ClipToBackdrop,
        source_rgb_domain: FlatSourceRgbDomain::Unit,
    })
}
