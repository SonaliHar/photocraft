//! Paid mixed affine scalar compatibility; no ordinary Point identity alias or Source authority by name.
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
pub const BRIGHTNESS_CONTRAST_EPOCH: &str = "xcf22-gegl-brightness-contrast-mixed-wide-v1";
pub const GEGL_LEVELS_EPOCH: &str = "xcf22-gegl-levels-mixed-wide-v1";
const INPUT_WORK: u64 = 128;
const TRANSFER_WORK: u64 = 3 * 128;
const SCALAR_WORK: u64 = 3 * 128;
const OUTPUT_WORK: u64 = 128;
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = 512;
fn scope() -> E {
    E::Unsupported("fixed implicit RGB mixed affine policy")
}
fn length() -> E {
    E::Invalid("affine preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    BrightnessContrast,
    GeglLevels,
}
impl Kind {
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::BrightnessContrast => "gegl:brightness-contrast",
            Self::GeglLevels => "gegl:levels",
        }
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::BrightnessContrast => BRIGHTNESS_CONTRAST_EPOCH,
            Self::GeglLevels => GEGL_LEVELS_EPOCH,
        }
    }
    pub fn version(self) -> &'static str {
        "0:0"
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrightnessContrastParameters {
    words: [u32; 2],
}
impl BrightnessContrastParameters {
    pub fn from_words(brightness: u32, contrast: u32) -> Result<Self, CompositionError> {
        let p = Self { words: [brightness, contrast] };
        Parameters::BrightnessContrast(p).operation().validate()?;
        Ok(p)
    }
    pub fn raw_words(self) -> [u32; 2] {
        self.words
    }
    pub fn values(self) -> [f64; 2] {
        self.words.map(|w| f64::from(f32::from_bits(w)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeglLevelsParameters {
    words: [u32; 4],
}
impl GeglLevelsParameters {
    pub fn from_words(words: [u32; 4]) -> Result<Self, CompositionError> {
        let p = Self { words };
        Parameters::GeglLevels(p).operation().validate()?;
        Ok(p)
    }
    pub fn raw_words(self) -> [u32; 4] {
        self.words
    }
    pub fn values(self) -> [f64; 4] {
        self.words.map(|w| f64::from(f32::from_bits(w)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameters {
    BrightnessContrast(BrightnessContrastParameters),
    GeglLevels(GeglLevelsParameters),
}
impl Parameters {
    pub fn kind(self) -> Kind {
        match self {
            Self::BrightnessContrast(_) => Kind::BrightnessContrast,
            Self::GeglLevels(_) => Kind::GeglLevels,
        }
    }
    fn operation(self) -> PointOperation {
        match self {
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
    fn identity(self) -> bool {
        match self {
            Self::BrightnessContrast(p) => {
                let [b, c] = p.values();
                b == 0. && c == 1.
            }
            Self::GeglLevels(p) => p.values() == [0., 1., 0., 1.],
        }
    }
}
fn fields<const N: usize>(args: &[FilterArgument], names: [&str; N]) -> Result<[u32; N], CompositionError> {
    if args.len() != N {
        return Err(CompositionError::Unsupported("mixed affine exact argument count"));
    }
    let mut words = [None; N];
    for arg in args {
        let FilterValue::Number(ExactNumber::Float32Bits(bits)) = arg.value else {
            return Err(CompositionError::Unsupported("mixed affine F32 argument"));
        };
        let mut found = false;
        for (name, slot) in names.iter().zip(words.iter_mut()) {
            if arg.name == *name {
                if slot.replace(bits).is_some() {
                    return Err(CompositionError::Unsupported("mixed affine duplicate argument"));
                }
                found = true;
                break;
            }
        }
        if !found {
            return Err(CompositionError::Unsupported("mixed affine argument name"));
        }
    }
    let mut out = [0; N];
    for (slot, value) in out.iter_mut().zip(words) {
        *slot = value.ok_or(CompositionError::Unsupported("mixed affine missing argument"))?;
    }
    Ok(out)
}
pub fn brightness_contrast_parameters(args: &[FilterArgument]) -> Result<BrightnessContrastParameters, CompositionError> {
    let [b, c] = fields(args, ["brightness", "contrast"])?;
    BrightnessContrastParameters::from_words(b, c)
}
pub fn gegl_levels_parameters(args: &[FilterArgument]) -> Result<GeglLevelsParameters, CompositionError> {
    GeglLevelsParameters::from_words(fields(args, ["in-low", "in-high", "out-low", "out-high"])?)
}
pub(crate) fn claims(n: &FilterNode) -> bool {
    matches!(n.operation.name.as_str(), "gegl:brightness-contrast" | "gegl:levels")
        || matches!(n.operation.schema_epoch.as_str(), BRIGHTNESS_CONTRAST_EPOCH | GEGL_LEVELS_EPOCH)
}
pub(crate) fn canonical(n: &FilterNode, stored: SampleEncoding, version: u32, extent: photocraft_geom::Rect) -> Result<Parameters, CompositionError> {
    canonical_mode(n, stored, version, extent, false)
}
pub(crate) fn canonical_mixed_opacity(
    n: &FilterNode,
    stored: SampleEncoding,
    version: u32,
    extent: photocraft_geom::Rect,
) -> Result<Parameters, CompositionError> {
    canonical_mode(n, stored, version, extent, true)
}
fn canonical_mode(
    n: &FilterNode,
    stored: SampleEncoding,
    version: u32,
    extent: photocraft_geom::Rect,
    allow_opacity: bool,
) -> Result<Parameters, CompositionError> {
    crate::gimp_semi_flatten::positive_origin(extent)?;
    let kind = match n.operation.name.as_str() {
        "gegl:brightness-contrast" => Kind::BrightnessContrast,
        "gegl:levels" => Kind::GeglLevels,
        _ => return Err(CompositionError::Unsupported("mixed affine operation")),
    };
    if version != 22
        || n.operation.stored_version != kind.version()
        || n.operation.schema_epoch != kind.epoch()
        || stored != SampleEncoding::NonlinearProfile
        || if allow_opacity { crate::gimp_mixed_filter_opacity::validate_opacity(n.opacity_f64_bits).is_err() } else { n.opacity_f64_bits != 1f64.to_bits() }
        || n.source_clipping != Some(0)
        || n.source_region != Some(0)
        || !n.auxiliary.is_empty()
        || !matches!(n.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || n.composition
            .as_ref()
            .is_none_or(|p| p.compatibility_epoch != kind.epoch() || p.source_mode != 62 || p.legacy || p.pass_through || p.resolved.is_some())
    {
        return Err(CompositionError::Unsupported("mixed affine canonical envelope"));
    }
    match kind {
        Kind::BrightnessContrast => Ok(Parameters::BrightnessContrast(brightness_contrast_parameters(&n.arguments)?)),
        Kind::GeglLevels => Ok(Parameters::GeglLevels(gegl_levels_parameters(&n.arguments)?)),
    }
}
pub struct PreparedAffine {
    working: WorkingSpace,
    operation: PreparedPointOperation,
    identity: bool,
}
impl PreparedAffine {
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
        let result = Self { working, operation, identity: p.identity() };
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
#[path="gimp_affine_tests.rs"]
mod tests;
