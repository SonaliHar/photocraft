//! Paid mixed adapters for exact stock RGB and checked-finite HSL kernels; no ordinary Point alias.
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

pub const COLOR_TEMPERATURE_EPOCH: &str = "xcf22-gegl-color-temperature-mixed-float-v1";
pub const SEPIA_EPOCH: &str = "xcf22-gegl-sepia-mixed-float-v1";
pub const ALIEN_MAP_RGB_EPOCH: &str = "xcf22-gegl-alien-map-rgb-mixed-float-v1";
pub const ALIEN_MAP_HSL_EPOCH: &str = "xcf22-gegl-alien-map-hsl-mixed-checked-finite-v1";
pub(crate) const CANONICAL_WORK: u64 = 4096;
pub(crate) const PIXEL_SCRATCH_BYTES: u64 = 512;
const INPUT_WORK: u64 = 128;
const OUTPUT_WORK: u64 = 128;
const TRANSFER_WORK: u64 = 384;
fn scope() -> E {
    E::Unsupported("fixed implicit RGB mixed stock policy")
}
fn shape() -> CompositionError {
    CompositionError::Unsupported("mixed stock exact typed parameters")
}
fn length() -> E {
    E::Invalid("mixed stock preparation length")
}
fn charge(b: &mut impl PlanBudget, i: &Interrupt<'_>, n: u64) -> Result<(), E> {
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))?;
    b.check(n)?;
    i.check().map_err(|_| E::Backing(BackingError::Cancelled))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ColorTemperature,
    Sepia,
    AlienRgb,
    AlienHsl,
}
impl Kind {
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::ColorTemperature => "gegl:color-temperature",
            Self::Sepia => "gegl:sepia",
            Self::AlienRgb | Self::AlienHsl => "gegl:alien-map",
        }
    }
    pub fn epoch(self) -> &'static str {
        match self {
            Self::ColorTemperature => COLOR_TEMPERATURE_EPOCH,
            Self::Sepia => SEPIA_EPOCH,
            Self::AlienRgb => ALIEN_MAP_RGB_EPOCH,
            Self::AlienHsl => ALIEN_MAP_HSL_EPOCH,
        }
    }
    pub fn version(self) -> &'static str {
        "0:0"
    }
}
fn domain(word: u32, low: f32, high: f32) -> Result<(), CompositionError> {
    let value = f32::from_bits(word);
    if !value.is_finite() || !(low..=high).contains(&value) {
        return Err(shape());
    }
    Ok(())
}
/// Cheap exact domain/word validation only; actual gain proof is paid preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemperatureParameters {
    words: [u32; 2],
}
impl TemperatureParameters {
    pub fn from_words(original: u32, intended: u32) -> Result<Self, CompositionError> {
        domain(original, 1000., 12000.)?;
        domain(intended, 1000., 12000.)?;
        Ok(Self { words: [original, intended] })
    }
    pub fn raw_words(self) -> [u32; 2] {
        self.words
    }
    pub fn values(self) -> [f64; 2] {
        self.words.map(|w| f64::from(f32::from_bits(w)))
    }
}
/// Cheap fixed shape/domain validation; does not establish an operative recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SepiaParameters {
    scale: u32,
    srgb: bool,
}
impl SepiaParameters {
    pub fn from_words(scale: u32, srgb: bool) -> Result<Self, CompositionError> {
        domain(scale, 0., 1.)?;
        Ok(Self { scale, srgb })
    }
    pub fn raw_scale_word(self) -> u32 {
        self.scale
    }
    pub fn srgb(self) -> bool {
        self.srgb
    }
    pub fn scale(self) -> f64 {
        f64::from(f32::from_bits(self.scale))
    }
}
/// RGB model only. Parsing never evaluates sine or calls the prepared kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlienRgbParameters {
    frequency: [u32; 3],
    phase: [u32; 3],
    keep: [bool; 3],
}
impl AlienRgbParameters {
    pub fn from_words(frequency: [u32; 3], phase: [u32; 3], keep: [bool; 3]) -> Result<Self, CompositionError> {
        for word in frequency {
            domain(word, 0., 20.)?;
        }
        for word in phase {
            domain(word, 0., 360.)?;
        }
        Ok(Self { frequency, phase, keep })
    }
    pub fn raw_words(self) -> ([u32; 3], [u32; 3]) {
        (self.frequency, self.phase)
    }
    pub fn keep(self) -> [bool; 3] {
        self.keep
    }
    pub fn values(self) -> ([f64; 3], [f64; 3]) {
        (self.frequency.map(|w| f64::from(f32::from_bits(w))), self.phase.map(|w| f64::from(f32::from_bits(w))))
    }
}
/// HSL model only. Cheap shape/domain validation is not an operative HSL proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlienHslParameters {
    frequency: [u32; 3],
    phase: [u32; 3],
    keep: [bool; 3],
}
impl AlienHslParameters {
    pub fn from_words(frequency: [u32; 3], phase: [u32; 3], keep: [bool; 3]) -> Result<Self, CompositionError> {
        for word in frequency {
            domain(word, 0., 20.)?;
        }
        for word in phase {
            domain(word, 0., 360.)?;
        }
        Ok(Self { frequency, phase, keep })
    }
    pub fn raw_words(self) -> ([u32; 3], [u32; 3]) {
        (self.frequency, self.phase)
    }
    pub fn keep(self) -> [bool; 3] {
        self.keep
    }
    pub fn values(self) -> ([f64; 3], [f64; 3]) {
        (self.frequency.map(|w| f64::from(f32::from_bits(w))), self.phase.map(|w| f64::from(f32::from_bits(w))))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parameters {
    ColorTemperature(TemperatureParameters),
    Sepia(SepiaParameters),
    AlienRgb(AlienRgbParameters),
    AlienHsl(AlienHslParameters),
}
impl Parameters {
    pub fn kind(self) -> Kind {
        match self {
            Self::ColorTemperature(_) => Kind::ColorTemperature,
            Self::Sepia(_) => Kind::Sepia,
            Self::AlienRgb(_) => Kind::AlienRgb,
            Self::AlienHsl(_) => Kind::AlienHsl,
        }
    }
    fn operation(self) -> PointOperation {
        match self {
            Self::ColorTemperature(p) => {
                let [original, intended] = p.values();
                PointOperation::ColorTemperature { original, intended }
            }
            Self::Sepia(p) => PointOperation::Sepia { scale: p.scale(), srgb: p.srgb() },
            Self::AlienRgb(p) => {
                let (frequency, phase) = p.values();
                PointOperation::AlienMap { frequency, phase, keep: p.keep() }
            }
            Self::AlienHsl(p) => {
                let (frequency, phase) = p.values();
                PointOperation::AlienMapHsl { frequency, phase, keep: p.keep() }
            }
        }
    }
    fn scratch_bytes(self) -> u64 {
        if matches!(self, Self::AlienHsl(_)) { 1024 } else { PIXEL_SCRATCH_BYTES }
    }
    fn working(self) -> GimpSpace {
        match self {
            Self::ColorTemperature(_) => GimpSpace::LinearRgb,
            Self::Sepia(p) if !p.srgb() => GimpSpace::LinearRgb,
            _ => GimpSpace::ProfileRgb,
        }
    }
}
fn fields<'a, const N: usize>(args: &'a [FilterArgument], names: [&str; N]) -> Result<[&'a FilterValue; N], CompositionError> {
    if args.len() != N || args.iter().any(|a| a.name.len() > 32) {
        return Err(shape());
    }
    let mut slots: [Option<&FilterValue>; N] = [None; N];
    for arg in args {
        let mut found = false;
        for (name, slot) in names.iter().zip(slots.iter_mut()) {
            if arg.name == *name {
                if slot.replace(&arg.value).is_some() {
                    return Err(shape());
                }
                found = true;
                break;
            }
        }
        if !found {
            return Err(shape());
        }
    }
    // Fixed references, not an allocated clone of typed arguments.
    let first = args.first().ok_or_else(shape)?;
    let mut out = [&first.value; N];
    for (dst, slot) in out.iter_mut().zip(slots) {
        *dst = slot.ok_or_else(shape)?;
    }
    Ok(out)
}
fn word(value: &FilterValue) -> Result<u32, CompositionError> {
    match value {
        FilterValue::Number(ExactNumber::Float32Bits(w)) => Ok(*w),
        _ => Err(shape()),
    }
}
fn boolean(value: &FilterValue) -> Result<bool, CompositionError> {
    match value {
        FilterValue::Bool(v) => Ok(*v),
        _ => Err(shape()),
    }
}
pub fn color_temperature_parameters(args: &[FilterArgument]) -> Result<TemperatureParameters, CompositionError> {
    let [a, b] = fields(args, ["original-temperature", "intended-temperature"])?;
    TemperatureParameters::from_words(word(a)?, word(b)?)
}
pub fn sepia_parameters(args: &[FilterArgument]) -> Result<SepiaParameters, CompositionError> {
    let [a, b] = fields(args, ["scale", "srgb"])?;
    SepiaParameters::from_words(word(a)?, boolean(b)?)
}
pub fn alien_map_rgb_parameters(args: &[FilterArgument]) -> Result<AlienRgbParameters, CompositionError> {
    let [model, f1, f2, f3, p1, p2, p3, k1, k2, k3] = fields(
        args,
        [
            "color-model",
            "cpn-1-frequency",
            "cpn-2-frequency",
            "cpn-3-frequency",
            "cpn-1-phaseshift",
            "cpn-2-phaseshift",
            "cpn-3-phaseshift",
            "cpn-1-keep",
            "cpn-2-keep",
            "cpn-3-keep",
        ],
    )?;
    if !matches!(model,FilterValue::Enum{domain,value:0}if domain.len()<=32&&domain=="gegl:alien-map/color-model") {
        return Err(shape());
    }
    AlienRgbParameters::from_words([word(f1)?, word(f2)?, word(f3)?], [word(p1)?, word(p2)?, word(p3)?], [boolean(k1)?, boolean(k2)?, boolean(k3)?])
}
pub fn alien_map_hsl_parameters(args: &[FilterArgument]) -> Result<AlienHslParameters, CompositionError> {
    let [model, f1, f2, f3, p1, p2, p3, k1, k2, k3] = fields(
        args,
        [
            "color-model",
            "cpn-1-frequency",
            "cpn-2-frequency",
            "cpn-3-frequency",
            "cpn-1-phaseshift",
            "cpn-2-phaseshift",
            "cpn-3-phaseshift",
            "cpn-1-keep",
            "cpn-2-keep",
            "cpn-3-keep",
        ],
    )?;
    if !matches!(model,FilterValue::Enum{domain,value:1}if domain.len()<=32&&domain=="gegl:alien-map/color-model") {
        return Err(shape());
    }
    AlienHslParameters::from_words([word(f1)?, word(f2)?, word(f3)?], [word(p1)?, word(p2)?, word(p3)?], [boolean(k1)?, boolean(k2)?, boolean(k3)?])
}
pub(crate) fn claims(n: &FilterNode) -> bool {
    matches!(n.operation.name.as_str(), "gegl:color-temperature" | "gegl:sepia" | "gegl:alien-map")
        || matches!(n.operation.schema_epoch.as_str(), COLOR_TEMPERATURE_EPOCH | SEPIA_EPOCH | ALIEN_MAP_RGB_EPOCH | ALIEN_MAP_HSL_EPOCH)
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
    if n.operation.name.len() > 32 || n.operation.schema_epoch.len() > 64 || n.operation.stored_version.len() > 8 || n.arguments.len() > 10 {
        return Err(shape());
    }
    let kind = match n.operation.name.as_str() {
        "gegl:color-temperature" => Kind::ColorTemperature,
        "gegl:sepia" => Kind::Sepia,
        "gegl:alien-map" if n.operation.schema_epoch == ALIEN_MAP_HSL_EPOCH => Kind::AlienHsl,
        "gegl:alien-map" => Kind::AlienRgb,
        _ => return Err(shape()),
    };
    if version != 22
        || stored != SampleEncoding::NonlinearProfile
        || n.operation.stored_version != kind.version()
        || n.operation.schema_epoch != kind.epoch()
        || if allow_opacity { crate::gimp_mixed_filter_opacity::validate_opacity(n.opacity_f64_bits).is_err() } else { n.opacity_f64_bits != 1f64.to_bits() }
        || n.source_clipping != Some(0)
        || n.source_region != Some(0)
        || !n.auxiliary.is_empty()
        || !matches!(n.mask, Some(photocraft_doc::item_identity::ItemRef::Mask(_)))
        || n.composition.as_ref().is_none_or(|p| {
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
    match kind {
        Kind::ColorTemperature => Ok(Parameters::ColorTemperature(color_temperature_parameters(&n.arguments)?)),
        Kind::Sepia => Ok(Parameters::Sepia(sepia_parameters(&n.arguments)?)),
        Kind::AlienRgb => Ok(Parameters::AlienRgb(alien_map_rgb_parameters(&n.arguments)?)),
        Kind::AlienHsl => Ok(Parameters::AlienHsl(alien_map_hsl_parameters(&n.arguments)?)),
    }
}
pub struct PreparedStockRgb {
    working: WorkingSpace,
    operation: PreparedPointOperation,
    parameters: Parameters,
}
impl PreparedStockRgb {
    pub(crate) fn prepare<B: PlanBudget>(p: Parameters, outer: u64, i: &Interrupt<'_>, b: &mut B) -> Result<(Self, B::Lease), E> {
        charge(b, i, 512)?;
        let basis = photocraft_cms::Builtin::Srgb.profile();
        let semantic = photocraft_cms::xcf_color::semantic_analytic_profile_work_ceiling(basis).map_err(|_| scope())?;
        let header = (std::mem::size_of::<Self>() as u64).checked_add(outer).and_then(|n| n.checked_add(p.scratch_bytes() + 128)).ok_or_else(length)?;
        // prepare() calls validate()->gains(), then gains() for its cached result.
        let kernel_work = if matches!(p, Parameters::ColorTemperature(_)) { 32768 } else { 512 };
        charge(
            b,
            i,
            semantic.checked_mul(3).and_then(|n| n.checked_add(header.checked_mul(64)?)).and_then(|n| n.checked_add(512 + kernel_work)).ok_or_else(length)?,
        )?;
        let lease = b.reserve(header)?;
        let operation = p.operation().prepare().map_err(|e| E::Color(e.into()))?;
        let working = WorkingSpace::new_with_interrupt(basis, SampleEncoding::NonlinearProfile, p.working(), i).map_err(E::Color)?;
        let result = Self { working, operation, parameters: p };
        charge(b, i, 0)?;
        Ok((result, lease))
    }
    fn validate_mode(p: [f64; 4], derived: bool) -> Result<(), E> {
        if p.iter().any(|v| !v.is_finite()) || !(0.0..=1.0).contains(&p[3]) || (!derived && p.iter().take(3).any(|v| !(0.0..=1.0).contains(v))) {
            return Err(scope());
        }
        Ok(())
    }
    pub(crate) fn validation_work(&self) -> u64 {
        INPUT_WORK
    }
    pub(crate) fn evaluation_work(&self, enabled: bool) -> u64 {
        if !enabled {
            return INPUT_WORK;
        }
        let kernel = match self.parameters {
            Parameters::ColorTemperature(_) => 384,
            Parameters::Sepia(_) => 768,
            Parameters::AlienRgb(_) => 1536,
            Parameters::AlienHsl(_) => 4096,
        };
        INPUT_WORK + kernel + OUTPUT_WORK + if self.parameters.working() == GimpSpace::LinearRgb { 2 * TRANSFER_WORK } else { 0 }
    }
    fn apply(&self, p: [f64; 4], enabled: bool, derived: bool, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        Self::validate_mode(p, derived)?;
        if !enabled {
            return Ok(p);
        }
        c.check(0).map_err(E::Backing)?;
        let rgb = self.working.convert([p[0], p[1], p[2]], true).map_err(E::Color)?;
        let input = [rgb[0], rgb[1], rgb[2], p[3]];
        let mut output = [0.; 4];
        c.check(0).map_err(E::Backing)?;
        if derived {
            if matches!(self.parameters, Parameters::AlienHsl(_)) {
                self.operation.try_eval_derived_hsl_pixel(&input, &mut output)
            } else {
                self.operation.try_eval_derived_rgb_pixel(&input, &mut output)
            }
        } else {
            self.operation.try_eval_pixel(&input, 3, &mut output)
        }
        .map_err(|e| E::Color(e.into()))?;
        c.check(0).map_err(E::Backing)?;
        let rgb = self.working.convert([output[0], output[1], output[2]], false).map_err(E::Color)?;
        let out = [rgb[0], rgb[1], rgb[2], p[3]];
        Self::validate_mode(out, true)?;
        c.check(0).map_err(E::Backing)?;
        Ok(out)
    }
    pub(crate) fn evaluate(&self, p: [f64; 4], enabled: bool, derived: bool, q: &BackingQuota, c: &ReadControl<'_>) -> Result<[f64; 4], E> {
        c.check(self.validation_work()).map_err(E::Backing)?;
        Self::validate_mode(p, derived)?;
        c.check(self.evaluation_work(enabled)).map_err(E::Backing)?;
        let _scratch = q.reserve(QuotaKind::Resident, self.parameters.scratch_bytes()).map_err(E::Quota)?;
        let result = self.apply(p, enabled, derived, c)?;
        c.check(0).map_err(E::Backing)?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "gimp_stock_rgb_tests.rs"]
mod tests;
