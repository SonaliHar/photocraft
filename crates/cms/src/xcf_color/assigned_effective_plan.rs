//! Prospective work for planning an assigned analytic current interpretation.
use super::{Encoding, implicit_planner};
use crate::{
    ColorSpace, Curve, Profile,
    control::{Budget, Error, Limits, Result},
};

/// Allocation-free work ceiling for one subsequent call to
/// `effective_profile_requirements_with_control` on this borrowed definition.
///
/// The caller prepays this ceiling before invoking that planner. Its returned
/// requirements describe separate future construction work and storage, which
/// must also be admitted by the caller; this number is neither a quota receipt
/// nor profile qualification. Pass the actual future-operation limits to the
/// requirements call, rather than replacing its work limit with this ceiling.
///
/// Only constant-size analytic RGB/Gray definitions are planned here. Tables
/// (including inactive other-model curves) and every LUT are refused before
/// any variable-size traversal. Text and cached ICC capacities are read in O(1);
/// no text, encoded ICC, or curve samples are scanned or copied.
///
/// For this closed shape the existing base-effective-work envelope covers the
/// planner: at most three operative curves use 257 * 128 sample work each;
/// the writer count pass visits at most four fixed-size analytic curves plus
/// fixed matrix/point tags and two text tags. Its UTF-16 count/write census
/// charges at most three units per UTF-8 byte, including the fixed transfer
/// suffix. The existing four-per-retained-byte plus 200000 fixed envelope
/// therefore covers these visits without reproducing the writer cost formula.
/// Future curve variants must be considered explicitly in the match below.
pub fn assigned_effective_planning_work_ceiling(profile: &Profile, encoding: Encoding) -> Result<u64> {
    if profile.a2b.iter().chain(&profile.b2a).any(Option::is_some) {
        return Err(Error::Limit("assigned effective planner excludes LUTs"));
    }
    match profile.color_space {
        ColorSpace::Rgb if profile.matrix.is_some() && profile.trc.is_some() => (),
        ColorSpace::Gray if profile.gray_trc.is_some() => (),
        _ => return Err(Error::Limit("assigned effective planner requires analytic RGB/Gray")),
    }
    for curve in profile.trc.iter().flatten().chain(profile.gray_trc.iter()) {
        match curve {
            Curve::Identity | Curve::Gamma(_) => (),
            Curve::Parametric { kind: 0..=4, .. } => (),
            Curve::Parametric { .. } | Curve::Table(_) | Curve::Table64(_) => {
                return Err(Error::Limit("assigned effective planner requires constant-size curves"));
            }
        }
    }
    // Match all encodings explicitly; no future transfer policy is automatic.
    match encoding {
        Encoding::NonlinearProfile | Encoding::Linear | Encoding::Perceptual => (),
    }
    // With LUTs/tables excluded this existing capacity census has a fixed number
    // of visits. It includes cached encoded capacity, not just logical lengths.
    let metadata = Budget::new(Limits::default(), &|| false)?;
    let retained = profile.controlled_retained_bytes(&metadata)?;
    metadata.admit(retained, 0)?;
    let work = implicit_planner::base_effective_work(retained)?;
    metadata.admit(0, work)?;
    Ok(work)
}

/// Allocation-free ceiling for one legacy analytic semantic qualification.
///
/// Unlike assigned effective reconstruction planning, this does not inspect or
/// reject inactive other-model sampled curves: the existing analytic qualifier
/// and `validate_profile` evaluate only the declared model's operative curves.
/// LUTs and operative tables are outside that qualifier's existing analytic
/// class and are refused before variable-size traversal. This is only work
/// admission; all existing value, matrix, header and curve policy checks remain
/// with the actual qualifier. Cached/text/inactive-table capacities are counted
/// in O(1) by the existing retained census, without reading their contents.
pub fn semantic_analytic_profile_work_ceiling(profile: &Profile) -> Result<u64> {
    if profile.a2b.iter().chain(&profile.b2a).any(Option::is_some) {
        return Err(Error::Limit("analytic semantic planner excludes LUTs"));
    }
    fn constant(curve: &Curve) -> Result<()> {
        match curve {
            Curve::Identity | Curve::Gamma(_) | Curve::Parametric { kind: 0..=4, .. } => Ok(()),
            Curve::Parametric { .. } | Curve::Table(_) | Curve::Table64(_) => Err(Error::Limit("analytic semantic planner requires operative analytic curves")),
        }
    }
    match profile.color_space {
        ColorSpace::Rgb => {
            if profile.matrix.is_none() {
                return Err(Error::Limit("analytic RGB matrix missing"));
            }
            for curve in profile.trc.as_ref().ok_or(Error::Limit("analytic RGB curves missing"))? {
                constant(curve)?;
            }
        }
        ColorSpace::Gray => constant(profile.gray_trc.as_ref().ok_or(Error::Limit("analytic Gray curve missing"))?)?,
        _ => return Err(Error::Limit("analytic semantic planner requires RGB/Gray")),
    }
    let metadata = Budget::new(Limits::default(), &|| false)?;
    let retained = profile.controlled_retained_bytes(&metadata)?;
    metadata.admit(retained, 0)?;
    // Same existing envelope as effective planning; no writer/curve formula is
    // exported to consumers. At most three operative analytic curves are read.
    let work = implicit_planner::base_effective_work(retained)?;
    metadata.admit(0, work)?;
    Ok(work)
}

#[cfg(test)]
#[path = "assigned_effective_plan_tests.rs"]
mod tests;
