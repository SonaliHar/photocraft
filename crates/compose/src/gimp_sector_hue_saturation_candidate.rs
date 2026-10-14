//! Additive closed-sector retained GIMP HSL/Float hypothesis consumer.
//! Initial/mixed consumers remain unchanged; output saturation bound policy is explicit.
//! Result saturation clamps0..1 only as the declared equivalent-output schedule.
//! Combined coefficients/lightness/RGB remain unclipped, including finite HDR.
//! No GEGL HueChroma alias, raw-neutral shortcut, source-exact enrollment,
//! default renderer admission, hidden GIMP TRC, ICC alias or profile conversion.
//! Caller owns/retains/debits complete source/AST/mask/profile backing separately;
//! this lease covers known selected config/header/kernel storage, not those owners.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
use photocraft_algo::gimp::{sector_hue_saturation_candidate as math, wide_point::WorkingContext};
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
pub use math::NumericPolicy;
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
    bits: [u64; 23],
    kernel: math::PreparedSectorHueSaturation,
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
        math::Error::UnsupportedConfiguration => bad("GIMP sector Hue/Saturation unsupported named settings or combination"),
        math::Error::UnsupportedHueWrap => bad("GIMP sector Hue/Saturation adjacent mapped hue wrap unsupported"),
        _ => bad("GIMP hue/saturation declared parameters/context"),
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
        || b.context.encoding != SampleEncoding::NonlinearProfile
        || b.context.profile_identity == [0; 32]
        || format.mode != ColorMode::Rgb
        || !matches!(format.sample, SampleType::F32 | SampleType::F64)
    {
        return Err(bad("exact retained GIMP identity/schema/declared nonlinear Float domain"));
    }
    let bits = config_bits(&node.arguments)?;
    Ok(bits)
}
pub fn requirements(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    admitted(limits)?;
    let kernel = math::PreparedSectorHueSaturation::preparation_requirements();
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
        sample_work_units: match b.policy {
            NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl => math::ADJACENT_SAMPLE_WORK,
            NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound | NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound => {
                math::SAMPLE_WORK
            }
        },
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
    let kernel = math::PreparedSectorHueSaturation::prepare(
        format,
        b.context,
        parameters(bits),
        b.policy,
        math::Limits { resident_bytes: math::PreparedSectorHueSaturation::preparation_requirements().retained_bytes, work_units: math::PREPARATION_WORK },
        c,
    )
    .map_err(lower)?;
    let lease = budget.reserve_prepared_bytes(q.retained_bytes)?;
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
    /// Actual immutable numerical policy cost supplied by the prepared kernel.
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
    /// Caller prepays sample_work() for EACH RGB tuple call (prevalidation and
    /// rendering) in its held graph/job ledger. Unit native F64 is checked before
    /// explicit Float ingress/finalFloat widening. Alpha is never interpreted.
    pub fn evaluate_rgb_with_interrupt(&self, value: [f64; 3], actual: WorkingContext, c: &Interrupt<'_>) -> Result<[f64; 3]> {
        poll(c)?;
        if actual != self.context() {
            return Err(bad("GIMP working context changed"));
        }
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(self.sample_work()).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| limit())?;
        if value.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err(R::InvalidSample.into());
        }
        let output = self.kernel.try_eval_float_rgb(value.map(|v| v as f32)).map_err(lower)?;
        poll(c)?;
        Ok(output.map(f64::from))
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
        let mut bits = [0u64; 23];
        bits[1] = 0.125f64.to_bits();
        bits[4] = 0.25f64.to_bits();
        bits[8] = (-0f64).to_bits();
        bits[22] = 0.5f64.to_bits();
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:hue-saturation".into(), stored_version: "0:0".into(), schema_epoch: "caller-retained-82c932".into() },
            source_display_name: None,
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
                    FIELDS
                        .into_iter()
                        .zip(bits)
                        .map(|(name, b)| FilterArgument { name: name.into(), value: FilterValue::Number(ExactNumber::Float64Bits(b)) })
                        .collect(),
                ),
            }],
        }
    }
    fn bound() -> SourceBinding<'static> {
        SourceBinding {
            xcf_version: 22,
            expected_epoch: "caller-retained-82c932",
            schema_sha256: SCHEMA,
            context: WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [7; 32] },
            policy: NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound,
        }
    }
    #[test]
    fn all_raw_config_words_and_existing_math_survive_preparation() {
        let node = node();
        let before = node.clone();
        let mut budget = Budget::default();
        let prepared = prepare(&node, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        let bits = prepared.scalar_bits();
        assert_eq!(bits[8], (-0f64).to_bits());
        let direct = math::PreparedSectorHueSaturation::prepare(
            PixelFormat::RGBA32F,
            bound().context,
            parameters(bits),
            bound().policy,
            math::Limits::default(),
            &Interrupt::NONE,
        )
        .unwrap();
        let input = [1., 0.25, 0.25];
        let got = prepared.evaluate_rgb_with_interrupt(input, bound().context, &Interrupt::NONE).unwrap();
        assert_eq!(got.map(f64::to_bits), direct.try_eval_float_rgb(input.map(|v| v as f32)).unwrap().map(f64::from).map(f64::to_bits));
        assert_eq!(prepared.original_node(), &before);
        assert_eq!(budget.work, prepared.requirements().preparation_work_units);
    }
    #[test]
    fn unsupported_sector_and_wrong_time_width_never_reserve() {
        let mut malformed = node();
        if let FilterValue::Config(fields) = &mut malformed.arguments[0].value {
            fields[7].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        }
        let mut budget = Budget::default();
        assert!(matches!(
            prepare(&malformed, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE),
            Err(E::Unsupported("GIMP sector Hue/Saturation unsupported named settings or combination"))
        ));
        assert_eq!(budget.bytes, 0);
        let mut malformed = node();
        if let FilterValue::Config(fields) = &mut malformed.arguments[0].value {
            fields[0].value = FilterValue::Number(ExactNumber::Float32Bits(0));
        }
        assert!(prepare(&malformed, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
    }
    #[test]
    fn f64_unit_is_checked_before_float_rounding_and_quota_does_not_reset() {
        let node = node();
        let mut budget = Budget::default();
        let limits = Limits { work_units: BIND_WORK + math::PREPARATION_WORK + math::SAMPLE_WORK, ..Limits::default() };
        let prepared = prepare(&node, bound(), PixelFormat { sample: SampleType::F64, ..PixelFormat::RGBA32F }, limits, &mut budget, &Interrupt::NONE).unwrap();
        let above = f64::from_bits(1f64.to_bits() + 1);
        assert_eq!(above as f32, 1.0);
        assert!(matches!(prepared.evaluate_rgb_with_interrupt([above, 0.5, 0.25], bound().context, &Interrupt::NONE), Err(E::Raster(R::InvalidSample))));
        assert_eq!(prepared.requirements().remaining_work_units, 0);
        assert!(matches!(prepared.evaluate_rgb_with_interrupt([0.5, 0.5, 0.25], bound().context, &Interrupt::NONE), Err(E::Raster(R::LimitExceeded))));
    }
    #[test]
    fn lightness_hdr_is_unclipped_and_negative_saturation_uses_explicit_output_floor() {
        let mut light = node();
        if let FilterValue::Config(fields) = &mut light.arguments[0].value {
            for field in fields.iter_mut() {
                field.value = FilterValue::Number(ExactNumber::Float64Bits(0));
            }
            fields[3].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits()));
            fields[6].value = FilterValue::Number(ExactNumber::Float64Bits(1f64.to_bits()));
            fields[22].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits()));
        }
        let before = light.clone();
        let mut budget = Budget::default();
        let prepared = prepare(&light, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.policy(), NumericPolicy::OriginalHslF32SixPhaseOutputSaturationBound);
        let output = prepared.evaluate_rgb_with_interrupt([0.75, 0.25, 0.25], bound().context, &Interrupt::NONE).unwrap();
        assert!(output.iter().any(|v| *v > 1.0));
        assert!(output.iter().all(|v| v.is_finite()));
        assert_eq!(prepared.original_node(), &before);
        let mut saturation = node();
        if let FilterValue::Config(fields) = &mut saturation.arguments[0].value {
            for field in fields.iter_mut() {
                field.value = FilterValue::Number(ExactNumber::Float64Bits(0));
            }
            fields[2].value = FilterValue::Number(ExactNumber::Float64Bits((-1f64).to_bits()));
            fields[5].value = FilterValue::Number(ExactNumber::Float64Bits((-1f64).to_bits()));
            fields[22].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits()));
        }
        let prepared = prepare(&saturation, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        let gray = prepared.evaluate_rgb_with_interrupt([0.75, 0.25, 0.25], bound().context, &Interrupt::NONE).unwrap();
        assert_eq!(gray, [0.5, 0.5, 0.5]);
    }
    #[test]
    fn closed_named_hue_is_reachable_and_unproved_green_or_overlap_refuses() {
        for (field, input) in [(7, [1., 1., 0.]), (13, [0., 1., 1.]), (19, [1., 0., 1.])] {
            let mut named = node();
            if let FilterValue::Config(fields) = &mut named.arguments[0].value {
                for argument in fields.iter_mut() {
                    argument.value = FilterValue::Number(ExactNumber::Float64Bits(0));
                }
                fields[field].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
            }
            let before = named.clone();
            let mut budget = Budget::default();
            let prepared = prepare(&named, bound(), PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
            let output = prepared.evaluate_rgb_with_interrupt(input, bound().context, &Interrupt::NONE).unwrap();
            assert!(output.iter().all(|v| v.is_finite()));
            assert_ne!(output, input);
            assert_eq!(prepared.original_node(), &before);
            drop(prepared);
            if let FilterValue::Config(fields) = &mut named.arguments[0].value {
                fields[22].value = FilterValue::Number(ExactNumber::Float64Bits(0.5f64.to_bits()));
            }
            let mut refused = Budget::default();
            assert!(prepare(&named, bound(), PixelFormat::RGBA32F, Limits::default(), &mut refused, &Interrupt::NONE).is_err());
            assert_eq!(refused.bytes, 0);
        }
        let mut green = node();
        if let FilterValue::Config(fields) = &mut green.arguments[0].value {
            for field in fields.iter_mut() {
                field.value = FilterValue::Number(ExactNumber::Float64Bits(0));
            }
            fields[10].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
        }
        let mut refused = Budget::default();
        assert!(matches!(
            prepare(&green, bound(), PixelFormat::RGBA32F, Limits::default(), &mut refused, &Interrupt::NONE),
            Err(E::Unsupported("GIMP sector Hue/Saturation unsupported named settings or combination"))
        ));
        assert_eq!(refused.bytes, 0);
    }
    #[test]
    fn explicit_new_pure_hue_policy_admits_green_blue_without_changing_old_policy() {
        for (field, input, expected) in [(10, [0.35, 0.8, 0.2], [0.2, 0.8, 0.5]), (16, [0.2, 0.35, 0.8], [0.5, 0.2, 0.8])] {
            let mut original = node();
            if let FilterValue::Config(fields) = &mut original.arguments[0].value {
                for f in fields.iter_mut() {
                    f.value = FilterValue::Number(ExactNumber::Float64Bits(0));
                }
                fields[field].value = FilterValue::Number(ExactNumber::Float64Bits(0.25f64.to_bits()));
            }
            let mut rejected = Budget::default();
            assert!(prepare(&original, bound(), PixelFormat::RGBA32F, Limits::default(), &mut rejected, &Interrupt::NONE).is_err());
            assert_eq!(rejected.bytes, 0);
            let b = SourceBinding { policy: NumericPolicy::OriginalHslF32SixPhaseAllPureHueOutputSaturationBound, ..bound() };
            let mut budget = Budget::default();
            let prepared = prepare(&original, b, PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
            let output = prepared.evaluate_rgb_with_interrupt(input, b.context, &Interrupt::NONE).unwrap();
            assert!(output.iter().zip(expected).all(|(a, b)| (*a - b).abs() < 1e-6));
            assert_eq!(prepared.sample_work(), math::SAMPLE_WORK);
            assert_eq!(prepared.evaluate_rgb_with_interrupt([0.4; 3], b.context, &Interrupt::NONE).unwrap(), [f64::from(0.4f32); 3]);
        }
    }
    #[test]
    fn adjacent_policy_charges_its_actual_cost_and_retains_saturation_lightness_words() {
        let mut original = node();
        if let FilterValue::Config(fields) = &mut original.arguments[0].value {
            for f in fields.iter_mut() {
                f.value = FilterValue::Number(ExactNumber::Float64Bits(0));
            }
            for (i, v) in [(7, 0.125f64), (8, -0.25), (9, 0.25), (10, 0.125), (11, -0.5), (12, -0.25), (22, 0.5)] {
                fields[i].value = FilterValue::Number(ExactNumber::Float64Bits(v.to_bits()));
            }
        }
        let before = original.clone();
        let b = SourceBinding { policy: NumericPolicy::OriginalHslF32SixPhaseAdjacentMappedHsl, ..bound() };
        let limits = Limits { work_units: BIND_WORK + math::PREPARATION_WORK + math::ADJACENT_SAMPLE_WORK, ..Limits::default() };
        let mut budget = Budget::default();
        let prepared = prepare(&original, b, PixelFormat { sample: SampleType::F64, ..PixelFormat::RGBA32F }, limits, &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.sample_work(), 1536);
        assert_eq!(prepared.requirements().sample_work_units, prepared.sample_work());
        let got = prepared.evaluate_rgb_with_interrupt([0.5, 0.75, 0.25], b.context, &Interrupt::NONE).unwrap();
        assert!(got.iter().all(|v| v.is_finite()));
        assert_eq!(prepared.requirements().remaining_work_units, 0);
        assert!(prepared.evaluate_rgb_with_interrupt([0.5, 0.75, 0.25], b.context, &Interrupt::NONE).is_err());
        assert_eq!(prepared.original_node(), &before);
    }
}
