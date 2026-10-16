//! Paid fixed implicit-sRGB Exposure numeric adapter. This declared wide policy
//! grants no source/root admission and does not recover GEGL graph formats.
use crate::{
    backed_normal::BackedNormalError as E,
    backed_stock_point_context::PlanBudget,
    gimp_composite::{CompositionError, WorkingSpace},
};
use photocraft_algo::gimp::point::{PointOperation, PreparedPointOperation};
use photocraft_doc::{
    SampleEncoding,
    import_compositing::GimpSpace,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind},
    },
};
pub const EPOCH: &str = "xcf22-gegl-exposure-mixed-wide-v1";
const INPUT_WORK: u64 = 128;
const TRANSFER_WORK: u64 = 3 * 128;
const SCALAR_WORK: u64 = 3 * 128;
const OUTPUT_WORK: u64 = 128;
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = 512;
fn scope() -> E {
    E::Unsupported("fixed implicit RGB mixed Exposure numeric policy")
}
fn length() -> E {
    E::Invalid("Exposure preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
/// Exact stored F32 values, promoted to the existing f64 scalar kernel. Only
/// black level has the [-.1,.1] bound; stops are finite with checked scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    stops: f64,
    black_level: f64,
}
impl Parameters {
    pub fn from_words(stops: u32, black_level: u32) -> Result<Self, CompositionError> {
        let p = Self { stops: f64::from(f32::from_bits(stops)), black_level: f64::from(f32::from_bits(black_level)) };
        p.operation().validate()?;
        Ok(p)
    }
    pub fn values(self) -> [f64; 2] {
        [self.stops, self.black_level]
    }
    fn operation(self) -> PointOperation {
        PointOperation::Exposure { stops: self.stops, black_level: self.black_level }
    }
}
/// Argument transport only; epoch, envelope, model and actual owner are separate.
pub fn parameters(arguments: &[FilterArgument]) -> Result<Parameters, CompositionError> {
    if arguments.len() != 2 {
        return Err(CompositionError::Unsupported("Exposure two F32 arguments"));
    }
    let mut stops = None;
    let mut black = None;
    for a in arguments {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = a.value else { return Err(CompositionError::Unsupported("Exposure F32 argument")) };
        let target = match a.name.as_str() {
            "exposure" => &mut stops,
            "black-level" => &mut black,
            _ => return Err(CompositionError::Unsupported("Exposure argument name")),
        };
        if target.replace(bits).is_some() {
            return Err(CompositionError::Unsupported("Exposure duplicate argument"));
        }
    }
    Parameters::from_words(
        stops.ok_or(CompositionError::Unsupported("Exposure missing stops"))?,
        black.ok_or(CompositionError::Unsupported("Exposure missing black"))?,
    )
}
pub(crate) fn claims(node: &FilterNode) -> bool {
    node.operation.name == "gegl:exposure" || node.operation.schema_epoch == EPOCH
}
/// Mixed canonical metadata only; full actual root/native/profile/mask proof
/// and original-source geometry enrollment remain separate caller policies.
pub(crate) fn canonical(node: &FilterNode, stored: SampleEncoding, version: u32, extent: photocraft_geom::Rect) -> Result<Parameters, CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    if version != 22
        || node.operation.name != "gegl:exposure"
        || node.operation.stored_version != "1:0"
        || node.operation.schema_epoch != EPOCH
        || stored != SampleEncoding::NonlinearProfile
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| p.compatibility_epoch != EPOCH || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(CompositionError::Unsupported("mixed Exposure canonical envelope"));
    }
    // Stored three i32 words are retained opaque provenance, never Auto aliases.
    parameters(&node.arguments)
}
/// No arbitrary Profile input, dynamic Transform cache or borrowed profile
/// lifetime. Only the actual known nonlinear sRGB basis constructs this Space.
pub struct PreparedExposure {
    working: WorkingSpace,
    operation: PreparedPointOperation,
    identity: bool,
}
impl PreparedExposure {
    pub(crate) fn prepare<B: PlanBudget>(p: Parameters, outer_header: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        charge(b, i, 512)?;
        // Process-global builtin initialization has its existing cold boundary; this
        // call never treats it as source/profile authority or an allocation receipt.
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
        let header =
            (std::mem::size_of::<Self>() as u64).checked_add(outer_header).and_then(|v| v.checked_add(PIXEL_SCRATCH_BYTES + 128)).ok_or_else(length)?;
        charge(b, i, semantic.checked_mul(3).and_then(|v| v.checked_add(header.checked_mul(64)?)).and_then(|v| v.checked_add(512)).ok_or_else(length)?)?;
        let lease = b.reserve(header)?;
        // Lease is declared before every payload, including fallible constructions.
        let operation = p.operation().prepare().map_err(|e| E::Color(e.into()))?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, GimpSpace::LinearRgb, i).map_err(E::Color)?;
        let result = Self { working, operation, identity: p.stops == 0. && p.black_level == 0. };
        charge(b, i, 0)?;
        Ok((result, lease))
    }
    pub(crate) fn validation_work(&self) -> u64 {
        INPUT_WORK
    }
    pub(crate) fn evaluation_work(&self, enabled: bool) -> u64 {
        INPUT_WORK + if enabled && !self.identity { 2 * TRANSFER_WORK + SCALAR_WORK + OUTPUT_WORK } else { 0 }
    }
    fn validate_mode(pixel: [f64; 4], derived: bool) -> Result<(), E> {
        if pixel.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&pixel[3]) || (!derived && pixel.iter().take(3).any(|v| !(0.0..=1.0).contains(v))) {
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
        // Match existing singleton identity and preserve incoming wide alpha/bits.
        if !enabled || self.identity {
            return Ok(p);
        }
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
    /// Both the independent validation walk and application walk use this same
    /// real quota/control. No local work allowance and no retained callback.
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
#[path = "gimp_exposure_tests.rs"]
mod tests;
