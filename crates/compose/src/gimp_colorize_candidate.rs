//! Explicit retained Colorize scalar-authority mathematical consumer. Stored
//! HSL bytes and scalar words remain independent. Transfer role and Y weights
//! are supplied separately; the current ICC never supplies either by inference.
//! One lease covers known selected argument/header/kernel capacities; whole
//! source/AST/profile/mask/pruned backing must remain caller held/prepaid. No
//! source-exact, hidden GIMP arithmetic, mask/outer/default enrollment is granted.
use crate::{
    gimp_composite::{CompositionError as E, Result},
    gimp_render::PreparationBudget,
};
pub use math::{NumericPolicy, Parameters, TransferRole, TransferWitness, YWeights};
use photocraft_algo::gimp::{colorize_candidate as math, wide_point::WorkingContext};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    SampleEncoding,
    xcf_filters::{ExactNumber, FilterArgument, FilterNode, FilterValue},
};
use photocraft_raster::{Interrupt, MAX_CHECKED_RASTER_BYTES, RasterInputError as R};
use std::sync::atomic::{AtomicU64, Ordering};
pub const EPOCH: &str = "gimp-3.2.6-windows-03f514419a64c129";
pub const SCHEMA: &str = "03f514419a64c129d201b99b2853628e70a07b8947568339526d440c62c6a98c";
const HEADER: u64 = 4096;
const BIND_WORK: u64 = 128;
#[derive(Clone, Copy)]
pub struct SourceBinding<'a> {
    pub xcf_version: u32,
    pub expected_epoch: &'a str,
    pub schema_sha256: &'a str,
    pub context: WorkingContext,
    pub policy: NumericPolicy,
    pub transfer: TransferWitness,
    pub weights: YWeights,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub resident_bytes: u64,
    pub work_units: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self { resident_bytes: MAX_CHECKED_RASTER_BYTES, work_units: 1_000_000_000 }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    pub retained_bytes: u64,
    pub peak_resident_bytes: u64,
    pub index_bytes: u64,
    pub preparation_work_units: u64,
    pub sample_work_units: u64,
    pub remaining_work_units: u64,
}
pub struct Prepared<'a, L> {
    node: &'a FilterNode,
    binding: SourceBinding<'a>,
    kernel: math::PreparedColorize,
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
fn lower(e: math::Error) -> E {
    match e {
        math::Error::Raster(r) => r.into(),
        math::Error::NonFinite | math::Error::OutsideUnit => R::InvalidSample.into(),
        _ => bad("declared Colorize context/format"),
    }
}
/// Exact independent retained type3 scalars and raw HSL Float words. Original
/// declaration order remains in the borrowed node; neither triplet is derived.
pub fn stored_parameters(arguments: &[FilterArgument]) -> Result<Parameters> {
    if arguments.len() != 4 {
        return Err(bad("four exact Colorize arguments"));
    }
    let mut scalar = [0u32; 3];
    let mut hsl = [0u32; 3];
    let mut seen = 0u8;
    for a in arguments {
        let slot = match a.name.as_str() {
            "hue" => 0,
            "saturation" => 1,
            "lightness" => 2,
            "color" => 3,
            _ => return Err(bad("known Colorize argument")),
        };
        let bit = 1u8 << slot;
        if seen & bit != 0 {
            return Err(bad("duplicate Colorize argument"));
        }
        seen |= bit;
        if slot < 3 {
            let FilterValue::Number(ExactNumber::Float32Bits(word)) = &a.value else {
                return Err(bad("original Colorize Float32 scalar"));
            };
            *scalar.get_mut(slot).ok_or_else(limit)? = *word;
        } else {
            let FilterValue::StoredColor(storage) = &a.value else {
                return Err(bad("original raw HSL StoredColor"));
            };
            if storage.layout != "HSL float" || storage.samples.len() != 12 || !storage.opaque_tail.is_empty() {
                return Err(bad("observed exact HSL Float storage framing"));
            }
            for (out, bytes) in hsl.iter_mut().zip(storage.samples.chunks_exact(4)) {
                *out = u32::from_le_bytes(bytes.try_into().map_err(|_| bad("HSL native word width"))?);
            }
        }
    }
    Parameters::from_stored_bits(scalar, hsl).map_err(lower)
}
fn bind(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, c: &Interrupt<'_>) -> Result<Parameters> {
    poll(c)?;
    if b.xcf_version != 22
        || b.schema_sha256 != SCHEMA
        || b.expected_epoch != EPOCH
        || node.id.0 == 0
        || node.operation.name != "gimp:colorize"
        || node.operation.stored_version != "0:0"
        || node.operation.schema_epoch != b.expected_epoch
        || !node.auxiliary.is_empty()
        || b.context.encoding != SampleEncoding::NonlinearProfile
        || b.context.profile_identity == [0; 32]
        || b.transfer.profile_identity != b.context.profile_identity
        || format.mode != ColorMode::Rgb
        || format.sample != SampleType::F32
    {
        return Err(bad("exact retained Colorize/scalar authority/domain/transfer witness"));
    }
    YWeights::from_bits(b.weights.bits()).map_err(lower)?;
    stored_parameters(&node.arguments)
}
/// Allocation-free fixed four-argument census before any constructor/copy.
pub fn requirements(node: &FilterNode, b: SourceBinding<'_>, format: PixelFormat, l: Limits, c: &Interrupt<'_>) -> Result<Requirements> {
    poll(c)?;
    let m = Limits::default();
    if l.resident_bytes > m.resident_bytes || l.work_units > m.work_units {
        return Err(limit());
    }
    bind(node, b, format, c)?;
    let q = math::PreparedColorize::preparation_requirements();
    let mut selected = (node.arguments.capacity() as u64).checked_mul(std::mem::size_of::<FilterArgument>() as u64).ok_or_else(limit)?;
    for a in &node.arguments {
        poll(c)?;
        selected = add(selected, a.name.capacity() as u64)?;
        if let FilterValue::StoredColor(storage) = &a.value {
            selected = add(selected, add(storage.layout.capacity() as u64, add(storage.samples.capacity() as u64, storage.opaque_tail.capacity() as u64)?)?)?;
        }
    }
    let retained = add(add(HEADER, q.retained_bytes)?, selected)?;
    let work = add(BIND_WORK, q.work_units)?;
    if retained > l.resident_bytes || work > l.work_units {
        return Err(limit());
    }
    Ok(Requirements {
        retained_bytes: retained,
        peak_resident_bytes: retained,
        index_bytes: 0,
        preparation_work_units: work,
        sample_work_units: math::SAMPLE_WORK,
        remaining_work_units: l.work_units - work,
    })
}
pub fn prepare<'a, B: PreparationBudget>(
    node: &'a FilterNode,
    b: SourceBinding<'a>,
    format: PixelFormat,
    l: Limits,
    budget: &mut B,
    c: &Interrupt<'_>,
) -> Result<Prepared<'a, B::Lease>> {
    poll(c)?;
    if std::mem::size_of::<Prepared<'a, B::Lease>>() as u64 > HEADER {
        return Err(limit());
    }
    let q = requirements(node, b, format, l, c)?;
    budget.consume_preparation_work(q.preparation_work_units)?;
    let lease = budget.reserve_prepared_bytes(q.peak_resident_bytes)?;
    let parameters = bind(node, b, format, c)?;
    let k = math::PreparedColorize::preparation_requirements();
    let kernel = math::PreparedColorize::prepare(
        format,
        b.context,
        parameters,
        b.policy,
        b.transfer,
        b.weights,
        math::Limits { resident_bytes: k.retained_bytes, work_units: k.work_units },
        c,
    )
    .map_err(lower)?;
    poll(c)?;
    Ok(Prepared { node, binding: b, kernel, requirements: q, limits: l, work: AtomicU64::new(q.preparation_work_units), _lease: lease })
}
impl<'a, L> Prepared<'a, L> {
    pub fn original_node(&self) -> &'a FilterNode {
        self.node
    }
    pub fn context(&self) -> WorkingContext {
        self.kernel.context()
    }
    pub fn policy(&self) -> NumericPolicy {
        self.kernel.policy()
    }
    pub fn format(&self) -> PixelFormat {
        self.kernel.format()
    }
    pub fn transfer(&self) -> TransferWitness {
        self.kernel.transfer_witness()
    }
    pub fn weights(&self) -> YWeights {
        self.kernel.weights()
    }
    pub fn parameters(&self) -> Parameters {
        self.kernel.parameters()
    }
    pub fn binding(&self) -> SourceBinding<'a> {
        self.binding
    }
    pub fn requirements(&self) -> Requirements {
        let mut q = self.requirements;
        q.remaining_work_units = self.limits.work_units.saturating_sub(self.work.load(Ordering::Relaxed));
        q
    }
    /// Both ROI preflight and render consume this immutable cumulative tuple quota.
    pub fn evaluate_rgb_with_interrupt(&self, input: [f32; 3], actual: WorkingContext, transfer: TransferWitness, c: &Interrupt<'_>) -> Result<[f32; 3]> {
        poll(c)?;
        if actual != self.context() || transfer != self.transfer() {
            return Err(bad("Colorize transfer/context changed"));
        }
        self.work
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(self.kernel.sample_work()).filter(|n| *n <= self.limits.work_units))
            .map_err(|_| limit())?;
        let out = self.kernel.try_eval_float_color(input).map_err(lower)?;
        poll(c)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{
        item_identity::FilterId,
        xcf_filters::{ExactColorStorage, OperationIdentity},
    };
    #[derive(Default)]
    struct Budget {
        bytes: u64,
        work: u64,
    }
    impl PreparationBudget for Budget {
        type Lease = ();
        fn reserve_prepared_bytes(&mut self, n: u64) -> Result<()> {
            self.bytes = n;
            Ok(())
        }
        fn consume_preparation_work(&mut self, n: u64) -> Result<()> {
            self.work = n;
            Ok(())
        }
    }
    fn node() -> FilterNode {
        let samples = [0.125f32.to_bits(), 0.25f32.to_bits(), 0.75f32.to_bits()].into_iter().flat_map(u32::to_le_bytes).collect();
        FilterNode {
            id: FilterId(1),
            operation: OperationIdentity { name: "gimp:colorize".into(), stored_version: "0:0".into(), schema_epoch: EPOCH.into() },
            source_display_name: None,
            source_icon: None,
            arguments: vec![
                FilterArgument { name: "hue".into(), value: FilterValue::Number(ExactNumber::Float32Bits((-0f32).to_bits())) },
                FilterArgument {
                    name: "color".into(),
                    value: FilterValue::StoredColor(ExactColorStorage { layout: "HSL float".into(), samples, opaque_tail: vec![] }),
                },
                FilterArgument { name: "saturation".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0.5f32.to_bits())) },
                FilterArgument { name: "lightness".into(), value: FilterValue::Number(ExactNumber::Float32Bits(0f32.to_bits())) },
            ],
            enabled: false,
            opacity_f64_bits: 0.5f64.to_bits(),
            source_clipping: Some(0),
            source_region: Some(0),
            mask: None,
            auxiliary: vec![],
            composition: None,
        }
    }
    fn binding(role: TransferRole) -> SourceBinding<'static> {
        SourceBinding {
            xcf_version: 22,
            expected_epoch: EPOCH,
            schema_sha256: SCHEMA,
            context: WorkingContext { encoding: SampleEncoding::NonlinearProfile, profile_identity: [8; 32] },
            policy: NumericPolicy::DeclaredScalarTripletBinary64FinalFloat,
            transfer: TransferWitness { role, profile_identity: [8; 32] },
            weights: YWeights::from_bits([0.25f64.to_bits(), 0.5f64.to_bits(), 0.25f64.to_bits()]).unwrap(),
        }
    }
    #[test]
    fn independent_scalar_hsl_words_and_outer_metadata_survive_both_declared_roles() {
        for role in [TransferRole::CallerEstablishedIecSrgb, TransferRole::CallerEstablishedExactGamma2] {
            let n = node();
            let before = n.clone();
            let b = binding(role);
            let mut budget = Budget::default();
            let prepared = prepare(&n, b, PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).unwrap();
            assert_eq!(prepared.parameters().scalar_bits(), [(-0f32).to_bits(), 0.5f32.to_bits(), 0]);
            assert_eq!(prepared.parameters().stored_hsl_bits(), [0.125f32.to_bits(), 0.25f32.to_bits(), 0.75f32.to_bits()]);
            let left = prepared.requirements().remaining_work_units;
            let out = prepared.evaluate_rgb_with_interrupt([0.25, 0.5, 0.75], b.context, b.transfer, &Interrupt::NONE).unwrap();
            assert!(out.iter().all(|v| v.is_finite()));
            assert_eq!(prepared.requirements().remaining_work_units, left - 1024);
            assert_eq!(n, before);
            assert!(budget.bytes > 0 && budget.work == 640);
        }
    }
    #[test]
    fn bad_transfer_or_raw_storage_refuses_before_budget_and_capacity_is_actual() {
        let mut n = node();
        let b = binding(TransferRole::CallerEstablishedIecSrgb);
        let mut budget = Budget::default();
        let wrong = SourceBinding { transfer: TransferWitness { profile_identity: [7; 32], ..b.transfer }, ..b };
        assert!(prepare(&n, wrong, PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.bytes, 0);
        let a = requirements(&n, b, PixelFormat::RGBA32F, Limits::default(), &Interrupt::NONE).unwrap();
        if let FilterValue::StoredColor(storage) = &mut n.arguments[1].value {
            storage.opaque_tail.reserve_exact(8192);
        }
        let q = requirements(&n, b, PixelFormat::RGBA32F, Limits::default(), &Interrupt::NONE).unwrap();
        assert!(q.retained_bytes >= a.retained_bytes + 8192);
        if let FilterValue::StoredColor(storage) = &mut n.arguments[1].value {
            storage.opaque_tail.push(0);
        }
        assert!(prepare(&n, b, PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::NONE).is_err());
        assert_eq!(budget.work, 0);
    }
    #[test]
    fn generic_inline_lease_and_cancellation_are_refused_before_constructor() {
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
        let n = node();
        let b = binding(TransferRole::CallerEstablishedIecSrgb);
        assert!(prepare(&n, b, PixelFormat::RGBA32F, Limits::default(), &mut Large, &Interrupt::NONE).is_err());
        let stop = || true;
        let mut budget = Budget::default();
        assert!(matches!(prepare(&n, b, PixelFormat::RGBA32F, Limits::default(), &mut budget, &Interrupt::cancel_only(&stop)), Err(E::Raster(R::Cancelled))));
        assert_eq!(budget.bytes, 0);
    }
}
