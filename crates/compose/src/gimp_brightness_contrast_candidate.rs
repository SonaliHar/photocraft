//! Explicit retained GIMP wrapper -> DISTINCT analytic Float hypothesis consumer.
//! No GEGL affine/neutral1 alias, raw-neutral shortcut, source-exact enrollment,
//! default renderer admission, hidden GIMP TRC, ICC alias or profile conversion.
//! Caller owns/retains/debits complete source/AST/mask/profile backing separately;
//! this lease covers known selected config/header/kernel storage, not those owners.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
use photocraft_algo::gimp::{brightness_contrast_candidate as math, wide_point::WorkingContext};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R};
use std::sync::atomic::{AtomicU64, Ordering};
pub const SCHEMA: &str = "27409cfe3d3be690580fb52450cf72d17e07c65aecc41c86d20685696f6c7ae1";
pub const SCHEMA_308: &str = "f721196221bd240a6c539f67498831ecdfd5550c99315bc830be445427129fb5";
const BIND_WORK: u64 = 128;
const HEADER_ALLOWANCE: u64 = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericPolicy {
    DeclaredIecFloatHypothesis,
}
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub context: WorkingContext,
    pub policy: NumericPolicy,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: (64 << 20).min(MAX_CHECKED_RASTER_BYTES), work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub preparation_work_units: u64,
    pub sample_work_units: u64,
    pub remaining_work_units: u64,
}
pub struct Prepared<'a, L> {
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    bits: [u64; 3],
    kernel: math::PreparedBrightnessContrast,
    requirements: Requirements,
    limits: Limits,
    work: AtomicU64,
    _lease: L,
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| R::Cancelled.into())
}
fn limit() -> E {
    R::LimitExceeded.into()
}
fn bad(s: &'static str) -> E {
    E::Unsupported(s)
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn lower(e: math::Error) -> E {
    match e {
        math::Error::Raster(r) => r.into(),
        math::Error::NonFinite => R::InvalidSample.into(),
        _ => bad("GIMP brightness/contrast declared parameters/context"),
    }
}
/// Strict canonical output of the opt-in type6 storage mapper. Raw text/spans and
/// original outer state remain owned by that mapper; this adds no such authority.
pub fn config_bits(arguments: &[FilterArgument]) -> Result<[u64; 3]> {
    let [argument] = arguments else { return Err(bad("one GIMP brightness/contrast config")) };
    if argument.name != "config" {
        return Err(bad("GIMP config name"));
    }
    let FilterValue::Config(fields) = &argument.value else { return Err(bad("typed GIMP config")) };
    if fields.len() != 3 {
        return Err(bad("exact GIMP time/brightness/contrast fields"));
    }
    let mut bits = [0; 3];
    for ((field, name), out) in fields.iter().zip(["time", "brightness", "contrast"]).zip(bits.iter_mut()) {
        if field.name != name {
            return Err(bad("exact GIMP config declaration order"));
        }
        let FilterValue::Number(ExactNumber::Float64Bits(word)) = &field.value else { return Err(bad("GIMP config native Float64 words")) };
        *out = *word;
    }
    if bits[0] != 0 || [bits[1], bits[2]].into_iter().any(|b| !f64::from_bits(b).is_finite() || !(-1.0..=1.0).contains(&f64::from_bits(b))) {
        return Err(bad("explicit constant time0 and finite GIMP configuration"));
    }
    Ok(bits)
}
fn admitted(l: Limits) -> Result<()> {
    let m = Limits::default();
    if l.resident_bytes > m.resident_bytes || l.work_units > m.work_units {
        return Err(limit());
    }
    Ok(())
}
fn binding(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, c: &Interrupt<'_>) -> Result<[u64; 3]> {
    poll(c)?;
    if b.xcf_version != 22
        || !matches!(b.schema_sha256, SCHEMA | SCHEMA_308)
        || b.expected_epoch.is_empty()
        || b.expected_epoch.len() > 128
        || b.expected_epoch.contains('\0')
        || node.id.0 == 0
        || node.operation.name != "gimp:brightness-contrast"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != b.expected_epoch
        || b.context.encoding != SampleEncoding::NonlinearProfile
        || b.context.profile_identity == [0; 32]
        || !matches!(format.mode, ColorMode::Rgb | ColorMode::Grayscale)
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(bad("exact retained GIMP identity/schema/declared nonlinear Float domain"));
    }
    let bits = config_bits(&node.arguments)?;
    // Storage legitimately retains +/-1 contrast; this analytic tangent hypothesis
    // does not silently manufacture their singular endpoint interpretation.
    if f64::from_bits(bits[2]) <= -1. || f64::from_bits(bits[2]) >= 1. {
        return Err(bad("GIMP analytic contrast open interval"));
    }
    Ok(bits)
}
pub fn requirements(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    admitted(limits)?;
    let kernel = math::PreparedBrightnessContrast::preparation_requirements();
    if HEADER_ALLOWANCE > limits.resident_bytes || add(BIND_WORK, kernel.work_units)? > limits.work_units {
        return Err(limit());
    }
    binding(node, b, format, c)?;
    let selected = match node.arguments.first().map(|a| &a.value) {
        Some(FilterValue::Config(fields)) => {
            let mut n = add(
                mul(node.arguments.capacity() as u64, std::mem::size_of::<FilterArgument>() as u64)?,
                mul(fields.capacity() as u64, std::mem::size_of::<FilterArgument>() as u64)?,
            )?;
            for arg in node.arguments.iter().chain(fields) {
                n = add(n, arg.name.capacity() as u64)?;
            }
            n
        }
        _ => return Err(bad("GIMP selected config")),
    };
    let retained_bytes = add(add(HEADER_ALLOWANCE, kernel.retained_bytes)?, selected)?;
    if retained_bytes > limits.resident_bytes {
        return Err(limit());
    }
    let preparation_work_units = add(BIND_WORK, kernel.work_units)?;
    poll(c)?;
    Ok(Requirements {
        retained_bytes,
        preparation_work_units,
        sample_work_units: math::SAMPLE_WORK,
        remaining_work_units: limits.work_units - preparation_work_units,
    })
}
pub fn prepare<'a, B: PreparationBudget>(
    node: &'a FilterNode,
    b: SourceBinding<'a>,
    format: PixelFormat,
    limits: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<Prepared<'a, B::Lease>> {
    poll(c)?;
    if std::mem::size_of::<Prepared<'a, B::Lease>>() as u64 > HEADER_ALLOWANCE {
        return Err(limit());
    }
    let q = requirements(node, b, format, limits, c)?;
    budget.consume_preparation_work(q.preparation_work_units)?;
    let lease = budget.reserve_prepared_bytes(q.retained_bytes)?;
    let bits = binding(node, b, format, c)?;
    let kernel = math::PreparedBrightnessContrast::prepare(
        format,
        b.context,
        math::Parameters { brightness: f64::from_bits(bits[1]), contrast: f64::from_bits(bits[2]) },
        math::Limits { resident_bytes: math::PreparedBrightnessContrast::preparation_requirements().retained_bytes, work_units: math::PREPARATION_WORK },
        c,
    )
    .map_err(lower)?;
    poll(c)?;
    Ok(Prepared { node, binding: b, bits, kernel, requirements: q, limits, work: AtomicU64::new(q.preparation_work_units), _lease: lease })
}
impl<L> Prepared<'_, L> {
    pub fn original_node(&self) -> &FilterNode {
        self.node
    }
    pub fn binding(&self) -> SourceBinding<'_> {
        self.binding
    }
    pub fn format(&self) -> PixelFormat {
        self.kernel.format()
    }
    pub fn context(&self) -> WorkingContext {
        self.kernel.context()
    }
    pub fn scalar_bits(&self) -> [u64; 3] {
        self.bits
    }
    pub fn requirements(&self) -> Requirements {
        let mut q = self.requirements;
        q.remaining_work_units = self.limits.work_units.saturating_sub(self.work.load(Ordering::Relaxed));
        q
    }
    /// Caller prepays SAMPLE_WORK for EACH call (including prevalidation calls) in
    /// its held graph/job ledger. Selected F64 deliberately enters Float32; this is
    /// not exact wide/source arithmetic. Raw alpha is not a parameter to this API.
    pub fn evaluate_color_with_interrupt(&self, value: f64, actual: WorkingContext, c: &Interrupt<'_>) -> Result<f64> {
        poll(c)?;
        if actual != self.context() {
            return Err(bad("GIMP working context changed"));
        }
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(math::SAMPLE_WORK).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| limit())?;
        if !value.is_finite() || !(value as f32).is_finite() {
            return Err(R::InvalidSample.into());
        }
        let output = self.kernel.try_eval_float_color(value as f32).map_err(lower)?;
        poll(c)?;
        Ok(f64::from(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{item_identity::FilterId, xcf_filters::OperationIdentity};
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
            self.bytes = add(self.bytes, n)?;
            Ok(())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            self.work = add(self.work, n)?;
            Ok(())
        }
    }
    fn node() -> FilterNode {
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity {
                name: "gimp:brightness-contrast".into(),
                stored_version: "0:0".into(),
                schema_epoch: "caller-retained-27409".into(),
            },
            source_display_name: Some("Original wrapper".into()),
            source_icon: None,
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
            arguments: vec![FilterArgument {
                name: "config".into(),
                value: FilterValue::Config(
                    [("time", 0f64), ("brightness", -0f64), ("contrast", 0.125)]
                        .into_iter()
                        .map(|(name, v)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(v.to_bits())) })
                        .collect(),
                ),
            }],
        }
    }
    fn bound() -> SourceBinding<'static> {
        SourceBinding {
            xcf_version: 22,
            expected_epoch: "caller-retained-27409",
            schema_sha256: SCHEMA,
            context: WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] },
            policy: NumericPolicy::DeclaredIecFloatHypothesis,
        }
    }
    #[test]
    fn original_config_bits_outer_metadata_and_kernel_schedule_remain_distinct() {
        let node = node();
        let before = node.clone();
        let mut budget = Budget::default();
        let prepared = prepare(&node, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.scalar_bits(), [0, (-0f64).to_bits(), 0.125f64.to_bits()]);
        let direct = math::PreparedBrightnessContrast::prepare(
            PixelFormat::RGBA32F,
            bound().context,
            math::Parameters { brightness: -0., contrast: 0.125 },
            math::Limits::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let got = prepared.evaluate_color_with_interrupt(0.25, bound().context, &Interrupt::NONE).unwrap();
        assert_eq!(got.to_bits(), f64::from(direct.try_eval_float_color(0.25).unwrap()).to_bits());
        assert_eq!(prepared.original_node(), &before);
        assert_eq!(budget.work, prepared.requirements().preparation_work_units);
    }
    #[test]
    fn malformed_time_width_and_large_inline_lease_refuse_before_reserve() {
        let mut malformed = node();
        if let FilterValue::Config(fields) = &mut malformed.arguments[0].value {
            fields[0].value = FilterValue::Number(ExactNumber::Float32Bits(0));
        }
        let mut budget = Budget::default();
        assert!(prepare(&malformed, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
        struct Large;
        impl PreparationBudget for Large {
            type Lease = [u8; 8192];
            fn reserve_prepared_bytes(&mut self, _: u64) -> Result<Self::Lease> {
                panic!("must refuse before reserve")
            }
            fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
                panic!("must refuse before work")
            }
        }
        let good = node();
        assert!(prepare(&good, bound(), PixelFormat::RGBA32F, Limits::default(), &mut Large, &Interrupt::NONE).is_err());
    }
    #[test]
    fn two_scalar_phases_share_quota_and_f64_ingress_overflow_refuses() {
        let node = node();
        let mut budget = Budget::default();
        let limits = Limits { work_units: BIND_WORK + math::PREPARATION_WORK + math::SAMPLE_WORK, ..Limits::default() };
        let prepared = prepare(&node, bound(), PixelFormat { sample: SampleType::F64, ..PixelFormat::RGBA32F }, limits, &mut budget, &Interrupt::NONE).unwrap();
        assert!(matches!(prepared.evaluate_color_with_interrupt(f64::MAX, bound().context, &Interrupt::NONE), Err(E::Raster(R::InvalidSample))));
        assert_eq!(prepared.requirements().remaining_work_units, 0);
        assert!(matches!(prepared.evaluate_color_with_interrupt(0.25, bound().context, &Interrupt::NONE), Err(E::Raster(R::LimitExceeded))));
    }
}
