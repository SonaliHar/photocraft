//! Explicit retained Gray Hue/Saturation mathematical candidate, not source admission.
//! The caller independently establishes a Linear Gray or exact-gamma2 Gray input
//! role/profile. Float ingress -> declared IEC perceptual signed ALL lightness ->
//! inverse transfer -> final Float is a numerical/display policy, not a uniquely
//! identified GIMP schedule. The completed collector retains six 1-ULP replay
//! differences and the historical strict HOLD; neither is promoted here.
//! Original 23 words/outer/source/profile backing stay immutable and caller-held.
//! No profile conversion, Gray/RGB alias, neutral raw shortcut or scene enrollment.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
use photocraft_algo::gimp::{gray_hue_saturation_candidate as math, wide_point::WorkingContext};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R};
use std::sync::atomic::{AtomicU64, Ordering};
pub const SCHEMA: &str = "82c932da73a597a0a6e81da7aa286bb130dc377bd1a1ff3e56ef86b2d00ff68f";
const BIND_WORK: u64 = 1024;
const HEADER_ALLOWANCE: u64 = 4096;
pub use math::{InputProfileWitness, InputRole, NumericPolicy};
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub context: WorkingContext,
    pub policy: NumericPolicy,
    pub input_profile: InputProfileWitness,
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
    fn source() -> FilterNode {
        let mut words = [0u64; 23];
        words[1] = 0.25f64.to_bits();
        words[2] = (-0.5f64).to_bits();
        words[3] = 0.25f64.to_bits();
        words[4] = (-0f64).to_bits();
        words[22] = 0.5f64.to_bits();
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:hue-saturation".into(), stored_version: "0:0".into(), schema_epoch: "gray-explicit-caller".into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![FilterArgument {
                name: "config".into(),
                value: FilterValue::Config(
                    FIELDS
                        .into_iter()
                        .zip(words)
                        .map(|(name, b)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(b)) })
                        .collect(),
                ),
            }],
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(1),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    fn binding_for(role: InputRole) -> SourceBinding<'static> {
        let encoding = match role {
            InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
            InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
        };
        SourceBinding {
            xcf_version: 22,
            expected_epoch: "gray-explicit-caller",
            schema_sha256: SCHEMA,
            context: WorkingContext { encoding, profile_identity: [4; 32] },
            policy: NumericPolicy::DeclaredPerceptualIecSignedLightnessFinalFloat,
            input_profile: InputProfileWitness { role, profile_identity: [4; 32] },
        }
    }
    #[test]
    fn independently_declared_roles_use_lower_chain_and_preserve_all_source_words() {
        let original = source();
        let before = original.clone();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F64, true);
        let mut outcomes = Vec::new();
        for role in [InputRole::CallerEstablishedLinearGray, InputRole::CallerEstablishedExactGamma2Gray] {
            let b = binding_for(role);
            let mut budget = Budget::default();
            let prepared = prepare(&original, b, format, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
            let direct = math::PreparedGrayHueSaturation::prepare(
                format,
                b.context,
                parameters(prepared.scalar_bits()),
                b.policy,
                b.input_profile,
                math::Limits::default(),
                &Interrupt::NONE,
            )
            .unwrap();
            let got = prepared.evaluate_gray_with_interrupt(0.25, b.context, b.input_profile, &Interrupt::NONE).unwrap();
            assert_eq!(got.to_bits(), f64::from(direct.try_eval_float_gray(0.25f32).unwrap()).to_bits());
            assert_eq!(prepared.scalar_bits()[4], (-0f64).to_bits());
            assert_eq!(prepared.original_node(), &before);
            assert_eq!(budget.work, BIND_WORK + math::PREPARATION_WORK);
            outcomes.push(got.to_bits());
        }
        assert_ne!(outcomes[0], outcomes[1]);
    }
    #[test]
    fn local_and_profile_role_mismatches_refuse_before_constructor_lease() {
        let mut original = source();
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut b = binding_for(InputRole::CallerEstablishedLinearGray);
        b.input_profile.profile_identity = [5; 32];
        let mut budget = Budget::default();
        assert!(prepare(&original, b, format, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!((budget.bytes, budget.work), (0, 0));
        let b = binding_for(InputRole::CallerEstablishedExactGamma2Gray);
        if let FilterValue::Config(fields) = &mut original.arguments[0].value {
            fields[10].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        }
        assert!(matches!(
            prepare(&original, b, format, Limits::default(), &mut budget, &Interrupt::NONE),
            Err(E::Unsupported("GIMP Gray Hue/Saturation nonneutral local settings unsupported"))
        ));
        assert_eq!((budget.bytes, budget.work), (0, 0));
    }
    #[test]
    fn native_unit_guard_precedes_float_rounding_and_failed_samples_do_not_refund() {
        let original = source();
        let b = binding_for(InputRole::CallerEstablishedLinearGray);
        let mut budget = Budget::default();
        let limits = Limits { work_units: BIND_WORK + math::PREPARATION_WORK + math::SAMPLE_WORK, ..Limits::default() };
        let prepared = prepare(&original, b, PixelFormat::new(ColorMode::Grayscale, SampleType::F64, false), limits, &mut budget, &Interrupt::NONE).unwrap();
        let above = f64::from_bits(1f64.to_bits() + 1);
        assert_eq!(above as f32, 1f32);
        assert!(matches!(prepared.evaluate_gray_with_interrupt(above, b.context, b.input_profile, &Interrupt::NONE), Err(E::Raster(R::InvalidSample))));
        assert_eq!(prepared.requirements().remaining_work_units, 0);
        assert!(prepared.evaluate_gray_with_interrupt(0.25, b.context, b.input_profile, &Interrupt::NONE).is_err());
    }
    #[test]
    fn oversized_inline_lease_and_cancel_never_call_preparation_budget() {
        struct Large {
            called: bool,
        }
        impl PreparationBudget for Large {
            type Lease = [u8; 5000];
            fn reserve_prepared_bytes(&mut self, _: u64) -> Result<Self::Lease> {
                self.called = true;
                Ok([0; 5000])
            }
            fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
                self.called = true;
                Ok(())
            }
        }
        let original = source();
        let b = binding_for(InputRole::CallerEstablishedLinearGray);
        let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut budget = Large { called: false };
        assert!(prepare(&original, b, format, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert!(!budget.called);
        let stop = || true;
        let mut budget = Budget::default();
        assert!(matches!(prepare(&original, b, format, Limits::default(), &mut budget, &Interrupt::cancel_only(&stop)), Err(E::Raster(R::Cancelled))));
        assert_eq!((budget.bytes, budget.work), (0, 0));
    }
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
    bits: [u64; 23],
    kernel: math::PreparedGrayHueSaturation,
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
        math::Error::NonFinite | math::Error::OutsideUnit => R::InvalidSample.into(),
        _ => bad("GIMP Gray Hue/Saturation explicit parameters/profile role unsupported"),
    }
}
/// Strict canonical output of the opt-in type6 storage mapper. Raw text/spans and
/// original outer state remain owned by that mapper; this adds no such authority.
const FIELDS: [&str; 23] = [
    "time",
    "all.hue",
    "all.saturation",
    "all.lightness",
    "red.hue",
    "red.saturation",
    "red.lightness",
    "yellow.hue",
    "yellow.saturation",
    "yellow.lightness",
    "green.hue",
    "green.saturation",
    "green.lightness",
    "cyan.hue",
    "cyan.saturation",
    "cyan.lightness",
    "blue.hue",
    "blue.saturation",
    "blue.lightness",
    "magenta.hue",
    "magenta.saturation",
    "magenta.lightness",
    "overlap",
];
pub fn config_bits(arguments: &[FilterArgument]) -> Result<[u64; 23]> {
    let [argument] = arguments else { return Err(bad("one GIMP Hue/Saturation config")) };
    if argument.name != "config" {
        return Err(bad("GIMP config name"));
    }
    let FilterValue::Config(fields) = &argument.value else { return Err(bad("typed GIMP config")) };
    if fields.len() != 23 {
        return Err(bad("exact GIMP seven-range/time/overlap fields"));
    }
    let mut bits = [0u64; 23];
    for ((field, name), out) in fields.iter().zip(FIELDS).zip(bits.iter_mut()) {
        if field.name != name {
            return Err(bad("exact GIMP config declaration order"));
        }
        let FilterValue::Number(ExactNumber::Float64Bits(word)) = &field.value else { return Err(bad("GIMP config native Float64 words")) };
        *out = *word;
    }
    if bits[0] != 0
        || bits.iter().skip(1).take(21).any(|b| !f64::from_bits(*b).is_finite() || !(-1.0..=1.0).contains(&f64::from_bits(*b)))
        || !f64::from_bits(bits[22]).is_finite()
        || !(0.0..=1.0).contains(&f64::from_bits(bits[22]))
    {
        return Err(bad("explicit positive time0/finite seven-range/overlap configuration"));
    }
    Ok(bits)
}
fn parameters(bits: [u64; 23]) -> math::Parameters {
    let mut ranges = [[0u64; 3]; 7];
    for (range, words) in ranges.iter_mut().zip(bits[1..22].chunks_exact(3)) {
        range.copy_from_slice(words);
    }
    math::Parameters { time_bits: Some(bits[0]), ranges, overlap_bits: bits[22] }
}
fn admitted(l: Limits) -> Result<()> {
    let m = Limits::default();
    if l.resident_bytes > m.resident_bytes || l.work_units > m.work_units {
        return Err(limit());
    }
    Ok(())
}
fn binding(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, c: &Interrupt<'_>) -> Result<[u64; 23]> {
    poll(c)?;
    if b.xcf_version != 22
        || b.schema_sha256 != SCHEMA
        || b.expected_epoch.is_empty()
        || b.expected_epoch.len() > 128
        || b.expected_epoch.contains('\0')
        || node.id.0 == 0
        || node.operation.name != "gimp:hue-saturation"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != b.expected_epoch
        || b.input_profile.profile_identity != b.context.profile_identity
        || b.context.encoding
            != match b.input_profile.role {
                InputRole::CallerEstablishedLinearGray => SampleEncoding::Linear,
                InputRole::CallerEstablishedExactGamma2Gray => SampleEncoding::NonlinearProfile,
            }
        || b.context.profile_identity == [0; 32]
        || format.mode != ColorMode::Grayscale
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(bad("exact retained GIMP identity/schema/declared Gray Float input role/profile"));
    }
    let bits = config_bits(&node.arguments)?;
    if parameters(bits).ranges.iter().skip(1).flatten().any(|word| f64::from_bits(*word) != 0.0) {
        return Err(bad("GIMP Gray Hue/Saturation nonneutral local settings unsupported"));
    }
    Ok(bits)
}
pub fn requirements(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    admitted(limits)?;
    let kernel = math::PreparedGrayHueSaturation::preparation_requirements();
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
    let bits = binding(node, b, format, c)?;
    let lease = budget.reserve_prepared_bytes(q.retained_bytes)?;
    let kernel = math::PreparedGrayHueSaturation::prepare(
        format,
        b.context,
        parameters(bits),
        b.policy,
        b.input_profile,
        math::Limits { resident_bytes: math::PreparedGrayHueSaturation::preparation_requirements().retained_bytes, work_units: math::PREPARATION_WORK },
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
    pub fn policy(&self) -> NumericPolicy {
        self.kernel.policy()
    }
    pub fn input_profile(&self) -> InputProfileWitness {
        self.kernel.input_profile()
    }
    pub fn sample_work(&self) -> u64 {
        self.kernel.sample_work()
    }
    pub fn scalar_bits(&self) -> [u64; 23] {
        self.bits
    }
    pub fn requirements(&self) -> Requirements {
        let mut q = self.requirements;
        q.sample_work_units = self.sample_work();
        q.remaining_work_units = self.limits.work_units.saturating_sub(self.work.load(Ordering::Relaxed));
        q
    }
    /// Both prevalidation and rendering consume SAMPLE_WORK per scalar under
    /// this immutable cumulative ceiling. Original native F64 unit admission
    /// precedes Float narrowing. Raw optional alpha never reaches this method.
    pub fn evaluate_gray_with_interrupt(&self, value: f64, actual: WorkingContext, input_profile: InputProfileWitness, c: &Interrupt<'_>) -> Result<f64> {
        poll(c)?;
        if actual != self.context() || input_profile != self.input_profile() {
            return Err(bad("GIMP Gray working input role/profile changed"));
        }
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(self.sample_work()).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| limit())?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(R::InvalidSample.into());
        }
        let output = self.kernel.try_eval_float_gray(value as f32).map_err(lower)?;
        poll(c)?;
        Ok(f64::from(output))
    }
}
