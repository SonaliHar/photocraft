//! Paid fixed implicit-sRGB wide invert adapters and exact CURRENT leaf identity.
//! Original-source enrollment and producer-format qualification remain separate.
use crate::{
    backed_normal::BackedNormalError as E,
    backed_stock_point_context::PlanBudget,
    gimp_composite::{CompositionError, WorkingSpace},
};
use photocraft_algo::gimp::point::{PointOperation, PreparedPointOperation};
use photocraft_doc::{SampleEncoding, import_compositing::GimpSpace, xcf_filters::FilterNode};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind},
    },
};
pub const LINEAR_EPOCH: &str = "xcf22-gegl-invert-linear-mixed-wide-v1";
pub const GAMMA_EPOCH: &str = "xcf22-gegl-invert-gamma-mixed-wide-v1";
const INPUT_WORK: u64 = 128;
const THREE_CURVES_WORK: u64 = 3 * 128;
const SCALAR_WORK: u64 = 3 * 128;
const OUTPUT_WORK: u64 = 128;
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = 512;
fn scope() -> E {
    E::Unsupported("fixed implicit RGB mixed invert numeric policy")
}
fn length() -> E {
    E::Invalid("invert preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
/// Closed family identity: shared lower Invert arithmetic never aliases its
/// Linear versus nonlinear interpretation or its two retained operation names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Linear,
    Gamma,
}
impl Kind {
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::Linear => "gegl:invert-linear",
            Self::Gamma => "gegl:invert-gamma",
        }
    }
    pub fn version(self) -> &'static str {
        "0:0"
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::Linear => LINEAR_EPOCH,
            Self::Gamma => GAMMA_EPOCH,
        }
    }
    fn working(self) -> GimpSpace {
        match self {
            Self::Linear => GimpSpace::LinearRgb,
            Self::Gamma => GimpSpace::ProfileRgb,
        }
    }
    fn transfer_work(self) -> u64 {
        match self {
            Self::Linear => 2 * THREE_CURVES_WORK,
            Self::Gamma => 0,
        }
    }
}
pub(crate) fn claims(node: &FilterNode) -> bool {
    matches!(node.operation.name.as_str(), "gegl:invert-linear" | "gegl:invert-gamma")
        || matches!(node.operation.schema_epoch.as_str(), LINEAR_EPOCH | GAMMA_EPOCH)
}
/// Exact CURRENT metadata kind; no source identity follows from lower arithmetic.
pub(crate) fn canonical(node: &FilterNode, stored: SampleEncoding, version: u32, extent: photocraft_geom::Rect) -> Result<Kind, CompositionError> {
    canonical_mode(node, stored, version, extent, false)
}
pub(crate) fn canonical_mixed_opacity(node: &FilterNode, stored: SampleEncoding, version: u32, extent: photocraft_geom::Rect) -> Result<Kind, CompositionError> {
    canonical_mode(node, stored, version, extent, true)
}
fn canonical_mode(node: &FilterNode, stored: SampleEncoding, version: u32, extent: photocraft_geom::Rect, current: bool) -> Result<Kind, CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    let kind = match node.operation.name.as_str() {
        "gegl:invert-linear" => Kind::Linear,
        "gegl:invert-gamma" => Kind::Gamma,
        _ => return Err(CompositionError::Unsupported("mixed invert operation")),
    };
    if version != 22
        || node.operation.stored_version != kind.version()
        || node.operation.schema_epoch != kind.epoch()
        || !node.arguments.is_empty()
        || stored != SampleEncoding::NonlinearProfile
        || if current { crate::gimp_mixed_filter_opacity::validate_opacity(node.opacity_f64_bits).is_err() } else { node.opacity_f64_bits != 1f64.to_bits() }
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node
            .composition
            .as_ref()
            .is_none_or(|p| p.compatibility_epoch != kind.epoch() || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(CompositionError::Unsupported("mixed invert canonical envelope"));
    }
    // All three retained i32 words are opaque provenance, never Auto aliases.
    Ok(kind)
}
/// Fixed actual known basis and inline prepared scalar; no arbitrary Profile,
/// transform cache, self-reference or source/callback capability is stored.
pub struct PreparedInvert {
    kind: Kind,
    working: WorkingSpace,
    operation: PreparedPointOperation,
}
impl PreparedInvert {
    pub(crate) fn prepare<B: PlanBudget>(kind: Kind, outer_header: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        charge(b, i, 512)?;
        // Existing process-global builtin initialization is an explicit cold boundary.
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
        let header =
            (std::mem::size_of::<Self>() as u64).checked_add(outer_header).and_then(|v| v.checked_add(PIXEL_SCRATCH_BYTES + 128)).ok_or_else(length)?;
        // Conservative bound independently prepays constructor profile validation:
        // Gamma's same-space branch performs no per-pixel transfer curve evaluation.
        charge(b, i, semantic.checked_mul(3).and_then(|v| v.checked_add(header.checked_mul(64)?)).and_then(|v| v.checked_add(512)).ok_or_else(length)?)?;
        let lease = b.reserve(header)?;
        let operation = PointOperation::Invert.prepare().map_err(|e| E::Color(e.into()))?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, kind.working(), i).map_err(E::Color)?;
        let result = Self { kind, working, operation };
        charge(b, i, 0)?;
        Ok((result, lease))
    }
    pub(crate) fn validation_work(&self) -> u64 {
        INPUT_WORK
    }
    pub(crate) fn evaluation_work(&self, enabled: bool) -> u64 {
        INPUT_WORK + if enabled { self.kind.transfer_work() + SCALAR_WORK + OUTPUT_WORK } else { 0 }
    }
    fn validate_mode(p: [f64; 4], derived: bool) -> Result<(), E> {
        if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) || (!derived && p.iter().take(3).any(|v| !(0.0..=1.0).contains(v))) {
            return Err(scope());
        }
        Ok(())
    }
    pub(crate) fn validate_input(&self, p: [f64; 4]) -> Result<(), E> {
        Self::validate_mode(p, false)
    }
    pub(crate) fn validate_derived_input(&self, p: [f64; 4]) -> Result<(), E> {
        Self::validate_mode(p, true)
    }
    fn apply(&self, p: [f64; 4], enabled: bool, derived: bool, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        Self::validate_mode(p, derived)?;
        if !enabled {
            return Ok(p);
        }
        // No neutral, alpha-zero, double-invert, or other algebraic cancellation.
        c.check(0).map_err(E::Backing)?;
        let rgb = self.working.convert([p[0], p[1], p[2]], true).map_err(E::Color)?;
        let mut out = [0.; 4];
        c.check(0).map_err(E::Backing)?;
        self.operation.try_eval_pixel(&[rgb[0], rgb[1], rgb[2], p[3]], 3, &mut out).map_err(|e| E::Color(e.into()))?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.working.convert([out[0], out[1], out[2]], false).map_err(E::Color)?;
        let out = [rgb[0], rgb[1], rgb[2], p[3]];
        Self::validate_mode(out, true)?;
        c.check(0).map_err(E::Backing)?;
        Ok(out)
    }
    /// Actual original control/quota prices validation and application separately.
    /// Future stack NLFloat bridges are independent and are not performed here.
    pub(crate) fn evaluate(&self, p: [f64; 4], enabled: bool, derived: bool, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        c.check(self.validation_work()).map_err(E::Backing)?;
        if derived {
            self.validate_derived_input(p)?;
        } else {
            self.validate_input(p)?;
        }
        c.check(self.evaluation_work(enabled)).map_err(E::Backing)?;
        let _scratch = q.reserve(QuotaKind::Resident, PIXEL_SCRATCH_BYTES).map_err(E::Quota)?;
        let result = self.apply(p, enabled, derived, c)?;
        c.check(0).map_err(E::Backing)?;
        Ok(result)
    }
}
#[cfg(test)]
#[path = "gimp_invert_tests.rs"]
mod tests;
