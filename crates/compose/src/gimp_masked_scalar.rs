//! Four sealed ordinary Point scalar-mask adapters; no mixed or empty-mask authority.
use crate::{
    backed_normal::BackedNormalError as E, backed_stock_point_context::PlanBudget, gimp_composite::CompositionError, gimp_masked_exposure::PreparedMix,
};
use photocraft_algo::gimp::point::{PointOperation, PreparedPointOperation};
use photocraft_doc::xcf_filters::{FilterArgument, FilterNode};
use photocraft_raster::{
    Interrupt,
    backing::{
        BackingError, ReadControl,
        quota::{BackingQuota, QuotaKind, QuotaLease},
    },
};
pub const INVERT_LINEAR_EPOCH: &str = "xcf22-gegl-invert-linear-masked-wide-v1";
pub const INVERT_GAMMA_EPOCH: &str = "xcf22-gegl-invert-gamma-masked-wide-v1";
pub const BRIGHTNESS_CONTRAST_EPOCH: &str = "xcf22-gegl-brightness-contrast-masked-wide-v1";
pub const GEGL_LEVELS_EPOCH: &str = "xcf22-gegl-levels-masked-wide-v1";
const LEAF_SCRATCH_BYTES: u64 = 512;
fn scope() -> E {
    E::Unsupported("four masked Point scalar policy")
}
fn shape() -> CompositionError {
    CompositionError::Unsupported("masked scalar exact kind/typed parameters")
}
fn length() -> E {
    E::Invalid("masked scalar preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    InvertLinear,
    InvertGamma,
    BrightnessContrast,
    GeglLevels,
}
impl Kind {
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::InvertLinear => "gegl:invert-linear",
            Self::InvertGamma => "gegl:invert-gamma",
            Self::BrightnessContrast => "gegl:brightness-contrast",
            Self::GeglLevels => "gegl:levels",
        }
    }
    pub fn version(self) -> &'static str {
        "0:0"
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::InvertLinear => INVERT_LINEAR_EPOCH,
            Self::InvertGamma => INVERT_GAMMA_EPOCH,
            Self::BrightnessContrast => BRIGHTNESS_CONTRAST_EPOCH,
            Self::GeglLevels => GEGL_LEVELS_EPOCH,
        }
    }
    fn linear(self) -> bool {
        self != Self::InvertGamma
    }
}
/// Exact scalar-only validation retains the original F32 words. This enum does
/// not carry operation eligibility, selected mask or preparation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameters {
    InvertLinear,
    InvertGamma,
    BrightnessContrast(crate::BrightnessContrastParameters),
    GeglLevels(crate::GeglLevelsParameters),
}
impl Parameters {
    pub fn kind(self) -> Kind {
        match self {
            Self::InvertLinear => Kind::InvertLinear,
            Self::InvertGamma => Kind::InvertGamma,
            Self::BrightnessContrast(_) => Kind::BrightnessContrast,
            Self::GeglLevels(_) => Kind::GeglLevels,
        }
    }
    fn operation(self) -> PointOperation {
        match self {
            Self::InvertLinear | Self::InvertGamma => PointOperation::Invert,
            Self::BrightnessContrast(p) => {
                let [brightness, contrast] = p.values();
                PointOperation::BrightnessContrast { brightness, contrast }
            }
            Self::GeglLevels(p) => {
                let [input_low, input_high, output_low, output_high] = p.values();
                PointOperation::Levels { input_low, input_high, output_low, output_high }
            }
        }
    }
}
pub fn parameters(kind: Kind, args: &[FilterArgument]) -> Result<Parameters, CompositionError> {
    // Fixed name bound precedes old bounded scalar name comparisons; no CMS/gains.
    if args.len() > 4 || args.iter().any(|a| a.name.len() > 16) {
        return Err(shape());
    }
    match kind {
        Kind::InvertLinear if args.is_empty() => Ok(Parameters::InvertLinear),
        Kind::InvertGamma if args.is_empty() => Ok(Parameters::InvertGamma),
        Kind::BrightnessContrast => Ok(Parameters::BrightnessContrast(crate::gimp_affine::brightness_contrast_parameters(args)?)),
        Kind::GeglLevels => Ok(Parameters::GeglLevels(crate::gimp_affine::gegl_levels_parameters(args)?)),
        _ => Err(shape()),
    }
}
pub(crate) fn claims_epoch(epoch: &str) -> bool {
    matches!(epoch, INVERT_LINEAR_EPOCH | INVERT_GAMMA_EPOCH | BRIGHTNESS_CONTRAST_EPOCH | GEGL_LEVELS_EPOCH)
}
pub(crate) fn canonical(node: &FilterNode, extent: photocraft_geom::Rect) -> Result<Parameters, CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    if node.operation.name.len() > 32 || node.operation.schema_epoch.len() > 64 || node.operation.stored_version.len() > 8 || node.arguments.len() > 4 {
        return Err(shape());
    }
    let kind = match node.operation.name.as_str() {
        "gegl:invert-linear" => Kind::InvertLinear,
        "gegl:invert-gamma" => Kind::InvertGamma,
        "gegl:brightness-contrast" => Kind::BrightnessContrast,
        "gegl:levels" => Kind::GeglLevels,
        _ => return Err(shape()),
    };
    if node.operation.stored_version != kind.version()
        || node.operation.schema_epoch != kind.epoch()
        || node.opacity_f64_bits != 1f64.to_bits()
        || node.source_clipping != Some(0)
        || node.source_region != Some(0)
        || !node.auxiliary.is_empty()
        || !matches!(node.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || node.composition.as_ref().is_none_or(|p| {
            p.compatibility_epoch.len() > 64
                || p.compatibility_epoch != kind.epoch()
                || p.source_mode != 62
                || p.legacy
                || p.pass_through
                || p.resolved.is_some()
        })
    {
        return Err(shape());
    }
    parameters(kind, &node.arguments)
}
fn pixel(p: [f64; 4]) -> Result<(), E> {
    if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) {
        return Err(scope());
    }
    Ok(())
}
/// Owns the actual one analytic Linear Space used by both Linear leaves and
/// full mask mixing. Gamma explicitly uses the known profile-encoded copy lane.
pub(crate) struct PreparedMaskedScalar {
    mix: PreparedMix,
    operation: PreparedPointOperation,
    kind: Kind,
}
impl PreparedMaskedScalar {
    pub(crate) fn prepare<B: PlanBudget>(p: Parameters, outer: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        let outer = outer.checked_add(std::mem::size_of::<Self>() as u64).ok_or_else(length)?;
        let fee;
        let mix;
        (mix, fee) = PreparedMix::prepare(outer, i, b)?;
        charge(b, i, 512)?;
        let operation = p.operation().prepare().map_err(|e| E::Color(e.into()))?;
        let result = Self { mix, operation, kind: p.kind() };
        charge(b, i, 0)?;
        Ok((result, fee))
    }
    pub(crate) fn reserve_runtime(&self, q: &BackingQuota, c: &ReadControl<'_>) -> Result<QuotaLease, E> {
        self.mix.reserve_runtime(q, c)
    }
    /// Caller holds the independent1024 mix fee before before/mask decoding.
    pub(crate) fn evaluate(&self, before: [f64; 4], enabled: bool, raw: f64, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        c.check(128).map_err(E::Backing)?;
        pixel(before)?;
        if before.iter().take(3).any(|v| !(0.0..=1.0).contains(v)) {
            return Err(scope());
        }
        crate::gimp_masked_exposure::coverage(raw)?;
        if !enabled {
            c.check(0).map_err(E::Backing)?;
            return Ok(before);
        }
        c.check(if self.kind.linear() { 1408 } else { 640 }).map_err(E::Backing)?;
        let _leaf_scratch = q.reserve(QuotaKind::Resident, LEAF_SCRATCH_BYTES).map_err(E::Quota)?;
        c.check(0).map_err(E::Backing)?;
        let rgb = if self.kind.linear() {
            self.mix.linear_space().convert([before[0], before[1], before[2]], true).map_err(E::Color)?
        } else {
            [before[0], before[1], before[2]]
        };
        let mut leaf = [0.; 4];
        c.check(0).map_err(E::Backing)?;
        // Full lower operation is always invoked; its existing internal scalar
        // identity behavior is not an adapter/native/endpoint bypass.
        self.operation.try_eval_pixel(&[rgb[0], rgb[1], rgb[2], before[3]], 3, &mut leaf).map_err(|e| E::Color(e.into()))?;
        c.check(0).map_err(E::Backing)?;
        let rgb = if self.kind.linear() {
            self.mix.linear_space().convert([leaf[0], leaf[1], leaf[2]], false).map_err(E::Color)?
        } else {
            [leaf[0], leaf[1], leaf[2]]
        };
        let after = [rgb[0], rgb[1], rgb[2], before[3]];
        pixel(after)?;
        self.mix.apply(before, after, raw, c)
    }
}

#[cfg(test)]
#[path = "gimp_masked_scalar_tests.rs"]
mod tests;
