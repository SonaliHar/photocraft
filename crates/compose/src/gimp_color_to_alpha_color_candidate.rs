//! Explicit stored-linear color -> declared nonlinear working-color candidate.
//! Profiles are caller-interpreted effective definitions, not defaults inferred
//! from empty type8 ICC framing, names, native image encoding or source bytes.
//! Only RGB literal components are converted. Alpha/threshold words and original
//! records remain exact. No normal IO/PointStack/source qualification is changed.
//! Profile identities below are SHA256 of fresh canonical writer representation;
//! they are not original-file byte identities or arbitrary profile equivalence.
//! The enclosing owner retains/charges all borrowed raw/source/context backing.
use crate::gimp_composite::{CompositionError, Result};
use crate::gimp_render::PreparationBudget;
use photocraft_algo::gimp::color_to_alpha::ColorToAlpha;
use photocraft_cms::{ColorSpace, Curve, Intent, Pcs, Profile, Transform, control, transform::TransformOptions, xcf_color};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterValue},
};
use photocraft_raster::{Interrupt, RasterInputError};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub literal_encoding: SampleEncoding,
    pub literal_definition_sha256: [u8; 32],
    pub operation_encoding: SampleEncoding,
    pub operation_definition_sha256: [u8; 32],
}
#[derive(Clone, Copy)]
pub struct Binding<'a> {
    pub operation: &'a str,
    pub version: &'a str,
    pub schema_epoch: &'a str,
    pub arguments: &'a [FilterArgument],
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: (64 << 20).min(control::Limits::default().resident_bytes as u64), work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
struct ProfilePhase {
    peak: u64,
    work: u64,
    clone_work: u64,
    encode_work: u64,
    hash_bound: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub peak_resident_bytes: u64,
    pub work_units: u64,
    pub constructor_peak_bytes: u64,
    pub constructor_work_units: u64,
    profiles: [ProfilePhase; 2],
}
pub struct Prepared<'a, L> {
    original: Binding<'a>,
    literal_profile: &'a Profile,
    operation_profile: &'a Profile,
    context: Context,
    transform: Transform,
    stored_color: [u32; 4],
    thresholds: [u32; 2],
    kernel: ColorToAlpha,
    requirements: Requirements,
    limits: Limits,
    work: AtomicU64,
    _lease: L,
}
impl<L> Prepared<'_, L> {
    pub fn original(&self) -> Binding<'_> {
        self.original
    }
    pub fn literal_profile(&self) -> &Profile {
        self.literal_profile
    }
    pub fn operation_profile(&self) -> &Profile {
        self.operation_profile
    }
    /// Retained controlled parameter converter; no native image ingress authority.
    pub fn conversion(&self) -> &Transform {
        &self.transform
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn stored_color_bits(&self) -> [u32; 4] {
        self.stored_color
    }
    pub fn threshold_bits(&self) -> [u32; 2] {
        self.thresholds
    }
    pub fn working_color_bits(&self) -> [u32; 4] {
        self.kernel.color.map(f32::to_bits)
    }
    pub fn requirements(&self) -> Requirements {
        self.requirements
    }
    /// Prospective read-only ceiling for a consumer ROI. Concurrent calls may
    /// consume it afterward; each evaluation still performs atomic admission.
    pub fn remaining_evaluation_work(&self) -> u64 {
        self.limits.work_units.saturating_sub(self.work.load(Ordering::Relaxed))
    }
    /// Input is already caller-converted nonlinear RGB/unit alpha. This never
    /// converts native image pixels or claims an operation's hidden GIMP context.
    /// The supplied budget must be the same held enclosing ledger; per-pixel work
    /// is paid before evaluation and is never reset/refunded on failure.
    pub fn evaluate<B: PreparationBudget>(&self, input: &[f64], actual: Context, budget: &mut B, c: &Interrupt<'_>) -> Result<[f64; 4]> {
        poll(c)?;
        if actual != self.context {
            return Err(refused("working context changed"));
        }
        budget.consume_preparation_work(256)?;
        self.work.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(256).filter(|n| *n <= self.limits.work_units)).map_err(|_| limit())?;
        let mut output = [0.0; 4];
        self.kernel.try_eval_pixel(input, 3, &mut output)?;
        poll(c)?;
        Ok(output)
    }
}
fn refused(s: &'static str) -> CompositionError {
    CompositionError::Unsupported(s)
}
fn limit() -> CompositionError {
    RasterInputError::LimitExceeded.into()
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(limit)
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(limit)
}
fn poll(c: &Interrupt<'_>) -> Result<()> {
    c.check().map_err(|_| RasterInputError::Cancelled.into())
}
fn cms(e: control::Error) -> CompositionError {
    match e {
        control::Error::Cancelled => RasterInputError::Cancelled.into(),
        control::Error::Allocation => RasterInputError::AllocationFailed.into(),
        control::Error::Limit(_) => limit(),
        control::Error::Cms(e) => e.into(),
    }
}
fn caps(bytes: u64, work: u64) -> Result<control::Limits> {
    let max = control::Limits::default();
    if bytes > max.resident_bytes as u64 || work > max.work_units {
        return Err(limit());
    }
    Ok(control::Limits { resident_bytes: usize::try_from(bytes).map_err(|_| limit())?, work_units: work })
}
fn options() -> TransformOptions {
    TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, precise_float: true }
}
fn typed(binding: Binding<'_>, context: Context, c: &Interrupt<'_>) -> Result<([u32; 4], [u32; 2], u64)> {
    poll(c)?;
    if binding.operation != "gegl:color-to-alpha"
        || binding.version != "0:0"
        || binding.schema_epoch.is_empty()
        || binding.schema_epoch.len() > 256
        || binding.arguments.len() != 3
        || context.literal_encoding != SampleEncoding::Linear
        || context.operation_encoding != SampleEncoding::NonlinearProfile
        || context.literal_definition_sha256 == [0; 32]
        || context.operation_definition_sha256 == [0; 32]
    {
        return Err(refused("explicit Color to Alpha identity/contexts"));
    }
    let mut color = None;
    let mut t = [None, None];
    let mut held = (3 * std::mem::size_of::<FilterArgument>()) as u64;
    held = add(held, binding.schema_epoch.len() as u64 + binding.operation.len() as u64 + binding.version.len() as u64)?;
    for a in binding.arguments {
        poll(c)?;
        if a.name.len() > 64 {
            return Err(refused("literal argument name"));
        }
        held = add(held, a.name.capacity() as u64)?;
        match (a.name.as_str(), &a.value) {
            ("color", FilterValue::Color { components, encoding: SampleEncoding::Linear, profile: None }) if components.len() == 4 => {
                if color.is_some() {
                    return Err(refused("duplicate linear literal"));
                }
                held = add(held, mul(components.capacity() as u64, std::mem::size_of::<ExactNumber>() as u64)?)?;
                let mut b = [0u32; 4];
                for (v, n) in b.iter_mut().zip(components) {
                    let ExactNumber::Float32Bits(bits) = n else {
                        return Err(refused("literal native component width"));
                    };
                    let value = f32::from_bits(*bits);
                    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                        return Err(refused("literal finite unit domain"));
                    }
                    *v = *bits;
                }
                color = Some(b);
            }
            (name @ ("transparency-threshold" | "opacity-threshold"), FilterValue::Number(ExactNumber::Float32Bits(bits))) => {
                let i = usize::from(name == "opacity-threshold");
                let v = f32::from_bits(*bits);
                if t[i].is_some() || !v.is_finite() || !(0.0..=1.0).contains(&v) {
                    return Err(refused("threshold exact type/domain"));
                }
                t[i] = Some(*bits);
            }
            _ => return Err(refused("typed linear Color to Alpha arguments")),
        }
    }
    Ok((
        color.ok_or_else(|| refused("missing literal"))?,
        [t[0].ok_or_else(|| refused("missing transparency"))?, t[1].ok_or_else(|| refused("missing opacity"))?],
        held,
    ))
}
fn profile_shape(profile: &Profile, linear: bool, c: &Interrupt<'_>) -> Result<()> {
    poll(c)?;
    if profile.color_space != ColorSpace::Rgb
        || profile.pcs != Pcs::Xyz
        || profile.gray_trc.is_some()
        || profile.a2b.iter().chain(&profile.b2a).any(Option::is_some)
    {
        return Err(refused("explicit RGB matrix/TRC profile domain"));
    }
    let curves = profile.trc.as_ref().ok_or_else(|| refused("explicit RGB TRCs"))?;
    for curve in curves {
        poll(c)?;
        if linear && !matches!(curve, Curve::Identity) {
            return Err(refused("literal effective profile is not Linear"));
        }
        match curve {
            Curve::Identity => (),
            Curve::Gamma(v) if v.is_finite() => (),
            Curve::Parametric { kind, p } if *kind <= 4 && p.iter().all(|v| v.is_finite()) => (),
            Curve::Table(v) if v.len() <= 4096 && v.iter().all(|x| x.is_finite()) => (),
            Curve::Table64(v) if v.len() <= 4096 && v.iter().all(|x| x.is_finite()) => (),
            _ => return Err(refused("bounded finite RGB TRCs")),
        }
    }
    if profile.matrix.as_ref().is_none_or(|m| m.iter().flatten().any(|v| !v.is_finite()))
        || profile.chad.as_ref().is_some_and(|m| m.iter().flatten().any(|v| !v.is_finite()))
    {
        return Err(refused("finite RGB matrix"));
    }
    poll(c)
}
const PREPARED_HEADER_BYTES: u64 = 4096;
const VALIDATION_WORK: u64 = 165376;
// Includes bounded shape/numeric scans and repeated borrowed requirement census.
const SHAPE_WORK: u64 = 32768;
fn profile_requirements(p: &Profile, l: control::Limits, c: &Interrupt<'_>) -> Result<ProfilePhase> {
    let cancelled = || c.cancelled();
    xcf_color::validate_profile_with_control(p, caps(l.resident_bytes as u64, VALIDATION_WORK)?, &cancelled).map_err(cms)?;
    let clone = p.clone_requirements_with_control(l, &cancelled).map_err(cms)?;
    let enc = p.encode_requirements_with_control(l, &cancelled).map_err(cms)?;
    let peak = add(add(clone.retained_bytes as u64, clone.cloned_model_bytes as u64)?, enc.peak_resident_bytes as u64)?;
    let clone_work = mul(clone.work_units, 3)?;
    let encode_work = mul(enc.work_units, 3)?;
    let work = add(add(clone_work, encode_work)?, add(add(VALIDATION_WORK, SHAPE_WORK)?, mul(enc.encoded_bytes_upper_bound as u64, 2)?)?)?;
    Ok(ProfilePhase { peak, work, clone_work, encode_work, hash_bound: enc.encoded_bytes_upper_bound as u64 })
}
pub fn requirements(binding: Binding<'_>, literal: &Profile, operation: &Profile, context: Context, limits: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    let max = Limits::default();
    if limits.resident_bytes > max.resident_bytes || limits.work_units > max.work_units {
        return Err(limit());
    }
    let (_, _, held) = typed(binding, context, c)?;
    let held = add(held, PREPARED_HEADER_BYTES)?;
    if held >= limits.resident_bytes || limits.work_units < 2 * (VALIDATION_WORK + SHAPE_WORK) + 1024 {
        return Err(limit());
    }
    profile_shape(literal, true, c)?;
    profile_shape(operation, false, c)?;
    let remaining = caps(limits.resident_bytes - held, limits.work_units)?;
    let a = profile_requirements(literal, remaining, c)?;
    let b = profile_requirements(operation, remaining, c)?;
    let profile_peak = add(a.peak, b.peak)?;
    let profile_work = add(a.work, b.work)?;
    let constructor = Transform::requirements_with_control(
        literal,
        operation,
        options(),
        caps(limits.resident_bytes.checked_sub(add(held, profile_peak)?).ok_or_else(limit)?, limits.work_units.checked_sub(profile_work).ok_or_else(limit)?)?,
        &|| c.cancelled(),
    )
    .map_err(cms)?;
    let peak = add(add(held, profile_peak)?, constructor.peak_resident_bytes as u64)?;
    let work = add(profile_work, add(mul(constructor.work_units, 3)?, 1024)?)?;
    if peak > limits.resident_bytes || work > limits.work_units {
        return Err(limit());
    }
    poll(c)?;
    Ok(Requirements {
        peak_resident_bytes: peak,
        work_units: work,
        constructor_peak_bytes: constructor.peak_resident_bytes as u64,
        constructor_work_units: mul(constructor.work_units, 3)?,
        profiles: [a, b],
    })
}
fn verify_definition(profile: &Profile, expected: [u8; 32], q: ProfilePhase, c: &Interrupt<'_>) -> Result<()> {
    let cancelled = || c.cancelled();
    let copy = profile.try_clone_with_control(caps(q.peak, q.clone_work)?, &cancelled).map_err(cms)?;
    let fresh = copy.with_encoded_bytes_with_control(caps(q.peak, q.encode_work)?, &cancelled).map_err(cms)?;
    let bytes = fresh.encoded_bytes().ok_or_else(|| refused("canonical profile encoding"))?;
    if bytes.len() as u64 > q.hash_bound {
        return Err(limit());
    }
    let mut digest = Sha256::new();
    for chunk in bytes.chunks(4096) {
        poll(c)?;
        digest.update(chunk);
    }
    if <[u8; 32]>::from(digest.finalize()) != expected {
        return Err(refused("declared effective profile definition changed"));
    }
    poll(c)
}
/// Literal and operation effective profiles are interpreted by the caller first.
/// This prepares only parameter conversion, not a native image conversion path.
pub fn prepare<'a, B: PreparationBudget>(
    binding: Binding<'a>,
    literal: &'a Profile,
    operation: &'a Profile,
    context: Context,
    limits: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<Prepared<'a, B::Lease>> {
    poll(c)?;
    // The enclosing lease is arbitrary caller-owned inline data, not necessarily
    // a pointer or ZST. Refuse before reservations/copies if the actual prepared
    // header exceeds the immutable allowance included by requirements().
    if std::mem::size_of::<Prepared<'a, B::Lease>>() as u64 > PREPARED_HEADER_BYTES {
        return Err(limit());
    }
    let q = requirements(binding, literal, operation, context, limits, c)?;
    budget.consume_preparation_work(q.work_units)?;
    let lease = budget.reserve_prepared_bytes(q.peak_resident_bytes)?;
    verify_definition(literal, context.literal_definition_sha256, q.profiles[0], c)?;
    verify_definition(operation, context.operation_definition_sha256, q.profiles[1], c)?;
    let transform =
        Transform::with_options_with_control(literal, operation, options(), caps(q.constructor_peak_bytes, q.constructor_work_units)?, &|| c.cancelled())
            .map_err(cms)?;
    if transform.retained_bytes_with_control(caps(q.constructor_peak_bytes, q.constructor_work_units)?, &|| c.cancelled()).map_err(cms)? as u64
        > q.constructor_peak_bytes
    {
        return Err(limit());
    }
    let (bits, t, _) = typed(binding, context, c)?;
    let rgba = bits.map(f32::from_bits);
    let mut rgb = [0.0; 3];
    let input = [f64::from(rgba[0]), f64::from(rgba[1]), f64::from(rgba[2])];
    transform.try_eval_f64(&input, &mut rgb)?;
    if rgb.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(refused("converted literal finite unit domain"));
    }
    let converted = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, rgba[3]];
    let kernel = ColorToAlpha { color: converted, transparency: f32::from_bits(t[0]), opacity: f32::from_bits(t[1]) };
    kernel.validate()?;
    poll(c)?;
    Ok(Prepared {
        original: binding,
        literal_profile: literal,
        operation_profile: operation,
        context,
        transform,
        stored_color: bits,
        thresholds: t,
        kernel,
        requirements: q,
        limits,
        work: AtomicU64::new(q.work_units),
        _lease: lease,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> Vec<FilterArgument> {
        vec![
            FilterArgument {
                name: "color".into(),
                value: FilterValue::Color {
                    components: [0.25f32, 0.5, 0.75, 0.5].map(|v| ExactNumber::Float32Bits(v.to_bits())).to_vec(),
                    encoding: SampleEncoding::Linear,
                    profile: None,
                },
            },
            FilterArgument { name: "transparency-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.125f32.to_bits())) },
            FilterArgument { name: "opacity-threshold".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.875f32.to_bits())) },
        ]
    }
    #[test]
    fn typed_linear_is_not_relabelled_and_context_is_mandatory() {
        let a = args();
        let binding = Binding { operation: "gegl:color-to-alpha", version: "0:0", schema_epoch: "explicit-candidate", arguments: &a };
        let c = Context {
            literal_encoding: SampleEncoding::Linear,
            literal_definition_sha256: [1; 32],
            operation_encoding: SampleEncoding::NonlinearProfile,
            operation_definition_sha256: [2; 32],
        };
        let (bits, t, _) = typed(binding, c, &Interrupt::NONE).unwrap();
        assert_eq!(bits[3], 0.5f32.to_bits());
        assert_eq!(t, [0.125f32.to_bits(), 0.875f32.to_bits()]);
        let mut wrong = c;
        wrong.literal_encoding = SampleEncoding::NonlinearProfile;
        assert!(typed(binding, wrong, &Interrupt::NONE).is_err());
        assert!(matches!(typed(binding, c, &Interrupt::cancel_only(&|| true)), Err(CompositionError::Raster(RasterInputError::Cancelled))));
    }
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, bytes: u64) -> Result<()> {
            self.bytes = add(self.bytes, bytes)?;
            Ok(())
        }
        fn consume_preparation_work(&mut self, work: u64) -> Result<()> {
            self.work = add(self.work, work)?;
            Ok(())
        }
    }
    fn definitions() -> (Profile, Profile, Context) {
        let nonlinear = photocraft_cms::Builtin::Srgb.profile().clone();
        let mut linear = nonlinear.clone();
        linear.trc = Some([Curve::Identity, Curve::Identity, Curve::Identity]);
        let digest = |p: &Profile| {
            let fresh = p.clone().with_encoded_bytes_with_control(control::Limits::default(), &|| false).unwrap();
            <[u8; 32]>::from(Sha256::digest(fresh.encoded_bytes().unwrap()))
        };
        let context = Context {
            literal_encoding: SampleEncoding::Linear,
            literal_definition_sha256: digest(&linear),
            operation_encoding: SampleEncoding::NonlinearProfile,
            operation_definition_sha256: digest(&nonlinear),
        };
        (linear, nonlinear, context)
    }
    #[test]
    fn conversion_preserves_literal_alpha_thresholds_and_original_models() {
        let args = args();
        let binding = Binding { operation: "gegl:color-to-alpha", version: "0:0", schema_epoch: "caller-explicit", arguments: &args };
        let (linear, nonlinear, context) = definitions();
        let original_trcs = linear.trc.clone();
        let original_bytes = linear.encoded_bytes().map(<[u8]>::to_vec);
        let mut budget = Budget::default();
        let prepared = prepare(binding, &linear, &nonlinear, context, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
        assert_eq!(prepared.working_color_bits()[3], 0.5f32.to_bits());
        assert_eq!(prepared.threshold_bits(), [0.125f32.to_bits(), 0.875f32.to_bits()]);
        assert_ne!(prepared.working_color_bits()[0], prepared.stored_color_bits()[0]);
        assert_eq!(linear.trc, original_trcs);
        assert_eq!(linear.encoded_bytes(), original_bytes.as_deref());
        assert_eq!(budget.bytes, prepared.requirements().peak_resident_bytes);
        assert!(prepared.evaluate(&[0.25, 0.5, 0.75, 0.5], context, &mut budget, &Interrupt::NONE).is_ok());
        let mut stale = context;
        stale.literal_definition_sha256[0] ^= 1;
        assert!(prepared.evaluate(&[0.25, 0.5, 0.75, 0.5], stale, &mut budget, &Interrupt::NONE).is_err());
    }
    #[test]
    fn malformed_profile_and_tiny_envelope_refuse_before_prepared_reservation() {
        let args = args();
        let binding = Binding { operation: "gegl:color-to-alpha", version: "0:0", schema_epoch: "caller-explicit", arguments: &args };
        let (linear, mut nonlinear, context) = definitions();
        let mut budget = Budget::default();
        assert!(prepare(binding, &linear, &nonlinear, context, Limits { resident_bytes: 0, ..Limits::default() }, &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
        nonlinear.trc = Some([Curve::Gamma(f64::NAN), Curve::Identity, Curve::Identity]);
        assert!(prepare(binding, &linear, &nonlinear, context, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
        assert!(matches!(
            prepare(binding, &linear, &nonlinear, context, Limits::default(), &mut budget, &Interrupt::cancel_only(&|| true)),
            Err(CompositionError::Raster(RasterInputError::Cancelled))
        ));
    }
    #[test]
    fn oversized_inline_lease_refuses_before_reservation_or_profile_copy() {
        struct LargeLeaseBudget {
            called: bool,
        }
        impl PreparationBudget for LargeLeaseBudget {
            type Lease = [u8; 8192];
            fn reserve_prepared_bytes(&mut self, _: u64) -> Result<Self::Lease> {
                self.called = true;
                Ok([0; 8192])
            }
            fn consume_preparation_work(&mut self, _: u64) -> Result<()> {
                self.called = true;
                Ok(())
            }
        }
        let args = args();
        let binding = Binding { operation: "gegl:color-to-alpha", version: "0:0", schema_epoch: "caller-explicit", arguments: &args };
        let (linear, nonlinear, context) = definitions();
        let mut budget = LargeLeaseBudget { called: false };
        assert!(matches!(
            prepare(binding, &linear, &nonlinear, context, Limits::default(), &mut budget, &Interrupt::NONE),
            Err(CompositionError::Raster(RasterInputError::LimitExceeded))
        ));
        assert!(!budget.called);
    }
}
